//! Tone and colour: making a photograph look right.
//!
//! The geometry tools already crop, rotate and resize; this is the other half.
//! It exists for the picture everyone actually has — a document shot indoors
//! that came out dim and yellow, a passport photo too dark to pass, a scan
//! whose whites are grey.
//!
//! # Order matters, so it is fixed and written down
//!
//! Adjustments are not commutative: sharpening before a contrast boost
//! sharpens noise and then amplifies it, and correcting the colour cast after
//! stretching the levels fights the stretch. So the order is always:
//!
//! 1. Auto levels — stretch each channel to use the full range
//! 2. Auto colour — remove the cast from the light it was shot under
//! 3. Exposure — overall lightness, in stops
//! 4. Contrast
//! 5. Saturation
//! 6. Greyscale, if asked
//! 7. Sharpen, last, on a settled picture
//!
//! Everything in between runs in floating point. Doing it in bytes means
//! rounding seven times, and the banding that produces shows up in a sky or a
//! sheet of paper long before anyone can name why.
//!
//! # What it cannot do
//!
//! Lifting a very dark photo lifts its noise with it, and a JPEG's blocking
//! comes up too — there is no detail hiding in there to recover, only what the
//! sensor recorded. The report says when it has pushed hard enough for that to
//! show.

use image::{DynamicImage, RgbImage};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustOptions {
    /// Stretch each channel so the darkest pixels are black and the
    /// brightest white.
    pub auto_levels: bool,
    /// Neutralise the colour of the light it was taken under, read from
    /// the brightest part of the picture.
    pub auto_colour: bool,
    /// Stops of exposure, -2.0 to 2.0. Each whole step doubles or halves.
    pub exposure: f32,
    /// -1.0 flat to 1.0 hard.
    pub contrast: f32,
    /// -1.0 is grey, 0.0 unchanged, 1.0 twice as colourful.
    pub saturation: f32,
    /// -1.0 cold to 1.0 warm.
    pub warmth: f32,
    /// 0.0 to 1.0.
    pub sharpen: f32,
    pub greyscale: bool,
}

impl Default for AdjustOptions {
    fn default() -> Self {
        AdjustOptions {
            auto_levels: false,
            auto_colour: false,
            exposure: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            warmth: 0.0,
            sharpen: 0.0,
            greyscale: false,
        }
    }
}

