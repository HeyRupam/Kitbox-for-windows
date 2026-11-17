//! Numbering a document's pages.
//!
//! The part worth care is `/Rotate`. A scanned page is often stored upright and
//! rotated for display, and a tool that ignores that stamps the number sideways
//! in a corner that isn't the corner you picked. Positions here are given in
//! *displayed* space and transformed back into page space, so "bottom centre"
//! means the bottom centre of the page as the reader sees it.

use lopdf::{Document, ObjectId};
use serde::{Deserialize, Serialize};

use super::overlay::{escape_text, rotation_of, stamp_page, text_width, Frame, FONT_NAME, STATE_NAME};
use super::pages::{materialise_inherited, media_box, PageSelection};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberPosition {
    TopLeft,
    TopCentre,
    TopRight,
    BottomLeft,
    #[default]
    BottomCentre,
    BottomRight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageNumberOptions {
    /// `{n}` is the page's number and `{total}` how many are numbered, so
    /// "Page {n} of {total}" reads the way people write it.
    pub template: String,
    pub position: NumberPosition,
    /// What to call the first numbered page. Useful when a cover is skipped
    /// but the numbering should still start at 1 — or when it shouldn't.
    pub start_at: u32,
    pub font_size: f32,
    pub colour: [u8; 3],
    /// Distance from the edge of the page, in points.
    pub margin: f32,
}

impl Default for PageNumberOptions {
    fn default() -> Self {
        PageNumberOptions {
            template: "{n}".into(),
            position: NumberPosition::BottomCentre,
            start_at: 1,
            font_size: 11.0,
            colour: [40, 40, 40],
            margin: 28.0,
        }
    }
}

/// Number the selected pages. `None` means every page.
///
/// Returns the document and how many pages were numbered.
pub fn add_page_numbers(
    bytes: &[u8],
    selection: Option<&PageSelection>,
    options: &PageNumberOptions,
) -> Result<(Vec<u8>, u32)> {
    if options.font_size <= 0.0 {
        return Err(Error::invalid("the number needs a size bigger than zero"));
    }
    if !options.template.contains("{n}") {
        return Err(Error::invalid(
            "the format needs {n} in it somewhere, or every page gets the same label",
        ));
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

    let numbered: Vec<(usize, ObjectId)> = page_ids
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            selection.map(|s| s.contains(*index as u32 + 1)).unwrap_or(true)
        })
        .map(|(index, id)| (index, *id))
        .collect();

    if numbered.is_empty() {
        return Err(Error::invalid("that selection matched no pages"));
    }
    let total = numbered.len() as u32;

    let [r, g, b] = options.colour;
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);

    for (position, (_, page_id)) in numbered.iter().enumerate() {
        materialise_inherited(&mut doc, *page_id);
        let (media_w, media_h) = media_box(&doc, *page_id).unwrap_or((612.0, 792.0));
        let rotation = rotation_of(&doc, *page_id);
        let frame = Frame { media_w, media_h, rotation };

        let label = options
            .template
            .replace("{n}", &(options.start_at as usize + position).to_string())
            .replace("{total}", &total.to_string());

        let (dx, dy) = place(options, &label, frame.displayed());
        let matrix = frame.to_page_space(dx, dy);

        let content = format!(
            "q\n/{STATE_NAME} gs\n{rf:.3} {gf:.3} {bf:.3} rg\nBT\n/{FONT_NAME} {:.2} Tf\n{matrix} Tm\n({}) Tj\nET\nQ\n",
            options.font_size,
            escape_text(&label)
        );
        stamp_page(&mut doc, *page_id, &content, 1.0, None)?;
    }

    Ok((super::lossless::save_compact(&mut doc)?, total))
}

/// Where the number sits, in the page as the reader sees it.
fn place(options: &PageNumberOptions, label: &str, (width, height): (f32, f32)) -> (f32, f32) {
    use NumberPosition::*;
    let text = text_width(label, options.font_size);
    let x = match options.position {
        TopLeft | BottomLeft => options.margin,
        TopCentre | BottomCentre => (width - text) / 2.0,
        TopRight | BottomRight => width - options.margin - text,
    };
    let y = match options.position {
        TopLeft | TopCentre | TopRight => height - options.margin - options.font_size,
        BottomLeft | BottomCentre | BottomRight => options.margin,
    };
    (x.max(0.0), y.max(0.0))
}

