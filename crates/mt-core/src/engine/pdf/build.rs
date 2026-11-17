//! Building PDFs out of images.
//!
//! This is the other half of the form-filing problem. "Upload your documents as
//! a single PDF, under 2 MB" means photographing four pages with a phone and
//! somehow producing one file — which is why it's worth doing properly:
//! sensible page sizes, no stretching, and a resolution cap so four 12-megapixel
//! photos don't become a 40 MB document.
//!
//! JPEG input is embedded byte-for-byte when nothing needs changing, so a photo
//! that's already the right size is never re-encoded and never loses anything.

use image::{imageops::FilterType, DynamicImage};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::format::FileKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    /// The page takes the shape of the image, so nothing is letterboxed.
    FitImage,
    #[default]
    A4,
    Letter,
    Legal,
}

impl PageSize {
    /// Width and height in points, portrait.
    fn points(self) -> Option<(f32, f32)> {
        match self {
            PageSize::FitImage => None,
            PageSize::A4 => Some((595.28, 841.89)),
            PageSize::Letter => Some((612.0, 792.0)),
            PageSize::Legal => Some((612.0, 1008.0)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Follow each image: a landscape photo gets a landscape page.
    #[default]
    Auto,
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagesToPdfOptions {
    pub page_size: PageSize,
    pub orientation: Orientation,
    /// White space around the image, in points. 72 is one inch.
    pub margin_pt: f32,
    /// Quality used only when an image has to be re-encoded.
    pub jpeg_quality: u8,
    /// Cap the effective resolution. Phone photos are far past what any form
    /// needs, and this is where most of the size goes.
    pub max_dpi: Option<u16>,
}

impl Default for ImagesToPdfOptions {
    fn default() -> Self {
        ImagesToPdfOptions {
            page_size: PageSize::A4,
            orientation: Orientation::Auto,
            margin_pt: 18.0,
            jpeg_quality: 82,
            max_dpi: Some(200),
        }
    }
}

/// One image per page, in the order given.
pub fn images_to_pdf(images: &[Vec<u8>], opts: &ImagesToPdfOptions) -> Result<Vec<u8>> {
    if images.is_empty() {
        return Err(Error::invalid("no images to put in a PDF"));
    }

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut page_ids = Vec::new();

    for (index, bytes) in images.iter().enumerate() {
        let kind = FileKind::sniff(bytes);
        if !kind.is_image() {
            return Err(Error::unsupported(format!(
                "image {} is a {}, not something that can go on a page",
                index + 1,
                kind.label()
            )));
        }
        let decoded = image::load_from_memory(bytes)
            .map_err(|e| Error::invalid(format!("image {}: {e}", index + 1)))?;
        if decoded.width() == 0 || decoded.height() == 0 {
            return Err(Error::invalid(format!("image {} has no pixels", index + 1)));
        }

        let (page_w, page_h) = page_for(&decoded, opts);
        let margin = opts.margin_pt.max(0.0).min(page_w.min(page_h) / 2.0 - 1.0);
        let (draw_w, draw_h) = contain(
            decoded.width() as f32 / decoded.height() as f32,
            page_w - margin * 2.0,
            page_h - margin * 2.0,
        );

        // Resolution is decided by how big the image is *drawn*, so the cap is
        // applied here rather than against the raw pixel count.
        let (embed, embed_w, embed_h, gray) = prepare(&decoded, bytes, kind, draw_w, opts)?;

        let image_id = doc.add_object(super::images::image_stream(embed, embed_w, embed_h, gray));

        let x = (page_w - draw_w) / 2.0;
        let y = (page_h - draw_h) / 2.0;
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        Object::Real(draw_w),
                        Object::Real(0.0),
                        Object::Real(0.0),
                        Object::Real(draw_h),
                        Object::Real(x),
                        Object::Real(y),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().map_err(|e| Error::other(format!("page content: {e}")))?,
        ));

        page_ids.push(doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "MediaBox" => vec![
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(page_w),
                Object::Real(page_h),
            ],
        }));
    }

