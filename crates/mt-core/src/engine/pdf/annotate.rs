//! Drawing on a page: text, highlights, shapes and freehand ink.
//!
//! This is the fallback for every form that isn't fillable, which is most of
//! them. A scanned application, a printed invoice with a wrong figure, a
//! contract that needs a note in the margin — none of those have fields to
//! type into, and all of them need something written on them.
//!
//! The marks are drawn into the page's own content rather than added as PDF
//! annotations. Annotations are editable later, which sounds better until you
//! consider what they are: a layer any reader can hide, any tool can strip,
//! and some printers ignore. A note that silently fails to print is worse than
//! no note. So the marks become part of the page, and the report says plainly
//! that they can't be peeled off again.
//!
//! **Highlighting multiplies rather than covers.** A translucent yellow box
//! painted over black text greys the text; painted under it, the box vanishes
//! behind a scan's background image. Multiply blending is what a real highlight
//! annotation uses and the only thing that works on both: white paper takes the
//! colour, dark ink stays dark.
//!
//! **White-out covers; it does not delete.** Painting over a mistake hides it
//! from a reader and leaves every character of it in the file, exactly like the
//! black-rectangle trick [`super::redact`] exists to replace. The report says
//! so every time one is used, and points at the tool that actually removes it.

use lopdf::{dictionary, Dictionary, Document, ObjectId};
use serde::{Deserialize, Serialize};

use super::overlay::{escape_text, rotation_of, stamp_page_with, text_width, Frame, FONT_NAME};
use super::pages::{materialise_inherited, media_box};
use crate::error::{Error, Result};

/// What to draw.
#[derive(Debug, Clone, Serialize, Deserialize)]
// Every field here happens to be one word, so this changes nothing today —
// it is here so the next multi-word field added to a variant arrives instead
// of silently defaulting, the way redaction's `matchCase` did.
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Mark {
    /// Words on the page. The box is where they start and how wide they may
    /// run before wrapping.
    Text {
        text: String,
        size: f32,
    },
    /// Colour multiplied into the page, the way a marker pen works.
    Highlight,
    /// Opaque paint over a mistake. Covers; does not remove.
    Whiteout,
    Rectangle {
        filled: bool,
    },
    Ellipse {
        filled: bool,
    },
    /// From the box's bottom-left corner to its top-right.
    Line {
        arrow: bool,
    },
    /// Freehand, as fractions of the whole page.
    Ink {
        points: Vec<(f32, f32)>,
    },
}

/// One mark, placed in fractions of the page **as displayed**, with `(0,0)` at
/// the top left — the same frame a preview image uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub mark: Mark,
    pub colour: [u8; 3],
    /// Stroke width in points.
    pub line_width: f32,
    /// 0.0–1.0. Ignored by highlight, which multiplies instead.
    pub opacity: f32,
}