/// A page's geometry, including how the reader will see it.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdf::{page_count, rotate, PageSelection};
    use crate::testkit;

    fn contents(bytes: &[u8]) -> Vec<String> {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|p| String::from_utf8_lossy(&doc.get_page_content(p)).to_string())
            .collect()
    }

    #[test]
    fn numbers_every_page_in_order() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        let (out, total) = add_page_numbers(&src, None, &PageNumberOptions::default()).unwrap();
        assert_eq!(total, 3);
        assert_eq!(page_count(&out).unwrap(), 3);

        let pages = contents(&out);
        assert!(pages[0].contains("(1) Tj"), "{}", pages[0]);
        assert!(pages[1].contains("(2) Tj"));
        assert!(pages[2].contains("(3) Tj"));
    }

    #[test]
    fn the_template_fills_in_both_placeholders() {
        let src = testkit::pdf_scan(4, 200, 260, 70);
        let options = PageNumberOptions {
            template: "Page {n} of {total}".into(),
            ..Default::default()
        };
        let (out, _) = add_page_numbers(&src, None, &options).unwrap();
        let pages = contents(&out);
        assert!(pages[0].contains("(Page 1 of 4) Tj"), "{}", pages[0]);
        assert!(pages[3].contains("(Page 4 of 4) Tj"));
    }

    #[test]
    fn skipping_the_cover_still_starts_at_one() {
        let src = testkit::pdf_scan(4, 200, 260, 70);
        let selection = PageSelection::parse("2-4", 4).unwrap();
        let options =
            PageNumberOptions { template: "{n} of {total}".into(), ..Default::default() };
        let (out, total) = add_page_numbers(&src, Some(&selection), &options).unwrap();

        assert_eq!(total, 3, "the total counts the pages actually numbered");
        let pages = contents(&out);
        assert!(!pages[0].contains(" Tj"), "the cover should be left alone");
        assert!(pages[1].contains("(1 of 3) Tj"), "{}", pages[1]);
        assert!(pages[3].contains("(3 of 3) Tj"));
    }

    #[test]
    fn numbering_can_start_anywhere() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        let options = PageNumberOptions { start_at: 7, ..Default::default() };
        let (out, _) = add_page_numbers(&src, None, &options).unwrap();
        let pages = contents(&out);
        assert!(pages[0].contains("(7) Tj"));
        assert!(pages[1].contains("(8) Tj"));
    }

    #[test]
    fn positions_differ_across_the_page() {
        let options = PageNumberOptions::default();
        let page = (612.0f32, 792.0f32);
        let bottom_left = place(&options, "1", page);
        let bottom_right = place(&options, "1", page);
        let top_centre = place(
            &PageNumberOptions { position: NumberPosition::TopCentre, ..options.clone() },
            "1",
            page,
        );
        assert!(top_centre.1 > bottom_left.1, "the top should be higher up");

        let left = place(
            &PageNumberOptions { position: NumberPosition::BottomLeft, ..options.clone() },
            "1",
            page,
        );
        let right = place(
            &PageNumberOptions { position: NumberPosition::BottomRight, ..options.clone() },
            "1",
            page,
        );
        assert!(right.0 > left.0, "the right should be further across");
        let _ = bottom_right;
    }

    #[test]
    fn right_alignment_accounts_for_the_text_width() {
        let options =
            PageNumberOptions { position: NumberPosition::BottomRight, ..Default::default() };
        let short = place(&options, "1", (612.0, 792.0));
        let long = place(&options, "Page 10 of 10", (612.0, 792.0));
        assert!(long.0 < short.0, "a longer label has to start further left");
        // And the right edge of both lands at the same place.
        let right_edge = |x: f32, label: &str| x + text_width(label, options.font_size);
        assert!((right_edge(short.0, "1") - right_edge(long.0, "Page 10 of 10")).abs() < 0.01);
    }

    #[test]
    fn a_rotated_page_gets_an_upright_number() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        // Turn the first page a quarter turn, as a sideways scan would be.
        let selection = PageSelection::parse("1", 2).unwrap();
        let (turned, _) = rotate(&src, &selection, 90).unwrap();

        let (out, _) = add_page_numbers(&turned, None, &PageNumberOptions::default()).unwrap();
        let pages = contents(&out);

        // The rotated page's text matrix must counter the rotation; the
        // untouched one stays upright.
        assert!(pages[0].contains("0 1 -1 0"), "expected a rotated matrix: {}", pages[0]);
        assert!(pages[1].contains("1 0 0 1"), "expected an upright matrix: {}", pages[1]);
    }

    #[test]
    fn rotation_maps_the_corners_consistently() {
        let frame = Frame { media_w: 612.0, media_h: 792.0, rotation: 90 };
        // Displayed size is swapped.
        assert_eq!(frame.displayed(), (792.0, 612.0));
        // The displayed origin maps into page space, not outside it.
        let matrix = frame.to_page_space(0.0, 0.0);
        assert_eq!(matrix, "0 1 -1 0 612.00 0.00");

        let upright = Frame { media_w: 612.0, media_h: 792.0, rotation: 0 };
        assert_eq!(upright.to_page_space(30.0, 40.0), "1 0 0 1 30.00 40.00");
    }

    #[test]
    fn the_page_content_survives() {
        let src = testkit::pdf_scan(2, 300, 400, 70);
        let (out, _) = add_page_numbers(&src, None, &PageNumberOptions::default()).unwrap();
        let report = super::super::inspect::inspect(&out).unwrap();
        assert_eq!(report.images.len(), 2, "the scans must still be there");
        assert!(report.images[0].dpi.is_some(), "and still placed on their page");
    }

    #[test]
    fn a_format_without_the_placeholder_is_refused() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = PageNumberOptions { template: "Page".into(), ..Default::default() };
        let err = add_page_numbers(&src, None, &options).unwrap_err().to_string();
        assert!(err.contains("{n}"), "{err}");

        let tiny = PageNumberOptions { font_size: 0.0, ..Default::default() };
        assert!(add_page_numbers(&src, None, &tiny).is_err());
    }
}