    let count = page_ids.len() as i64;
    let kids: Vec<Object> = page_ids.into_iter().map(Object::Reference).collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);

    doc.compress();
    super::lossless::save_compact(&mut doc)
}

/// Decide the page size for one image.
fn page_for(image: &DynamicImage, opts: &ImagesToPdfOptions) -> (f32, f32) {
    let landscape_image = image.width() > image.height();

    let Some((portrait_w, portrait_h)) = opts.page_size.points() else {
        // Fit the page to the image at a comfortable size: the longest edge
        // becomes 10 inches, which prints sensibly and keeps the aspect exact.
        let long = 720.0f32;
        let aspect = image.width() as f32 / image.height() as f32;
        return if landscape_image { (long, long / aspect) } else { (long * aspect, long) };
    };

    let landscape_page = match opts.orientation {
        Orientation::Auto => landscape_image,
        Orientation::Portrait => false,
        Orientation::Landscape => true,
    };
    if landscape_page {
        (portrait_h, portrait_w)
    } else {
        (portrait_w, portrait_h)
    }
}

/// Largest box with the given aspect ratio that fits inside the available area.
fn contain(aspect: f32, avail_w: f32, avail_h: f32) -> (f32, f32) {
    let w = avail_w.min(avail_h * aspect);
    let h = w / aspect.max(0.0001);
    (w.max(1.0), h.max(1.0))
}

