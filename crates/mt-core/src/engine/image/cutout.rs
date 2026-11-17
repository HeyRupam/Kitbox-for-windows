//! Taking the background out of a photograph.
//!
//! The common case this exists for is a form that wants a passport photo on a
//! plain white or blue background, and what you have is a photo taken against
//! a wall. Cutting it out by hand around hair is miserable; a saliency model
//! does it in a fifth of a second.
//!
//! The model is [U²-Net](https://github.com/xuebinqin/U-2-Net) (Apache 2.0),
//! run through `rten` — the same pure-Rust runtime the OCR tool already uses,
//! so this adds no new inference dependency and `mt-core` keeps its rule about
//! having no platform dependencies. Model bytes arrive from the caller; this
//! module never opens a file or reaches the network.
//!
//! # It is a guess, and the edge is where it shows
//!
//! The model returns a confidence per pixel, not a decision. Hard-thresholding
//! that into keep-or-drop gives the jagged, obviously-cut-out look; the
//! confidence is instead mapped through a ramp so a pixel the model is unsure
//! about comes out semi-transparent, which is what hair actually looks like.
//!
//! The honest failure is a photo with no clear subject — a landscape, a flat
//! wall, a document filling the frame. The model will still return something,
//! and it will be wrong. So the report says how much of the picture was kept,
//! and calls out the two shapes that mean it went wrong: almost nothing kept,
//! or almost everything.

use image::{DynamicImage, GrayImage, Rgba, RgbaImage};
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The loaded segmentation model.
///
/// Loading takes a moment, so it is done once and reused across pictures.
pub struct Model {
    inner: rten::Model,
}

impl Model {
    pub fn load(bytes: Vec<u8>) -> Result<Model> {
        let inner = rten::Model::load(bytes).map_err(|e| {
            Error::invalid(format!("the background-removal model wouldn't load: {e}"))
        })?;
        Ok(Model { inner })
    }
}

/// What goes behind the subject once the background is gone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Backdrop {
    /// Nothing — a PNG with real transparency.
    Transparent,
    /// A flat colour, which is what forms ask for.
    Colour([u8; 3]),
}

