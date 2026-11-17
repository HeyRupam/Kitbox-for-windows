//! Cropping pages.
//!
//! # What cropping a PDF actually does
//!
//! It sets `/CropBox`, which tells readers which part of the page to show. The
//! content outside that box is **still in the file** — select-all and copy will
//! pick it up, and another tool can widen the box again. That is how every PDF
//! cropper works, and it is fine for trimming scan borders or tightening
//! margins for print.
//!
//! It is *not* a way to remove something from a document. [`CropReport`] says
//! so on every result, because someone cropping a bank statement to hide an
//! account number deserves to know before they send it.
//!
//! Margins are given the way the reader sees the page, so `/Rotate` is mapped
//! back to page space — "trim 20pt off the top" means the top as displayed,
//! not whichever edge that happens to be in the file.

use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use super::pages::{materialise_inherited, PageSelection};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CropSpec {
    /// Points to trim from each displayed edge.
    Margins { top: f32, right: f32, bottom: f32, left: f32 },
    /// Fraction of each edge to trim, 0.0–0.49.
    Percent { top: f32, right: f32, bottom: f32, left: f32 },
    /// Shrink each page to the ink on it. Works on scanned pages, where the
    /// content is an image whose borders can be measured.
    Auto { padding: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropReport {
    pub pages_cropped: u32,
    pub total_pages: u32,
    /// Pages the automatic crop could not measure, by number.
    pub skipped: Vec<u32>,
    /// Always set. Cropping hides content, it does not remove it.
    pub warning: String,
}

impl CropReport {
    fn new(total: u32) -> Self {
        CropReport {
            pages_cropped: 0,
            total_pages: total,
            skipped: Vec::new(),
            warning: "Cropping sets the area readers display. Anything outside it is still in \
                      the file and can be recovered, so don't use it to hide information."
                .into(),
        }
    }

    pub fn note(&self) -> Option<String> {
        if self.skipped.is_empty() {
            return None;
        }
        Some(format!(
            "{} page{} had no measurable content, so {} left alone.",
            self.skipped.len(),
            if self.skipped.len() == 1 { "" } else { "s" },
            if self.skipped.len() == 1 { "it was" } else { "they were" }
        ))
    }
}

pub fn crop(
    bytes: &[u8],
    selection: Option<&PageSelection>,
    spec: &CropSpec,
) -> Result<(Vec<u8>, CropReport)> {
    validate(spec)?;

    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }

    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    if page_ids.is_empty() {
        return Err(Error::invalid("this PDF has no pages"));
    }
    let mut report = CropReport::new(page_ids.len() as u32);

    for (index, page_id) in page_ids.iter().enumerate() {
        let number = index as u32 + 1;
        if let Some(selection) = selection {
            if !selection.contains(number) {
                continue;
            }
        }
        materialise_inherited(&mut doc, *page_id);

        let Some(media) = media_rect(&doc, *page_id) else {
            report.skipped.push(number);
            continue;
        };
        let rotation = rotation_of(&doc, *page_id);

        let box_rect = match spec {
            CropSpec::Margins { top, right, bottom, left } => {
                apply_margins(media, rotation, *top, *right, *bottom, *left)
            }
            CropSpec::Percent { top, right, bottom, left } => {
                let (w, h) = (media.2 - media.0, media.3 - media.1);
                // Percentages are of the *displayed* edge, which swaps on a
                // quarter-turned page.
                let (dw, dh) = if rotation.rem_euclid(180) == 90 { (h, w) } else { (w, h) };
                apply_margins(
                    media,
                    rotation,
                    top * dh,
                    right * dw,
                    bottom * dh,
                    left * dw,
                )
            }
            CropSpec::Auto { padding } => match content_rect(&doc, *page_id) {
                Some(content) => pad_and_clamp(content, media, *padding),
                None => {
                    report.skipped.push(number);
                    continue;
                }
            },
        };

        let Some(rect) = box_rect else {
            report.skipped.push(number);
            continue;
        };

        if let Ok(page) = doc.get_dictionary_mut(*page_id) {
            page.set(
                "CropBox",
                Object::Array(vec![
                    Object::Real(rect.0),
                    Object::Real(rect.1),
                    Object::Real(rect.2),
                    Object::Real(rect.3),
                ]),
            );
            // A TrimBox left at the old size contradicts the new CropBox and
            // confuses print workflows, so it goes with it.
            page.remove(b"TrimBox");
            page.remove(b"BleedBox");
        }
        report.pages_cropped += 1;
    }

    Ok((super::lossless::save_compact(&mut doc)?, report))
}

fn validate(spec: &CropSpec) -> Result<()> {
    match spec {
        CropSpec::Margins { top, right, bottom, left } => {
            if [top, right, bottom, left].iter().any(|v| **v < 0.0) {
                return Err(Error::invalid("margins can't be negative"));
            }
        }
        CropSpec::Percent { top, right, bottom, left } => {
            if [top, right, bottom, left].iter().any(|v| **v < 0.0 || **v >= 0.5) {
                return Err(Error::invalid(
                    "each side has to be between 0 and 50% — more than that leaves no page",
                ));
            }
        }
        CropSpec::Auto { padding } => {
            if *padding < 0.0 {
                return Err(Error::invalid("padding can't be negative"));
            }
        }
    }
    Ok(())
}

/// Trim displayed margins, mapped onto the page's own axes.
fn apply_margins(
    media: (f32, f32, f32, f32),
    rotation: i32,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
) -> Option<(f32, f32, f32, f32)> {
    // Which page edge each displayed edge becomes once /Rotate is applied.
    let (page_left, page_right, page_bottom, page_top) = match rotation.rem_euclid(360) {
        90 => (top, bottom, left, right),
        180 => (right, left, top, bottom),
        270 => (bottom, top, right, left),
        _ => (left, right, bottom, top),
    };

    let rect = (
        media.0 + page_left,
        media.1 + page_bottom,
        media.2 - page_right,
        media.3 - page_top,
    );
    sane(rect, media)
}

fn pad_and_clamp(
    content: (f32, f32, f32, f32),
    media: (f32, f32, f32, f32),
    padding: f32,
) -> Option<(f32, f32, f32, f32)> {
    let rect = (
        (content.0 - padding).max(media.0),
        (content.1 - padding).max(media.1),
        (content.2 + padding).min(media.2),
        (content.3 + padding).min(media.3),
    );
    sane(rect, media)
}

/// A crop box has to sit inside the page and still have area.
fn sane(
    rect: (f32, f32, f32, f32),
    media: (f32, f32, f32, f32),
) -> Option<(f32, f32, f32, f32)> {
    let clamped = (
        rect.0.max(media.0),
        rect.1.max(media.1),
        rect.2.min(media.2),
        rect.3.min(media.3),
    );
    if clamped.2 - clamped.0 < 1.0 || clamped.3 - clamped.1 < 1.0 {
        return None;
    }
    Some(clamped)
}

/// The MediaBox as `(x0, y0, x1, y1)`, normalised so the corners are in order.
fn media_rect(doc: &Document, page_id: ObjectId) -> Option<(f32, f32, f32, f32)> {
    let value = inherited(doc, page_id, b"MediaBox")?;
    let Object::Array(items) = value else { return None };
    if items.len() < 4 {
        return None;
    }
    let n = |i: usize| items[i].as_float().ok();
    let (a, b, c, d) = (n(0)?, n(1)?, n(2)?, n(3)?);
    Some((a.min(c), b.min(d), a.max(c), b.max(d)))
}

fn rotation_of(doc: &Document, page_id: ObjectId) -> i32 {
    inherited(doc, page_id, b"Rotate").and_then(|o| o.as_float().ok()).unwrap_or(0.0) as i32
}

fn inherited(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut current = Some(page_id);
    for _ in 0..32 {
        let id = current?;
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(value) = dict.get(key) {
            return doc.dereference(value).ok().map(|(_, o)| o.clone());
        }
        current = match dict.get(b"Parent") {
            Ok(Object::Reference(parent)) => Some(*parent),
            _ => None,
        };
    }
    None
}

/// The ink on a page, in page coordinates.
///
/// Measured from the page's dominant image: decode it, find the rows and
/// columns that aren't background, and map that back through the rectangle the
/// image is drawn into. A page built from text and vectors has no image to
/// measure, so it comes back `None` and is reported as skipped rather than
/// cropped to a guess.
fn content_rect(doc: &Document, page_id: ObjectId) -> Option<(f32, f32, f32, f32)> {
    let rects = super::inspect::drawn_rects(doc, &[page_id]);
    let (image_id, (x, y, w, h)) = rects
        .into_iter()
        .filter(|(id, _)| {
            matches!(doc.get_object(*id), Ok(Object::Stream(s))
                if s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image".as_ref()))
        })
        .max_by(|a, b| {
            let area = |v: &(ObjectId, (f32, f32, f32, f32))| v.1 .2 * v.1 .3;
            area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal)
        })?;

    if w <= 1.0 || h <= 1.0 {
        return None;
    }
    let image = super::images::decode_image(doc, image_id)?;
    let (u0, v0, u1, v1) = ink_bounds(&image)?;

    // Image space has v growing downwards; page space has y growing up.
    Some((x + u0 * w, y + (1.0 - v1) * h, x + u1 * w, y + (1.0 - v0) * h))
}

/// Fractional bounds of the non-background pixels, as `(u0, v0, u1, v1)` with
/// the origin at the image's top left.
fn ink_bounds(image: &image::DynamicImage) -> Option<(f32, f32, f32, f32)> {
    let grey = image.to_luma8();
    let (w, h) = (grey.width(), grey.height());
    if w == 0 || h == 0 {
        return None;
    }

    // Take the background from the corners rather than assuming white: plenty
    // of scans are grey, cream or slightly blue.
    let corners = [
        grey.get_pixel(0, 0)[0],
        grey.get_pixel(w - 1, 0)[0],
        grey.get_pixel(0, h - 1)[0],
        grey.get_pixel(w - 1, h - 1)[0],
    ];
    let background = corners.iter().map(|v| *v as u32).sum::<u32>() / 4;
    // Generous enough to ignore scanner noise, tight enough to catch pale ink.
    let threshold = 28u32;

    let is_ink = |x: u32, y: u32| {
        let value = grey.get_pixel(x, y)[0] as i32;
        (value - background as i32).unsigned_abs() > threshold
    };
    // A row counts as content only if enough of it is ink, so a speck of dust
    // in the margin doesn't defeat the whole crop.
    let row_minimum = ((w as f32 * 0.004).ceil() as u32).max(2);
    let column_minimum = ((h as f32 * 0.004).ceil() as u32).max(2);

    let mut top = None;
    let mut bottom = None;
    for y in 0..h {
        let count = (0..w).filter(|x| is_ink(*x, y)).count() as u32;
        if count >= row_minimum {
            top.get_or_insert(y);
            bottom = Some(y);
        }
    }
    let (top, bottom) = (top?, bottom?);

    let mut left = None;
    let mut right = None;
    for x in 0..w {
        let count = (top..=bottom).filter(|y| is_ink(x, *y)).count() as u32;
        if count >= column_minimum {
            left.get_or_insert(x);
            right = Some(x);
        }
    }
    let (left, right) = (left?, right?);

    Some((
        left as f32 / w as f32,
        top as f32 / h as f32,
        (right + 1) as f32 / w as f32,
        (bottom + 1) as f32 / h as f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdf::{page_count, rotate, PageSelection};
    use crate::testkit;

    fn crop_box(bytes: &[u8], page: usize) -> Option<(f32, f32, f32, f32)> {
        let doc = Document::load_mem(bytes).unwrap();
        let id = doc.page_iter().nth(page)?;
        let value = doc.get_dictionary(id).ok()?.get(b"CropBox").ok()?;
        let Object::Array(items) = value else { return None };
        let n = |i: usize| items[i].as_float().ok();
        Some((n(0)?, n(1)?, n(2)?, n(3)?))
    }

    #[test]
    fn margins_come_off_each_edge() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        let spec = CropSpec::Margins { top: 10.0, right: 20.0, bottom: 30.0, left: 40.0 };
        let (out, report) = crop(&src, None, &spec).unwrap();

        assert_eq!(report.pages_cropped, 2);
        assert_eq!(page_count(&out).unwrap(), 2);
        // The fixture pages are 612 × 792 at the origin.
        let rect = crop_box(&out, 0).unwrap();
        assert_eq!(rect, (40.0, 30.0, 592.0, 782.0));
    }

    #[test]
    fn percentages_are_of_the_page_size() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let spec = CropSpec::Percent { top: 0.1, right: 0.0, bottom: 0.1, left: 0.0 };
        let (out, _) = crop(&src, None, &spec).unwrap();
        let rect = crop_box(&out, 0).unwrap();
        assert!((rect.1 - 79.2).abs() < 0.1, "bottom 10% of 792: {rect:?}");
        assert!((rect.3 - 712.8).abs() < 0.1, "top 10% of 792: {rect:?}");
    }

    #[test]
    fn only_the_selected_pages_are_cropped() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        let selection = PageSelection::parse("2", 3).unwrap();
        let spec = CropSpec::Margins { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 };
        let (out, report) = crop(&src, Some(&selection), &spec).unwrap();

        assert_eq!(report.pages_cropped, 1);
        assert!(crop_box(&out, 0).is_none(), "page 1 should be untouched");
        assert!(crop_box(&out, 1).is_some());
        assert!(crop_box(&out, 2).is_none());
    }

    #[test]
    fn a_rotated_page_trims_the_edge_the_reader_sees() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let selection = PageSelection::parse("1", 1).unwrap();
        let (turned, _) = rotate(&src, &selection, 90).unwrap();

        // Trim only the displayed top. On a 90° page that is the page's left.
        let spec = CropSpec::Margins { top: 50.0, right: 0.0, bottom: 0.0, left: 0.0 };
        let (out, _) = crop(&turned, None, &spec).unwrap();
        let rect = crop_box(&out, 0).unwrap();
        assert_eq!(rect.0, 50.0, "the left edge should move in: {rect:?}");
        assert_eq!(rect.3, 792.0, "and the page top should be untouched");
    }

    #[test]
    fn auto_crop_finds_the_edges_of_a_bordered_scan() {
        // A page whose image is mostly white with a block of ink in the middle.
        let mut canvas = image::RgbImage::from_pixel(400, 400, image::Rgb([255, 255, 255]));
        for y in 100..300 {
            for x in 80..320 {
                canvas.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let jpeg = {
            let mut buf = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 92)
                .encode_image(&canvas)
                .unwrap();
            buf
        };
        let src = testkit::pdf_with_jpeg(&jpeg, 400, 400, 400.0, 400.0);

        let (out, report) = crop(&src, None, &CropSpec::Auto { padding: 0.0 }).unwrap();
        assert_eq!(report.pages_cropped, 1);
        assert!(report.skipped.is_empty());

        let rect = crop_box(&out, 0).unwrap();
        // The image is drawn 400pt wide at the origin, so a pixel is a point.
        assert!((rect.0 - 80.0).abs() < 6.0, "left: {rect:?}");
        assert!((rect.2 - 320.0).abs() < 6.0, "right: {rect:?}");
        // Image rows run downwards, page y runs up.
        assert!((rect.1 - 100.0).abs() < 6.0, "bottom: {rect:?}");
        assert!((rect.3 - 300.0).abs() < 6.0, "top: {rect:?}");
    }

    #[test]
    fn auto_crop_reports_pages_it_cannot_measure() {
        let src = testkit::pdf_text_only();
        let (out, report) = crop(&src, None, &CropSpec::Auto { padding: 4.0 }).unwrap();
        assert_eq!(report.pages_cropped, 0);
        assert_eq!(report.skipped, vec![1]);
        assert!(report.note().unwrap().contains("left alone"));
        assert!(crop_box(&out, 0).is_none(), "a page it can't measure stays as it was");
    }

    #[test]
    fn every_result_says_the_content_is_still_there() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let spec = CropSpec::Margins { top: 5.0, right: 5.0, bottom: 5.0, left: 5.0 };
        let (_, report) = crop(&src, None, &spec).unwrap();
        assert!(report.warning.contains("still in"), "{}", report.warning);
        assert!(report.warning.contains("hide"), "{}", report.warning);
    }

    #[test]
    fn a_crop_that_would_leave_nothing_is_refused() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        // Wider than the page.
        let spec = CropSpec::Margins { top: 500.0, right: 0.0, bottom: 500.0, left: 0.0 };
        let (out, report) = crop(&src, None, &spec).unwrap();
        assert_eq!(report.pages_cropped, 0);
        assert_eq!(report.skipped, vec![1]);
        assert!(crop_box(&out, 0).is_none());

        assert!(crop(&src, None, &CropSpec::Margins { top: -1.0, right: 0.0, bottom: 0.0, left: 0.0 })
            .is_err());
        assert!(crop(
            &src,
            None,
            &CropSpec::Percent { top: 0.6, right: 0.0, bottom: 0.0, left: 0.0 }
        )
        .is_err());
    }

    #[test]
    fn cropping_leaves_the_page_content_alone() {
        let src = testkit::pdf_scan(2, 300, 400, 70);
        let spec = CropSpec::Margins { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 };
        let (out, _) = crop(&src, None, &spec).unwrap();
        let report = super::super::inspect::inspect(&out).unwrap();
        assert_eq!(report.images.len(), 2, "the scans are untouched");
    }
}