impl AdjustOptions {
    /// Whether this would change anything at all.
    pub fn changes_anything(&self) -> bool {
        self.auto_levels
            || self.auto_colour
            || self.greyscale
            || self.exposure != 0.0
            || self.contrast != 0.0
            || self.saturation != 0.0
            || self.warmth != 0.0
            || self.sharpen != 0.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustReport {
    pub width: u32,
    pub height: u32,
    /// What auto levels did, per channel: the input black and white points it
    /// found, 0–255. Empty when it wasn't used.
    pub levels: Vec<(u8, u8)>,
    /// What auto colour did: the multiplier applied to each channel.
    pub colour_gain: Option<[f32; 3]>,
    pub warnings: Vec<String>,
}

/// Fraction of pixels allowed to clip at each end by auto levels.
///
/// Not zero: one stuck bright pixel or one black speck would otherwise define
/// the whole range and the stretch would do nothing.
const CLIP: f32 = 0.005;

/// Fraction of the picture treated as highlight when reading the light's
/// colour.
const HIGHLIGHT: f32 = 0.05;

/// Apply the adjustments.
pub fn adjust(bytes: &[u8], options: &AdjustOptions) -> Result<(DynamicImage, AdjustReport)> {
    for (name, value, limit) in [
        ("exposure", options.exposure, 2.0),
        ("contrast", options.contrast, 1.0),
        ("saturation", options.saturation, 1.0),
        ("warmth", options.warmth, 1.0),
    ] {
        if !(-limit..=limit).contains(&value) {
            return Err(Error::invalid(format!("{name} has to be between -{limit} and {limit}")));
        }
    }
    if !(0.0..=1.0).contains(&options.sharpen) {
        return Err(Error::invalid("sharpening has to be between 0 and 1"));
    }

    let source = image::load_from_memory(bytes)?;
    Ok(apply(&source, options))
}

/// The same, on an image already decoded.
pub fn apply(source: &DynamicImage, options: &AdjustOptions) -> (DynamicImage, AdjustReport) {
    let rgb = source.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let mut warnings = Vec::new();

    let levels = if options.auto_levels { channel_levels(&rgb) } else { Vec::new() };

    // Levels are applied first and in full, because the colour reading has to
    // be taken from the picture as the colour step will actually see it.
    //
    // Stretching each channel to its own range is itself a colour correction —
    // that is most of what it does to a cast. Reading the light from the
    // *original* and then applying that on top corrects the same cast twice,
    // and the result was a document photograph turned conspicuously blue.
    let levelled = if levels.is_empty() {
        None
    } else {
        let mut out = RgbImage::new(w, h);
        for (x, y, pixel) in rgb.enumerate_pixels() {
            let mut v = [0f32; 3];
            for c in 0..3 {
                let (lo, hi) = levels[c];
                let (lo, hi) = (lo as f32 / 255.0, hi as f32 / 255.0);
                let value = pixel.0[c] as f32 / 255.0;
                v[c] = if hi - lo > 0.004 {
                    ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
                } else {
                    value
                };
            }
            out.put_pixel(x, y, image::Rgb([to_byte(v[0]), to_byte(v[1]), to_byte(v[2])]));
        }
        Some(out)
    };
    let working = levelled.as_ref().unwrap_or(&rgb);
    let gain = options.auto_colour.then(|| white_patch(working));

    let mut out = RgbImage::new(w, h);
    for (x, y, pixel) in working.enumerate_pixels() {
        let mut v =
            [pixel.0[0] as f32 / 255.0, pixel.0[1] as f32 / 255.0, pixel.0[2] as f32 / 255.0];

        // Levels are already in `working`; the colour correction is read from
        // that and applied here.
        if let Some(gain) = gain {
            for (c, slot) in v.iter_mut().enumerate() {
                *slot = (*slot * gain[c]).clamp(0.0, 1.0);
            }
        }

        // 3. Exposure, as stops.
        if options.exposure != 0.0 {
            let factor = 2f32.powf(options.exposure);
            for slot in v.iter_mut() {
                *slot *= factor;
            }
        }

        // 4. Contrast, pivoting on mid-grey.
        if options.contrast != 0.0 {
            let k = if options.contrast >= 0.0 {
                1.0 + options.contrast * 2.0
            } else {
                1.0 + options.contrast
            };
            for slot in v.iter_mut() {
                *slot = (*slot - 0.5) * k + 0.5;
            }
        }

        // 5. Warmth: red and blue pulled in opposite directions, which is
        // what moving the light's temperature actually does.
        if options.warmth != 0.0 {
            v[0] *= 1.0 + options.warmth * 0.18;
            v[2] *= 1.0 - options.warmth * 0.18;
        }

        // 6. Saturation, as a move away from or towards the pixel's own
        // brightness.
        if options.saturation != 0.0 {
            let luma = luma(&v);
            let amount = 1.0 + options.saturation;
            for slot in v.iter_mut() {
                *slot = luma + (*slot - luma) * amount;
            }
        }

        if options.greyscale {
            let luma = luma(&v);
            v = [luma, luma, luma];
        }

        out.put_pixel(x, y, image::Rgb([to_byte(v[0]), to_byte(v[1]), to_byte(v[2])]));
    }

    // 7. Sharpening last, so it works on the picture as it will be seen
    // rather than on tones that are about to move.
    let finished = if options.sharpen > 0.0 {
        let radius = (w.max(h) as f32 / 900.0).clamp(0.6, 2.2);
        DynamicImage::ImageRgb8(image::imageops::unsharpen(
            &out,
            radius,
            (options.sharpen * 26.0).round() as i32 + 2,
        ))
    } else {
        DynamicImage::ImageRgb8(out)
    };

    // Pushing a dark picture up cannot invent detail, only magnify what the
    // sensor managed, and people are routinely surprised by that.
    if options.exposure > 0.9 || (options.auto_levels && stretched_hard(&levels)) {
        warnings.push(
            "This has been lifted a long way. Grain and JPEG blocking come up with it — there is \
             no hidden detail to recover, only what the camera recorded."
                .into(),
        );
    }
    if options.contrast > 0.75 && options.saturation > 0.5 {
        warnings.push(
            "Hard contrast and strong colour together clip easily: the brightest and darkest \
             parts lose their detail altogether."
                .into(),
        );
    }

    let report = AdjustReport { width: w, height: h, levels, colour_gain: gain, warnings };
    (finished, report)
}

fn luma(v: &[f32; 3]) -> f32 {
    // Rec. 601 weights: green carries most of what the eye reads as
    // brightness, and an even average turns a red shirt into mud.
    0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2]
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The black and white point of each channel, ignoring the extreme tails.
fn channel_levels(rgb: &RgbImage) -> Vec<(u8, u8)> {
    let total = (rgb.width() as u64 * rgb.height() as u64).max(1);
    let cut = ((total as f32 * CLIP) as u64).max(1);

    let mut histograms = [[0u64; 256]; 3];
    for pixel in rgb.pixels() {
        for c in 0..3 {
            histograms[c][pixel.0[c] as usize] += 1;
        }
    }

    (0..3)
        .map(|c| {
            let hist = &histograms[c];
            let mut seen = 0u64;
            let mut lo = 0u8;
            for (value, count) in hist.iter().enumerate() {
                seen += count;
                if seen > cut {
                    lo = value as u8;
                    break;
                }
            }
            let mut seen = 0u64;
            let mut hi = 255u8;
            for (value, count) in hist.iter().enumerate().rev() {
                seen += count;
                if seen > cut {
                    hi = value as u8;
                    break;
                }
            }
            if hi <= lo {
                (0, 255)
            } else {
                (lo, hi)
            }
        })
        .collect()
}

/// Multipliers that neutralise the colour of the light, judged from the
/// brightest part of the picture.
///
/// The obvious method is grey-world — average the whole picture, assume it
/// ought to be neutral, correct the difference. It is also wrong here. A
/// document photographed on a wooden desk genuinely *is* mostly brown, so
/// grey-world reads the desk as a colour cast and swings the whole picture
/// blue trying to cancel it. That was not a hypothetical: it is what the first
/// version did to a test photograph, and it looked plainly wrong.
///
/// The brightest pixels are a far better clue, and especially so for this
/// app's usual subject: the paper is white, so whatever colour the paper came
/// out is the colour of the light. Where there is no bright region to read —
/// a dark photograph with no highlight at all — nothing is changed, which is
/// better than guessing.
fn white_patch(rgb: &RgbImage) -> [f32; 3] {
    let total = (rgb.width() as u64 * rgb.height() as u64).max(1);

    // The brightness above which a pixel counts as highlight: the top slice
    // of the picture, found from its own histogram.
    let mut histogram = [0u64; 256];
    for pixel in rgb.pixels() {
        let v = luma(&[
            pixel.0[0] as f32 / 255.0,
            pixel.0[1] as f32 / 255.0,
            pixel.0[2] as f32 / 255.0,
        ]);
        histogram[(v * 255.0).round().clamp(0.0, 255.0) as usize] += 1;
    }
    let wanted = ((total as f32 * HIGHLIGHT) as u64).max(1);
    let mut seen = 0u64;
    let mut threshold = 255u8;
    for (value, count) in histogram.iter().enumerate().rev() {
        seen += count;
        if seen >= wanted {
            threshold = value as u8;
            break;
        }
    }

    let mut totals = [0f64; 3];
    let mut counted = 0u64;
    for pixel in rgb.pixels() {
        let v = luma(&[
            pixel.0[0] as f32 / 255.0,
            pixel.0[1] as f32 / 255.0,
            pixel.0[2] as f32 / 255.0,
        ]);
        if (v * 255.0) as u8 >= threshold {
            for (total, channel) in totals.iter_mut().zip(pixel.0) {
                *total += channel as f64;
            }
            counted += 1;
        }
    }
    if counted == 0 {
        return [1.0, 1.0, 1.0];
    }

    let means =
        [totals[0] / counted as f64, totals[1] / counted as f64, totals[2] / counted as f64];
    // Too dark to be a highlight at all — nothing reliable to read, so leave
    // the colour alone rather than invent a correction.
    if means.iter().all(|m| *m < 40.0) {
        return [1.0, 1.0, 1.0];
    }
    let neutral = (means[0] + means[1] + means[2]) / 3.0;

    let mut gain = [1.0f32; 3];
    for c in 0..3 {
        if means[c] > 1.0 {
            // Tighter bounds than grey-world needs, because this estimate is
            // the more trustworthy one and a big correction here would mean
            // the reading was wrong rather than the light extreme.
            gain[c] = ((neutral / means[c]) as f32).clamp(0.75, 1.35);
        }
    }
    gain
}

/// Whether auto levels had to pull hard, which means the original was flat.
fn stretched_hard(levels: &[(u8, u8)]) -> bool {
    levels.iter().any(|(lo, hi)| (*hi as i32 - *lo as i32) < 120)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(colour: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 30, image::Rgb(colour)))
    }