impl Default for Backdrop {
    fn default() -> Self {
        Backdrop::Transparent
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutoutOptions {
    pub backdrop: Backdrop,
    /// How much of the model's uncertainty to keep, 0.0–1.0.
    ///
    /// Higher keeps more of the doubtful edge — kinder to hair, more likely to
    /// bring a halo of background with it.
    pub softness: f32,
    /// Shifts the keep-or-drop point. Below 0.5 keeps more, above keeps less.
    pub threshold: f32,
    /// Blur the cut edge by this many pixels, so it doesn't look stamped out.
    pub feather: u32,
}

impl Default for CutoutOptions {
    fn default() -> Self {
        CutoutOptions {
            backdrop: Backdrop::Transparent,
            softness: 0.35,
            threshold: 0.5,
            feather: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutoutReport {
    pub width: u32,
    pub height: u32,
    /// Fraction of the picture kept as subject, 0.0–1.0.
    pub coverage: f32,
    /// Always present: what this is and where it goes wrong.
    pub note: String,
    pub warnings: Vec<String>,
}

pub const A_GUESS: &str =
    "The model decides what the subject is; it is not told. It is good at a person or an object \
     against a background and poor at a picture with no clear subject. Check the edges — hair, \
     glasses and thin straps are where it shows.";

/// The size U²-Net was trained at. Anything else and the mask degrades.
const SIDE: usize = 320;
/// Standardisation the model expects, after scaling to its own peak.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Cut the subject out of `bytes`.
pub fn remove_background(
    bytes: &[u8],
    model: &Model,
    options: &CutoutOptions,
) -> Result<(DynamicImage, CutoutReport)> {
    if !(0.0..=1.0).contains(&options.softness) {
        return Err(Error::invalid("softness has to be between 0 and 1"));
    }
    if !(0.05..=0.95).contains(&options.threshold) {
        return Err(Error::invalid("the threshold has to be between 0.05 and 0.95"));
    }
    let source = image::load_from_memory(bytes)?;
    if source.width() < 16 || source.height() < 16 {
        return Err(Error::invalid("that image is too small to find anything in"));
    }
    cut(&source, model, options)
}

/// The same, on an image already decoded.
pub fn cut(
    source: &DynamicImage,
    model: &Model,
    options: &CutoutOptions,
) -> Result<(DynamicImage, CutoutReport)> {
    let mask = saliency(source, model)?;
    let mask = image::imageops::resize(
        &mask,
        source.width(),
        source.height(),
        image::imageops::FilterType::Triangle,
    );
    // Feather after scaling up, so the blur is in the picture's own pixels
    // rather than the model's 320.
    let mask = if options.feather > 0 {
        super::scan::box_blur(&mask, options.feather)
    } else {
        mask
    };

    Ok(composite(source, &mask, options))
}

/// Put the subject over the chosen backdrop, given a confidence map the size
/// of the picture.
///
/// Split from the model so the part that decides what a pixel ends up looking
/// like can be tested on a mask built by hand — the model is a 4.6 MB download
/// and a test suite that quietly needs one fails for the wrong reasons.
fn composite(
    source: &DynamicImage,
    mask: &GrayImage,
    options: &CutoutOptions,
) -> (DynamicImage, CutoutReport) {
    let rgb = source.to_rgb8();
    let (w, h) = (source.width(), source.height());
    let mut out = RgbaImage::new(w, h);
    let mut kept = 0u64;

    // The ramp: fully dropped below `low`, fully kept above `high`, and
    // graded between. A single cutoff is what makes a cutout look cut out.
    let span = (options.softness * 0.5).max(0.002);
    let low = (options.threshold - span).clamp(0.0, 1.0);
    let high = (options.threshold + span).clamp(low + 0.001, 1.0);

    for y in 0..h {
        for x in 0..w {
            let confidence = mask.get_pixel(x, y)[0] as f32 / 255.0;
            let alpha = ((confidence - low) / (high - low)).clamp(0.0, 1.0);
            if alpha > 0.5 {
                kept += 1;
            }
            let px = rgb.get_pixel(x, y).0;
            let pixel = match options.backdrop {
                Backdrop::Transparent => {
                    Rgba([px[0], px[1], px[2], (alpha * 255.0).round() as u8])
                }
                Backdrop::Colour(back) => {
                    // Mixed here rather than left to whoever saves it: a JPEG
                    // has no alpha, and flattening onto white by default would
                    // quietly ignore the colour that was asked for.
                    let mix = |fore: u8, back: u8| {
                        (fore as f32 * alpha + back as f32 * (1.0 - alpha)).round() as u8
                    };
                    Rgba([
                        mix(px[0], back[0]),
                        mix(px[1], back[1]),
                        mix(px[2], back[2]),
                        255,
                    ])
                }
            };
            out.put_pixel(x, y, pixel);
        }
    }

    let coverage = kept as f32 / (w as f32 * h as f32);
    let mut warnings = Vec::new();
    if coverage < 0.02 {
        warnings.push(
            "Almost nothing was kept. The model didn't find a subject — that happens with a \
             landscape, a flat surface, or a picture that is all one thing."
                .into(),
        );
    } else if coverage > 0.97 {
        warnings.push(
            "Almost everything was kept, so there is effectively no background to remove. That \
             usually means the subject fills the frame."
                .into(),
        );
    }

    let report = CutoutReport {
        width: w,
        height: h,
        coverage,
        note: A_GUESS.to_string(),
        warnings,
    };
    (DynamicImage::ImageRgba8(out), report)
}

/// Run the model and return its confidence map at its own resolution.
fn saliency(source: &DynamicImage, model: &Model) -> Result<GrayImage> {
    let small =
        source.resize_exact(SIDE as u32, SIDE as u32, image::imageops::FilterType::Triangle);
    let rgb = small.to_rgb8();

    // Scaled to the picture's own brightest channel before standardising,
    // which is how these weights were trained — skip it and a dim photo reads
    // as a dim subject.
    let peak = rgb
        .pixels()
        .map(|p| p.0.iter().copied().max().unwrap_or(0))
        .max()
        .unwrap_or(255) as f32
        / 255.0;
    let peak = peak.max(1e-6);

    let mut input = NdTensor::zeros([1, 3, SIDE, SIDE]);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let px = rgb.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                let value = (px[c] as f32 / 255.0) / peak;
                input[[0, c, y, x]] = (value - MEAN[c]) / STD[c];
            }
        }
    }

    let output = model
        .inner
        .run_one(input.view().into(), None)
        .map_err(|e| Error::other(format!("the model couldn't be run: {e}")))?;
    let map: NdTensor<f32, 4> = output
        .try_into()
        .map_err(|_| Error::other("the model returned something unexpected"))?;
    if map.shape()[2] != SIDE || map.shape()[3] != SIDE {
        return Err(Error::other("the model returned a map of the wrong size"));
    }

    // Stretched to its own range: the raw map's scale varies per picture, and
    // a fixed cutoff over a varying scale is a different cutoff every time.
    let (lo, hi) = map.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    if !(lo.is_finite() && hi.is_finite()) {
        return Err(Error::other("the model returned a map that isn't numbers"));
    }
    if hi - lo < 1e-6 {
        return Err(Error::unsupported(
            "the model gave the same answer for every pixel, which means it couldn't read this \
             picture at all",
        ));
    }

    let mut grey = GrayImage::new(SIDE as u32, SIDE as u32);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let value = (map[[0, 0, y, x]] - lo) / (hi - lo);
            grey.put_pixel(x as u32, y as u32, image::Luma([(value * 255.0).round() as u8]));
        }
    }
    Ok(grey)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture split down the middle, with a mask that keeps the left half.
    fn halves() -> (DynamicImage, GrayImage) {
        let mut rgb = image::RgbImage::new(40, 20);
        for (x, _, px) in rgb.enumerate_pixels_mut() {
            *px = if x < 20 { image::Rgb([200, 40, 40]) } else { image::Rgb([40, 40, 200]) };
        }
        let mut mask = GrayImage::new(40, 20);
        for (x, _, px) in mask.enumerate_pixels_mut() {
            *px = image::Luma([if x < 20 { 255 } else { 0 }]);
        }
        (DynamicImage::ImageRgb8(rgb), mask)
    }

