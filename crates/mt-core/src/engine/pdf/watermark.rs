//! Stamping text or a logo across a PDF's pages.
//!
//! Text needs no embedded font: PDF readers are required to provide Helvetica,
//! so a watermark costs a few hundred bytes rather than a font program. The
//! trade is that the widths have to come from somewhere to centre the text, so
//! the standard Helvetica metrics are table below.
//!
//! The mark is drawn *over* the page, and the existing content is wrapped in
//! `q`/`Q` first. Without that, a page whose content stream leaves the graphics
//! state unbalanced — which is common in files produced by older software —
//! would drag the watermark's transform along with it and put the text
//! somewhere unpredictable.

use lopdf::{Document, ObjectId};
use serde::{Deserialize, Serialize};

pub use crate::engine::placement::Placement;

use super::overlay::{escape_text, stamp_page, text_width, FONT_NAME, IMAGE_NAME, STATE_NAME};
use super::pages::{materialise_inherited, media_box, PageSelection};
use crate::error::{Error, Result};
use crate::format::FileKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Mark {
    Text(String),
    /// JPEG or PNG bytes.
    Image(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkOptions {
    pub mark: Mark,
    pub placement: Placement,
    /// 0.0–1.0. Around 0.15 is a readable stamp that doesn't obscure the text.
    pub opacity: f32,
    /// Degrees anticlockwise. 45 is the classic diagonal.
    pub rotation: f32,
    /// Text size in points, or `None` to scale with the page width.
    pub font_size: Option<f32>,
    /// For image marks: width as a fraction of the page width.
    pub image_scale: f32,
    /// Red, green, blue, 0–255.
    pub colour: [u8; 3],
    /// Distance from the page edge in points, for the non-centre placements.
    pub margin: f32,
}

impl Default for WatermarkOptions {
    fn default() -> Self {
        WatermarkOptions {
            mark: Mark::Text("DRAFT".into()),
            placement: Placement::Centre,
            opacity: 0.15,
            rotation: 45.0,
            font_size: None,
            image_scale: 0.35,
            colour: [90, 90, 90],
            margin: 36.0,
        }
    }
}

/// Stamp the selected pages. `None` means every page.
pub fn watermark(
    bytes: &[u8],
    selection: Option<&PageSelection>,
    options: &WatermarkOptions,
) -> Result<Vec<u8>> {
    if let Mark::Text(text) = &options.mark {
        if text.trim().is_empty() {
            return Err(Error::invalid("the watermark has no text in it"));
        }
    }
    if !(0.0..=1.0).contains(&options.opacity) {
        return Err(Error::invalid("opacity has to be between 0 and 1"));
    }

    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }

    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    if page_ids.is_empty() {
        return Err(Error::invalid("this PDF has no pages"));
    }

    // One image object shared by every page it lands on, rather than a copy per
    // page — a logo on forty pages should cost one logo.
    let image_id = match &options.mark {
        Mark::Image(data) => Some(embed_image(&mut doc, data)?),
        Mark::Text(_) => None,
    };
    let image_size = match &options.mark {
        Mark::Image(data) => Some(
            image::load_from_memory(data)
                .map(|i| (i.width() as f32, i.height() as f32))
                .map_err(|e| Error::invalid(format!("could not read the logo: {e}")))?,
        ),
        Mark::Text(_) => None,
    };

    for (index, page_id) in page_ids.iter().enumerate() {
        let number = index as u32 + 1;
        if let Some(selection) = selection {
            if !selection.contains(number) {
                continue;
            }
        }
        materialise_inherited(&mut doc, *page_id);
        let (page_w, page_h) = media_box(&doc, *page_id).unwrap_or((612.0, 792.0));
        let content = draw(options, page_w, page_h, image_size);
        stamp_page(&mut doc, *page_id, &content, options.opacity, image_id)?;
    }

    super::lossless::save_compact(&mut doc)
}

/// Build the content stream that paints the mark.
fn draw(
    options: &WatermarkOptions,
    page_w: f32,
    page_h: f32,
    image_size: Option<(f32, f32)>,
) -> String {
    let [r, g, b] = options.colour;
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);

    // Size of the mark itself, used for placement.
    let (mark_w, mark_h, font_size) = match (&options.mark, image_size) {
        (Mark::Text(text), _) => {
            // Default to something that spans about half the page.
            let size = options
                .font_size
                .unwrap_or_else(|| (page_w * 0.5 / text_width(text, 1.0).max(0.1)).clamp(8.0, 200.0));
            (text_width(text, size), size * 0.72, size)
        }
        (Mark::Image(_), Some((iw, ih))) => {
            let w = page_w * options.image_scale.clamp(0.01, 1.0);
            (w, w * ih / iw.max(1.0), 0.0)
        }
        (Mark::Image(_), None) => (page_w * 0.3, page_h * 0.1, 0.0),
    };

    let mut out = String::new();
    out.push_str(&format!("q\n/{STATE_NAME} gs\n"));
    out.push_str(&format!("{rf:.3} {gf:.3} {bf:.3} rg\n"));

    let spots: Vec<(f32, f32)> = match options.placement {
        Placement::Tiled => tile_spots(page_w, page_h, mark_w, mark_h),
        other => vec![other.anchor(page_w, page_h, mark_w, mark_h, options.margin)],
    };

    for (cx, cy) in spots {
        let radians = options.rotation.to_radians();
        let (cos, sin) = (radians.cos(), radians.sin());
        out.push_str("q\n");
        // Rotate about the mark's own centre.
        out.push_str(&format!("{cos:.5} {sin:.5} {:.5} {cos:.5} {cx:.2} {cy:.2} cm\n", -sin));

        match &options.mark {
            Mark::Text(text) => {
                out.push_str("BT\n");
                out.push_str(&format!("/{FONT_NAME} {font_size:.2} Tf\n"));
                // Offset so the text's own centre sits on the anchor.
                out.push_str(&format!(
                    "1 0 0 1 {:.2} {:.2} Tm\n",
                    -mark_w / 2.0,
                    -font_size * 0.36
                ));
                out.push_str(&format!("({}) Tj\n", escape_text(text)));
                out.push_str("ET\n");
            }
            Mark::Image(_) => {
                out.push_str(&format!(
                    "{mark_w:.2} 0 0 {mark_h:.2} {:.2} {:.2} cm\n/{IMAGE_NAME} Do\n",
                    -mark_w / 2.0,
                    -mark_h / 2.0
                ));
            }
        }
        out.push_str("Q\n");
    }

    out.push_str("Q\n");
    out
}