/// Produce the bytes to embed, resizing only when the resolution cap demands it.
///
/// Returns `(jpeg, width, height, grayscale)`.
fn prepare(
    decoded: &DynamicImage,
    original: &[u8],
    kind: FileKind,
    draw_w_pt: f32,
    opts: &ImagesToPdfOptions,
) -> Result<(Vec<u8>, u32, u32, bool)> {
    let gray = matches!(decoded, DynamicImage::ImageLuma8(_) | DynamicImage::ImageLuma16(_));

    let target_pixels = opts.max_dpi.and_then(|dpi| {
        let inches = draw_w_pt / 72.0;
        let allowed = (inches * dpi as f32).round() as u32;
        (allowed > 0 && allowed < decoded.width()).then_some(allowed)
    });

    match target_pixels {
        Some(width) => {
            let height =
                ((decoded.height() as f32 * width as f32 / decoded.width() as f32).round() as u32)
                    .max(1);
            let resized = decoded.resize_exact(width, height, FilterType::Lanczos3);
            let jpeg = super::images::encode_jpeg_for_pdf(&resized, opts.jpeg_quality, gray)?;
            Ok((jpeg, width, height, gray))
        }
        // Nothing to change, and it's already a JPEG: embed the exact bytes.
        // Re-encoding here would throw away quality for no reason at all.
        None if kind == FileKind::Jpeg => {
            Ok((original.to_vec(), decoded.width(), decoded.height(), gray))
        }
        None => {
            let jpeg = super::images::encode_jpeg_for_pdf(decoded, opts.jpeg_quality, gray)?;
            Ok((jpeg, decoded.width(), decoded.height(), gray))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdf::{inspect, page_summaries};
    use crate::testkit;

    fn photo_jpeg(w: u32, h: u32) -> Vec<u8> {
        testkit::encode_jpeg(&testkit::photo(w, h), 90)
    }

    #[test]
    fn makes_one_page_per_image() {
        let images = vec![photo_jpeg(800, 600), photo_jpeg(600, 800), photo_jpeg(400, 400)];
        let pdf = images_to_pdf(&images, &ImagesToPdfOptions::default()).unwrap();
        let pages = page_summaries(&pdf).unwrap();
        assert_eq!(pages.len(), 3);
        assert_eq!(inspect(&pdf).unwrap().images.len(), 3);
    }

    #[test]
    fn orientation_follows_each_image() {
        let images = vec![photo_jpeg(1200, 600), photo_jpeg(600, 1200)];
        let pdf = images_to_pdf(&images, &ImagesToPdfOptions::default()).unwrap();
        let pages = page_summaries(&pdf).unwrap();
        assert!(pages[0].width_pt > pages[0].height_pt, "a wide photo gets a wide page");
        assert!(pages[1].height_pt > pages[1].width_pt, "a tall photo gets a tall page");
    }

    #[test]
    fn forcing_portrait_overrides_the_image() {
        let opts = ImagesToPdfOptions { orientation: Orientation::Portrait, ..Default::default() };
        let pdf = images_to_pdf(&[photo_jpeg(1200, 600)], &opts).unwrap();
        let page = &page_summaries(&pdf).unwrap()[0];
        assert!(page.height_pt > page.width_pt);
        assert_eq!(page.label, "A4 portrait");
    }

    #[test]
    fn the_image_is_never_stretched() {
        let images = vec![photo_jpeg(1000, 500)];
        let pdf = images_to_pdf(&images, &ImagesToPdfOptions::default()).unwrap();
        let report = inspect(&pdf).unwrap();
        let img = &report.images[0];
        // Pixel aspect and drawn aspect must agree, or the picture is distorted.
        let pixel_aspect = img.width as f32 / img.height as f32;
        assert!(
            (pixel_aspect - 2.0).abs() < 0.02,
            "pixel aspect drifted: {pixel_aspect}"
        );
    }

    #[test]
    fn an_unchanged_jpeg_is_embedded_byte_for_byte() {
        let jpeg = photo_jpeg(500, 400);
        // No resolution cap, so there is no reason to touch the bytes.
        let opts = ImagesToPdfOptions { max_dpi: None, ..Default::default() };
        let pdf = images_to_pdf(&[jpeg.clone()], &opts).unwrap();
        assert!(
            find_bytes(&pdf, &jpeg),
            "the original JPEG should appear in the file untouched"
        );
    }

    #[test]
    fn the_resolution_cap_shrinks_oversized_photos() {
        let big = photo_jpeg(4000, 3000);
        let capped =
            images_to_pdf(&[big.clone()], &ImagesToPdfOptions { max_dpi: Some(150), ..Default::default() })
                .unwrap();
        let uncapped =
            images_to_pdf(&[big], &ImagesToPdfOptions { max_dpi: None, ..Default::default() })
                .unwrap();
        assert!(
            capped.len() < uncapped.len() / 2,
            "capping should save real size: {} vs {}",
            capped.len(),
            uncapped.len()
        );
        let dpi = inspect(&capped).unwrap().images[0].dpi.unwrap();
        assert!(dpi <= 152.0, "should land at the cap, got {dpi}");
    }

    #[test]
    fn fit_image_pages_match_the_photo_shape() {
        let opts = ImagesToPdfOptions {
            page_size: PageSize::FitImage,
            margin_pt: 0.0,
            ..Default::default()
        };
        let pdf = images_to_pdf(&[photo_jpeg(1000, 500)], &opts).unwrap();
        let page = &page_summaries(&pdf).unwrap()[0];
        let aspect = page.width_pt / page.height_pt;
        assert!((aspect - 2.0).abs() < 0.02, "page aspect should match the image: {aspect}");
    }

    #[test]
    fn mixed_formats_all_land_on_pages() {
        let images = vec![photo_jpeg(400, 300), testkit::tiny_png()];
        let pdf = images_to_pdf(&images, &ImagesToPdfOptions::default()).unwrap();
        assert_eq!(page_summaries(&pdf).unwrap().len(), 2);
    }

    #[test]
    fn refuses_things_that_are_not_images() {
        let err = images_to_pdf(&[b"%PDF-1.7\n".to_vec()], &ImagesToPdfOptions::default());
        assert!(err.is_err());
        assert!(images_to_pdf(&[], &ImagesToPdfOptions::default()).is_err());
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
}
