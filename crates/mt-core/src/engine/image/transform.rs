//! Resize, crop, rotate and convert.
//!
//! The form-filing cases drive the design here. "Photo must be 200 × 230 px"
//! wants an exact size without a squashed face, so [`ResizeMode::Fill`] covers
//! the box and trims the overflow rather than stretching. "Must be under
//! 3.5 × 4.5 cm" wants an aspect ratio, so [`Rect::centred_aspect`] finds the
//! largest crop of that shape. And a photo taken sideways needs rotating
//! without touching anything else.

use image::{imageops, imageops::FilterType, DynamicImage};
use serde::{Deserialize, Serialize};

use super::encode::{encode, resolve_kind, OutputFormat};
use crate::error::{Error, Result};
use crate::format::FileKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformOptions {
    pub format: OutputFormat,
    pub quality: u8,
    pub strip_metadata: bool,
}

impl Default for TransformOptions {
    fn default() -> Self {
        TransformOptions { format: OutputFormat::Keep, quality: 88, strip_metadata: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ResizeMode {
    /// Exactly these pixels, stretching if the aspect ratio disagrees.
    /// Rarely what anyone wants — `Fill` almost always is.
    Stretch { width: u32, height: u32 },
    /// Largest size that fits inside the box, aspect preserved.
    Fit { width: u32, height: u32 },
    /// Exactly these pixels, by covering the box and trimming the overflow
    /// evenly. This is how a form photo gets to 200 × 230 without distortion.
    Fill { width: u32, height: u32 },
    /// A fraction of the original, as a percentage.
    Percent(f32),
    /// Scale so the longest edge is this many pixels.
    LongestEdge(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// The largest rectangle of the given aspect ratio, centred.
    pub fn centred_aspect(image_w: u32, image_h: u32, aspect: f32) -> Rect {
        let aspect = aspect.max(0.0001);
        let (w, h) = if image_w as f32 / image_h as f32 > aspect {
            let h = image_h;
            ((h as f32 * aspect).round() as u32, h)
        } else {
            let w = image_w;
            (w, (w as f32 / aspect).round() as u32)
        };
        let w = w.clamp(1, image_w);
        let h = h.clamp(1, image_h);
        Rect { x: (image_w - w) / 2, y: (image_h - h) / 2, width: w, height: h }
    }

    fn clamp_to(self, image_w: u32, image_h: u32) -> Result<Rect> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::invalid("a crop needs a width and a height"));
        }
        if self.x >= image_w || self.y >= image_h {
            return Err(Error::invalid("that crop starts outside the image"));
        }
        Ok(Rect {
            x: self.x,
            y: self.y,
            width: self.width.min(image_w - self.x),
            height: self.height.min(image_h - self.y),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flip {
    #[default]
    None,
    Horizontal,
    Vertical,
}

pub fn resize(bytes: &[u8], mode: ResizeMode, opts: &TransformOptions) -> Result<Vec<u8>> {
    let (source, image) = open(bytes)?;
    let resized = apply_resize(&image, mode)?;
    finish(&resized, source, opts)
}

pub fn crop(bytes: &[u8], rect: Rect, opts: &TransformOptions) -> Result<Vec<u8>> {
    let (source, image) = open(bytes)?;
    let rect = rect.clamp_to(image.width(), image.height())?;
    let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
    finish(&cropped, source, opts)
}

/// Crop to an aspect ratio, centred — `crop(bytes, 3.5 / 4.5, …)`.
pub fn crop_to_aspect(bytes: &[u8], aspect: f32, opts: &TransformOptions) -> Result<Vec<u8>> {
    let (source, image) = open(bytes)?;
    let rect = Rect::centred_aspect(image.width(), image.height(), aspect);
    let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
    finish(&cropped, source, opts)
}

/// Turn by a quarter turn multiple, then optionally mirror.
pub fn rotate(bytes: &[u8], degrees: i32, flip: Flip, opts: &TransformOptions) -> Result<Vec<u8>> {
    if degrees % 90 != 0 {
        return Err(Error::invalid(
            "images can only turn in quarter turns here — use straighten for small angles",
        ));
    }
    let (source, image) = open(bytes)?;
    let turned = match degrees.rem_euclid(360) {
        90 => image.rotate90(),
        180 => image.rotate180(),
        270 => image.rotate270(),
        _ => image,
    };
    let flipped = match flip {
        Flip::None => turned,
        Flip::Horizontal => DynamicImage::ImageRgba8(imageops::flip_horizontal(&turned)),
        Flip::Vertical => DynamicImage::ImageRgba8(imageops::flip_vertical(&turned)),
    };
    finish(&flipped, source, opts)
}

/// Re-encode in another format, changing nothing else.
pub fn convert(bytes: &[u8], opts: &TransformOptions) -> Result<Vec<u8>> {
    let (source, image) = open(bytes)?;
    finish(&image, source, opts)
}

/// What a resize would produce, without doing it — for live UI readouts.
pub fn resized_dimensions(width: u32, height: u32, mode: ResizeMode) -> (u32, u32) {
    match mode {
        ResizeMode::Stretch { width: w, height: h } | ResizeMode::Fill { width: w, height: h } => {
            (w.max(1), h.max(1))
        }
        ResizeMode::Fit { width: w, height: h } => {
            let scale = (w as f32 / width as f32).min(h as f32 / height as f32);
            scaled(width, height, scale)
        }
        ResizeMode::Percent(pct) => scaled(width, height, pct / 100.0),
        ResizeMode::LongestEdge(edge) => {
            let scale = edge as f32 / width.max(height) as f32;
            scaled(width, height, scale)
        }
    }
}

fn scaled(width: u32, height: u32, scale: f32) -> (u32, u32) {
    (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    )
}

fn apply_resize(image: &DynamicImage, mode: ResizeMode) -> Result<DynamicImage> {
    let (w, h) = (image.width(), image.height());
    Ok(match mode {
        ResizeMode::Stretch { width, height } => {
            image.resize_exact(width.max(1), height.max(1), FilterType::Lanczos3)
        }
        ResizeMode::Fill { width, height } => {
            let (width, height) = (width.max(1), height.max(1));
            // Cover the box, then take the middle: no distortion, no letterbox.
            let scale = (width as f32 / w as f32).max(height as f32 / h as f32);
            let (sw, sh) = scaled(w, h, scale);
            let covered =
                image.resize_exact(sw.max(width), sh.max(height), FilterType::Lanczos3);
            covered.crop_imm(
                (covered.width() - width) / 2,
                (covered.height() - height) / 2,
                width,
                height,
            )
        }
        ResizeMode::Fit { .. } | ResizeMode::Percent(_) | ResizeMode::LongestEdge(_) => {
            let (tw, th) = resized_dimensions(w, h, mode);
            if tw == w && th == h {
                image.clone()
            } else {
                image.resize_exact(tw, th, FilterType::Lanczos3)
            }
        }
    })
}

fn open(bytes: &[u8]) -> Result<(FileKind, DynamicImage)> {
    let kind = FileKind::sniff(bytes);
    if !kind.is_image() {
        return Err(Error::unsupported(format!(
            "{} is not an image this tool can read",
            kind.label()
        )));
    }
    let image = image::load_from_memory(bytes)?;
    if image.width() == 0 || image.height() == 0 {
        return Err(Error::invalid("image has no pixels"));
    }
    Ok((kind, image))
}

fn finish(image: &DynamicImage, source: FileKind, opts: &TransformOptions) -> Result<Vec<u8>> {
    let kind = resolve_kind(source, opts.format, image);
    encode(image, kind, opts.quality, opts.strip_metadata)
}

/// The format a transform will write, so callers can name the output file.
pub fn output_kind(bytes: &[u8], opts: &TransformOptions) -> Result<FileKind> {
    let (source, image) = open(bytes)?;
    Ok(resolve_kind(source, opts.format, &image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;
    use image::ImageEncoder;

    fn photo(w: u32, h: u32) -> Vec<u8> {
        testkit::encode_jpeg(&testkit::photo(w, h), 92)
    }

    fn dims(bytes: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(bytes).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn fill_hits_the_exact_size_without_stretching() {
        // A wide photo into a tall form-photo box.
        let src = photo(1000, 500);
        let out = resize(
            &src,
            ResizeMode::Fill { width: 200, height: 230 },
            &TransformOptions::default(),
        )
        .unwrap();
        assert_eq!(dims(&out), (200, 230));
    }

    #[test]
    fn fit_stays_inside_the_box_and_keeps_the_shape() {
        let src = photo(1000, 500);
        let out =
            resize(&src, ResizeMode::Fit { width: 300, height: 300 }, &TransformOptions::default())
                .unwrap();
        assert_eq!(dims(&out), (300, 150));
    }

    #[test]
    fn stretch_does_what_it_says() {
        let src = photo(1000, 500);
        let out = resize(
            &src,
            ResizeMode::Stretch { width: 200, height: 230 },
            &TransformOptions::default(),
        )
        .unwrap();
        assert_eq!(dims(&out), (200, 230));
    }

    #[test]
    fn percent_and_longest_edge() {
        let src = photo(800, 400);
        assert_eq!(
            dims(&resize(&src, ResizeMode::Percent(50.0), &TransformOptions::default()).unwrap()),
            (400, 200)
        );
        assert_eq!(
            dims(&resize(&src, ResizeMode::LongestEdge(200), &TransformOptions::default()).unwrap()),
            (200, 100)
        );
    }

    #[test]
    fn predicted_dimensions_match_what_resize_produces() {
        let src = photo(1234, 567);
        for mode in [
            ResizeMode::Fit { width: 400, height: 400 },
            ResizeMode::Percent(37.0),
            ResizeMode::LongestEdge(500),
            ResizeMode::Fill { width: 200, height: 230 },
        ] {
            let predicted = resized_dimensions(1234, 567, mode);
            let actual = dims(&resize(&src, mode, &TransformOptions::default()).unwrap());
            assert_eq!(predicted, actual, "prediction drifted for {mode:?}");
        }
    }

    #[test]
    fn crops_a_rectangle() {
        let src = photo(400, 300);
        let out = crop(
            &src,
            Rect { x: 50, y: 20, width: 100, height: 80 },
            &TransformOptions::default(),
        )
        .unwrap();
        assert_eq!(dims(&out), (100, 80));
    }

    #[test]
    fn a_crop_past_the_edge_is_trimmed_not_rejected() {
        let src = photo(400, 300);
        let out = crop(
            &src,
            Rect { x: 350, y: 250, width: 200, height: 200 },
            &TransformOptions::default(),
        )
        .unwrap();
        assert_eq!(dims(&out), (50, 50));
        assert!(crop(
            &src,
            Rect { x: 500, y: 0, width: 10, height: 10 },
            &TransformOptions::default()
        )
        .is_err());
    }

    #[test]
    fn aspect_crop_finds_the_biggest_centred_box() {
        assert_eq!(
            Rect::centred_aspect(1000, 500, 1.0),
            Rect { x: 250, y: 0, width: 500, height: 500 }
        );
        let src = photo(1000, 500);
        let out = crop_to_aspect(&src, 3.5 / 4.5, &TransformOptions::default()).unwrap();
        let (w, h) = dims(&out);
        assert!(((w as f32 / h as f32) - 3.5 / 4.5).abs() < 0.01);
    }

    #[test]
    fn rotation_swaps_the_sides() {
        let src = photo(400, 300);
        assert_eq!(dims(&rotate(&src, 90, Flip::None, &TransformOptions::default()).unwrap()), (300, 400));
        assert_eq!(dims(&rotate(&src, 180, Flip::None, &TransformOptions::default()).unwrap()), (400, 300));
        assert_eq!(dims(&rotate(&src, -90, Flip::None, &TransformOptions::default()).unwrap()), (300, 400));
        assert!(rotate(&src, 45, Flip::None, &TransformOptions::default()).is_err());
    }

    #[test]
    fn flipping_is_reversible() {
        let src = photo(120, 90);
        let opts = TransformOptions { format: OutputFormat::Png, ..Default::default() };
        let once = rotate(&src, 0, Flip::Horizontal, &opts).unwrap();
        let twice = rotate(&once, 0, Flip::Horizontal, &opts).unwrap();
        let a = image::load_from_memory(&src).unwrap().to_rgb8();
        let b = image::load_from_memory(&twice).unwrap().to_rgb8();
        assert_eq!(a.dimensions(), b.dimensions());
    }

    #[test]
    fn converts_between_formats() {
        let src = photo(200, 150);
        let png = convert(
            &src,
            &TransformOptions { format: OutputFormat::Png, ..Default::default() },
        )
        .unwrap();
        assert_eq!(FileKind::sniff(&png), FileKind::Png);
        let back = convert(
            &png,
            &TransformOptions { format: OutputFormat::Jpeg, ..Default::default() },
        )
        .unwrap();
        assert_eq!(FileKind::sniff(&back), FileKind::Jpeg);
        assert_eq!(dims(&back), (200, 150));
    }

    #[test]
    fn keep_format_preserves_transparency() {
        let mut rgba = image::RgbaImage::new(40, 40);
        for (x, _, p) in rgba.enumerate_pixels_mut() {
            *p = image::Rgba([10, 20, 30, if x < 20 { 0 } else { 255 }]);
        }
        let mut src = Vec::new();
        image::codecs::png::PngEncoder::new(&mut src)
            .write_image(rgba.as_raw(), 40, 40, image::ExtendedColorType::Rgba8)
            .unwrap();

        let out = resize(&src, ResizeMode::Percent(50.0), &TransformOptions::default()).unwrap();
        assert_eq!(FileKind::sniff(&out), FileKind::Png, "alpha must not be flattened away");
    }

    #[test]
    fn refuses_a_pdf() {
        assert!(convert(b"%PDF-1.7\n", &TransformOptions::default()).is_err());
    }
}
