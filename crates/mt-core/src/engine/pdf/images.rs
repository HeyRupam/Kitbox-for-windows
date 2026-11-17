//! Reading and rewriting the image XObjects inside a PDF.
//!
//! The traps here are the ones that make naive PDF compressors produce
//! discoloured or corrupt documents, so each is handled explicitly: CMYK data
//! is converted rather than reinterpreted, indexed and spot-colour images are
//! left alone entirely, and every dictionary entry that describes the old
//! stream (`Filter`, `ColorSpace`, `BitsPerComponent`, `DecodeParms`) is
//! rewritten to match the new one.

use image::{DynamicImage, ImageEncoder};
use lopdf::{Document, Object, ObjectId, Stream};

use crate::error::{Error, Result};

/// Decode an image XObject into pixels, or `None` if this engine shouldn't
/// touch it.
pub fn decode_image(doc: &Document, id: ObjectId) -> Option<DynamicImage> {
    let Ok(Object::Stream(stream)) = doc.get_object(id) else { return None };
    let filters: Vec<String> = stream
        .filters()
        .map(|fs| fs.into_iter().map(|f| String::from_utf8_lossy(f).into_owned()).collect())
        .unwrap_or_default();

    if filters.iter().any(|f| f == "DCTDecode") {
        // Already a JPEG: hand the bytes straight to the decoder.
        return image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg).ok();
    }
    if filters.iter().any(|f| f != "FlateDecode" && f != "LZWDecode") {
        return None;
    }

    let width = stream.dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let height = stream.dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    let bpc = stream.dict.get(b"BitsPerComponent").ok()?.as_i64().ok()?;
    if bpc != 8 || width == 0 || height == 0 {
        return None;
    }

    let raw = stream.decompressed_content().ok()?;
    let pixels = (width as usize).checked_mul(height as usize)?;
    match raw.len().checked_div(pixels) {
        Some(1) if raw.len() == pixels => {
            image::GrayImage::from_raw(width, height, raw).map(DynamicImage::ImageLuma8)
        }
        Some(3) if raw.len() == pixels * 3 => {
            image::RgbImage::from_raw(width, height, raw).map(DynamicImage::ImageRgb8)
        }
        Some(4) if raw.len() == pixels * 4 => {
            // Four components in a PDF image is CMYK, not RGBA. Converting is
            // the only safe move — reinterpreting the bytes would invert the
            // colours of every such image.
            Some(cmyk_to_rgb(width, height, &raw))
        }
        _ => None,
    }
}

/// PDF stores CMYK as ink coverage, so 0 is no ink. Some encoders write it
/// inverted and flag that with a `/Decode` array; this handles the common,
/// non-inverted case and anything else is caught by the colour-space filter in
/// `inspect`.
fn cmyk_to_rgb(width: u32, height: u32, raw: &[u8]) -> DynamicImage {
    let mut out = image::RgbImage::new(width, height);
    for (i, pixel) in out.pixels_mut().enumerate() {
        let base = i * 4;
        let c = raw[base] as f32 / 255.0;
        let m = raw[base + 1] as f32 / 255.0;
        let y = raw[base + 2] as f32 / 255.0;
        let k = raw[base + 3] as f32 / 255.0;
        let to8 = |v: f32| (255.0 * (1.0 - v) * (1.0 - k)).round().clamp(0.0, 255.0) as u8;
        *pixel = image::Rgb([to8(c), to8(m), to8(y)]);
    }
    DynamicImage::ImageRgb8(out)
}

/// Encode pixels as the JPEG that will go into the PDF.
///
/// Kept separate from writing it so a whole document's images can be encoded in
/// parallel before any of them touch the (single-threaded) document.
pub fn encode_jpeg_for_pdf(pixels: &DynamicImage, quality: u8, grayscale: bool) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    if grayscale {
        let gray = pixels.to_luma8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.max(1)).write_image(
            gray.as_raw(),
            gray.width(),
            gray.height(),
            image::ExtendedColorType::L8,
        )?;
    } else {
        let rgb = pixels.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.max(1))
            .encode_image(&rgb)?;
    }
    Ok(buf)
}

/// Replace an image XObject with a JPEG of the given pixels.
pub fn replace_image(
    doc: &mut Document,
    id: ObjectId,
    pixels: &DynamicImage,
    quality: u8,
    grayscale: bool,
) -> Result<()> {
    let jpeg = encode_jpeg_for_pdf(pixels, quality, grayscale)?;
    apply_image(doc, id, jpeg, pixels.width(), pixels.height(), grayscale)
}

/// Write already-encoded JPEG bytes over an existing image XObject.
pub fn apply_image(
    doc: &mut Document,
    id: ObjectId,
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
    grayscale: bool,
) -> Result<()> {
    let color_space: &str = if grayscale { "DeviceGray" } else { "DeviceRGB" };
    let buf = jpeg;

    let Ok(Object::Stream(stream)) = doc.get_object_mut(id) else {
        return Err(Error::other("image object vanished"));
    };

    stream.dict.set("Width", width as i64);
    stream.dict.set("Height", height as i64);
    stream.dict.set("ColorSpace", Object::Name(color_space.as_bytes().to_vec()));
    stream.dict.set("BitsPerComponent", 8i64);
    stream.dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    // These described the stream we just threw away.
    stream.dict.remove(b"DecodeParms");
    stream.dict.remove(b"DecodeParams");
    stream.dict.remove(b"Decode");
    stream.set_content(buf);
    // Already compressed; a Flate wrapper would only make it bigger.
    stream.allows_compression = false;

    Ok(())
}