fn tile_spots(page_w: f32, page_h: f32, mark_w: f32, mark_h: f32) -> Vec<(f32, f32)> {
    let step_x = (mark_w * 1.6).max(60.0);
    let step_y = (mark_h * 3.5).max(60.0);
    let mut spots = Vec::new();
    let mut y = step_y / 2.0;
    while y < page_h && spots.len() < 400 {
        let mut x = step_x / 2.0;
        while x < page_w {
            spots.push((x, y));
            x += step_x;
        }
        y += step_y;
    }
    spots
}

fn embed_image(doc: &mut Document, data: &[u8]) -> Result<ObjectId> {
    let kind = FileKind::sniff(data);
    if !kind.is_image() {
        return Err(Error::unsupported("the logo isn't an image this tool can read"));
    }
    let decoded = image::load_from_memory(data)
        .map_err(|e| Error::invalid(format!("could not read the logo: {e}")))?;
    // Through JPEG so the stream is a format every reader handles, and because
    // a watermark at 15% opacity has no use for a lossless logo.
    let jpeg = super::images::encode_jpeg_for_pdf(&decoded, 86, false)?;
    Ok(doc.add_object(super::images::image_stream(
        jpeg,
        decoded.width(),
        decoded.height(),
        false,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdf::{page_count, page_summaries};
    use crate::testkit;

    fn text_of(bytes: &[u8]) -> String {
        // The content streams, decompressed, so the drawing can be inspected.
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|p| String::from_utf8_lossy(&doc.get_page_content(p)).to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn stamps_every_page_by_default() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        let out = watermark(&src, None, &WatermarkOptions::default()).unwrap();
        assert_eq!(page_count(&out).unwrap(), 3);

        let doc = Document::load_mem(&out).unwrap();
        for page in doc.page_iter() {
            let content = String::from_utf8_lossy(&doc.get_page_content(page)).to_string();
            assert!(content.contains("(DRAFT) Tj"), "page missing its mark: {content}");
        }
    }

    #[test]
    fn only_the_selected_pages_are_marked() {
        let src = testkit::pdf_scan(4, 200, 260, 70);
        let selection = PageSelection::parse("2,4", 4).unwrap();
        let out = watermark(&src, Some(&selection), &WatermarkOptions::default()).unwrap();

        let doc = Document::load_mem(&out).unwrap();
        let marked: Vec<bool> = doc
            .page_iter()
            .map(|p| String::from_utf8_lossy(&doc.get_page_content(p)).contains("(DRAFT)"))
            .collect();
        assert_eq!(marked, vec![false, true, false, true]);
    }

    #[test]
    fn the_original_content_is_fenced_off() {
        // The page's own drawing must be wrapped, or an unbalanced content
        // stream would carry its state into the watermark.
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let out = watermark(&src, None, &WatermarkOptions::default()).unwrap();
        let content = text_of(&out);
        assert!(content.starts_with("q"), "should open with a save: {content}");
        let watermark_at = content.find("/KbxWmGs gs").expect("watermark state");
        let restore_at = content[..watermark_at].rfind("Q\n").expect("restore before the mark");
        assert!(restore_at < watermark_at);
    }

    #[test]
    fn resources_are_merged_not_replaced() {
        // The scan's image must survive, or the page goes blank under the mark.
        let src = testkit::pdf_scan(2, 300, 400, 70);
        let out = watermark(&src, None, &WatermarkOptions::default()).unwrap();
        let report = super::super::inspect::inspect(&out).unwrap();
        assert_eq!(report.images.len(), 2, "the page images should still be there");
        assert!(report.images[0].dpi.is_some(), "and still placed on their page");
    }

    #[test]
    fn opacity_reaches_the_graphics_state() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = WatermarkOptions { opacity: 0.4, ..Default::default() };
        let out = watermark(&src, None, &options).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let page = doc.page_iter().next().unwrap();
        let resources = doc.get_dictionary(page).unwrap().get(b"Resources").unwrap();
        let resources = resources.as_dict().unwrap();
        let gs = resources.get(b"ExtGState").unwrap().as_dict().unwrap();
        let entry = gs.get(b"KbxWmGs").unwrap().as_dict().unwrap();
        assert!((entry.get(b"ca").unwrap().as_float().unwrap() - 0.4).abs() < 0.01);
        assert!((entry.get(b"CA").unwrap().as_float().unwrap() - 0.4).abs() < 0.01);
    }

    #[test]
    fn a_logo_is_embedded_once_however_many_pages() {
        let logo = testkit::encode_jpeg(&testkit::photo(200, 80), 85);
        let src = testkit::pdf_scan(4, 200, 260, 70);
        let options = WatermarkOptions {
            mark: Mark::Image(logo),
            placement: Placement::BottomRight,
            rotation: 0.0,
            ..Default::default()
        };
        let out = watermark(&src, None, &options).unwrap();

        let report = super::super::inspect::inspect(&out).unwrap();
        // Four page scans plus exactly one logo.
        assert_eq!(report.images.len(), 5, "the logo should not be copied per page");
        let doc = Document::load_mem(&out).unwrap();
        for page in doc.page_iter() {
            assert!(String::from_utf8_lossy(&doc.get_page_content(page)).contains("/KbxWmImg Do"));
        }
    }

    #[test]
    fn tiling_repeats_the_mark() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = WatermarkOptions {
            mark: Mark::Text("COPY".into()),
            placement: Placement::Tiled,
            font_size: Some(18.0),
            ..Default::default()
        };
        let out = watermark(&src, None, &options).unwrap();
        let content = text_of(&out);
        let count = content.matches("(COPY) Tj").count();
        assert!(count > 5, "tiling should repeat the mark, found {count}");
    }

    #[test]
    fn brackets_and_backslashes_cannot_break_the_stream() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = WatermarkOptions {
            mark: Mark::Text(r"(c) 2026 \ ACME".into()),
            ..Default::default()
        };
        let out = watermark(&src, None, &options).unwrap();
        let content = text_of(&out);
        assert!(content.contains(r"\(c\) 2026 \\ ACME"), "{content}");
        // And it is still a readable document.
        assert_eq!(page_summaries(&out).unwrap().len(), 1);
    }

    #[test]
    fn empty_text_and_bad_opacity_are_refused() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let blank = WatermarkOptions { mark: Mark::Text("   ".into()), ..Default::default() };
        assert!(watermark(&src, None, &blank).is_err());
        let loud = WatermarkOptions { opacity: 2.0, ..Default::default() };
        assert!(watermark(&src, None, &loud).is_err());
    }
}