    /// A dim picture with a yellow cast and a patch of what ought to be white
    /// paper — the shape of a document photographed indoors.
    fn dim_and_yellow() -> DynamicImage {
        let mut rgb = RgbImage::from_pixel(100, 100, image::Rgb([70, 60, 38]));
        for y in 20..70 {
            for x in 20..80 {
                // The "paper": bright, and carrying the same cast.
                rgb.put_pixel(x, y, image::Rgb([150, 134, 96]));
            }
        }
        DynamicImage::ImageRgb8(rgb)
    }

    fn mean(image: &DynamicImage) -> [f32; 3] {
        let rgb = image.to_rgb8();
        let n = rgb.pixels().len() as f32;
        let mut total = [0f32; 3];
        for p in rgb.pixels() {
            for (sum, channel) in total.iter_mut().zip(p.0) {
                *sum += channel as f32;
            }
        }
        [total[0] / n, total[1] / n, total[2] / n]
    }

    #[test]
    fn levels_stretch_a_flat_picture_onto_the_full_range() {
        let (out, report) =
            apply(&dim_and_yellow(), &AdjustOptions { auto_levels: true, ..Default::default() });
        assert_eq!(report.levels.len(), 3);
        for (lo, hi) in &report.levels {
            assert!(hi > lo, "a channel needs a range: {lo}..{hi}");
        }
        // Something has to reach the top after stretching.
        let rgb = out.to_rgb8();
        let brightest = rgb.pixels().map(|p| p.0.iter().copied().max().unwrap_or(0)).max().unwrap();
        assert!(brightest > 240, "the brightest pixel should reach white, got {brightest}");
    }

