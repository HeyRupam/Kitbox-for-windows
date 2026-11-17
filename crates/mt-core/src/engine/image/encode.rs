//! One place that turns pixels into file bytes.
//!
//! Shared by compression and the transform tools so they can't disagree about
//! what "keep the format" means, how transparency is handled, or whether PNG
//! output gets optimised.

use image::{DynamicImage, ImageEncoder};

use crate::error::{Error, Result};
use crate::format::FileKind;

pub use super::compress::OutputFormat;

/// Which format bytes should be written in, given the source and the request.
///
/// Only JPEG and PNG have encoders here. Anything else becomes a JPEG, except
/// when the image carries transparency that JPEG would destroy — then it stays
/// PNG whatever the source was.
pub fn resolve_kind(source: FileKind, format: OutputFormat, image: &DynamicImage) -> FileKind {
    match format {
        OutputFormat::Jpeg => FileKind::Jpeg,
        OutputFormat::Png => FileKind::Png,
        OutputFormat::Keep => match source {
            FileKind::Png | FileKind::Gif | FileKind::WebP => {
                if has_alpha(image) {
                    FileKind::Png
                } else {
                    FileKind::Jpeg
                }
            }
            _ => FileKind::Jpeg,
        },
    }
}

pub fn encode(
    image: &DynamicImage,
    kind: FileKind,
    quality: u8,
    strip_metadata: bool,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    match kind {
        FileKind::Jpeg => {
            // JPEG has no alpha channel; flattening onto white is what a
            // printer or a form scanner would do with it anyway.
            let rgb = if has_alpha(image) { flatten_on_white(image) } else { image.to_rgb8() };
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.max(1))
                .encode_image(&rgb)?;
        }
        FileKind::Png => {
            let rgba = image.to_rgba8();
            let mut raw = Vec::new();
            image::codecs::png::PngEncoder::new(&mut raw).write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            let mut options = oxipng::Options::from_preset(3);
            options.strip = if strip_metadata {
                oxipng::StripChunks::Safe
            } else {
                oxipng::StripChunks::None
            };
            buf = oxipng::optimize_from_memory(&raw, &options).unwrap_or(raw);
        }
        other => return Err(Error::unsupported(format!("cannot write {}", other.label()))),
    }
    Ok(buf)
}

pub fn has_alpha(image: &DynamicImage) -> bool {
    use image::DynamicImage::*;
    matches!(image, ImageLumaA8(_) | ImageLumaA16(_) | ImageRgba8(_) | ImageRgba16(_) | ImageRgba32F(_))
}

pub fn flatten_on_white(image: &DynamicImage) -> image::RgbImage {
    let rgba = image.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let a = p[3] as f32 / 255.0;
        let blend = |c: u8| ((c as f32 * a) + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
        image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])])
    })
}
