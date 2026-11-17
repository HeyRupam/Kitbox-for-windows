//! Turning a PDF back into text with structure.
//!
//! A PDF has no paragraphs, no headings and no lists. It has glyphs at
//! coordinates. Anything that looks like structure has to be inferred from
//! where the text sits and how big it is, which is why this walks the content
//! streams itself rather than taking a flat string: positions and font sizes
//! are exactly the information a flat extraction throws away.
//!
//! What it infers:
//!
//! * **Lines** — runs sharing a baseline, joined with spaces where the gap
//!   between them is wider than a space would be.
//! * **Headings** — lines set noticeably larger than the body size, which is
//!   taken as the most common size weighted by how much text is set in it.
//! * **Paragraphs** — broken where the vertical gap grows beyond the usual
//!   line spacing.
//! * **Lists** — lines starting with a bullet or a number.
//!
//! A scanned page has no text at all, and that is reported rather than
//! returned as a blank section: the honest answer is "this needs OCR".

use std::collections::BTreeMap;

use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownOptions {
    /// Put a rule and a comment between pages.
    pub page_breaks: bool,
    /// Infer headings from font size. Off, everything becomes a paragraph.
    pub detect_headings: bool,
    /// Infer bullet and numbered lists.
    pub detect_lists: bool,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        MarkdownOptions { page_breaks: false, detect_headings: true, detect_lists: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownReport {
    pub pages: u32,
    /// Pages that actually had a text layer.
    pub pages_with_text: u32,
    pub characters: u32,
    /// Page numbers that look like scans — an image and no text.
    pub scanned_pages: Vec<u32>,
    /// Pages whose text is laid out in columns or scattered labels rather than
    /// prose, where reading top-to-bottom gives a jumbled order.
    pub layout_pages: Vec<u32>,
    pub note: Option<String>,
}

/// Convert a PDF's text to Markdown.
pub fn to_markdown(bytes: &[u8], options: &MarkdownOptions) -> Result<(String, MarkdownReport)> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }

    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    if page_ids.is_empty() {
        return Err(Error::invalid("this PDF has no pages"));
    }

    let mut out = String::new();
    let mut pages_with_text = 0u32;
    let mut scanned_pages = Vec::new();
    let mut layout_pages = Vec::new();

    for (index, page_id) in page_ids.iter().enumerate() {
        let number = index as u32 + 1;
        let runs = extract_runs(&doc, *page_id);

        if runs.is_empty() {
            // No text. If there is an image filling the page, it is a scan.
            if super::pages::page_has_image(&doc, *page_id) {
                scanned_pages.push(number);
            }
            continue;
        }
        pages_with_text += 1;

        if options.page_breaks && !out.is_empty() {
            out.push_str("\n---\n\n");
        }
        if options.page_breaks {
            out.push_str(&format!("<!-- page {number} -->\n\n"));
        }

        let lines = group_into_lines(runs);
        if looks_like_layout(&lines) {
            layout_pages.push(number);
        }
        let body_size = body_size_of(&lines);
        out.push_str(&render(&lines, body_size, options));
    }

    let markdown = tidy(&out);
    let note = describe(
        &scanned_pages,
        &layout_pages,
        pages_with_text,
        page_ids.len() as u32,
    );

    Ok((
        markdown.clone(),
        MarkdownReport {
            pages: page_ids.len() as u32,
            pages_with_text,
            characters: markdown.chars().count() as u32,
            scanned_pages,
            layout_pages,
            note,
        },
    ))
}