    #[test]
    fn the_colour_of_the_light_is_read_from_the_brightest_part() {
        let (out, report) =
            apply(&dim_and_yellow(), &AdjustOptions { auto_colour: true, ..Default::default() });
        let gain = report.colour_gain.expect("a gain");
        assert!(gain[2] > gain[0], "blue was crushed, so it should be lifted most: {gain:?}");

        // And the cast should be measurably smaller afterwards.
        let before = mean(&dim_and_yellow());
        let after = mean(&out);
        let spread = |m: [f32; 3]| {
            m.iter().cloned().fold(f32::MIN, f32::max) - m.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(spread(after) < spread(before), "{before:?} -> {after:?}");
    }

    #[test]
    fn levels_and_colour_together_do_not_correct_the_same_cast_twice() {
        // The bug this guards: the colour reading was taken from the original
        // while levels had already neutralised most of the cast, so the
        // correction landed on top of itself and swung a document photograph
        // conspicuously blue. Levels first, then read, then correct.
        let (_, report) = apply(
            &dim_and_yellow(),
            &AdjustOptions { auto_levels: true, auto_colour: true, ..Default::default() },
        );
        let gain = report.colour_gain.expect("a gain");
        for (c, g) in gain.iter().enumerate() {
            assert!(
                (*g - 1.0).abs() < 0.2,
                "channel {c} wants {g}, which means levels left it something big to fix"
            );
        }
    }

    #[test]
    fn a_picture_with_no_highlight_is_left_alone_rather_than_guessed_at() {
        let (_, report) =
            apply(&flat([12, 10, 9]), &AdjustOptions { auto_colour: true, ..Default::default() });
        assert_eq!(report.colour_gain, Some([1.0, 1.0, 1.0]));
    }

    #[test]
    fn exposure_is_in_stops() {
        let grey = flat([60, 60, 60]);
        let (up, _) = apply(&grey, &AdjustOptions { exposure: 1.0, ..Default::default() });
        let (down, _) = apply(&grey, &AdjustOptions { exposure: -1.0, ..Default::default() });
        assert!((mean(&up)[0] - 120.0).abs() < 2.0, "one stop up doubles: {:?}", mean(&up));
        assert!((mean(&down)[0] - 30.0).abs() < 2.0, "one stop down halves: {:?}", mean(&down));
    }

    #[test]
    fn contrast_pivots_on_mid_grey() {
        // Mid-grey is the hinge: it must not move whichever way contrast goes.
        let mid = flat([128, 128, 128]);
        for contrast in [-0.8, -0.3, 0.4, 1.0] {
            let (out, _) = apply(&mid, &AdjustOptions { contrast, ..Default::default() });
            assert!(
                (mean(&out)[0] - 128.0).abs() <= 1.0,
                "contrast {contrast} moved mid-grey to {:?}",
                mean(&out)
            );
        }
        // And it pushes the ends apart.
        let dark = flat([80, 80, 80]);
        let (harder, _) = apply(&dark, &AdjustOptions { contrast: 0.5, ..Default::default() });
        assert!(mean(&harder)[0] < 80.0, "a dark tone goes darker");
    }

    #[test]
    fn saturation_at_its_lowest_is_grey() {
        let (out, _) =
            apply(&flat([200, 60, 40]), &AdjustOptions { saturation: -1.0, ..Default::default() });
        let p = out.to_rgb8().get_pixel(0, 0).0;
        assert!(p[0] == p[1] && p[1] == p[2], "should be neutral, got {p:?}");
    }

    #[test]
    fn greyscale_weights_the_channels_the_way_an_eye_does() {
        // A flat average would make pure green and pure blue the same shade,
        // and they are nothing alike to look at.
        let green =
            apply(&flat([0, 255, 0]), &AdjustOptions { greyscale: true, ..Default::default() })
                .0
                .to_rgb8()
                .get_pixel(0, 0)
                .0[0];
        let blue =
            apply(&flat([0, 0, 255]), &AdjustOptions { greyscale: true, ..Default::default() })
                .0
                .to_rgb8()
                .get_pixel(0, 0)
                .0[0];
        assert!(green > blue + 80, "green reads far brighter than blue: {green} vs {blue}");
    }

    #[test]
    fn warmth_moves_red_and_blue_in_opposite_directions() {
        let grey = flat([128, 128, 128]);
        let (warm, _) = apply(&grey, &AdjustOptions { warmth: 1.0, ..Default::default() });
        let p = warm.to_rgb8().get_pixel(0, 0).0;
        assert!(p[0] > 128 && p[2] < 128, "warmer means more red, less blue: {p:?}");

        let (cold, _) = apply(&grey, &AdjustOptions { warmth: -1.0, ..Default::default() });
        let p = cold.to_rgb8().get_pixel(0, 0).0;
        assert!(p[0] < 128 && p[2] > 128, "colder is the other way: {p:?}");
    }

    #[test]
    fn doing_nothing_changes_nothing() {
        let source = dim_and_yellow();
        let options = AdjustOptions::default();
        assert!(!options.changes_anything());
        let (out, report) = apply(&source, &options);
        assert_eq!(out.to_rgb8().as_raw(), source.to_rgb8().as_raw());
        assert!(report.levels.is_empty());
        assert!(report.colour_gain.is_none());
    }

    #[test]
    fn lifting_a_dark_picture_a_long_way_says_what_that_costs() {
        let (_, report) =
            apply(&flat([40, 40, 40]), &AdjustOptions { exposure: 1.5, ..Default::default() });
        assert!(
            report.warnings.iter().any(|w| w.contains("no hidden detail")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn nonsense_settings_are_refused() {
        let mut encoded = Vec::new();
        flat([100, 100, 100])
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .unwrap();

        for options in [
            AdjustOptions { exposure: 9.0, ..Default::default() },
            AdjustOptions { contrast: -4.0, ..Default::default() },
            AdjustOptions { saturation: 3.0, ..Default::default() },
            AdjustOptions { warmth: -2.5, ..Default::default() },
            AdjustOptions { sharpen: 5.0, ..Default::default() },
        ] {
            assert!(adjust(&encoded, &options).is_err(), "{options:?} should be refused");
        }
    }

    #[test]
    fn the_options_cross_as_the_ui_writes_them() {
        let options: AdjustOptions = serde_json::from_str(
            r#"{ "autoLevels": true, "autoColour": true, "exposure": 0.5, "contrast": 0.2,
                  "saturation": -0.1, "warmth": 0.3, "sharpen": 0.4, "greyscale": false }"#,
        )
        .expect("options");
        assert!(options.auto_levels);
        assert!(options.auto_colour);
        assert!((options.warmth - 0.3).abs() < 1e-6);
        assert!(options.changes_anything());
    }
}