impl Annotation {
    /// A sensible mark of the given kind in the given box.
    pub fn new(page: u32, x: f32, y: f32, width: f32, height: f32, mark: Mark) -> Annotation {
        Annotation {
            page,
            x,
            y,
            width,
            height,
            mark,
            colour: [194, 65, 12],
            line_width: 1.5,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateOptions {
    pub marks: Vec<Annotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateReport {
    pub drawn: u32,
    pub pages_changed: Vec<u32>,
    pub warnings: Vec<String>,
    /// Always present: what drawing on a page does and doesn't do.
    pub note: String,
}

pub const PART_OF_THE_PAGE: &str =
    "These marks are now part of the page. They print and show up everywhere, and they can't be \
     selected or peeled off later — keep the original if you may need a clean copy.";

pub const WHITEOUT_COVERS: &str =
    "White-out hides what is underneath; it does not remove it. The text is still in the file and \
     anyone can select or extract it. Use Redact to actually delete it.";

/// Draw the marks onto the document.
pub fn annotate(bytes: &[u8], options: &AnnotateOptions) -> Result<(Vec<u8>, AnnotateReport)> {
    if options.marks.is_empty() {
        return Err(Error::invalid("there is nothing to draw"));
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

    for mark in &options.marks {
        if mark.page == 0 || mark.page as usize > page_ids.len() {
            return Err(Error::invalid(format!(
                "there is no page {} — this document has {}",
                mark.page,
                page_ids.len()
            )));
        }
        if !(0.0..=1.0).contains(&mark.opacity) {
            return Err(Error::invalid("opacity has to be between 0 and 1"));
        }
        if let Mark::Text { text, size } = &mark.mark {
            if text.trim().is_empty() {
                return Err(Error::invalid("one of the text marks has nothing in it"));
            }
            if *size <= 0.0 {
                return Err(Error::invalid("text size has to be greater than zero"));
            }
        }
        if let Mark::Ink { points } = &mark.mark {
            if points.len() < 2 {
                return Err(Error::invalid("a freehand mark needs at least two points"));
            }
        }
    }

    let mut report = AnnotateReport {
        drawn: 0,
        pages_changed: Vec::new(),
        warnings: Vec::new(),
        note: PART_OF_THE_PAGE.to_string(),
    };

    // Grouped by page, so a page is stamped once however many marks land on it.
    let mut by_page: std::collections::BTreeMap<u32, Vec<&Annotation>> = Default::default();
    for mark in &options.marks {
        by_page.entry(mark.page).or_default().push(mark);
    }

    for (number, marks) in by_page {
        let page_id = page_ids[number as usize - 1];
        materialise_inherited(&mut doc, page_id);
        let (media_w, media_h) = media_box(&doc, page_id).unwrap_or((612.0, 792.0));
        let frame = Frame { media_w, media_h, rotation: rotation_of(&doc, page_id) };
        let (page_w, page_h) = frame.displayed();

        let mut content = String::new();
        let mut states: Vec<(String, Dictionary)> = Vec::new();

        for mark in marks {
            let state = state_for(mark, &mut states);
            content.push_str(&draw(mark, &frame, page_w, page_h, &state, &mut report));
            report.drawn += 1;

            if matches!(mark.mark, Mark::Whiteout)
                && !report.warnings.contains(&WHITEOUT_COVERS.to_string())
            {
                report.warnings.push(WHITEOUT_COVERS.to_string());
            }
        }

        stamp_page_with(&mut doc, page_id, &content, 1.0, None, &states)?;
        report.pages_changed.push(number);
    }

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;
    Ok((out, report))
}

/// The graphics state this mark needs, made once and shared by any other mark
/// wanting the same thing.
fn state_for(mark: &Annotation, states: &mut Vec<(String, Dictionary)>) -> String {
    let multiply = matches!(mark.mark, Mark::Highlight);
    let alpha = if multiply { 1.0 } else { mark.opacity };
    // Two marks at the same opacity and blend share one state rather than
    // adding a near-duplicate to the page's resources for each.
    let key =
        format!("KbxAn{}{}", if multiply { "M" } else { "N" }, (alpha * 100.0).round() as u32);

    if !states.iter().any(|(name, _)| *name == key) {
        let mut state = dictionary! {
            "Type" => "ExtGState",
            "ca" => alpha as f64,
            "CA" => alpha as f64,
        };
        if multiply {
            state.set("BM", lopdf::Object::Name(b"Multiply".to_vec()));
        }
        states.push((key.clone(), state));
    }
    key
}

/// The drawing for one mark, in its own fenced graphics state.
fn draw(
    mark: &Annotation,
    frame: &Frame,
    page_w: f32,
    page_h: f32,
    state: &str,
    report: &mut AnnotateReport,
) -> String {
    let [r, g, b] = mark.colour;
    let (cr, cg, cb) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let line = mark.line_width.max(0.1);

    // Freehand is given in whole-page fractions, so it is drawn in the page's
    // own displayed frame rather than inside the mark's box.
    if let Mark::Ink { points } = &mark.mark {
        let mut out = format!(
            "q\n/{state} gs\n{} cm\n{cr:.3} {cg:.3} {cb:.3} RG\n{line} w\n1 J\n1 j\n",
            frame.to_page_space(0.0, 0.0)
        );
        for (index, (px, py)) in points.iter().enumerate() {
            let x = px.clamp(0.0, 1.0) * page_w;
            let y = page_h - py.clamp(0.0, 1.0) * page_h;
            out.push_str(&format!("{x:.2} {y:.2} {}\n", if index == 0 { "m" } else { "l" }));
        }
        out.push_str("S\nQ\n");
        return out;
    }

    // Everything else is placed inside its own box.
    let w = (mark.width * page_w).max(0.1);
    let h = (mark.height * page_h).max(0.1);
    let dx = mark.x * page_w;
    // Displayed y runs down the page; the drawing frame runs up from the
    // box's bottom-left corner.
    let dy = page_h - (mark.y + mark.height) * page_h;

    let mut out = format!("q\n/{state} gs\n{} cm\n", frame.to_page_space(dx, dy));
    out.push_str(&format!("{cr:.3} {cg:.3} {cb:.3} rg\n{cr:.3} {cg:.3} {cb:.3} RG\n{line} w\n"));

    match &mark.mark {
        Mark::Text { text, size } => {
            let lines = wrap(text, *size, w);
            if lines.len() as f32 * size * 1.2 > h + size {
                report.warnings.push(format!(
                    "The text on page {} is taller than the box it was given.",
                    mark.page
                ));
            }
            out.push_str("BT\n");
            out.push_str(&format!("/{FONT_NAME} {size} Tf\n"));
            // From the top of the box downwards, which is how text is read.
            let mut y = h - size;
            for piece in lines {
                out.push_str(&format!("1 0 0 1 0 {y:.2} Tm\n({}) Tj\n", escape_text(&piece)));
                y -= size * 1.2;
            }
            out.push_str("ET\n");
        }
        Mark::Highlight => {
            out.push_str(&format!("0 0 {w:.2} {h:.2} re\nf\n"));
        }
        Mark::Whiteout => {
            // Paper, not the mark's colour: white-out is white by definition.
            out.push_str(&format!("1 1 1 rg\n0 0 {w:.2} {h:.2} re\nf\n"));
        }
        Mark::Rectangle { filled } => {
            // Inset by half the stroke, or the outline straddles the box.
            let inset = line / 2.0;
            out.push_str(&format!(
                "{inset:.2} {inset:.2} {:.2} {:.2} re\n{}\n",
                (w - line).max(0.1),
                (h - line).max(0.1),
                if *filled { "f" } else { "S" }
            ));
        }
        Mark::Ellipse { filled } => {
            out.push_str(&ellipse(w, h, line));
            out.push_str(if *filled { "f\n" } else { "S\n" });
        }
        Mark::Line { arrow } => {
            out.push_str("1 J\n");
            out.push_str(&format!("0 0 m\n{w:.2} {h:.2} l\nS\n"));
            if *arrow {
                out.push_str(&arrow_head(w, h, line));
            }
        }
        Mark::Ink { .. } => unreachable!("handled above"),
    }

    out.push_str("Q\n");
    out
}

/// Four Bézier curves, which is how a PDF draws a circle.
fn ellipse(w: f32, h: f32, line: f32) -> String {
    let inset = line / 2.0;
    let (x0, y0) = (inset, inset);
    let (x1, y1) = ((w - inset).max(x0 + 0.1), (h - inset).max(y0 + 0.1));
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    // The magic constant that makes four curves look like a circle.
    const K: f32 = 0.5523;
    let (ox, oy) = (rx * K, ry * K);
    format!(
        "{:.2} {cy:.2} m\n\
         {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c\n\
         {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c\n",
        x0,
        x0,
        cy + oy,
        cx - ox,
        y1,
        y1,
        cx + ox,
        y1,
        x1,
        cy + oy,
        x1,
        x1,
        cy - oy,
        cx + ox,
        y0,
        y0,
        cx - ox,
        y0,
        x0,
        cy - oy,
        x0,
    )
}

/// Two short strokes at the far end of a line.
fn arrow_head(w: f32, h: f32, line: f32) -> String {
    let length = (w * w + h * h).sqrt();
    if length < 0.001 {
        return String::new();
    }
    let size = (line * 4.0).clamp(4.0, length * 0.4);
    let (ux, uy) = (w / length, h / length);
    // Rotate the unit vector back by about 25 degrees each way.
    let (cos, sin) = (0.906f32, 0.423f32);
    let left = (-(ux * cos - uy * sin) * size, -(ux * sin + uy * cos) * size);
    let right = (-(ux * cos + uy * sin) * size, (ux * sin - uy * cos) * size);
    format!(
        "{:.2} {:.2} m\n{w:.2} {h:.2} l\n{:.2} {:.2} l\nS\n",
        w + left.0,
        h + left.1,
        w + right.0,
        h + right.1,
    )
}

/// Break text to fit the box's width.
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate =
                if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&candidate, size) <= width || line.is_empty() {
                line = candidate;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdf::{page_count, rotate, PageSelection};
    use crate::testkit;

    fn content_of(bytes: &[u8]) -> Vec<String> {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|p| String::from_utf8_lossy(&doc.get_page_content(p)).to_string())
            .collect()
    }

    fn text_of(bytes: &[u8]) -> String {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|id| super::super::markdown::page_lines(&doc, id).join(" "))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn one(mark: Mark) -> AnnotateOptions {
        AnnotateOptions { marks: vec![Annotation::new(1, 0.1, 0.2, 0.4, 0.1, mark)] }
    }

    fn page() -> Vec<u8> {
        testkit::pdf_with_sized_text(&[(14.0, "the original words")])
    }

    #[test]
    fn written_text_can_be_read_back_off_the_page() {
        let (out, report) =
            annotate(&page(), &one(Mark::Text { text: "Approved 11 Nov".into(), size: 11.0 }))
                .unwrap();
        assert_eq!(report.drawn, 1);
        assert_eq!(report.pages_changed, vec![1]);

        let text = text_of(&out);
        assert!(text.contains("Approved 11 Nov"), "the note should be real text: {text}");
        assert!(text.contains("the original words"), "the page should survive: {text}");
    }

    #[test]
    fn long_text_wraps_inside_its_box() {
        let long = "This is a long remark that will not fit on one line inside a narrow box \
                    and therefore has to wrap onto several";
        let (out, _) =
            annotate(&page(), &one(Mark::Text { text: long.into(), size: 10.0 })).unwrap();
        let drawn = &content_of(&out)[0];
        assert!(drawn.matches(" Tj").count() > 2, "it should wrap: {drawn}");
    }

    #[test]
    fn a_highlight_multiplies_so_it_works_on_ink_and_on_a_scan() {
        // Painted opaquely it hides the text; painted underneath it vanishes
        // behind a scan's background. Multiply is the only thing that does
        // both, and it has to reach the page as a real graphics state.
        let (out, _) = annotate(&page(), &one(Mark::Highlight)).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let page_id = doc.page_iter().next().unwrap();
        let resources = super::super::overlay::sub_dictionary(
            &doc,
            doc.get_dictionary(page_id).unwrap(),
            b"Resources",
        );
        let states = super::super::overlay::sub_dictionary(&doc, &resources, b"ExtGState");
        let multiply = states.iter().any(|(_, value)| {
            matches!(value, lopdf::Object::Dictionary(d)
                if d.get(b"BM").ok().and_then(|o| o.as_name().ok()) == Some(b"Multiply".as_ref()))
        });
        assert!(multiply, "the highlight needs a multiply blend: {states:?}");
    }

    #[test]
    fn whiteout_says_it_only_covers() {
        let (_, report) = annotate(&page(), &one(Mark::Whiteout)).unwrap();
        let warning =
            report.warnings.iter().find(|w| w.contains("does not remove")).expect("it must say so");
        assert!(warning.contains("Redact"), "it should point at what does: {warning}");
    }

    #[test]
    fn whiteout_paints_white_whatever_colour_was_asked_for() {
        let mut options = one(Mark::Whiteout);
        options.marks[0].colour = [10, 200, 30];
        let (out, _) = annotate(&page(), &options).unwrap();
        let drawn = &content_of(&out)[0];
        assert!(drawn.contains("1 1 1 rg"), "white-out has to be white: {drawn}");
    }

    #[test]
    fn every_report_says_the_marks_are_part_of_the_page_now() {
        let (_, report) = annotate(&page(), &one(Mark::Rectangle { filled: false })).unwrap();
        assert!(report.note.contains("part of the page"), "{}", report.note);
        assert!(report.note.contains("keep the original"), "{}", report.note);
    }

    #[test]
    fn shapes_draw_the_operators_they_should() {
        let hollow = annotate(&page(), &one(Mark::Rectangle { filled: false })).unwrap().0;
        let drawn = &content_of(&hollow)[0];
        assert!(drawn.contains(" re"), "a rectangle: {drawn}");
        assert!(drawn.contains("\nS\n"), "stroked, not filled: {drawn}");

        let solid = annotate(&page(), &one(Mark::Rectangle { filled: true })).unwrap().0;
        assert!(content_of(&solid)[0].contains("\nf\n"), "filled");

        let round = annotate(&page(), &one(Mark::Ellipse { filled: false })).unwrap().0;
        assert!(content_of(&round)[0].contains(" c\n"), "an ellipse is four curves");
    }

    #[test]
    fn an_arrow_adds_a_head_a_plain_line_does_not() {
        let plain = annotate(&page(), &one(Mark::Line { arrow: false })).unwrap().0;
        let pointed = annotate(&page(), &one(Mark::Line { arrow: true })).unwrap().0;
        let plain_moves = content_of(&plain)[0].matches(" m\n").count();
        let pointed_moves = content_of(&pointed)[0].matches(" m\n").count();
        assert!(pointed_moves > plain_moves, "{plain_moves} vs {pointed_moves}");
    }

    #[test]
    fn freehand_draws_through_all_its_points() {
        let points: Vec<(f32, f32)> = (0..8).map(|i| (0.1 + i as f32 * 0.05, 0.5)).collect();
        let (out, _) = annotate(&page(), &one(Mark::Ink { points })).unwrap();
        let drawn = &content_of(&out)[0];
        assert_eq!(drawn.matches(" m\n").count(), 1, "one start: {drawn}");
        assert_eq!(drawn.matches(" l\n").count(), 7, "seven segments: {drawn}");
    }

    #[test]
    fn marks_on_different_pages_each_land_on_their_own() {
        let src = testkit::pdf_scan(3, 120, 160, 70);
        let options = AnnotateOptions {
            marks: vec![
                Annotation::new(1, 0.1, 0.1, 0.2, 0.1, Mark::Rectangle { filled: true }),
                Annotation::new(3, 0.1, 0.1, 0.2, 0.1, Mark::Rectangle { filled: true }),
            ],
        };
        let (out, report) = annotate(&src, &options).unwrap();
        assert_eq!(report.pages_changed, vec![1, 3]);
        assert_eq!(report.drawn, 2);
        assert_eq!(page_count(&out).unwrap(), 3);

        let marked: Vec<bool> = content_of(&out).iter().map(|c| c.contains("/KbxAn")).collect();
        assert_eq!(marked, vec![true, false, true]);
    }

    #[test]
    fn several_marks_on_one_page_are_stamped_together() {
        let options = AnnotateOptions {
            marks: vec![
                Annotation::new(1, 0.1, 0.1, 0.2, 0.05, Mark::Highlight),
                Annotation::new(1, 0.1, 0.3, 0.2, 0.05, Mark::Rectangle { filled: false }),
                Annotation::new(
                    1,
                    0.1,
                    0.5,
                    0.3,
                    0.05,
                    Mark::Text { text: "note".into(), size: 9.0 },
                ),
            ],
        };
        let (out, report) = annotate(&page(), &options).unwrap();
        assert_eq!(report.drawn, 3);
        assert_eq!(report.pages_changed, vec![1], "one page, stamped once");
        assert!(text_of(&out).contains("note"));
    }

    #[test]
    fn marks_sharing_an_opacity_share_one_graphics_state() {
        let options = AnnotateOptions {
            marks: (0..4)
                .map(|i| {
                    Annotation::new(
                        1,
                        0.1,
                        0.1 * i as f32,
                        0.2,
                        0.05,
                        Mark::Rectangle { filled: true },
                    )
                })
                .collect(),
        };
        let (out, _) = annotate(&page(), &options).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let page_id = doc.page_iter().next().unwrap();
        let resources = super::super::overlay::sub_dictionary(
            &doc,
            doc.get_dictionary(page_id).unwrap(),
            b"Resources",
        );
        let states = super::super::overlay::sub_dictionary(&doc, &resources, b"ExtGState");
        let ours = states.iter().filter(|(k, _)| k.starts_with(b"KbxAn")).count();
        assert_eq!(ours, 1, "four identical marks need one state, not four");
    }

    #[test]
    fn a_rotated_page_gets_upright_marks() {
        let src = page();
        let selection = PageSelection::parse("1", 1).unwrap();
        let (turned, _) = rotate(&src, &selection, 90).unwrap();
        let (out, _) =
            annotate(&turned, &one(Mark::Text { text: "sideways".into(), size: 10.0 })).unwrap();
        let drawn = &content_of(&out)[0];
        assert!(drawn.contains("0 1 -1 0"), "expected a rotated matrix: {drawn}");
    }

    #[test]
    fn the_original_drawing_is_fenced_not_replaced() {
        let src = page();
        let before = content_of(&src)[0].clone();
        let (out, _) = annotate(&src, &one(Mark::Highlight)).unwrap();
        let after = content_of(&out)[0].clone();
        for line in before.lines().filter(|l| l.contains("Tj")) {
            assert!(after.contains(line.trim()), "lost '{line}'");
        }
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(annotate(&page(), &AnnotateOptions::default()).is_err());

        let no_page = AnnotateOptions {
            marks: vec![Annotation::new(9, 0.1, 0.1, 0.2, 0.1, Mark::Highlight)],
        };
        let error = annotate(&page(), &no_page).unwrap_err().to_string();
        assert!(error.contains("no page 9"), "{error}");

        let empty_text = one(Mark::Text { text: "   ".into(), size: 10.0 });
        assert!(annotate(&page(), &empty_text).is_err());

        let tiny_text = one(Mark::Text { text: "hi".into(), size: 0.0 });
        assert!(annotate(&page(), &tiny_text).is_err());

        let short_ink = one(Mark::Ink { points: vec![(0.1, 0.1)] });
        assert!(annotate(&page(), &short_ink).is_err());

        let mut loud = one(Mark::Highlight);
        loud.marks[0].opacity = 4.0;
        assert!(annotate(&page(), &loud).is_err());
    }

    #[test]
    fn text_taller_than_its_box_is_mentioned() {
        let long = "word ".repeat(80);
        let mut options = one(Mark::Text { text: long, size: 12.0 });
        options.marks[0].height = 0.02;
        let (_, report) = annotate(&page(), &options).unwrap();
        assert!(
            report.warnings.iter().any(|w| w.contains("taller than the box")),
            "{:?}",
            report.warnings
        );
    }

    #[test]
    fn wrapping_never_returns_an_empty_run_for_real_words() {
        let lines = wrap("alpha beta gamma delta epsilon", 10.0, 40.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| !l.is_empty()));
    }
}