fn describe(scanned: &[u32], layout: &[u32], with_text: u32, total: u32) -> Option<String> {
    if with_text == 0 {
        return Some(
            "None of these pages has a text layer \u{2014} this is a scan, and reading it needs \
             OCR, which isn't built yet."
                .into(),
        );
    }

    let mut parts = Vec::new();
    if !scanned.is_empty() {
        parts.push(format!(
            "{} of {total} pages are scans with no text layer, so nothing came out of them; \
             reading those needs OCR, which isn't built yet.",
            scanned.len()
        ));
    }
    if !layout.is_empty() {
        parts.push(format!(
            "{} page{} laid out in columns or scattered labels rather than prose, so the reading \
             order there will be jumbled \u{2014} this works best on ordinary documents.",
            layout.len(),
            if layout.len() == 1 { " is" } else { "s are" }
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Whether a page reads as prose or as a layout.
///
/// Columns, forms and diagrams produce many short lines, and reading those top
/// to bottom interleaves things that were never meant to be adjacent. Saying so
/// is far more useful than silently producing nonsense \u{2014} and far cheaper than
/// reconstructing the columns.
fn looks_like_layout(lines: &[Line]) -> bool {
    if lines.len() < 8 {
        return false;
    }
    let words: usize = lines.iter().map(|l| l.text.split_whitespace().count()).sum();
    let average = words as f32 / lines.len() as f32;
    let short = lines.iter().filter(|l| l.text.split_whitespace().count() <= 3).count();
    average < 5.0 && short * 2 > lines.len()
}

/* ---------- extraction ---------- */

/// A piece of text with where and how big it was drawn.
#[derive(Debug, Clone)]
struct Run {
    text: String,
    x: f32,
    y: f32,
    size: f32,
    bold: bool,
}

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

    /// How much this matrix scales vertically, which is what sets text size.
    fn vertical_scale(self) -> f32 {
        (self.0[2] * self.0[2] + self.0[3] * self.0[3]).sqrt()
    }
}

fn extract_runs(doc: &Document, page_id: ObjectId) -> Vec<Run> {
    let Ok(fonts) = doc.get_page_fonts(page_id) else { return Vec::new() };

    // Encodings come from lopdf so ToUnicode maps and the standard encodings
    // are handled properly; getting that wrong turns text into mojibake.
    let mut encodings = BTreeMap::new();
    let mut bold_fonts = BTreeMap::new();
    for (name, font) in &fonts {
        if let Ok(encoding) = font.get_font_encoding(doc) {
            encodings.insert(name.clone(), encoding);
        }
        let base = font
            .get(b"BaseFont")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).to_ascii_lowercase())
            .unwrap_or_default();
        bold_fonts.insert(name.clone(), base.contains("bold") || base.contains("black"));
    }

    let content_data = doc.get_page_content(page_id);
    let Ok(content) = Content::decode(&content_data) else { return Vec::new() };

    let mut runs = Vec::new();
    let mut ctm = Matrix::IDENTITY;
    let mut stack: Vec<Matrix> = Vec::new();
    let mut text_matrix = Matrix::IDENTITY;
    let mut line_matrix = Matrix::IDENTITY;
    let mut font_name: Vec<u8> = Vec::new();
    let mut font_size = 12.0f32;
    let mut leading = 0.0f32;

    let number = |operands: &[Object], i: usize| -> f32 {
        operands.get(i).and_then(|o| o.as_float().ok()).unwrap_or(0.0)
    };

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(Matrix::IDENTITY),
            "cm" if op.operands.len() >= 6 => {
                let mut m = [0.0f32; 6];
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
            "Tm" if op.operands.len() >= 6 => {
                let mut m = [0.0f32; 6];
                for (i, slot) in m.iter_mut().enumerate() {
                    *slot = number(&op.operands, i);
                }
                text_matrix = Matrix(m);
                line_matrix = text_matrix;
            }
            "Td" => {
                let shift = Matrix([1.0, 0.0, 0.0, 1.0, number(&op.operands, 0), number(&op.operands, 1)]);
                line_matrix = shift.times(line_matrix);
                text_matrix = line_matrix;
            }
            "TD" => {
                leading = -number(&op.operands, 1);
                let shift = Matrix([1.0, 0.0, 0.0, 1.0, number(&op.operands, 0), number(&op.operands, 1)]);
                line_matrix = shift.times(line_matrix);
                text_matrix = line_matrix;
            }
            "T*" => {
                let shift = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, -leading]);
                line_matrix = shift.times(line_matrix);
                text_matrix = line_matrix;
            }
            "Tj" | "'" | "\"" => {
                if op.operator != "Tj" {
                    let shift = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, -leading]);
                    line_matrix = shift.times(line_matrix);
                    text_matrix = line_matrix;
                }
                let text = op
                    .operands
                    .iter()
                    .rev()
                    .find_map(|o| show(o, &encodings, &font_name))
                    .unwrap_or_default();
                push_run(&mut runs, text, text_matrix, ctm, font_size, &font_name, &bold_fonts);
            }
            "TJ" => {
                let Some(Object::Array(items)) = op.operands.first() else { continue };
                let mut text = String::new();
                for item in items {
                    match item {
                        Object::String(..) => {
                            if let Some(decoded) = show(item, &encodings, &font_name) {
                                text.push_str(&decoded);
                            }
                        }
                        // A large negative kern is how a PDF writes a space
                        // between words without a space character.
                        other => {
                            if let Ok(kern) = other.as_float() {
                                if kern < -170.0 && !text.ends_with(' ') {
                                    text.push(' ');
                                }
                            }
                        }
                    }
                }
                push_run(&mut runs, text, text_matrix, ctm, font_size, &font_name, &bold_fonts);
            }
            _ => {}
        }
    }

    runs
}