/// Resize to the pixel width that renders at `target_dpi`, never upwards.
pub fn pixels_for_dpi(
    pixels: &DynamicImage,
    current_dpi: Option<f32>,
    target_dpi: f32,
) -> Option<(u32, u32)> {
    let current = current_dpi?;
    if !current.is_finite() || current <= target_dpi {
        return None;
    }
    let factor = target_dpi / current;
    let w = ((pixels.width() as f32 * factor).round() as u32).max(1);
    let h = ((pixels.height() as f32 * factor).round() as u32).max(1);
    if w >= pixels.width() {
        return None;
    }
    Some((w, h))
}

/// A fresh image stream, used by the tests and by JPG-to-PDF later.
pub fn image_stream(jpeg: Vec<u8>, width: u32, height: u32, gray: bool) -> Stream {
    use lopdf::dictionary;
    Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width as i64,
            "Height" => height as i64,
            "ColorSpace" => if gray { "DeviceGray" } else { "DeviceRGB" },
            "BitsPerComponent" => 8i64,
            "Filter" => "DCTDecode",
        },
        jpeg,
    )
    .with_compression(false)
}

/// Embed an RGBA image, with its transparency intact.
///
/// The watermark path goes through JPEG, which has no alpha — fine for a logo
/// at 15% opacity, useless for a signature. A signature stored as a white
/// rectangle covers the line it is meant to sit on, and every form has a line.
///
/// Transparency in PDF is a second image: a `/SMask` in DeviceGray holding the
/// alpha channel, the same size as the colour data. Both streams go in raw and
/// are deflated by `Document::compress` on the way out.
pub fn rgba_image_stream(doc: &mut Document, image: &image::RgbaImage) -> ObjectId {
    use lopdf::dictionary;
    let (w, h) = (image.width(), image.height());

    let mut colour = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    for pixel in image.pixels() {
        colour.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
    }

    let mask = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8i64,
        },
        alpha,
    ));

    doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8i64,
            "SMask" => mask,
        },
        colour,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn round_trips_a_jpeg_image() {
        let pdf = testkit::pdf_with_image(400, 300, 288.0, 216.0, 85);
        let doc = Document::load_mem(&pdf).unwrap();
        let id = *doc
            .objects
            .iter()
            .find(|(_, o)| matches!(o, Object::Stream(s) if s.dict.get(b"Subtype").ok().and_then(|x| x.as_name().ok()) == Some(b"Image".as_ref())))
            .unwrap()
            .0;
        let decoded = decode_image(&doc, id).expect("should decode");
        assert_eq!((decoded.width(), decoded.height()), (400, 300));
    }

    #[test]
    fn replacing_updates_every_dictionary_entry() {
        let pdf = testkit::pdf_with_image(400, 300, 288.0, 216.0, 85);
        let mut doc = Document::load_mem(&pdf).unwrap();
        let id = *doc
            .objects
            .iter()
            .find(|(_, o)| matches!(o, Object::Stream(s) if s.dict.get(b"Subtype").ok().and_then(|x| x.as_name().ok()) == Some(b"Image".as_ref())))
            .unwrap()
            .0;

        let small = testkit::photo(100, 75);
        replace_image(&mut doc, id, &small, 60, true).unwrap();

        let Object::Stream(stream) = doc.get_object(id).unwrap() else { panic!() };
        assert_eq!(stream.dict.get(b"Width").unwrap().as_i64().unwrap(), 100);
        assert_eq!(stream.dict.get(b"Height").unwrap().as_i64().unwrap(), 75);
        assert_eq!(stream.dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
        assert_eq!(stream.dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        assert!(stream.dict.get(b"DecodeParms").is_err());
        // And the new bytes must still be a readable JPEG of the right size.
        let back = image::load_from_memory(&stream.content).unwrap();
        assert_eq!((back.width(), back.height()), (100, 75));
    }

    #[test]
    fn dpi_targeting_never_upscales() {
        let img = testkit::photo(600, 600);
        assert_eq!(pixels_for_dpi(&img, Some(300.0), 150.0), Some((300, 300)));
        assert_eq!(pixels_for_dpi(&img, Some(96.0), 150.0), None, "already below target");
        assert_eq!(pixels_for_dpi(&img, None, 150.0), None, "unknown DPI means leave it");
    }

    #[test]
    fn cmyk_is_converted_not_reinterpreted() {
        // Pure cyan ink: c=255, m=y=k=0 → bright cyan, not a red-tinted mess.
        let raw = vec![255u8, 0, 0, 0];
        let img = cmyk_to_rgb(1, 1, &raw);
        let p = img.to_rgb8();
        let px = p.get_pixel(0, 0);
        assert_eq!(px.0[0], 0, "no red where cyan ink is full");
        assert_eq!(px.0[1], 255);
        assert_eq!(px.0[2], 255);
    }
}
