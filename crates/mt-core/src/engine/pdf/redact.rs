//! Removing content from a PDF, rather than covering it up.
//!
//! Most tools that offer redaction draw a black rectangle over the page. The
//! text is still underneath it: select it, copy it, or run any extractor and
//! it comes straight back out. People have lost court cases and leaked witness
//! names this way. A black rectangle is a *decoration*, not a redaction.
//!
//! So this deletes the glyphs from the content stream, paints out the pixels
//! of any image underneath, removes annotations that overlap, and strips the
//! places text hides when nobody is looking: document metadata, XMP, embedded
//! files, JavaScript, outlines. The black box is drawn last, and only because
//! a reader should be able to see that something was taken out.
//!
//! Deleting a glyph is not the same as deleting its space. A glyph is removed
//! by replacing it with the negative kerning adjustment that equals its own
//! width, so everything after it stays exactly where it was. Delete the glyph
//! and nothing else and the rest of the line slides left, which is both ugly
//! and a tell.
//!
//! # What this cannot do
//!
//! Vector drawings — lines, filled shapes, a signature stored as paths — are
//! covered by the box but their geometry stays in the file. Removing arbitrary
//! path geometry safely is a different problem, and quietly pretending
//! otherwise is exactly the failure this module exists to avoid. Every report
//! says so when a box lands on something that isn't text or an image.
//!
//! Every run finishes by re-reading its own output and checking the content is
//! actually gone. That check is the only thing that makes the claim worth
//! anything, and its result travels in the report.

use std::collections::BTreeMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};

use super::overlay::{rotation_of, Frame};
use super::pages::{materialise_inherited, media_box};
use crate::error::{Error, Result};

