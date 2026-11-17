//! Stamping text or a logo onto an image.
//!
//! The PDF side gets Helvetica for free, because readers are required to
//! provide it. An image has no such luxury: something has to turn glyph
//! outlines into pixels, and something has to supply the outlines.
//!
//! So the font arrives as bytes from the caller. `mt-core` stays free of
//! platform code, and finding a font on the machine — which is inherently
//! platform-specific — belongs to the shell that knows what machine it is on.
//! Rasterised output is pixels, not a redistributed font.
//!
//! Text and logos take the same path once the mark exists: build an RGBA
//! stamp, scale it to a fraction of the image width, rotate it, then composite.
//! Doing it uniformly means a diagonal logo and a diagonal caption behave the
//! same way instead of being two code paths that drift.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::{DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use super::encode::{encode, resolve_kind, OutputFormat};
use crate::engine::placement::Placement;
use crate::error::{Error, Result};
use crate::format::FileKind;

/// Where a font usually lives, most preferred first.
///
/// This is a list of strings, not a lookup: `mt-core` does not touch the
/// filesystem, so the shell reads whichever of these exists and passes the
/// bytes in. Sharing the list here keeps the CLI and the desktop app agreeing
/// on which font a watermark gets.
pub const FONT_CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\segoeui.ttf",
    r"C:\Windows\Fonts\arial.ttf",
    r"C:\Windows\Fonts\calibri.ttf",
    r"C:\Windows\Fonts\verdana.ttf",
    r"C:\Windows\Fonts\tahoma.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/Library/Fonts/Arial.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
];

/// Size the text is rasterised at before being scaled to fit. Large enough
/// that scaling down stays crisp, small enough to be cheap.
const RASTER_PX: f32 = 220.0;