fn show(
    object: &Object,
    encodings: &BTreeMap<Vec<u8>, lopdf::Encoding>,
    font_name: &[u8],
) -> Option<String> {
    let Object::String(bytes, _) = object else { return None };
    match encodings.get(font_name) {
        Some(encoding) => Document::decode_text(encoding, bytes).ok(),
        // Without an encoding the bytes are a guess at best; Latin-1 is the
        // least surprising one.
        None => Some(bytes.iter().map(|b| *b as char).collect()),
    }
}

#[allow(clippy::too_many_arguments)]
fn push_run(
    runs: &mut Vec<Run>,
    text: String,
    text_matrix: Matrix,
    ctm: Matrix,
    font_size: f32,
    font_name: &[u8],
    bold_fonts: &BTreeMap<Vec<u8>, bool>,
) {
    if text.trim().is_empty() {
        return;
    }
    let combined = text_matrix.times(ctm);
    let (x, y) = combined.translation();
    let size = (font_size * combined.vertical_scale()).abs();
    if !x.is_finite() || !y.is_finite() || !size.is_finite() {
        return;
    }
    runs.push(Run {
        text,
        x,
        y,
        size: if size > 0.1 { size } else { font_size.abs().max(1.0) },
        bold: bold_fonts.get(font_name).copied().unwrap_or(false),
    });
}

/// The text of one page, as lines in reading order.
///
/// Shared with the compare tool, which needs the same line grouping: diffing
/// raw glyph runs would report a change every time a word wrapped.
pub(super) fn page_lines(doc: &Document, page_id: ObjectId) -> Vec<String> {
    group_into_lines(extract_runs(doc, page_id))
        .into_iter()
        .map(|line| line.text)
        .collect()
}

/* ---------- structure ---------- */

#[derive(Debug, Clone)]
struct Line {
    text: String,
    y: f32,
    size: f32,
    bold: bool,
}

fn group_into_lines(mut runs: Vec<Run>) -> Vec<Line> {
    // Top to bottom, then left to right.
    runs.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Run> = Vec::new();

    for run in runs {
        let same_line = current
            .first()
            .map(|first| (first.y - run.y).abs() <= first.size.max(run.size) * 0.45)
            .unwrap_or(true);
        if same_line {
            current.push(run);
        } else {
            lines.push(assemble(std::mem::take(&mut current)));
            current.push(run);
        }
    }
    if !current.is_empty() {
        lines.push(assemble(current));
    }
    lines.retain(|line| !line.text.trim().is_empty());
    lines
}