/// What to take out.
#[derive(Debug, Clone, Serialize, Deserialize)]
// `rename_all` renames an enum's *variants*; the fields inside a struct
// variant need `rename_all_fields`. Without it `match_case` expected
// snake_case while the screen sent `matchCase`, serde dropped the unknown
// field, and the `default` below quietly supplied `false` — so asking for a
// case-sensitive redaction did nothing and said nothing.
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Target {
    /// Every occurrence of this text, on every page.
    Text {
        needle: String,
        #[serde(default)]
        match_case: bool,
    },
    /// A box on one page, as fractions of the page **as displayed**, with
    /// `(0,0)` at the top left — the frame a preview image uses.
    Area { page: u32, x: f32, y: f32, width: f32, height: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactOptions {
    pub targets: Vec<Target>,
    /// Draw a filled box where content was removed. On by default: a redaction
    /// nobody can see is one nobody can check.
    pub draw_boxes: bool,
    pub box_colour: [u8; 3],
    /// Strip metadata, embedded files, JavaScript and outlines as well.
    pub strip_extras: bool,
}

impl Default for RedactOptions {
    fn default() -> Self {
        RedactOptions {
            targets: Vec::new(),
            draw_boxes: true,
            box_colour: [0, 0, 0],
            strip_extras: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactReport {
    pub pages_changed: Vec<u32>,
    /// How many glyphs were deleted from content streams.
    pub glyphs_removed: u32,
    /// How many embedded images had pixels painted out.
    pub images_edited: u32,
    pub annotations_removed: u32,
    /// Whether re-reading the finished file found the content gone.
    pub verified: bool,
    /// Anything still findable afterwards. Empty is the only acceptable value,
    /// and `verified` is false whenever it isn't.
    pub leaks: Vec<String>,
    pub warnings: Vec<String>,
    pub note: String,
}

/// What every report says about the limits of this.
pub const VECTOR_CAVEAT: &str =
    "Text and image content under each box has been deleted, not hidden. Vector drawings — ruled \
     lines, shapes, a signature stored as paths — are covered by the box but their geometry stays \
     in the file.";

/// Take the targeted content out of the document.
pub fn redact(bytes: &[u8], options: &RedactOptions) -> Result<(Vec<u8>, RedactReport)> {
    if options.targets.is_empty() {
        return Err(Error::invalid("nothing was marked for redaction"));
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

    for target in &options.targets {
        match target {
            Target::Text { needle, .. } if needle.trim().is_empty() => {
                return Err(Error::invalid("the text to redact is empty"));
            }
            Target::Area { page, .. } if *page == 0 || *page as usize > page_ids.len() => {
                return Err(Error::invalid(format!(
                    "there is no page {page} — this document has {}",
                    page_ids.len()
                )));
            }
            Target::Area { x, y, width, height, .. } => {
                if !(0.0..=1.0).contains(x)
                    || !(0.0..=1.0).contains(y)
                    || *width <= 0.0
                    || *height <= 0.0
                {
                    return Err(Error::invalid("that redaction box isn't on the page"));
                }
            }
            _ => {}
        }
    }

    let mut report = RedactReport {
        pages_changed: Vec::new(),
        glyphs_removed: 0,
        images_edited: 0,
        annotations_removed: 0,
        verified: false,
        leaks: Vec::new(),
        warnings: Vec::new(),
        note: VECTOR_CAVEAT.to_string(),
    };

    for (index, page_id) in page_ids.iter().enumerate() {
        let number = index as u32 + 1;
        materialise_inherited(&mut doc, *page_id);
        let (media_w, media_h) = media_box(&doc, *page_id).unwrap_or((612.0, 792.0));
        let frame = Frame { media_w, media_h, rotation: rotation_of(&doc, *page_id) };

        // The areas aimed at this page, in the page's own coordinates.
        let areas: Vec<Rect> = options
            .targets
            .iter()
            .filter_map(|t| match t {
                Target::Area { page, x, y, width, height } if *page == number => {
                    Some(area_to_page_rect(&frame, *x, *y, *width, *height))
                }
                _ => None,
            })
            .collect();
        let needles: Vec<(&str, bool)> = options
            .targets
            .iter()
            .filter_map(|t| match t {
                Target::Text { needle, match_case } => Some((needle.as_str(), *match_case)),
                _ => None,
            })
            .collect();

        if areas.is_empty() && needles.is_empty() {
            continue;
        }

        let outcome = redact_page(&mut doc, *page_id, &areas, &needles, options, &frame)?;
        if outcome.changed {
            report.pages_changed.push(number);
        }
        report.glyphs_removed += outcome.glyphs_removed;
        report.images_edited += outcome.images_edited;
        report.annotations_removed += outcome.annotations_removed;
        for warning in outcome.warnings {
            report.warnings.push(format!("Page {number}: {warning}"));
        }
    }

    if options.strip_extras {
        strip_extras(&mut doc, &mut report);
    }

    // Unhooking an object is not removing it. Dropping /Info from the trailer
    // leaves the dictionary — title, author, keywords and all — sitting in the
    // file, reachable by anything that doesn't bother following references
    // from the root. The same goes for an outline tree or a replaced content
    // stream. Sweep up everything nothing points at any more.
    doc.prune_objects();

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;

    verify(&out, options, &mut report);
    Ok((out, report))
}

/* ---------- geometry ---------- */

#[derive(Debug, Clone, Copy)]
struct Rect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Rect {
    fn of(ax: f32, ay: f32, bx: f32, by: f32) -> Rect {
        Rect { x0: ax.min(bx), y0: ay.min(by), x1: ax.max(bx), y1: ay.max(by) }
    }

    fn overlaps(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    fn union(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

/// A box given in displayed fractions, as the page itself sees it.
fn area_to_page_rect(frame: &Frame, x: f32, y: f32, width: f32, height: f32) -> Rect {
    let (dw, dh) = frame.displayed();
    // Displayed space has y growing downwards; PDF space has it growing up.
    let left = x * dw;
    let right = (x + width) * dw;
    let top = dh - y * dh;
    let bottom = dh - (y + height) * dh;
    let (ax, ay) = frame.point_in_page_space(left, bottom);
    let (bx, by) = frame.point_in_page_space(right, top);
    Rect::of(ax, ay, bx, by)
}

/* ---------- text-space state ---------- */

#[derive(Debug, Clone, Copy)]
struct Matrix([f32; 6]);

impl Matrix {
    const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn times(self, other: Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Matrix([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }

    fn translation(self) -> (f32, f32) {
        (self.0[4], self.0[5])
    }

    fn vertical_scale(self) -> f32 {
        (self.0[2] * self.0[2] + self.0[3] * self.0[3]).sqrt()
    }

    fn horizontal_scale(self) -> f32 {
        (self.0[0] * self.0[0] + self.0[1] * self.0[1]).sqrt()
    }
}

/// Glyph widths for one font, in 1/1000 of the text size.
#[derive(Debug, Clone)]
struct Metrics {
    first_char: i64,
    widths: Vec<f32>,
    missing: f32,
    /// A composite font: one glyph is not one byte, so a string in it cannot
    /// be cut up safely.
    composite: bool,
}

impl Metrics {
    fn width_of(&self, byte: u8) -> f32 {
        let index = byte as i64 - self.first_char;
        if index >= 0 {
            if let Some(w) = self.widths.get(index as usize) {
                if *w > 0.0 {
                    return *w;
                }
            }
        }
        self.missing
    }
}

fn metrics_for(doc: &Document, page_id: ObjectId) -> BTreeMap<Vec<u8>, Metrics> {
    let mut out = BTreeMap::new();
    let Ok(fonts) = doc.get_page_fonts(page_id) else { return out };

    for (name, dict) in fonts {
        let subtype = dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| n.to_vec())
            .unwrap_or_default();
        let composite = subtype == b"Type0";

        let first_char = dict.get(b"FirstChar").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
        let widths: Vec<f32> = dict
            .get(b"Widths")
            .ok()
            .and_then(|o| resolve(doc, o).and_then(|o| o.as_array().ok().cloned()))
            .map(|items| {
                items
                    .iter()
                    .map(|o| resolve(doc, o).and_then(|o| o.as_float().ok()).unwrap_or(0.0))
                    .collect()
            })
            .unwrap_or_default();

        let missing = dict
            .get(b"FontDescriptor")
            .ok()
            .and_then(|o| resolve(doc, o))
            .and_then(|o| o.as_dict().ok().cloned())
            .and_then(|d| d.get(b"MissingWidth").ok().and_then(|o| o.as_float().ok()))
            // Nothing says what this glyph is worth, so assume a middling
            // letter. It is only used to decide what a box touches.
            .unwrap_or(500.0);

        out.insert(name.to_vec(), Metrics { first_char, widths, missing, composite });
    }
    out
}

fn resolve(doc: &Document, object: &Object) -> Option<Object> {
    match object {
        Object::Reference(id) => doc.get_object(*id).ok().cloned(),
        other => Some(other.clone()),
    }
}

/* ---------- the page ---------- */

#[derive(Debug, Default)]
struct PageOutcome {
    changed: bool,
    glyphs_removed: u32,
    images_edited: u32,
    annotations_removed: u32,
    warnings: Vec<String>,
}

fn redact_page(
    doc: &mut Document,
    page_id: ObjectId,
    areas: &[Rect],
    needles: &[(&str, bool)],
    options: &RedactOptions,
    frame: &Frame,
) -> Result<PageOutcome> {
    let mut outcome = PageOutcome::default();

    let metrics = metrics_for(doc, page_id);
    let encodings = doc.get_page_fonts(page_id).ok().and_then(|fonts| {
        let mut map = BTreeMap::new();
        for (name, dict) in fonts {
            if let Ok(encoding) = dict.get_font_encoding(doc) {
                map.insert(name.to_vec(), encoding);
            }
        }
        Some(map)
    }).unwrap_or_default();

    let data = doc.get_page_content(page_id);
    let content = Content::decode(&data)
        .map_err(|e| Error::invalid(format!("could not read the page's drawing: {e}")))?;

    let mut rewritten: Vec<Operation> = Vec::with_capacity(content.operations.len() + 8);
    // Boxes to paint, collected from whatever actually got removed.
    let mut struck: Vec<Rect> = Vec::new();
    // Where images are drawn, so pixels underneath an area can be painted out.
    let mut image_draws: Vec<(Vec<u8>, Matrix)> = Vec::new();

    let mut ctm = Matrix::IDENTITY;
    let mut stack: Vec<Matrix> = Vec::new();
    let mut text_matrix = Matrix::IDENTITY;
    let mut line_matrix = Matrix::IDENTITY;
    let mut font_name: Vec<u8> = Vec::new();
    let mut font_size = 0.0f32;
    let mut leading = 0.0f32;
    let mut char_spacing = 0.0f32;
    let mut word_spacing = 0.0f32;
    let mut horizontal = 1.0f32;

    let number = |operands: &[Object], i: usize| -> f32 {
        operands.get(i).and_then(|o| o.as_float().ok()).unwrap_or(0.0)
    };

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(Matrix::IDENTITY),
            "cm" if op.operands.len() >= 6 => {
                let mut m = [0.0; 6];
                for (i, slot) in m.iter_mut().enumerate() {
                    *slot = number(&op.operands, i);
                }
                ctm = Matrix(m).times(ctm);
            }
            "BT" => {
                text_matrix = Matrix::IDENTITY;
                line_matrix = Matrix::IDENTITY;
            }
            "Tf" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    font_name = name.clone();
                }
                font_size = number(&op.operands, 1);
            }
            "TL" => leading = number(&op.operands, 0),
            "Tc" => char_spacing = number(&op.operands, 0),
            "Tw" => word_spacing = number(&op.operands, 0),
            "Tz" => horizontal = number(&op.operands, 0) / 100.0,
            "Tm" if op.operands.len() >= 6 => {
                let mut m = [0.0; 6];
                for (i, slot) in m.iter_mut().enumerate() {
                    *slot = number(&op.operands, i);
                }
                line_matrix = Matrix(m);
                text_matrix = line_matrix;
            }
            "Td" => {
                let shift =
                    Matrix([1.0, 0.0, 0.0, 1.0, number(&op.operands, 0), number(&op.operands, 1)]);
                line_matrix = shift.times(line_matrix);
                text_matrix = line_matrix;
            }
            "TD" => {
                leading = -number(&op.operands, 1);
                let shift =
                    Matrix([1.0, 0.0, 0.0, 1.0, number(&op.operands, 0), number(&op.operands, 1)]);
                line_matrix = shift.times(line_matrix);
                text_matrix = line_matrix;
            }
            "T*" => {
                let shift = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, -leading]);
                line_matrix = shift.times(line_matrix);
                text_matrix = line_matrix;
            }
            "Do" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    image_draws.push((name.clone(), ctm));
                }
            }
            _ => {}
        }

        // Only the show-text operators get rewritten; everything else passes
        // through untouched, including drawing nobody asked about.
        let show = matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"");
        if !show {
            rewritten.push(op.clone());
            continue;
        }

        if matches!(op.operator.as_str(), "'" | "\"") {
            // These move to the next line before showing.
            let shift = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, -leading]);
            line_matrix = shift.times(line_matrix);
            text_matrix = line_matrix;
        }

        let state = TextState {
            font_size,
            char_spacing,
            word_spacing,
            horizontal,
            metrics: metrics.get(&font_name),
            encoding: encodings.get(&font_name),
        };

        let (new_op, removed, boxes, warning) =
            rewrite_show(op, &state, &mut text_matrix, ctm, areas, needles);

        if removed > 0 {
            outcome.changed = true;
            outcome.glyphs_removed += removed;
            struck.extend(boxes);
        }
        if let Some(w) = warning {
            if !outcome.warnings.contains(&w) {
                outcome.warnings.push(w);
            }
        }
        rewritten.push(new_op);
    }

    // Images: paint out the pixels under every box, including the ones that
    // came from matching text.
    //
    // This matters most on a scanned document. There the visible page is a
    // picture and the text is an invisible OCR layer on top, so deleting the
    // matched text removes it from extraction while the *photograph* of it
    // sits untouched under the bar. Peel the bar off and read it. The word has
    // to come out of the image too, or this is the black-rectangle trick with
    // extra steps.
    let mut pixel_targets: Vec<Rect> = areas.to_vec();
    pixel_targets.extend(struck.iter().copied());
    if !pixel_targets.is_empty() {
        match blank_images(doc, page_id, &image_draws, &pixel_targets) {
            Ok((edited, warnings)) => {
                if edited > 0 {
                    outcome.changed = true;
                }
                outcome.images_edited += edited;
                outcome.warnings.extend(warnings);
            }
            Err(e) => outcome.warnings.push(e.to_string()),
        }
    }

    // Annotations carry their own text — a comment, a form field's value, a
    // link's URL — and are drawn by the reader, not by the content stream.
    outcome.annotations_removed += strip_annotations(doc, page_id, areas, needles);
    if outcome.annotations_removed > 0 {
        outcome.changed = true;
    }

    let mut boxes: Vec<Rect> = areas.to_vec();
    boxes.extend(struck);
    if options.draw_boxes && !boxes.is_empty() {
        // The whole original drawing is fenced, so an unbalanced stream can't
        // drag the boxes somewhere unexpected.
        rewritten.insert(0, Operation::new("q", vec![]));
        rewritten.push(Operation::new("Q", vec![]));
        let [r, g, b] = options.box_colour;
        rewritten.push(Operation::new("q", vec![]));
        rewritten.push(Operation::new(
            "rg",
            vec![
                (r as f32 / 255.0).into(),
                (g as f32 / 255.0).into(),
                (b as f32 / 255.0).into(),
            ],
        ));
        for rect in merge_boxes(&boxes) {
            rewritten.push(Operation::new(
                "re",
                vec![
                    rect.x0.into(),
                    rect.y0.into(),
                    (rect.x1 - rect.x0).into(),
                    (rect.y1 - rect.y0).into(),
                ],
            ));
            rewritten.push(Operation::new("f", vec![]));
        }
        rewritten.push(Operation::new("Q", vec![]));
        outcome.changed = true;
    }

    if outcome.changed {
        let encoded = Content { operations: rewritten }
            .encode()
            .map_err(|e| Error::other(format!("could not rewrite the page: {e}")))?;
        replace_content(doc, page_id, encoded);
    }

    let _ = frame;
    Ok(outcome)
}