    #[test]
    fn the_background_becomes_transparent_and_the_subject_does_not() {
        let (source, mask) = halves();
        let (out, report) = composite(&source, &mask, &CutoutOptions::default());
        let rgba = out.to_rgba8();

        assert_eq!(rgba.get_pixel(5, 10).0[3], 255, "the subject stays opaque");
        assert_eq!(rgba.get_pixel(35, 10).0[3], 0, "the background goes");
        assert!((report.coverage - 0.5).abs() < 0.02, "half of it was kept: {report:?}");
    }

    #[test]
    fn a_colour_backdrop_is_mixed_in_rather_than_left_to_whoever_saves_it() {
        // A JPEG has no alpha. If the colour weren't mixed here, asking for a
        // blue background and saving as JPEG would silently give white.
        let (source, mask) = halves();
        let options =
            CutoutOptions { backdrop: Backdrop::Colour([0, 0, 255]), ..Default::default() };
        let (out, _) = composite(&source, &mask, &options);
        let rgba = out.to_rgba8();

        assert_eq!(rgba.get_pixel(35, 10).0, [0, 0, 255, 255], "the backdrop shows through");
        assert_eq!(rgba.get_pixel(5, 10).0, [200, 40, 40, 255], "the subject is untouched");
        assert!(rgba.pixels().all(|p| p.0[3] == 255), "a colour backdrop leaves no holes");
    }

    #[test]
    fn an_uncertain_edge_comes_out_part_transparent() {
        // A hard cutoff is what makes a cutout look cut out, and it is what
        // destroys hair.
        let mut rgb = image::RgbImage::new(10, 1);
        for px in rgb.pixels_mut() {
            *px = image::Rgb([10, 20, 30]);
        }
        let mut mask = GrayImage::new(10, 1);
        for (x, _, px) in mask.enumerate_pixels_mut() {
            // A clean ramp from nothing to certain.
            *px = image::Luma([(x as f32 / 9.0 * 255.0) as u8]);
        }
        let options = CutoutOptions { feather: 0, softness: 0.8, ..Default::default() };
        let (out, _) = composite(&DynamicImage::ImageRgb8(rgb), &mask, &options);
        let alphas: Vec<u8> = out.to_rgba8().pixels().map(|p| p.0[3]).collect();

        assert_eq!(alphas[0], 0, "{alphas:?}");
        assert_eq!(alphas[9], 255, "{alphas:?}");
        let partial = alphas.iter().filter(|a| **a > 10 && **a < 245).count();
        assert!(partial >= 3, "the edge should be graded, got {alphas:?}");
        // And it has to climb, never wander.
        for pair in alphas.windows(2) {
            assert!(pair[1] >= pair[0], "{alphas:?}");
        }
    }