fn assemble(mut runs: Vec<Run>) -> Line {
    runs.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let y = runs.first().map(|r| r.y).unwrap_or(0.0);
    // The dominant size on the line, by how much text is set in it.
    let size = runs
        .iter()
        .max_by(|a, b| {
            (a.text.len() as f32 * a.size)
                .partial_cmp(&(b.text.len() as f32 * b.size))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|r| r.size)
        .unwrap_or(12.0);
    let bold = runs.iter().all(|r| r.bold);

    let mut text = String::new();
    let mut previous_end: Option<f32> = None;
    for run in &runs {
        if let Some(end) = previous_end {
            // Roughly half a character of blank means the writer meant a space.
            if run.x - end > run.size * 0.22 && !text.ends_with(' ') {
                text.push(' ');
            }
        }
        text.push_str(&run.text);
        previous_end = Some(run.x + run.text.chars().count() as f32 * run.size * 0.5);
    }

    Line { text: text.split_whitespace().collect::<Vec<_>>().join(" "), y, size, bold }
}

/// The body text size: the most common size, weighted by how much is set in it.
fn body_size_of(lines: &[Line]) -> f32 {
    let mut weights: BTreeMap<u32, usize> = BTreeMap::new();
    for line in lines {
        // Buckets of half a point, so near-identical sizes count together.
        *weights.entry((line.size * 2.0).round() as u32).or_default() += line.text.len();
    }
    weights
        .into_iter()
        .max_by_key(|(_, weight)| *weight)
        .map(|(bucket, _)| bucket as f32 / 2.0)
        .unwrap_or(12.0)
}

fn render(lines: &[Line], body_size: f32, options: &MarkdownOptions) -> String {
    let mut out = String::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut previous: Option<&Line> = None;

    let flush = |out: &mut String, paragraph: &mut Vec<String>| {
        if !paragraph.is_empty() {
            out.push_str(&paragraph.join(" "));
            out.push_str("\n\n");
            paragraph.clear();
        }
    };

    for line in lines {
        let heading = if options.detect_headings { heading_level(line, body_size) } else { None };
        let list = if options.detect_lists { list_marker(&line.text) } else { None };

        // A big vertical jump is a new block, whatever else is true.
        let gap_break = previous
            .map(|p| (p.y - line.y).abs() > line.size.max(p.size) * 1.75)
            .unwrap_or(false);

        if heading.is_some() || list.is_some() || gap_break {
            flush(&mut out, &mut paragraph);
        }

        match (heading, list) {
            (Some(level), _) => {
                out.push_str(&format!("{} {}\n\n", "#".repeat(level), escape(&line.text)));
            }
            (None, Some((marker, rest))) => {
                out.push_str(&format!("{marker} {}\n", escape(rest)));
            }
            (None, None) => {
                let text = escape(&line.text);
                paragraph.push(if line.bold && !text.is_empty() {
                    format!("**{text}**")
                } else {
                    text
                });
            }
        }
        previous = Some(line);
    }
    flush(&mut out, &mut paragraph);
    out
}

fn heading_level(line: &Line, body_size: f32) -> Option<usize> {
    if body_size <= 0.0 || line.text.len() > 120 {
        return None;
    }
    let ratio = line.size / body_size;
    if ratio >= 1.5 {
        Some(1)
    } else if ratio >= 1.25 {
        Some(2)
    } else if ratio >= 1.1 {
        Some(3)
    } else {
        None
    }
}

/// A leading bullet or number, and the text after it.
fn list_marker(text: &str) -> Option<(String, &str)> {
    let trimmed = text.trim_start();
    for bullet in ['\u{2022}', '\u{25cf}', '\u{25aa}', '\u{2023}', '\u{00b7}', '-', '*'] {
        if let Some(rest) = trimmed.strip_prefix(bullet) {
            if rest.starts_with(' ') || rest.starts_with('\t') {
                return Some(("-".to_string(), rest.trim_start()));
            }
        }
    }

    // "1." or "2)" — keep the number so an interrupted list stays readable.
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && digits.len() <= 3 {
        let rest = &trimmed[digits.len()..];
        for separator in [". ", ") "] {
            if let Some(after) = rest.strip_prefix(separator) {
                return Some((format!("{digits}."), after.trim_start()));
            }
        }
    }
    None
}

/// Escape the characters that would otherwise become Markdown syntax.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, ch) in text.chars().enumerate() {
        let at_start = index == 0;
        match ch {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(ch);
            }
            '#' | '>' | '|' if at_start => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Collapse runs of blank lines so the output reads cleanly.
fn tidy(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut blanks = 0;
    for line in markdown.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_start_matches('\n').trim_end().to_string() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn a_scan_is_reported_rather_than_returned_empty() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        let (markdown, report) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert!(markdown.trim().is_empty());
        assert_eq!(report.pages_with_text, 0);
        assert_eq!(report.scanned_pages, vec![1, 2]);
        let note = report.note.unwrap();
        assert!(note.contains("OCR"), "{note}");
    }

    #[test]
    fn text_comes_out_as_paragraphs() {
        let src = testkit::pdf_text_only();
        let (markdown, report) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert_eq!(report.pages_with_text, 1);
        assert!(report.scanned_pages.is_empty());
        assert!(report.note.is_none());
        assert!(
            markdown.contains("Declaration"),
            "the text should survive: {markdown:.200}"
        );
        assert!(report.characters > 50);
    }

    #[test]
    fn bigger_text_becomes_a_heading() {
        // A title at 24pt over body text at 11pt.
        let src = testkit::pdf_with_sized_text(&[
            (24.0, "Annual Report"),
            (11.0, "This paragraph is set at the body size of the document."),
            (11.0, "So is this one, which makes eleven point the common size."),
            (16.0, "Methods"),
            (11.0, "More body text follows the second heading here."),
        ]);
        let (markdown, _) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert!(markdown.contains("# Annual Report"), "{markdown}");
        assert!(
            markdown.contains("## Methods") || markdown.contains("### Methods"),
            "the smaller heading should still be one: {markdown}"
        );
        assert!(!markdown.contains("# This paragraph"), "body text is not a heading");
    }

    #[test]
    fn headings_can_be_turned_off() {
        let src = testkit::pdf_with_sized_text(&[
            (24.0, "Annual Report"),
            (11.0, "Body text that sets the common size for the page."),
        ]);
        let options = MarkdownOptions { detect_headings: false, ..Default::default() };
        let (markdown, _) = to_markdown(&src, &options).unwrap();
        assert!(!markdown.contains('#'), "{markdown}");
        assert!(markdown.contains("Annual Report"));
    }

    #[test]
    fn bullets_and_numbers_become_lists() {
        let src = testkit::pdf_with_sized_text(&[
            (11.0, "Bring the following with you to the appointment today:"),
            (11.0, "\u{2022} Photo identification"),
            (11.0, "\u{2022} Proof of address"),
            (11.0, "1. Fill in the form"),
            (11.0, "2. Sign at the bottom"),
        ]);
        let (markdown, _) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert!(markdown.contains("- Photo identification"), "{markdown}");
        assert!(markdown.contains("- Proof of address"));
        assert!(markdown.contains("1. Fill in the form"), "{markdown}");
        assert!(markdown.contains("2. Sign at the bottom"));
    }

    #[test]
    fn markdown_characters_in_the_text_are_escaped() {
        let src = testkit::pdf_with_sized_text(&[
            (11.0, "The rate is 5*3 and the file is a_b_c today."),
            (11.0, "Second line of body text to set the common size."),
        ]);
        let (markdown, _) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert!(markdown.contains(r"5\*3"), "{markdown}");
        assert!(markdown.contains(r"a\_b\_c"), "{markdown}");
    }

    #[test]
    fn page_breaks_are_optional() {
        let src = testkit::pdf_text_only();
        let without = to_markdown(&src, &MarkdownOptions::default()).unwrap().0;
        let with = to_markdown(
            &src,
            &MarkdownOptions { page_breaks: true, ..Default::default() },
        )
        .unwrap()
        .0;
        assert!(!without.contains("<!-- page"));
        assert!(with.contains("<!-- page 1 -->"), "{with:.120}");
    }

    #[test]
    fn a_protected_file_is_sent_to_unlock_first() {
        let locked = super::super::security::protect(
            &testkit::pdf_text_only(),
            &super::super::security::Protection {
                open_password: Some("pw".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let err = to_markdown(&locked, &MarkdownOptions::default()).unwrap_err().to_string();
        assert!(err.contains("unlock it first"), "{err}");
    }

    #[test]
    fn list_markers_are_recognised_but_not_invented() {
        assert_eq!(list_marker("\u{2022} item").unwrap().0, "-");
        assert_eq!(list_marker("- item").unwrap().0, "-");
        assert_eq!(list_marker("12. item").unwrap().0, "12.");
        assert_eq!(list_marker("3) item").unwrap().0, "3.");
        // Not lists.
        assert!(list_marker("2024 was a good year").is_none());
        assert!(list_marker("well-known problem").is_none());
        assert!(list_marker("1.5 litres of water").is_none());
    }

    #[test]
    fn a_page_of_scattered_labels_is_flagged_as_a_layout() {
        // Short, column-ish fragments: what a form or a diagram produces.
        let fragments: Vec<(f32, &str)> = vec![
            (11.0, "Name"),
            (11.0, "Date"),
            (11.0, "Signature"),
            (11.0, "Address"),
            (11.0, "Postcode"),
            (11.0, "Phone"),
            (11.0, "Email"),
            (11.0, "Reference"),
            (11.0, "Amount"),
            (11.0, "Branch"),
        ];
        let src = testkit::pdf_with_sized_text(&fragments);
        let (_, report) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert_eq!(report.layout_pages, vec![1]);
        let note = report.note.unwrap();
        assert!(note.contains("jumbled"), "{note}");
    }

    #[test]
    fn ordinary_prose_is_not_flagged() {
        let prose: Vec<(f32, &str)> = (0..10)
            .map(|_| (11.0, "This is an ordinary sentence of running text on the page."))
            .collect();
        let src = testkit::pdf_with_sized_text(&prose);
        let (_, report) = to_markdown(&src, &MarkdownOptions::default()).unwrap();
        assert!(report.layout_pages.is_empty());
        assert!(report.note.is_none());
    }

    #[test]
    fn the_body_size_is_the_commonest_not_the_smallest() {
        let lines = vec![
            Line { text: "A tiny footnote line".into(), y: 10.0, size: 7.0, bold: false },
            Line { text: "Body text one that is long".into(), y: 100.0, size: 11.0, bold: false },
            Line { text: "Body text two that is long".into(), y: 80.0, size: 11.0, bold: false },
            Line { text: "Title".into(), y: 120.0, size: 24.0, bold: false },
        ];
        assert_eq!(body_size_of(&lines), 11.0);
    }
}