#[derive(Debug, Clone)]
pub enum ImageMark {
    /// Text, plus the font to draw it with. See the module note on why the
    /// bytes come from the caller.
    Text { text: String, font: Vec<u8> },
    /// A logo, as JPEG/PNG/WebP bytes. Transparency is respected.
    Logo(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageWatermarkOptions {
    pub placement: Placement,
    /// 0.0–1.0.
    pub opacity: f32,
    /// Degrees anticlockwise.
    pub rotation: f32,
    /// Width of the mark as a fraction of the image's width.
    pub scale: f32,
    /// Text colour. Ignored for a logo.
    pub colour: [u8; 3],
    /// Distance from the edge, as a fraction of the image's shorter side.
    pub margin: f32,
    pub format: OutputFormat,
    pub quality: u8,
}

impl Default for ImageWatermarkOptions {
    fn default() -> Self {
        ImageWatermarkOptions {
            placement: Placement::BottomRight,
            opacity: 0.45,
            rotation: 0.0,
            scale: 0.3,
            colour: [255, 255, 255],
            margin: 0.04,
            format: OutputFormat::Keep,
            quality: 90,
        }
    }
}

pub fn watermark(
    bytes: &[u8],
    mark: &ImageMark,
    options: &ImageWatermarkOptions,
) -> Result<Vec<u8>> {
    if !(0.0..=1.0).contains(&options.opacity) {
        return Err(Error::invalid("opacity has to be between 0 and 1"));
    }
    if !(0.01..=1.0).contains(&options.scale) {
        return Err(Error::invalid("the mark has to be between 1% and 100% of the width"));
    }

    let source = FileKind::sniff(bytes);
    if !source.is_image() {
        return Err(Error::unsupported(format!(
            "{} is not an image this tool can read",
            source.label()
        )));
    }
    let base = image::load_from_memory(bytes)?;
    if base.width() == 0 || base.height() == 0 {
        return Err(Error::invalid("image has no pixels"));
    }

    let stamp = build_stamp(mark, options)?;
    let stamp = fit_to_width(&stamp, (base.width() as f32 * options.scale) as u32);
    let stamp = if options.rotation.abs() > 0.01 {
        rotate_rgba(&stamp, options.rotation)
    } else {
        stamp
    };

    let mut canvas = base.to_rgba8();
    composite(&mut canvas, &stamp, options);

    let output = DynamicImage::ImageRgba8(canvas);
    let kind = resolve_kind(source, options.format, &output);
    encode(&output, kind, options.quality, true)
}

/// The format the result will be written in, so a caller can name the file.
pub fn output_kind(bytes: &[u8], options: &ImageWatermarkOptions) -> Result<FileKind> {
    let source = FileKind::sniff(bytes);
    let image = image::load_from_memory(bytes)?;
    Ok(resolve_kind(source, options.format, &image))
}

fn build_stamp(mark: &ImageMark, options: &ImageWatermarkOptions) -> Result<RgbaImage> {
    match mark {
        ImageMark::Text { text, font } => {
            if text.trim().is_empty() {
                return Err(Error::invalid("the watermark has no text in it"));
            }
            rasterise(text, font, options.colour)
        }
        ImageMark::Logo(data) => {
            if !FileKind::sniff(data).is_image() {
                return Err(Error::unsupported("the logo isn't an image this tool can read"));
            }
            Ok(image::load_from_memory(data)?.to_rgba8())
        }
    }
}

/// Draw text into a transparent RGBA bitmap, tight to the glyphs.
pub(super) fn rasterise(text: &str, font_bytes: &[u8], colour: [u8; 3]) -> Result<RgbaImage> {
    let font = FontRef::try_from_slice(font_bytes)
        .map_err(|e| Error::invalid(format!("that font could not be read: {e}")))?;
    let scaled = font.as_scaled(PxScale::from(RASTER_PX));

    // Lay the glyphs out along a baseline, applying kerning.
    let mut glyphs = Vec::new();
    let mut caret = 0.0f32;
    let mut previous: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(previous) = previous {
            caret += scaled.kern(previous, id);
        }
        glyphs.push(id.with_scale_and_position(RASTER_PX, point(caret, scaled.ascent())));
        caret += scaled.h_advance(id);
        previous = Some(id);
    }

    let width = (caret.ceil() as u32).max(1) + 2;
    let height = ((scaled.ascent() - scaled.descent()).ceil() as u32).max(1) + 2;
    let mut canvas = RgbaImage::new(width, height);

    for glyph in glyphs {
        let Some(outlined) = font.outline_glyph(glyph) else { continue };
        let bounds = outlined.px_bounds();
        outlined.draw(|x, y, coverage| {
            let px = bounds.min.x as i32 + x as i32;
            let py = bounds.min.y as i32 + y as i32;
            if px < 0 || py < 0 || px as u32 >= width || py as u32 >= height {
                return;
            }
            // Glyphs can overlap; keep the strongest coverage rather than
            // letting a later one erase an earlier one's edge.
            let pixel = canvas.get_pixel_mut(px as u32, py as u32);
            let alpha = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
            if alpha > pixel[3] {
                *pixel = Rgba([colour[0], colour[1], colour[2], alpha]);
            }
        });
    }

    Ok(canvas)
}

/// Scale a stamp so it is `target_width` across, keeping its shape.
fn fit_to_width(stamp: &RgbaImage, target_width: u32) -> RgbaImage {
    let target_width = target_width.max(1);
    if stamp.width() == target_width {
        return stamp.clone();
    }
    let height =
        ((stamp.height() as f32 * target_width as f32 / stamp.width() as f32).round() as u32).max(1);
    image::imageops::resize(stamp, target_width, height, image::imageops::FilterType::Lanczos3)
}

/// Rotate anticlockwise, growing the canvas so nothing is clipped.
///
/// Sampled backwards from the destination with bilinear interpolation, which
/// keeps diagonal text from looking like a staircase.
fn rotate_rgba(stamp: &RgbaImage, degrees: f32) -> RgbaImage {
    let radians = degrees.to_radians();
    let (sin, cos) = (radians.sin(), radians.cos());
    let (w, h) = (stamp.width() as f32, stamp.height() as f32);

    let out_w = (w * cos.abs() + h * sin.abs()).ceil().max(1.0);
    let out_h = (w * sin.abs() + h * cos.abs()).ceil().max(1.0);
    let mut out = RgbaImage::new(out_w as u32, out_h as u32);

    let (cx, cy) = (w / 2.0, h / 2.0);
    let (ocx, ocy) = (out_w / 2.0, out_h / 2.0);

    for y in 0..out.height() {
        for x in 0..out.width() {
            let dx = x as f32 + 0.5 - ocx;
            let dy = y as f32 + 0.5 - ocy;
            // Inverse rotation: screen y grows downwards, so the sign flips.
            let sx = cos * dx - sin * dy + cx - 0.5;
            let sy = sin * dx + cos * dy + cy - 0.5;
            if let Some(pixel) = sample_bilinear(stamp, sx, sy) {
                out.put_pixel(x, y, pixel);
            }
        }
    }
    out
}

fn sample_bilinear(image: &RgbaImage, x: f32, y: f32) -> Option<Rgba<u8>> {
    let (w, h) = (image.width() as i32, image.height() as i32);
    if x < -1.0 || y < -1.0 || x > w as f32 || y > h as f32 {
        return None;
    }
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let at = |px: i32, py: i32| -> [f32; 4] {
        if px < 0 || py < 0 || px >= w || py >= h {
            return [0.0; 4];
        }
        let p = image.get_pixel(px as u32, py as u32).0;
        [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
    };

    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    let mut out = [0u8; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
    }
    if out[3] == 0 {
        return None;
    }
    Some(Rgba(out))
}

fn composite(canvas: &mut RgbaImage, stamp: &RgbaImage, options: &ImageWatermarkOptions) {
    let (cw, ch) = (canvas.width() as f32, canvas.height() as f32);
    let (sw, sh) = (stamp.width() as f32, stamp.height() as f32);
    let margin = options.margin * cw.min(ch);

    let spots: Vec<(f32, f32)> = if options.placement == Placement::Tiled {
        tile_spots(cw, ch, sw, sh)
    } else {
        vec![options.placement.anchor(cw, ch, sw, sh, margin)]
    };

    for (centre_x, centre_y) in spots {
        // Anchors come back with y growing upwards; pixels grow downwards.
        let left = (centre_x - sw / 2.0).round() as i32;
        let top = (ch - centre_y - sh / 2.0).round() as i32;
        blend(canvas, stamp, left, top, options.opacity);
    }
}

fn tile_spots(width: f32, height: f32, mark_w: f32, mark_h: f32) -> Vec<(f32, f32)> {
    let step_x = (mark_w * 1.5).max(32.0);
    let step_y = (mark_h * 2.2).max(32.0);
    let mut spots = Vec::new();
    let mut y = step_y / 2.0;
    while y < height && spots.len() < 400 {
        let mut x = step_x / 2.0;
        while x < width {
            spots.push((x, y));
            x += step_x;
        }
        y += step_y;
    }
    spots
}

/// Source-over alpha blend, with a global opacity on top of the stamp's own.
fn blend(canvas: &mut RgbaImage, stamp: &RgbaImage, left: i32, top: i32, opacity: f32) {
    for y in 0..stamp.height() {
        let ty = top + y as i32;
        if ty < 0 || ty as u32 >= canvas.height() {
            continue;
        }
        for x in 0..stamp.width() {
            let tx = left + x as i32;
            if tx < 0 || tx as u32 >= canvas.width() {
                continue;
            }
            let source = stamp.get_pixel(x, y).0;
            let alpha = source[3] as f32 / 255.0 * opacity;
            if alpha <= 0.0 {
                continue;
            }
            let target = canvas.get_pixel_mut(tx as u32, ty as u32);
            for i in 0..3 {
                let blended = source[i] as f32 * alpha + target[i] as f32 * (1.0 - alpha);
                target[i] = blended.round().clamp(0.0, 255.0) as u8;
            }
            // The base keeps whatever opacity it had; a watermark shouldn't
            // punch a hole in a transparent PNG.
            target[3] = target[3].max((alpha * 255.0).round() as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// A font is needed to test text at all, and the machine running the tests
    /// has to provide one. Windows always has these; the test says so rather
    /// than failing mysteriously if none is found.
    fn test_font() -> Option<Vec<u8>> {
        const CANDIDATES: &[&str] = &[
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\verdana.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ];
        CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
    }

    fn photo(w: u32, h: u32) -> Vec<u8> {
        testkit::encode_jpeg(&testkit::photo(w, h), 92)
    }

    fn decode(bytes: &[u8]) -> RgbaImage {
        image::load_from_memory(bytes).unwrap().to_rgba8()
    }

    #[test]
    fn a_logo_changes_the_corner_it_lands_in() {
        let base = photo(400, 300);
        let logo = testkit::encode_jpeg(&testkit::photo(80, 40), 90);
        let options = ImageWatermarkOptions {
            placement: Placement::BottomRight,
            opacity: 1.0,
            scale: 0.3,
            // Lossless out, or re-encoding would shift every pixel and the
            // comparison below would mean nothing.
            format: OutputFormat::Png,
            ..Default::default()
        };
        let out = watermark(&base, &ImageMark::Logo(logo), &options).unwrap();

        let before = decode(&base);
        let after = decode(&out);
        assert_eq!(before.dimensions(), after.dimensions());

        // The stamped corner differs; the opposite one does not.
        let changed = |x: u32, y: u32| before.get_pixel(x, y) != after.get_pixel(x, y);
        assert!(changed(360, 270), "the bottom-right should have the logo on it");
        assert!(!changed(10, 10), "the top-left should be untouched");
    }

    #[test]
    fn text_is_drawn_when_a_font_is_supplied() {
        let Some(font) = test_font() else {
            eprintln!("skipping: no system font on this machine");
            return;
        };
        let base = photo(500, 400);
        let mark = ImageMark::Text { text: "CONFIDENTIAL".into(), font };
        let options = ImageWatermarkOptions {
            placement: Placement::Centre,
            opacity: 1.0,
            scale: 0.8,
            colour: [255, 0, 0],
            format: OutputFormat::Png,
            ..Default::default()
        };
        let out = watermark(&base, &mark, &options).unwrap();

        let before = decode(&base);
        let after = decode(&out);
        let differing = before
            .pixels()
            .zip(after.pixels())
            .filter(|(a, b)| a != b)
            .count();
        assert!(differing > 500, "text should mark a good number of pixels, got {differing}");
    }

    #[test]
    fn opacity_controls_how_much_shows_through() {
        let Some(font) = test_font() else { return };
        let base = photo(400, 300);
        let make = |opacity: f32| {
            let mark = ImageMark::Text { text: "DRAFT".into(), font: font.clone() };
            let options = ImageWatermarkOptions {
                placement: Placement::Centre,
                opacity,
                scale: 0.8,
                ..Default::default()
            };
            decode(&watermark(&base, &mark, &options).unwrap())
        };
        let original = decode(&base);
        let faint = make(0.15);
        let solid = make(1.0);

        let drift = |img: &RgbaImage| -> u64 {
            original
                .pixels()
                .zip(img.pixels())
                .map(|(a, b)| (a.0[0] as i32 - b.0[0] as i32).unsigned_abs() as u64)
                .sum()
        };
        assert!(drift(&solid) > drift(&faint) * 2, "a solid mark should change far more");
    }

    #[test]
    fn rotation_grows_the_stamp_without_clipping_it() {
        let stamp = RgbaImage::from_pixel(100, 20, Rgba([255, 255, 255, 255]));
        let turned = rotate_rgba(&stamp, 45.0);
        // A 100×20 box turned 45° needs about 85 on each side.
        assert!(turned.width() >= 84 && turned.height() >= 84, "{:?}", turned.dimensions());

        // The corners of the new canvas are outside the rotated box, so clear.
        assert_eq!(turned.get_pixel(0, 0).0[3], 0);
        // And the centre is still solid.
        let (cx, cy) = (turned.width() / 2, turned.height() / 2);
        assert_eq!(turned.get_pixel(cx, cy).0[3], 255);
    }

    #[test]
    fn tiling_covers_the_whole_image() {
        let Some(font) = test_font() else { return };
        let base = photo(600, 400);
        let mark = ImageMark::Text { text: "COPY".into(), font };
        let options = ImageWatermarkOptions {
            placement: Placement::Tiled,
            opacity: 1.0,
            scale: 0.15,
            format: OutputFormat::Png,
            ..Default::default()
        };
        let out = watermark(&base, &mark, &options).unwrap();
        let before = decode(&base);
        let after = decode(&out);

        // Each quadrant should have been touched.
        for (qx, qy) in [(0u32, 0u32), (300, 0), (0, 200), (300, 200)] {
            let touched = (qx..qx + 300)
                .step_by(7)
                .flat_map(|x| (qy..qy + 200).step_by(7).map(move |y| (x, y)))
                .any(|(x, y)| before.get_pixel(x, y) != after.get_pixel(x, y));
            assert!(touched, "quadrant at {qx},{qy} was missed");
        }
    }

    #[test]
    fn transparency_in_the_base_survives() {
        let mut rgba = RgbaImage::new(100, 100);
        for (x, _, p) in rgba.enumerate_pixels_mut() {
            *p = Rgba([10, 20, 30, if x < 50 { 0 } else { 255 }]);
        }
        let mut src = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut src)
                .write_image(rgba.as_raw(), 100, 100, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        let logo = testkit::encode_jpeg(&testkit::photo(20, 20), 90);
        let options = ImageWatermarkOptions {
            placement: Placement::BottomRight,
            format: OutputFormat::Keep,
            ..Default::default()
        };
        let out = watermark(&src, &ImageMark::Logo(logo), &options).unwrap();
        assert_eq!(FileKind::sniff(&out), FileKind::Png, "alpha must not force a JPEG");

        let after = decode(&out);
        assert_eq!(after.get_pixel(5, 5).0[3], 0, "the clear half stays clear");
    }

    #[test]
    fn bad_settings_are_refused() {
        let base = photo(100, 100);
        let logo = testkit::encode_jpeg(&testkit::photo(10, 10), 90);
        let loud = ImageWatermarkOptions { opacity: 2.0, ..Default::default() };
        assert!(watermark(&base, &ImageMark::Logo(logo.clone()), &loud).is_err());
        let huge = ImageWatermarkOptions { scale: 4.0, ..Default::default() };
        assert!(watermark(&base, &ImageMark::Logo(logo), &huge).is_err());

        let blank = ImageMark::Text { text: "  ".into(), font: vec![0; 4] };
        assert!(watermark(&base, &blank, &ImageWatermarkOptions::default()).is_err());
    }

    #[test]
    fn a_font_that_is_not_a_font_says_so() {
        let base = photo(100, 100);
        let mark = ImageMark::Text { text: "hello".into(), font: b"not a font".to_vec() };
        let err = watermark(&base, &mark, &ImageWatermarkOptions::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("font"), "{err}");
    }
}
