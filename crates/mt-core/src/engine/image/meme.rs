//! Captions on a picture, in the usual shape.
//!
//! The look is a specific one and getting it wrong is obvious: heavy upright
//! capitals, white, with a thick black outline, centred top and bottom. The
//! outline is the part that matters — white text alone disappears into a pale
//! sky, and a drop shadow reads as a different thing entirely.
//!
//! The outline is made by spreading the letters' own coverage outwards and
//! filling that with black, then laying the white on top. Stroking each glyph
//! would be more faithful to how a type designer would do it and needs the
//! outline geometry, which the rasteriser here does not keep; spreading the
//! coverage gives the same result at the sizes this is used at.
//!
//! The font arrives as bytes from the caller, like every other font in this
//! crate: `mt-core` opens no files. The caller is expected to offer something
//! heavy — Impact is the traditional answer and is on every Windows machine.

use image::{imageops::FilterType, DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use super::watermark::rasterise;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeOptions {
    pub top: String,
    pub bottom: String,
    /// Shout it, which is the convention.
    pub uppercase: bool,
    /// Height of one line as a fraction of the picture's height, 0.03–0.25.
    pub size: f32,
    /// Outline thickness relative to the text height, 0.0–0.2.
    pub outline: f32,
    pub colour: [u8; 3],
    pub outline_colour: [u8; 3],
    /// Put the top caption on a white band above the picture instead of over
    /// it, which is the other common arrangement and keeps the image clear.
    pub band: bool,
}

impl Default for MemeOptions {
    fn default() -> Self {
        MemeOptions {
            top: String::new(),
            bottom: String::new(),
            uppercase: true,
            size: 0.11,
            outline: 0.06,
            colour: [255, 255, 255],
            outline_colour: [0, 0, 0],
            band: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeReport {
    pub width: u32,
    pub height: u32,
    /// How many lines each caption wrapped onto.
    pub top_lines: u32,
    pub bottom_lines: u32,
    pub warnings: Vec<String>,
}

/// Fraction of the width the text is allowed to fill before wrapping.
const USABLE: f32 = 0.94;

/// Put the captions on.
pub fn meme(
    bytes: &[u8],
    font: &[u8],
    options: &MemeOptions,
) -> Result<(DynamicImage, MemeReport)> {
    if options.top.trim().is_empty() && options.bottom.trim().is_empty() {
        return Err(Error::invalid("there is no caption to add"));
    }
    if !(0.03..=0.25).contains(&options.size) {
        return Err(Error::invalid("the text size has to be between 3% and 25% of the height"));
    }
    if !(0.0..=0.2).contains(&options.outline) {
        return Err(Error::invalid("the outline has to be between 0 and 20% of the text height"));
    }

    let source = image::load_from_memory(bytes)?;
    draw(&source, font, options)
}

/// The same, on an image already decoded.
pub fn draw(
    source: &DynamicImage,
    font: &[u8],
    options: &MemeOptions,
) -> Result<(DynamicImage, MemeReport)> {
    let (w, h) = (source.width(), source.height());
    if w < 32 || h < 32 {
        return Err(Error::invalid("that image is too small to caption"));
    }

    let shout = |text: &str| {
        if options.uppercase {
            text.to_uppercase()
        } else {
            text.to_string()
        }
    };
    let top = shout(options.top.trim());
    let bottom = shout(options.bottom.trim());

    let line_height = (h as f32 * options.size).max(8.0);
    let usable = w as f32 * USABLE;

    let top_lines = wrap(&top, font, line_height, usable)?;
    let bottom_lines = wrap(&bottom, font, line_height, usable)?;

    // A band pushes the picture down rather than covering it, so the canvas
    // grows and everything below moves.
    let band_height = if options.band && !top_lines.is_empty() {
        (line_height * 1.35 * top_lines.len() as f32).round() as u32
    } else {
        0
    };

    let mut canvas = RgbaImage::from_pixel(w, h + band_height, Rgba([255, 255, 255, 255]));
    let base = source.to_rgba8();
    image::imageops::replace(&mut canvas, &base, 0, band_height as i64);

    let mut warnings = Vec::new();
    let thickness = (line_height * options.outline).round() as u32;

    // Top: on the band if there is one, otherwise over the picture.
    let mut y = if band_height > 0 { line_height * 0.18 } else { line_height * 0.22 };
    for line in &top_lines {
        let stamp = stamp(line, font, line_height, thickness, options, band_height > 0)?;
        place(&mut canvas, &stamp, y.round() as i64);
        y += line_height * 1.18;
    }

    // Bottom: stacked up from the foot of the picture.
    let mut y = (h + band_height) as f32
        - line_height * 0.30
        - line_height * 1.18 * bottom_lines.len() as f32;
    for line in &bottom_lines {
        let stamp = stamp(line, font, line_height, thickness, options, false)?;
        place(&mut canvas, &stamp, y.round() as i64);
        y += line_height * 1.18;
    }

    let total = top_lines.len() + bottom_lines.len();
    if total as f32 * line_height * 1.18 > h as f32 * 0.8 {
        warnings.push(
            "The captions take up most of the picture. A smaller text size, or fewer words, \
             would leave something to look at."
                .into(),
        );
    }

    let report = MemeReport {
        width: canvas.width(),
        height: canvas.height(),
        top_lines: top_lines.len() as u32,
        bottom_lines: bottom_lines.len() as u32,
        warnings,
    };
    Ok((DynamicImage::ImageRgba8(canvas), report))
}

/// Break a caption into lines that fit.
fn wrap(text: &str, font: &[u8], line_height: f32, usable: f32) -> Result<Vec<String>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let width_of = |piece: &str| -> Result<f32> {
        let stamp = rasterise(piece, font, [255, 255, 255])?;
        // The rasteriser works at its own fixed size; scale its measurement
        // to the height actually being drawn.
        Ok(stamp.width() as f32 * (line_height / stamp.height() as f32))
    };

    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if width_of(&candidate)? <= usable || line.is_empty() {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    Ok(lines)
}

/// One line, outlined and scaled to the height it will be drawn at.
fn stamp(
    text: &str,
    font: &[u8],
    line_height: f32,
    thickness: u32,
    options: &MemeOptions,
    on_band: bool,
) -> Result<RgbaImage> {
    // On a white band the usual white-on-black reverses, or the words vanish.
    let fill = if on_band { options.outline_colour } else { options.colour };
    let edge = if on_band { options.colour } else { options.outline_colour };

    let drawn = rasterise(text, font, fill)?;
    let scale = line_height / drawn.height() as f32;
    let target_w = ((drawn.width() as f32 * scale).round() as u32).max(1);
    let target_h = ((drawn.height() as f32 * scale).round() as u32).max(1);
    let scaled = image::imageops::resize(&drawn, target_w, target_h, FilterType::Lanczos3);

    if thickness == 0 || on_band {
        return Ok(scaled);
    }
    Ok(outline(&scaled, thickness, edge))
}

/// Spread the letters' coverage outwards and fill it, then lay the text back
/// on top.
fn outline(text: &RgbaImage, thickness: u32, colour: [u8; 3]) -> RgbaImage {
    let pad = thickness;
    let (w, h) = (text.width() + pad * 2, text.height() + pad * 2);
    let mut out = RgbaImage::new(w, h);

    // The spread alpha: for each pixel, the strongest coverage within reach.
    let reach = thickness as i32;
    let squared = reach * reach;
    for y in 0..h {
        for x in 0..w {
            let mut strongest = 0u8;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    // Round, not square: a square spread gives the letters
                    // visible corners where there should be none.
                    if dx * dx + dy * dy > squared {
                        continue;
                    }
                    let sx = x as i32 - pad as i32 + dx;
                    let sy = y as i32 - pad as i32 + dy;
                    if sx < 0 || sy < 0 || sx >= text.width() as i32 || sy >= text.height() as i32 {
                        continue;
                    }
                    let a = text.get_pixel(sx as u32, sy as u32).0[3];
                    if a > strongest {
                        strongest = a;
                        if strongest == 255 {
                            break;
                        }
                    }
                }
                if strongest == 255 {
                    break;
                }
            }
            if strongest > 0 {
                out.put_pixel(x, y, Rgba([colour[0], colour[1], colour[2], strongest]));
            }
        }
    }

    // The letters themselves, over their own outline.
    for y in 0..text.height() {
        for x in 0..text.width() {
            let px = *text.get_pixel(x, y);
            if px.0[3] == 0 {
                continue;
            }
            let under = *out.get_pixel(x + pad, y + pad);
            let a = px.0[3] as f32 / 255.0;
            let mixed = |fore: u8, back: u8| (fore as f32 * a + back as f32 * (1.0 - a)) as u8;
            out.put_pixel(
                x + pad,
                y + pad,
                Rgba([
                    mixed(px.0[0], under.0[0]),
                    mixed(px.0[1], under.0[1]),
                    mixed(px.0[2], under.0[2]),
                    under.0[3].max(px.0[3]),
                ]),
            );
        }
    }
    out
}

/// Centre a stamp horizontally and composite it.
fn place(canvas: &mut RgbaImage, stamp: &RgbaImage, y: i64) {
    let x = (canvas.width() as i64 - stamp.width() as i64) / 2;
    for sy in 0..stamp.height() {
        let ty = y + sy as i64;
        if ty < 0 || ty >= canvas.height() as i64 {
            continue;
        }
        for sx in 0..stamp.width() {
            let tx = x + sx as i64;
            if tx < 0 || tx >= canvas.width() as i64 {
                continue;
            }
            let px = *stamp.get_pixel(sx, sy);
            if px.0[3] == 0 {
                continue;
            }
            let under = *canvas.get_pixel(tx as u32, ty as u32);
            let a = px.0[3] as f32 / 255.0;
            let mixed = |fore: u8, back: u8| (fore as f32 * a + back as f32 * (1.0 - a)) as u8;
            canvas.put_pixel(
                tx as u32,
                ty as u32,
                Rgba([
                    mixed(px.0[0], under.0[0]),
                    mixed(px.0[1], under.0[1]),
                    mixed(px.0[2], under.0[2]),
                    255,
                ]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// The same arrangement the watermark tests use: a font has to come from
    /// the machine, and a test that cannot find one says so rather than
    /// failing somewhere confusing.
    fn test_font() -> Option<Vec<u8>> {
        const CANDIDATES: &[&str] = &[
            r"C:\Windows\Fonts\impact.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ];
        CANDIDATES.iter().find_map(|path| std::fs::read(path).ok())
    }

    fn picture() -> DynamicImage {
        // Mid-grey: a white caption and its black outline are both visible
        // against it, so either going missing shows up.
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(400, 500, image::Rgb([128, 128, 128])))
    }

    #[test]
    fn a_caption_with_nothing_in_it_is_refused() {
        let Some(font) = test_font() else { return };
        let mut encoded = Vec::new();
        picture()
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .unwrap();
        assert!(meme(&encoded, &font, &MemeOptions::default()).is_err());

        let spaces = MemeOptions { top: "   ".into(), ..Default::default() };
        assert!(meme(&encoded, &font, &spaces).is_err());
    }

    #[test]
    fn silly_sizes_are_refused() {
        let Some(font) = test_font() else { return };
        let mut encoded = Vec::new();
        picture()
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Png)
            .unwrap();
        for options in [
            MemeOptions { top: "hi".into(), size: 0.9, ..Default::default() },
            MemeOptions { top: "hi".into(), size: 0.0, ..Default::default() },
            MemeOptions { top: "hi".into(), outline: 0.9, ..Default::default() },
        ] {
            assert!(meme(&encoded, &font, &options).is_err(), "{options:?}");
        }
    }

    #[test]
    fn the_caption_is_shouted_unless_told_otherwise() {
        let Some(font) = test_font() else { return };
        // Hard to read letters back off a bitmap, so this checks the two
        // renders differ — upper-case glyphs are a different width.
        let quiet =
            MemeOptions { top: "quiet please".into(), uppercase: false, ..Default::default() };
        let loud = MemeOptions { uppercase: true, ..quiet.clone() };
        let (a, _) = draw(&picture(), &font, &quiet).unwrap();
        let (b, _) = draw(&picture(), &font, &loud).unwrap();
        assert_ne!(a.to_rgb8().as_raw(), b.to_rgb8().as_raw());
    }

    #[test]
    fn a_long_caption_wraps_rather_than_running_off_the_edge() {
        let Some(font) = test_font() else { return };
        let options = MemeOptions {
            top: "a caption long enough that it cannot possibly fit on one single line".into(),
            ..Default::default()
        };
        let (out, report) = draw(&picture(), &font, &options).unwrap();
        assert!(report.top_lines > 1, "it should wrap: {report:?}");
        assert_eq!(out.width(), 400, "and never widen the picture");

        // Nothing should be painted in the outermost columns.
        let rgb = out.to_rgb8();
        for y in 0..rgb.height() {
            for x in [0u32, rgb.width() - 1] {
                assert_eq!(
                    rgb.get_pixel(x, y).0,
                    [128, 128, 128],
                    "text reached the edge at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn the_outline_puts_dark_pixels_around_the_light_ones() {
        // The outline is what makes this readable on a pale picture, and its
        // absence is the obvious way to get the look wrong.
        let Some(font) = test_font() else { return };
        let options = MemeOptions { top: "HELLO".into(), outline: 0.08, ..Default::default() };
        let (out, _) = draw(&picture(), &font, &options).unwrap();
        let rgb = out.to_rgb8();

        let light = rgb.pixels().filter(|p| p.0[0] > 230).count();
        let dark = rgb.pixels().filter(|p| p.0[0] < 40).count();
        assert!(light > 200, "the letters should be there: {light}");
        assert!(dark > 100, "and be outlined: {dark}");

        let bare = MemeOptions { outline: 0.0, ..options.clone() };
        let (plain, _) = draw(&picture(), &font, &bare).unwrap();
        let plain_dark = plain.to_rgb8().pixels().filter(|p| p.0[0] < 40).count();
        assert!(dark > plain_dark * 2, "an outline means far more dark pixels");
    }

    #[test]
    fn a_band_grows_the_canvas_instead_of_covering_the_picture() {
        let Some(font) = test_font() else { return };
        let options = MemeOptions { top: "ON A BAND".into(), band: true, ..Default::default() };
        let (out, report) = draw(&picture(), &font, &options).unwrap();

        assert_eq!(out.width(), 400);
        assert!(out.height() > 500, "the band adds height: {}", out.height());
        assert_eq!(report.height, out.height());

        // The picture itself is untouched: its first row is still grey, and
        // it has simply moved down.
        let rgb = out.to_rgb8();
        let moved = out.height() - 500;
        assert_eq!(rgb.get_pixel(5, moved + 5).0, [128, 128, 128]);
    }

    #[test]
    fn a_bottom_caption_sits_at_the_bottom() {
        let Some(font) = test_font() else { return };
        let options = MemeOptions { bottom: "DOWN HERE".into(), ..Default::default() };
        let (out, report) = draw(&picture(), &font, &options).unwrap();
        assert_eq!(report.bottom_lines, 1);
        assert_eq!(report.top_lines, 0);

        let rgb = out.to_rgb8();
        let painted = |from: u32, to: u32| {
            (from..to)
                .flat_map(|y| (0..rgb.width()).map(move |x| (x, y)))
                .filter(|(x, y)| rgb.get_pixel(*x, *y).0 != [128, 128, 128])
                .count()
        };
        assert_eq!(painted(0, 250), 0, "nothing in the top half");
        assert!(painted(250, 500) > 100, "and something in the bottom");
    }

    #[test]
    fn captions_that_swamp_the_picture_are_mentioned() {
        let Some(font) = test_font() else { return };
        let options = MemeOptions {
            top: "one two three four five six seven eight nine ten eleven twelve".into(),
            bottom: "thirteen fourteen fifteen sixteen seventeen eighteen nineteen".into(),
            size: 0.14,
            ..Default::default()
        };
        let (_, report) = draw(&picture(), &font, &options).unwrap();
        assert!(
            report.warnings.iter().any(|w| w.contains("most of the picture")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn a_picture_too_small_to_caption_is_refused() {
        let Some(font) = test_font() else { return };
        let tiny = DynamicImage::ImageRgb8(image::RgbImage::new(20, 20));
        let options = MemeOptions { top: "NO".into(), ..Default::default() };
        assert!(draw(&tiny, &font, &options).is_err());
    }

    #[test]
    fn the_options_cross_as_the_ui_writes_them() {
        let options: MemeOptions = serde_json::from_str(
            r#"{ "top": "A", "bottom": "B", "uppercase": true, "size": 0.11,
                  "outline": 0.06, "colour": [255,255,255],
                  "outlineColour": [0,0,0], "band": false }"#,
        )
        .expect("options");
        assert_eq!(options.top, "A");
        assert!(options.uppercase);
        assert_eq!(options.outline_colour, [0, 0, 0]);
    }

    #[test]
    fn a_real_photograph_round_trips() {
        let Some(font) = test_font() else { return };
        let photo = testkit::encode_jpeg(&testkit::photo(300, 200), 88);
        let options =
            MemeOptions { top: "TOP".into(), bottom: "BOTTOM".into(), ..Default::default() };
        let (out, report) = meme(&photo, &font, &options).unwrap();
        assert_eq!((out.width(), out.height()), (300, 200));
        assert_eq!((report.top_lines, report.bottom_lines), (1, 1));
    }
}
