//! Cutting a signature out of a photograph of one.
//!
//! The point is to do this **once**. You sign a piece of paper, photograph it,
//! and from then on it is a transparent PNG that drops onto any form. What
//! makes that work is the transparency: a signature saved as a white rectangle
//! covers the line it is supposed to sit on, and every form has a line.
//!
//! The paper is removed the same way [`super::scan`] removes it — divide by a
//! blurred copy of the image, which is an estimate of the background, so the
//! shadow across the page goes and the ink stays. The difference is what
//! happens next: instead of pushing the result to white, the inverse becomes
//! the alpha channel.
//!
//! Alpha is taken on a ramp rather than at a threshold. A hard cutoff turns a
//! pen stroke into a jagged two-colour shape — it reads as a bad fax, and the
//! edges crawl when the signature is scaled down onto a form. The ramp keeps
//! the soft edge the pen actually made.

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InkColour {
    /// Whatever colour the pen was.
    #[default]
    Keep,
    Black,
    /// The blue most forms ask for, so a signature photographed in black ink
    /// can still satisfy "sign in blue".
    Blue,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureOptions {
    pub ink: InkColour,
    /// How hard to cut the paper away, 0.0–1.0. Higher removes more paper and
    /// more of a faint stroke with it.
    pub strength: f32,
    /// Crop to the ink. Almost always wanted: the photograph is mostly paper.
    pub trim: bool,
}

impl Default for SignatureOptions {
    fn default() -> Self {
        SignatureOptions { ink: InkColour::Keep, strength: 0.5, trim: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureReport {
    /// Whether anything that looks like a pen stroke was found.
    pub found_ink: bool,
    pub width: u32,
    pub height: u32,
    /// Fraction of the trimmed box that is ink. A signature is a few percent;
    /// far more than that means something other than a signature was cut out.
    pub coverage: f32,
    pub note: Option<String>,
}

/// Cut the signature out of a photograph, as RGBA with the paper transparent.
pub fn extract(bytes: &[u8], options: &SignatureOptions) -> Result<(DynamicImage, SignatureReport)> {
    if !(0.0..=1.0).contains(&options.strength) {
        return Err(Error::invalid("strength has to be between 0 and 1"));
    }
    let source = image::load_from_memory(bytes)?;
    if source.width() < 16 || source.height() < 16 {
        return Err(Error::invalid("that image is too small to hold a signature"));
    }
    Ok(extract_image(&source, options))
}

/// The same, on an image already decoded.
pub fn extract_image(
    source: &DynamicImage,
    options: &SignatureOptions,
) -> (DynamicImage, SignatureReport) {
    let (w, h) = (source.width(), source.height());
    let rgb = source.to_rgb8();
    let grey = source.to_luma8();

    // Wide enough to blur the strokes away entirely, so what is left is the
    // paper. Too narrow and a thick stroke becomes its own background and
    // disappears into it.
    let radius = (w.min(h) / 8).clamp(6, 160);
    let background = super::scan::box_blur(&grey, radius);

    // Paper sits near its own local background; ink sits well below it.
    let paper = 0.93 - 0.06 * options.strength;
    let solid = 0.62 - 0.22 * options.strength;

    let mut out = RgbaImage::new(w, h);
    let mut ink_pixels = 0u64;
    for y in 0..h {
        for x in 0..w {
            let value = grey.get_pixel(x, y)[0] as f32;
            let local = background.get_pixel(x, y)[0] as f32;
            let ratio = if local > 1.0 { value / local } else { 1.0 };
            let alpha = ((paper - ratio) / (paper - solid)).clamp(0.0, 1.0);
            if alpha > 0.5 {
                ink_pixels += 1;
            }

            let source_px = rgb.get_pixel(x, y).0;
            let colour = match options.ink {
                InkColour::Keep => source_px,
                InkColour::Black => [24, 24, 28],
                InkColour::Blue => [21, 48, 140],
            };
            out.put_pixel(
                x,
                y,
                Rgba([colour[0], colour[1], colour[2], (alpha * 255.0).round() as u8]),
            );
        }
    }

    let found_ink = ink_pixels > 0;
    let cropped = if options.trim && found_ink { trim(&out) } else { out };

    let (cw, ch) = (cropped.width(), cropped.height());
    let coverage = if cw * ch > 0 { ink_pixels as f32 / (cw * ch) as f32 } else { 0.0 };

    let note = if !found_ink {
        Some(
            "No pen strokes were found. Photograph the signature on plain paper in even light, \
             or raise the strength if the ink is faint."
                .into(),
        )
    } else if coverage > 0.55 {
        Some(
            "Most of this image came out as ink, which a signature never is. Check it is a \
             signature on light paper and not a photograph of something darker."
                .into(),
        )
    } else {
        None
    };

    let report = SignatureReport { found_ink, width: cw, height: ch, coverage, note };
    (DynamicImage::ImageRgba8(cropped), report)
}

/// Crop to the ink, keeping a small breathing margin.
fn trim(image: &RgbaImage) -> RgbaImage {
    let (w, h) = (image.width(), image.height());
    // Judged at a firmer alpha than the ramp's foot, so the faint halo around
    // a stroke doesn't drag the box back out to the full frame.
    const SOLID: u8 = 40;

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if image.get_pixel(x, y).0[3] > SOLID {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        return image.clone();
    }

    let margin = (w.max(h) / 100).max(2);
    let x0 = min_x.saturating_sub(margin);
    let y0 = min_y.saturating_sub(margin);
    let x1 = (max_x + margin + 1).min(w);
    let y1 = (max_y + margin + 1).min(h);

    image.view(x0, y0, x1 - x0, y1 - y0).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// A photograph of a signature: ink on paper, lit unevenly.
    fn photo_of_a_signature() -> DynamicImage {
        let (w, h) = (600u32, 400u32);
        let mut canvas = image::RgbImage::new(w, h);
        for (x, y, pixel) in canvas.enumerate_pixels_mut() {
            // Paper, brighter on the left than the right.
            let shade = (235.0 - 70.0 * (x as f32 / w as f32) - 20.0 * (y as f32 / h as f32))
                .clamp(0.0, 255.0) as u8;
            *pixel = Rgb([shade, shade, shade.saturating_sub(4)]);
        }
        // A stroke across the middle third, in dark blue ink.
        for t in 0..420 {
            let x = 90 + t;
            let y = 200.0 + 42.0 * ((t as f32) / 46.0).sin();
            for dy in -3i32..=3 {
                let py = (y as i32 + dy).clamp(0, h as i32 - 1) as u32;
                canvas.put_pixel(x.min(w - 1), py, Rgb([26, 34, 92]));
            }
        }
        DynamicImage::ImageRgb8(canvas)
    }

    fn encode(image: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn the_paper_becomes_transparent_and_the_ink_does_not() {
        let photo = photo_of_a_signature();
        let (cut, report) = extract_image(&photo, &SignatureOptions::default());
        assert!(report.found_ink, "the stroke should be found: {report:?}");
        assert!(report.note.is_none(), "{:?}", report.note);

        let rgba = cut.to_rgba8();
        let opaque = rgba.pixels().filter(|p| p.0[3] > 200).count();
        let clear = rgba.pixels().filter(|p| p.0[3] < 20).count();
        assert!(opaque > 500, "the stroke should survive, {opaque} solid pixels");
        // A signature is mostly air, even after trimming.
        assert!(
            clear as f32 / rgba.pixels().len() as f32 > 0.5,
            "most of it should be transparent"
        );
    }

    #[test]
    fn the_uneven_lighting_does_not_become_ink() {
        // The right-hand side of the paper is much darker than the left. A
        // plain brightness threshold would call it ink; dividing by the local
        // background must not.
        let photo = photo_of_a_signature();
        let (cut, _) = extract_image(&photo, &SignatureOptions { trim: false, ..Default::default() });
        let rgba = cut.to_rgba8();
        // Sample the dark corner of the paper, well away from the stroke.
        for (x, y) in [(560u32, 30u32), (580, 370), (570, 60)] {
            let alpha = rgba.get_pixel(x, y).0[3];
            assert!(alpha < 40, "paper at {x},{y} came out as ink (alpha {alpha})");
        }
    }

    #[test]
    fn trimming_crops_to_the_stroke() {
        let photo = photo_of_a_signature();
        let (trimmed, report) = extract_image(&photo, &SignatureOptions::default());
        let (untrimmed, _) =
            extract_image(&photo, &SignatureOptions { trim: false, ..Default::default() });

        assert!(trimmed.width() < untrimmed.width(), "should be narrower");
        assert!(trimmed.height() < untrimmed.height(), "should be shorter");
        assert_eq!((report.width, report.height), (trimmed.width(), trimmed.height()));
        // The stroke spans x 90..510 of 600, y about 158..242 of 400.
        assert!(trimmed.height() < 160, "too tall to be cropped to the stroke");
    }

    #[test]
    fn the_ink_can_be_recoloured() {
        let photo = photo_of_a_signature();
        let (blue, _) =
            extract_image(&photo, &SignatureOptions { ink: InkColour::Blue, ..Default::default() });
        let rgba = blue.to_rgba8();
        let solid = rgba.pixels().find(|p| p.0[3] > 220).expect("some solid ink");
        assert_eq!([solid.0[0], solid.0[1], solid.0[2]], [21, 48, 140]);

        let (black, _) = extract_image(
            &photo,
            &SignatureOptions { ink: InkColour::Black, ..Default::default() },
        );
        let solid = black.to_rgba8().pixels().find(|p| p.0[3] > 220).copied().unwrap();
        assert_eq!([solid.0[0], solid.0[1], solid.0[2]], [24, 24, 28]);
    }

    #[test]
    fn blank_paper_is_reported_rather_than_returned_empty() {
        let blank = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            300,
            200,
            Rgb([242, 241, 238]),
        ));
        let (_, report) = extract_image(&blank, &SignatureOptions::default());
        assert!(!report.found_ink);
        let note = report.note.expect("should say why");
        assert!(note.contains("No pen strokes"), "{note}");
    }

    #[test]
    fn a_photograph_of_something_dark_is_called_out() {
        let dark = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(300, 200, Rgb([30, 30, 34])));
        // A uniformly dark frame divides out to its own background, so what
        // makes this suspicious is reported rather than silently returned.
        let (_, report) = extract_image(&dark, &SignatureOptions { strength: 1.0, ..Default::default() });
        assert!(!report.found_ink || report.note.is_some(), "{report:?}");
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(extract(b"not an image", &SignatureOptions::default()).is_err());
        let photo = encode(&photo_of_a_signature());
        let loud = SignatureOptions { strength: 9.0, ..Default::default() };
        assert!(extract(&photo, &loud).is_err());
    }

    #[test]
    fn a_real_photograph_round_trips_through_png() {
        let photo = encode(&photo_of_a_signature());
        let (cut, report) = extract(&photo, &SignatureOptions::default()).unwrap();
        assert!(report.found_ink);
        assert!(cut.to_rgba8().pixels().any(|p| p.0[3] > 200));
    }
}