/// Put the rewritten drawing in place of the old one — and get rid of the old
/// one.
///
/// Pointing the page at a new stream and leaving the old object behind is the
/// subtle version of the mistake this module exists to prevent: the page looks
/// right, the extractor finds nothing, and the deleted text is still sitting
/// in the file for anyone who opens it in a hex editor. The verification pass
/// caught exactly this, which is the argument for having one.
fn replace_content(doc: &mut Document, page_id: ObjectId, encoded: Vec<u8>) {
    let old: Vec<ObjectId> = match doc.get_dictionary(page_id).and_then(|p| p.get(b"Contents")) {
        Ok(Object::Reference(id)) => vec![*id],
        Ok(Object::Array(items)) => items
            .iter()
            .filter_map(|o| match o {
                Object::Reference(id) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    // Reuse the first stream's object number so nothing else pointing at the
    // page breaks, and delete every other one outright.
    let target = match old.first() {
        Some(id) => {
            if let Ok(Object::Stream(stream)) = doc.get_object_mut(*id) {
                stream.dict.remove(b"Filter");
                stream.dict.remove(b"DecodeParms");
                stream.set_content(encoded);
                *id
            } else {
                doc.add_object(Stream::new(Dictionary::new(), encoded))
            }
        }
        None => doc.add_object(Stream::new(Dictionary::new(), encoded)),
    };
    for id in old.iter().skip(1) {
        doc.objects.remove(id);
    }

    if let Ok(page) = doc.get_dictionary_mut(page_id) {
        page.set("Contents", Object::Reference(target));
    }
}

struct TextState<'a> {
    font_size: f32,
    char_spacing: f32,
    word_spacing: f32,
    horizontal: f32,
    metrics: Option<&'a Metrics>,
    encoding: Option<&'a lopdf::Encoding<'a>>,
}

/// Rewrite one show-text operator with the targeted glyphs taken out.
///
/// Returns the replacement, how many glyphs went, the boxes they occupied, and
/// a warning when the font could not be cut up safely.
fn rewrite_show(
    op: &Operation,
    state: &TextState,
    text_matrix: &mut Matrix,
    ctm: Matrix,
    areas: &[Rect],
    needles: &[(&str, bool)],
) -> (Operation, u32, Vec<Rect>, Option<String>) {
    // Flatten the operator into the pieces it shows: strings and kerns.
    let mut pieces: Vec<Piece> = Vec::new();
    match op.operator.as_str() {
        "TJ" => {
            if let Some(Object::Array(items)) = op.operands.first() {
                for item in items {
                    match item {
                        Object::String(bytes, format) => {
                            pieces.push(Piece::Text(bytes.clone(), *format))
                        }
                        other => {
                            if let Ok(kern) = other.as_float() {
                                pieces.push(Piece::Kern(kern));
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // Tj, ' and " all show one string; " also sets spacing first.
            if let Some(Object::String(bytes, format)) =
                op.operands.iter().rev().find(|o| matches!(o, Object::String(..)))
            {
                pieces.push(Piece::Text(bytes.clone(), *format));
            }
        }
    }

    let composite = state.metrics.map(|m| m.composite).unwrap_or(true);

    // Which byte offsets a needle covers, judged on the decoded text. Only
    // meaningful when one byte is one glyph.
    let decoded: String = if composite {
        String::new()
    } else {
        pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text(bytes, _) => Some(decode(bytes, state.encoding)),
                Piece::Kern(_) => None,
            })
            .collect()
    };
    let needle_hits = if composite { Vec::new() } else { find_hits(&decoded, needles) };

    let mut out_items: Vec<Object> = Vec::new();
    let mut removed = 0u32;
    let mut boxes: Vec<Rect> = Vec::new();
    let mut warning = None;
    let mut global_index = 0usize;
    let mut pending: Vec<u8> = Vec::new();
    let mut pending_format = lopdf::StringFormat::Literal;

    for piece in &pieces {
        match piece {
            Piece::Kern(kern) => {
                if !pending.is_empty() {
                    out_items.push(Object::String(std::mem::take(&mut pending), pending_format));
                }
                out_items.push(Object::Real(*kern));
                advance(text_matrix, -kern / 1000.0 * state.font_size * state.horizontal);
            }
            Piece::Text(bytes, format) => {
                pending_format = *format;
                if composite {
                    // One glyph is not one byte here, so the string cannot be
                    // cut up without corrupting it. The unit becomes the whole
                    // string: all of it or none.
                    let width = composite_width(bytes, state);
                    let rect = glyph_rect(*text_matrix, ctm, state.font_size, width);
                    let hit = areas.iter().any(|a| a.overlaps(&rect))
                        || (!needles.is_empty() && composite_text_hit(bytes, state, needles));
                    if hit {
                        removed += bytes.len().max(1) as u32;
                        boxes.push(rect);
                        if !pending.is_empty() {
                            out_items
                                .push(Object::String(std::mem::take(&mut pending), pending_format));
                        }
                        out_items.push(Object::Real(-width * 1000.0 / state.font_size.max(0.001)));
                        warning = Some(
                            "this page uses a font whose glyphs aren't single bytes, so whole \
                             pieces of text were removed rather than individual letters"
                                .into(),
                        );
                    } else {
                        pending.extend_from_slice(bytes);
                    }
                    advance(text_matrix, width);
                    continue;
                }

                for byte in bytes {
                    let w1000 = state.metrics.map(|m| m.width_of(*byte)).unwrap_or(500.0);
                    let width = (w1000 / 1000.0 * state.font_size
                        + state.char_spacing
                        + if *byte == b' ' { state.word_spacing } else { 0.0 })
                        * state.horizontal;
                    let rect = glyph_rect(*text_matrix, ctm, state.font_size, width);

                    let hit = areas.iter().any(|a| a.overlaps(&rect))
                        || needle_hits.contains(&global_index);

                    if hit {
                        if !pending.is_empty() {
                            out_items
                                .push(Object::String(std::mem::take(&mut pending), pending_format));
                        }
                        // The glyph's own width, as a kern, so nothing after it
                        // moves.
                        let kern = -(width / state.horizontal.max(0.001)) * 1000.0
                            / state.font_size.max(0.001);
                        out_items.push(Object::Real(kern));
                        removed += 1;
                        boxes.push(rect);
                    } else {
                        pending.push(*byte);
                    }
                    advance(text_matrix, width);
                    global_index += 1;
                }
            }
        }
    }

    if !pending.is_empty() {
        out_items.push(Object::String(pending, pending_format));
    }

    if removed == 0 {
        // Nothing to do: hand back exactly what was there, so an untouched
        // page is byte-for-byte untouched.
        return (op.clone(), 0, Vec::new(), None);
    }

    (Operation::new("TJ", vec![Object::Array(out_items)]), removed, boxes, warning)
}

enum Piece {
    Text(Vec<u8>, lopdf::StringFormat),
    Kern(f32),
}

fn decode(bytes: &[u8], encoding: Option<&lopdf::Encoding>) -> String {
    match encoding {
        Some(encoding) => Document::decode_text(encoding, bytes)
            .unwrap_or_else(|_| bytes.iter().map(|b| *b as char).collect()),
        None => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// Character offsets covered by any needle.
fn find_hits(text: &str, needles: &[(&str, bool)]) -> Vec<usize> {
    let mut hits = Vec::new();
    for (needle, match_case) in needles {
        let (hay, pin) = if *match_case {
            (text.to_string(), needle.to_string())
        } else {
            (text.to_lowercase(), needle.to_lowercase())
        };
        let mut from = 0usize;
        while let Some(found) = hay[from..].find(&pin) {
            let start = from + found;
            // Character offsets, not byte offsets: the caller indexes glyphs.
            let char_start = hay[..start].chars().count();
            let char_len = pin.chars().count();
            hits.extend(char_start..char_start + char_len);
            from = start + pin.len().max(1);
        }
    }
    hits
}

fn composite_width(bytes: &[u8], state: &TextState) -> f32 {
    // Two bytes per glyph is the overwhelmingly common case for Type0, and
    // this only decides what a box touches.
    let glyphs = (bytes.len() as f32 / 2.0).ceil();
    glyphs * (0.5 * state.font_size + state.char_spacing) * state.horizontal
}

fn composite_text_hit(bytes: &[u8], state: &TextState, needles: &[(&str, bool)]) -> bool {
    let text = decode(bytes, state.encoding);
    needles.iter().any(|(needle, match_case)| {
        if *match_case {
            text.contains(needle)
        } else {
            text.to_lowercase().contains(&needle.to_lowercase())
        }
    })
}

/// The box one glyph occupies, in page coordinates.
fn glyph_rect(text_matrix: Matrix, ctm: Matrix, font_size: f32, width: f32) -> Rect {
    let combined = text_matrix.times(ctm);
    let (x, y) = combined.translation();
    let scale = combined.vertical_scale().max(0.0001);
    let h_scale = combined.horizontal_scale().max(0.0001);
    let height = font_size.abs() * scale;
    // A little below the baseline for descenders, a little above for the
    // cap height — a box that clips the top of a letter is not a redaction.
    Rect::of(x, y - height * 0.25, x + width * h_scale, y + height * 0.9)
}

fn advance(text_matrix: &mut Matrix, by: f32) {
    let shift = Matrix([1.0, 0.0, 0.0, 1.0, by, 0.0]);
    *text_matrix = shift.times(*text_matrix);
}

/// Join boxes that touch, so a redacted word is one bar rather than a row of
/// separate little rectangles with gaps the letters show through.
fn merge_boxes(boxes: &[Rect]) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for rect in boxes {
        // Grown slightly, so adjacent glyphs count as touching.
        let probe = Rect {
            x0: rect.x0 - 1.0,
            y0: rect.y0 - 0.5,
            x1: rect.x1 + 1.0,
            y1: rect.y1 + 0.5,
        };
        match out.iter_mut().find(|existing| existing.overlaps(&probe)) {
            Some(existing) => *existing = existing.union(rect),
            None => out.push(*rect),
        }
    }
    out
}

/* ---------- images ---------- */

/// Paint out the part of each drawn image that falls under an area.
fn blank_images(
    doc: &mut Document,
    page_id: ObjectId,
    draws: &[(Vec<u8>, Matrix)],
    areas: &[Rect],
) -> Result<(u32, Vec<String>)> {
    let mut edited = 0;
    let mut warnings = Vec::new();

    // Resources can sit inline on the page or behind a reference, and a page
    // that inherits them has neither — so take whichever is actually there.
    let mut xobjects = Dictionary::new();
    if let Ok((inline, referenced)) = doc.get_page_resources(page_id) {
        let mut dicts: Vec<Dictionary> = Vec::new();
        if let Some(dict) = inline {
            dicts.push(dict.clone());
        }
        for id in referenced {
            if let Ok(dict) = doc.get_dictionary(id) {
                dicts.push(dict.clone());
            }
        }
        for dict in dicts {
            if let Some(found) = dict
                .get(b"XObject")
                .ok()
                .and_then(|o| resolve(doc, o))
                .and_then(|o| o.as_dict().ok().cloned())
            {
                for (key, value) in found.iter() {
                    xobjects.set(key.to_vec(), value.clone());
                }
            }
        }
    }

    for (name, ctm) in draws {
        let Ok(Object::Reference(id)) = xobjects.get(name).map(|o| o.clone()) else { continue };
        // The unit square maps through the CTM, so the drawn area is the
        // image's footprint on the page.
        let (x, y) = ctm.translation();
        let drawn = Rect::of(x, y, x + ctm.0[0] + ctm.0[2], y + ctm.0[1] + ctm.0[3]);
        let covering: Vec<&Rect> = areas.iter().filter(|a| a.overlaps(&drawn)).collect();
        if covering.is_empty() {
            continue;
        }

        let Some(decoded) = super::images::decode_image(doc, id) else {
            warnings.push(
                "an image under a redaction box couldn't be read, so its pixels are still in the \
                 file — the box only covers it"
                    .into(),
            );
            continue;
        };

        let mut pixels = decoded.to_rgb8();
        let (iw, ih) = (pixels.width() as f32, pixels.height() as f32);
        if iw < 1.0 || ih < 1.0 || drawn.x1 - drawn.x0 <= 0.0 || drawn.y1 - drawn.y0 <= 0.0 {
            continue;
        }

        for area in covering {
            // Page coordinates back into the image's own pixel grid. The
            // image's y runs downwards, the page's upwards.
            let u0 = ((area.x0 - drawn.x0) / (drawn.x1 - drawn.x0) * iw).floor().max(0.0) as u32;
            let u1 = ((area.x1 - drawn.x0) / (drawn.x1 - drawn.x0) * iw).ceil().min(iw) as u32;
            let v0 = ((drawn.y1 - area.y1) / (drawn.y1 - drawn.y0) * ih).floor().max(0.0) as u32;
            let v1 = ((drawn.y1 - area.y0) / (drawn.y1 - drawn.y0) * ih).ceil().min(ih) as u32;

            for v in v0..v1.min(pixels.height()) {
                for u in u0..u1.min(pixels.width()) {
                    pixels.put_pixel(u, v, image::Rgb([0, 0, 0]));
                }
            }
        }

        let replacement = image::DynamicImage::ImageRgb8(pixels);
        super::images::replace_image(doc, id, &replacement, 84, false)?;
        edited += 1;
    }

    Ok((edited, warnings))
}

/* ---------- the places text hides ---------- */

/// Remove annotations that overlap an area or carry a needle.
fn strip_annotations(
    doc: &mut Document,
    page_id: ObjectId,
    areas: &[Rect],
    needles: &[(&str, bool)],
) -> u32 {
    let Ok(page) = doc.get_dictionary(page_id) else { return 0 };
    let Ok(annots) = page.get(b"Annots") else { return 0 };
    let Some(list) = resolve(doc, annots).and_then(|o| o.as_array().ok().cloned()) else {
        return 0;
    };

    let mut keep = Vec::new();
    let mut removed = 0;
    for entry in list {
        let dict = resolve(doc, &entry).and_then(|o| o.as_dict().ok().cloned());
        let doomed = dict.as_ref().is_some_and(|d| {
            let overlaps = d
                .get(b"Rect")
                .ok()
                .and_then(|o| o.as_array().ok())
                .filter(|r| r.len() >= 4)
                .map(|r| {
                    let v: Vec<f32> =
                        r.iter().map(|o| o.as_float().unwrap_or(0.0)).collect();
                    Rect::of(v[0], v[1], v[2], v[3])
                })
                .map(|rect| areas.iter().any(|a| a.overlaps(&rect)))
                .unwrap_or(false);

            // Everything an annotation can say, gathered up and searched.
            let words: String = [&b"Contents"[..], b"T", b"Subj", b"RC", b"V", b"DV"]
                .iter()
                .filter_map(|key| d.get(key).ok())
                .filter_map(|o| o.as_str().ok())
                .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let named = needles.iter().any(|(needle, match_case)| {
                if *match_case {
                    words.contains(needle)
                } else {
                    words.to_lowercase().contains(&needle.to_lowercase())
                }
            });

            overlaps || named
        });

        if doomed {
            removed += 1;
        } else {
            keep.push(entry);
        }
    }

    if removed > 0 {
        if let Ok(page) = doc.get_dictionary_mut(page_id) {
            if keep.is_empty() {
                page.remove(b"Annots");
            } else {
                page.set("Annots", Object::Array(keep));
            }
        }
    }
    removed
}

/// Strip the document-wide hiding places.
fn strip_extras(doc: &mut Document, report: &mut RedactReport) {
    // Metadata: the title, author and keywords routinely repeat what the
    // document says, and XMP keeps its own copy of all of it.
    doc.trailer.remove(b"Info");
    let catalog_id = doc.catalog().ok().and_then(|_| {
        doc.trailer.get(b"Root").ok().and_then(|o| match o {
            Object::Reference(id) => Some(*id),
            _ => None,
        })
    });
    if let Some(id) = catalog_id {
        if let Ok(catalog) = doc.get_dictionary_mut(id) {
            for key in [
                &b"Metadata"[..],
                // Bookmarks name sections, and a section is often the thing
                // being redacted.
                b"Outlines",
                b"Names",
                b"OpenAction",
                b"AA",
                b"AcroForm",
                b"StructTreeRoot",
            ] {
                catalog.remove(key);
            }
        }
    }
    report.warnings.push(
        "Document metadata, bookmarks, embedded files and any form data were removed as well."
            .into(),
    );
}

/* ---------- checking the work ---------- */

/// Re-read the finished file and look for what was supposed to be gone.
///
/// This is the only part that turns the claim into a fact, so it runs on the
/// saved bytes rather than the in-memory document: whatever a reader will open
/// is what gets checked.
fn verify(bytes: &[u8], options: &RedactOptions, report: &mut RedactReport) {
    let Ok(doc) = Document::load_mem(bytes) else {
        report.warnings.push("The result couldn't be re-read to check it.".into());
        report.verified = false;
        return;
    };

    let pages: Vec<String> = doc
        .page_iter()
        .map(|id| super::markdown::page_lines(&doc, id).join("\n"))
        .collect();

    for target in &options.targets {
        let Target::Text { needle, match_case } = target else { continue };
        for (index, text) in pages.iter().enumerate() {
            let found = if *match_case {
                text.contains(needle)
            } else {
                text.to_lowercase().contains(&needle.to_lowercase())
            };
            if found {
                report.leaks.push(format!(
                    "'{needle}' can still be read on page {}",
                    index + 1
                ));
            }
        }
    }

    // Then every object, because the text extractor only reads the streams a
    // page points at. A leftover one it no longer points at is invisible to
    // extraction and perfectly readable to anyone else — which is how the
    // first version of this module leaked.
    for target in &options.targets {
        let Target::Text { needle, .. } = target else { continue };
        // Two or three characters match far too much to mean anything.
        if needle.trim().len() < 4 {
            continue;
        }
        if let Some(where_) = find_in_objects(&doc, needle) {
            let message = format!("'{needle}' is still readable in the file ({where_})");
            if !report.leaks.contains(&message) {
                report.leaks.push(message);
            }
        }
    }

    report.verified = report.leaks.is_empty();
}

/// Hunt for `needle` in every object of the document, streams decompressed.
///
/// Deliberately paranoid: it looks at orphans, metadata, annotation values and
/// anything else carrying bytes, not only the streams a page is wired to.
fn find_in_objects(doc: &Document, needle: &str) -> Option<String> {
    let lowered = needle.to_lowercase();
    let hit = |bytes: &[u8]| -> bool {
        String::from_utf8_lossy(bytes).to_lowercase().contains(&lowered)
    };

    for (id, object) in &doc.objects {
        match object {
            Object::Stream(stream) => {
                let content = stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone());
                if hit(&content) {
                    return Some(format!("object {} {}", id.0, id.1));
                }
            }
            Object::String(bytes, _) => {
                if hit(bytes) {
                    return Some(format!("a string in object {} {}", id.0, id.1));
                }
            }
            Object::Dictionary(dict) => {
                for (_, value) in dict.iter() {
                    if let Object::String(bytes, _) = value {
                        if hit(bytes) {
                            return Some(format!("a value in object {} {}", id.0, id.1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn text_of(bytes: &[u8]) -> String {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|id| super::super::markdown::page_lines(&doc, id).join("\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn by_text(needle: &str) -> RedactOptions {
        RedactOptions {
            targets: vec![Target::Text { needle: needle.into(), match_case: false }],
            ..Default::default()
        }
    }

    #[test]
    fn the_text_is_gone_from_the_file_not_just_hidden() {
        let src = testkit::pdf_with_sized_text(&[
            (14.0, "Name: Jane Doe"),
            (14.0, "Account 55512345"),
            (14.0, "Everything else stays"),
        ]);
        assert!(text_of(&src).contains("55512345"), "the fixture should contain it");

        let (out, report) = redact(&src, &by_text("55512345")).unwrap();

        // The claim and the check.
        assert!(report.verified, "leaks: {:?}", report.leaks);
        assert!(report.leaks.is_empty());
        assert!(report.glyphs_removed >= 8, "{report:?}");

        let after = text_of(&out);
        assert!(!after.contains("55512345"), "still extractable: {after}");
        // And the raw bytes, which is where the black-box tools fail.
        assert!(
            !String::from_utf8_lossy(&out).contains("55512345"),
            "the digits are still in the file"
        );
    }

    #[test]
    fn the_old_drawing_is_deleted_rather_than_left_lying_around() {
        // The bug this guards: pointing the page at a rewritten stream and
        // leaving the original behind. The page looks redacted, extraction
        // finds nothing, and the text is still in the file.
        let src = testkit::pdf_with_sized_text(&[(14.0, "confidential matter here")]);
        let (out, report) = redact(&src, &by_text("confidential")).unwrap();
        assert!(report.verified, "{:?}", report.leaks);

        let doc = Document::load_mem(&out).unwrap();
        assert!(
            find_in_objects(&doc, "confidential").is_none(),
            "the word survives in some object"
        );
    }

    #[test]
    fn the_object_sweep_finds_what_page_extraction_misses() {
        // An orphan: a stream no page points at. Extraction can't see it, so
        // the sweep is the only thing standing between it and a leak.
        let mut doc = Document::load_mem(&testkit::pdf_with_sized_text(&[(14.0, "visible")]))
            .unwrap();
        doc.add_object(Stream::new(
            Dictionary::new(),
            b"BT (classified-dossier) Tj ET".to_vec(),
        ));
        let mut orphaned = Vec::new();
        doc.save_to(&mut orphaned).unwrap();

        let reloaded = Document::load_mem(&orphaned).unwrap();
        let pages: String = reloaded
            .page_iter()
            .map(|id| super::super::markdown::page_lines(&reloaded, id).join("
"))
            .collect();
        assert!(!pages.contains("classified-dossier"), "extraction shouldn't see it");
        assert!(
            find_in_objects(&reloaded, "classified-dossier").is_some(),
            "but the sweep must"
        );
    }

    /// A scanned page: a picture of the document, with an invisible text layer
    /// over it. What an OCR'd contract actually looks like.
    fn scanned_page_with_a_text_layer() -> Vec<u8> {
        use lopdf::{dictionary, Stream};

        let mut doc = Document::with_version("1.5");
        // A bright image standing in for the scan.
        let picture =
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(400, 300, image::Rgb([235, 232, 226])));
        let jpeg = super::super::images::encode_jpeg_for_pdf(&picture, 92, false).unwrap();
        let image_id =
            doc.add_object(super::super::images::image_stream(jpeg, 400, 300, false));

        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        // Tr 3 is invisible text: the reader sees the picture, the extractor
        // sees the words.
        let content = b"q 400 0 0 300 0 0 cm /Im0 Do Q
BT /F1 24 Tf 3 Tr 1 0 0 1 40 150 Tm (CLASSIFIED) Tj ET
".to_vec();
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));

        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 400.into(), 300.into()],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Im0" => image_id },
                "Font" => dictionary! { "F1" => font_id },
            },
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn on_a_scan_the_picture_of_the_word_goes_too() {
        // The failure this guards is the whole reason the tool exists. On an
        // OCR'd scan, deleting the text layer makes the word unextractable
        // while the photograph of it stays under the bar — lift the bar and
        // read it. Both have to go.
        let src = scanned_page_with_a_text_layer();
        assert!(text_of(&src).contains("CLASSIFIED"), "the fixture should carry the layer");

        let (out, report) = redact(&src, &by_text("CLASSIFIED")).unwrap();
        assert!(report.verified, "{:?}", report.leaks);
        assert!(!text_of(&out).contains("CLASSIFIED"), "the text layer should be gone");
        assert_eq!(report.images_edited, 1, "the scan itself must be edited: {report:?}");

        // And the pixels really changed.
        let doc = Document::load_mem(&out).unwrap();
        let id = *doc
            .objects
            .iter()
            .find(|(_, o)| matches!(o, Object::Stream(s)
                if s.dict.get(b"Subtype").ok().and_then(|x| x.as_name().ok())
                    == Some(b"Image".as_ref())))
            .unwrap()
            .0;
        let decoded = super::super::images::decode_image(&doc, id).expect("should decode");
        let black = decoded.to_rgb8().pixels().filter(|p| p.0[0] < 30).count();
        assert!(black > 200, "only {black} dark pixels — the scan wasn't painted out");
    }

    #[test]
    fn unhooked_objects_are_swept_up_rather_than_left_in_the_file() {
        // Removing /Info from the trailer orphans the dictionary; the title
        // and keywords stay in the file and routinely repeat the secret. This
        // is the same mistake as leaving the old content stream behind, one
        // level up, and the checker found it the same way.
        let mut doc =
            Document::load_mem(&testkit::pdf_with_sized_text(&[(14.0, "the body")])).unwrap();
        let mut info = Dictionary::new();
        info.set("Title", Object::string_literal("Operation Moonlight briefing"));
        let info = doc.add_object(info);
        doc.trailer.set("Info", info);
        let mut with_title = Vec::new();
        doc.save_to(&mut with_title).unwrap();

        let (out, report) = redact(&with_title, &by_text("Moonlight")).unwrap();
        assert!(report.verified, "{:?}", report.leaks);
        assert!(
            !String::from_utf8_lossy(&out).contains("Moonlight"),
            "the title survived in the output"
        );
    }

    #[test]
    fn the_rest_of_the_line_survives() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "Keep this REMOVE and this too")]);
        let (out, _) = redact(&src, &by_text("REMOVE")).unwrap();
        let after = text_of(&out);
        assert!(after.contains("Keep this"), "{after}");
        assert!(after.contains("and this too"), "{after}");
        assert!(!after.contains("REMOVE"), "{after}");
    }

    #[test]
    fn untargeted_pages_are_left_alone() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "nothing to see")]);
        let (out, report) = redact(&src, &by_text("absent-from-this-document")).unwrap();
        assert!(report.pages_changed.is_empty(), "{report:?}");
        assert_eq!(report.glyphs_removed, 0);
        assert!(text_of(&out).contains("nothing to see"));
    }

    #[test]
    fn matching_can_be_case_sensitive() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "Secret and secret")]);
        let exact = RedactOptions {
            targets: vec![Target::Text { needle: "Secret".into(), match_case: true }],
            ..Default::default()
        };
        let (out, _) = redact(&src, &exact).unwrap();
        let after = text_of(&out);
        assert!(after.contains("secret"), "the lower-case one should remain: {after}");
        assert!(!after.contains("Secret"), "{after}");
    }

    #[test]
    fn an_area_removes_whatever_is_under_it() {
        let src = testkit::pdf_with_sized_text(&[
            (14.0, "first line near the top"),
            (14.0, "second line below it"),
        ]);
        // The whole page, so everything must go.
        let everything = RedactOptions {
            targets: vec![Target::Area { page: 1, x: 0.0, y: 0.0, width: 1.0, height: 1.0 }],
            ..Default::default()
        };
        let (out, report) = redact(&src, &everything).unwrap();
        assert!(report.glyphs_removed > 20, "{report:?}");
        let after = text_of(&out);
        assert!(!after.contains("first line"), "{after}");
        assert!(!after.contains("second line"), "{after}");
    }

    #[test]
    fn an_area_elsewhere_on_the_page_leaves_the_text_alone() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "this text is near the top")]);
        // A small box in the bottom corner, nowhere near the writing.
        let corner = RedactOptions {
            targets: vec![Target::Area { page: 1, x: 0.7, y: 0.9, width: 0.2, height: 0.08 }],
            ..Default::default()
        };
        let (out, report) = redact(&src, &corner).unwrap();
        assert_eq!(report.glyphs_removed, 0, "nothing was under the box");
        assert!(text_of(&out).contains("this text is near the top"));
    }

    #[test]
    fn the_pixels_under_a_box_are_painted_out_of_the_image() {
        // The fixture draws its image in the bottom-left corner of a Letter
        // page, so the box has to be aimed there.
        let src = testkit::pdf_with_image(400, 300, 288.0, 216.0, 90);
        let area = RedactOptions {
            targets: vec![Target::Area { page: 1, x: 0.0, y: 0.75, width: 0.4, height: 0.25 }],
            ..Default::default()
        };
        let (out, report) = redact(&src, &area).unwrap();
        assert_eq!(report.images_edited, 1, "{report:?}");

        // The image that comes back out must actually carry black pixels.
        let doc = Document::load_mem(&out).unwrap();
        let id = *doc
            .objects
            .iter()
            .find(|(_, o)| matches!(o, Object::Stream(s)
                if s.dict.get(b"Subtype").ok().and_then(|x| x.as_name().ok())
                    == Some(b"Image".as_ref())))
            .unwrap()
            .0;
        let decoded = super::super::images::decode_image(&doc, id).expect("should decode");
        let rgb = decoded.to_rgb8();
        let black = rgb.pixels().filter(|p| p.0[0] < 12 && p.0[1] < 12 && p.0[2] < 12).count();
        assert!(
            black > (rgb.width() * rgb.height() / 20) as usize,
            "only {black} black pixels — the image wasn't painted out"
        );
    }

    #[test]
    fn every_report_states_the_vector_limit() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "something to remove")]);
        let (_, report) = redact(&src, &by_text("remove")).unwrap();
        assert!(report.note.contains("Vector drawings"), "{}", report.note);
        assert!(report.note.contains("deleted, not hidden"), "{}", report.note);
    }

    #[test]
    fn the_black_box_can_be_turned_off_but_the_text_still_goes() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "delete me please")]);
        let bare = RedactOptions { draw_boxes: false, ..by_text("delete me") };
        let (out, report) = redact(&src, &bare).unwrap();
        assert!(report.verified);
        assert!(!text_of(&out).contains("delete me"));

        let doc = Document::load_mem(&out).unwrap();
        let content = String::from_utf8_lossy(&doc.get_page_content(doc.page_iter().next().unwrap()))
            .to_string();
        assert!(!content.contains(" re\n"), "no box should have been drawn: {content}");
    }

    #[test]
    fn metadata_is_stripped_so_the_title_cannot_give_it_away() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "body text")]);
        let (out, report) = redact(&src, &by_text("body")).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        assert!(doc.trailer.get(b"Info").is_err(), "the Info dictionary should be gone");
        assert!(
            report.warnings.iter().any(|w| w.contains("metadata")),
            "it should say so: {:?}",
            report.warnings
        );
    }

    #[test]
    fn the_verification_reports_a_leak_rather_than_claiming_success() {
        // Verification has to be capable of saying no, or it means nothing.
        let mut report = RedactReport {
            pages_changed: vec![],
            glyphs_removed: 0,
            images_edited: 0,
            annotations_removed: 0,
            verified: true,
            leaks: vec![],
            warnings: vec![],
            note: String::new(),
        };
        let src = testkit::pdf_with_sized_text(&[(14.0, "still here")]);
        verify(&src, &by_text("still here"), &mut report);
        assert!(!report.verified, "an un-redacted file must not verify");
        assert!(!report.leaks.is_empty());
    }

    #[test]
    fn nothing_to_do_is_refused_rather_than_copied() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "text")]);
        assert!(redact(&src, &RedactOptions::default()).is_err());
        let blank = RedactOptions {
            targets: vec![Target::Text { needle: "   ".into(), match_case: false }],
            ..Default::default()
        };
        assert!(redact(&src, &blank).is_err());
    }

    #[test]
    fn a_page_that_does_not_exist_is_refused() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "text")]);
        let bad = RedactOptions {
            targets: vec![Target::Area { page: 7, x: 0.1, y: 0.1, width: 0.2, height: 0.2 }],
            ..Default::default()
        };
        let error = redact(&src, &bad).unwrap_err().to_string();
        assert!(error.contains("no page 7"), "{error}");
    }

    #[test]
    fn a_box_off_the_page_is_refused() {
        let src = testkit::pdf_with_sized_text(&[(14.0, "text")]);
        let bad = RedactOptions {
            targets: vec![Target::Area { page: 1, x: 1.5, y: 0.1, width: 0.2, height: 0.2 }],
            ..Default::default()
        };
        assert!(redact(&src, &bad).is_err());
    }

    #[test]
    fn several_occurrences_all_go() {
        let src = testkit::pdf_with_sized_text(&[
            (14.0, "alpha target beta"),
            (14.0, "target again here"),
            (14.0, "and one more target"),
        ]);
        let (out, report) = redact(&src, &by_text("target")).unwrap();
        assert!(report.verified, "{:?}", report.leaks);
        let after = text_of(&out);
        assert!(!after.contains("target"), "{after}");
        assert!(after.contains("alpha"), "{after}");
        assert!(after.contains("again here"), "{after}");
    }

    #[test]
    fn the_page_count_and_other_content_survive() {
        let src = testkit::pdf_scan(3, 160, 200, 70);
        let area = RedactOptions {
            targets: vec![Target::Area { page: 2, x: 0.2, y: 0.2, width: 0.3, height: 0.2 }],
            ..Default::default()
        };
        let (out, report) = redact(&src, &area).unwrap();
        assert_eq!(super::super::page_count(&out).unwrap(), 3);
        assert_eq!(report.pages_changed, vec![2]);
    }
}