    #[test]
    fn softness_at_its_lowest_still_gives_a_usable_edge() {
        let (source, mask) = halves();
        let options = CutoutOptions { softness: 0.0, feather: 0, ..Default::default() };
        let (out, _) = composite(&source, &mask, &options);
        let rgba = out.to_rgba8();
        assert_eq!(rgba.get_pixel(5, 10).0[3], 255);
        assert_eq!(rgba.get_pixel(35, 10).0[3], 0);
    }

    #[test]
    fn the_threshold_shifts_how_much_is_kept() {
        let mut rgb = image::RgbImage::new(100, 1);
        for px in rgb.pixels_mut() {
            *px = image::Rgb([0, 0, 0]);
        }
        let mut mask = GrayImage::new(100, 1);
        for (x, _, px) in mask.enumerate_pixels_mut() {
            *px = image::Luma([(x as f32 / 99.0 * 255.0) as u8]);
        }
        let source = DynamicImage::ImageRgb8(rgb);

        let generous = CutoutOptions { threshold: 0.2, feather: 0, ..Default::default() };
        let strict = CutoutOptions { threshold: 0.8, feather: 0, ..Default::default() };
        let (_, lenient) = composite(&source, &mask, &generous);
        let (_, mean) = composite(&source, &mask, &strict);
        assert!(
            lenient.coverage > mean.coverage,
            "a lower threshold keeps more: {} vs {}",
            lenient.coverage,
            mean.coverage
        );
    }

    #[test]
    fn keeping_almost_nothing_is_called_out() {
        let rgb = image::RgbImage::from_pixel(30, 30, image::Rgb([9, 9, 9]));
        let mask = GrayImage::new(30, 30); // all zero
        let (_, report) = composite(
            &DynamicImage::ImageRgb8(rgb),
            &mask,
            &CutoutOptions::default(),
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("Almost nothing")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn keeping_almost_everything_is_called_out_too() {
        let rgb = image::RgbImage::from_pixel(30, 30, image::Rgb([9, 9, 9]));
        let mask = GrayImage::from_pixel(30, 30, image::Luma([255]));
        let (_, report) = composite(
            &DynamicImage::ImageRgb8(rgb),
            &mask,
            &CutoutOptions::default(),
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("Almost everything")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn every_report_says_it_is_a_guess() {
        let (source, mask) = halves();
        let (_, report) = composite(&source, &mask, &CutoutOptions::default());
        assert!(report.note.contains("not told"), "{}", report.note);
        assert!(report.note.contains("hair"), "it should say where it shows: {}", report.note);
    }

    #[test]
    fn nonsense_settings_are_refused() {
        let model = Model::load(b"not a model".to_vec());
        assert!(model.is_err(), "rubbish bytes are not a model");

        // Validation happens before the model is consulted, so a tiny stub is
        // enough to reach it.
        let tiny = {
            let mut out = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 8))
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        assert!(Model::load(tiny).is_err());
    }

    #[test]
    fn the_backdrop_choice_crosses_as_the_ui_writes_it() {
        let clear: Backdrop =
            serde_json::from_str(r#"{ "kind": "transparent" }"#).expect("transparent");
        assert_eq!(clear, Backdrop::Transparent);
        let blue: Backdrop =
            serde_json::from_str(r#"{ "kind": "colour", "value": [0, 0, 255] }"#).expect("colour");
        assert_eq!(blue, Backdrop::Colour([0, 0, 255]));

        let options: CutoutOptions = serde_json::from_str(
            r#"{ "backdrop": { "kind": "colour", "value": [255,255,255] },
                  "softness": 0.35, "threshold": 0.5, "feather": 1 }"#,
        )
        .expect("options");
        assert_eq!(options.backdrop, Backdrop::Colour([255, 255, 255]));
        assert_eq!(options.feather, 1);
    }
}
