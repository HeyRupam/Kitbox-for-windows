//! Comparing two PDFs.
//!
//! The naive version of this tool compares page 1 to page 1, page 2 to page 2,
//! and so on — which means inserting a single page at the front reports every
//! page after it as changed. That is the behaviour people complain about, so
//! pages are aligned first and lines are aligned within them, using the same
//! diff in both cases.
//!
//! Scanned pages have no text to compare. Rather than reporting them as
//! identical because neither had anything to say, their images are compared
//! and the difference is reported as a number, clearly labelled as a visual
//! comparison rather than a textual one.

use lopdf::{Document, ObjectId};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Beyond this many lines on a side, the exact alignment is skipped: the
/// quadratic table would be larger than it is worth, and two documents that
/// different need a summary rather than a line-by-line reading.
const ALIGN_LIMIT: usize = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// Page in whichever document the line belongs to.
    pub page: u32,
    pub kind: ChangeKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualDifference {
    pub page_before: u32,
    pub page_after: u32,
    /// 0.0 identical, 1.0 completely different.
    pub difference: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareReport {
    pub pages_before: u32,
    pub pages_after: u32,
    /// Pages present in one document and not the other.
    pub pages_added: Vec<u32>,
    pub pages_removed: Vec<u32>,
    pub changes: Vec<Change>,
    pub lines_added: u32,
    pub lines_removed: u32,
    /// Pages compared as pictures because neither had text.
    pub visual: Vec<VisualDifference>,
    pub identical: bool,
    pub notes: Vec<String>,
}

impl CompareReport {
    /// A readable summary, as Markdown.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str("# What changed\n\n");

        if self.identical {
            out.push_str("The two documents have the same text on every page.\n");
            if !self.visual.is_empty() {
                out.push_str("\nSome pages are scans and were compared as pictures; see below.\n");
            }
        } else {
            out.push_str(&format!(
                "{} line{} added, {} removed, across {} and {} pages.\n",
                self.lines_added,
                if self.lines_added == 1 { "" } else { "s" },
                self.lines_removed,
                self.pages_before,
                self.pages_after
            ));
        }

        for note in &self.notes {
            out.push_str(&format!("\n> {note}\n"));
        }

        if !self.pages_added.is_empty() {
            out.push_str(&format!("\n**Pages added:** {}\n", join(&self.pages_added)));
        }
        if !self.pages_removed.is_empty() {
            out.push_str(&format!("\n**Pages removed:** {}\n", join(&self.pages_removed)));
        }

        if !self.changes.is_empty() {
            out.push_str("\n## Lines\n\n```diff\n");
            let mut page = 0;
            for change in &self.changes {
                if change.page != page {
                    page = change.page;
                    out.push_str(&format!("@@ page {page} @@\n"));
                }
                let sign = match change.kind {
                    ChangeKind::Added => '+',
                    ChangeKind::Removed => '-',
                };
                out.push_str(&format!("{sign} {}\n", change.text));
            }
            out.push_str("```\n");
        }

        if !self.visual.is_empty() {
            out.push_str("\n## Scanned pages\n\n");
            out.push_str("These have no text, so they were compared as pictures.\n\n");
            for diff in &self.visual {
                let percent = diff.difference * 100.0;
                let verdict = if percent < 1.0 {
                    "looks the same"
                } else if percent < 8.0 {
                    "slightly different"
                } else {
                    "clearly different"
                };
                out.push_str(&format!(
                    "- page {} vs {}: {percent:.1}% different — {verdict}\n",
                    diff.page_before, diff.page_after
                ));
            }
        }

        out
    }
}

fn join(pages: &[u32]) -> String {
    pages.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

pub fn compare(before: &[u8], after: &[u8]) -> Result<CompareReport> {
    let a = load(before, "the first")?;
    let b = load(after, "the second")?;

    let a_pages: Vec<ObjectId> = a.page_iter().collect();
    let b_pages: Vec<ObjectId> = b.page_iter().collect();
    if a_pages.is_empty() || b_pages.is_empty() {
        return Err(Error::invalid("one of these documents has no pages"));
    }

    let a_text: Vec<Vec<String>> =
        a_pages.iter().map(|id| super::markdown::page_lines(&a, *id)).collect();
    let b_text: Vec<Vec<String>> =
        b_pages.iter().map(|id| super::markdown::page_lines(&b, *id)).collect();

    // Align pages by their whole text, so an inserted page doesn't make
    // everything after it look rewritten.
    let a_keys: Vec<String> = a_text.iter().map(|lines| fingerprint(lines)).collect();
    let b_keys: Vec<String> = b_text.iter().map(|lines| fingerprint(lines)).collect();
    let page_ops = diff(&a_keys, &b_keys);

    let mut report = CompareReport {
        pages_before: a_pages.len() as u32,
        pages_after: b_pages.len() as u32,
        pages_added: Vec::new(),
        pages_removed: Vec::new(),
        changes: Vec::new(),
        lines_added: 0,
        lines_removed: 0,
        visual: Vec::new(),
        identical: false,
        notes: Vec::new(),
    };

    // Pages that align but aren't identical get a line-level comparison;
    // unmatched ones are reported whole.
    let mut pending_removed: Vec<usize> = Vec::new();
    let mut pending_added: Vec<usize> = Vec::new();
    let mut scanned_pairs = 0usize;

    let flush = |report: &mut CompareReport, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
        // A removal immediately followed by an addition is a rewritten page,
        // which reads far better as a line diff than as two whole pages.
        while !removed.is_empty() && !added.is_empty() {
            let i = removed.remove(0);
            let j = added.remove(0);
            compare_lines(report, &a_text[i], &b_text[j], i as u32 + 1, j as u32 + 1);
        }
        for i in removed.drain(..) {
            report.pages_removed.push(i as u32 + 1);
            for line in &a_text[i] {
                report.lines_removed += 1;
                report.changes.push(Change {
                    page: i as u32 + 1,
                    kind: ChangeKind::Removed,
                    text: line.clone(),
                });
            }
        }
        for j in added.drain(..) {
            report.pages_added.push(j as u32 + 1);
            for line in &b_text[j] {
                report.lines_added += 1;
                report.changes.push(Change {
                    page: j as u32 + 1,
                    kind: ChangeKind::Added,
                    text: line.clone(),
                });
            }
        }
    };

    for op in &page_ops {
        match op {
            Op::Equal(i, j) => {
                flush(&mut report, &mut pending_removed, &mut pending_added);
                // Identical text. If there is none, the pages may still be
                // scans that differ, so look at them.
                if a_text[*i].is_empty() && b_text[*j].is_empty() {
                    if let Some(difference) = visual_difference(&a, a_pages[*i], &b, b_pages[*j]) {
                        scanned_pairs += 1;
                        report.visual.push(VisualDifference {
                            page_before: *i as u32 + 1,
                            page_after: *j as u32 + 1,
                            difference,
                        });
                    }
                }
            }
            Op::Delete(i) => pending_removed.push(*i),
            Op::Insert(j) => pending_added.push(*j),
        }
    }
    flush(&mut report, &mut pending_removed, &mut pending_added);

    report.pages_added.sort_unstable();
    report.pages_removed.sort_unstable();
    report.identical = report.changes.is_empty()
        && report.pages_added.is_empty()
        && report.pages_removed.is_empty();

    if scanned_pairs > 0 {
        report.notes.push(format!(
            "{scanned_pairs} page{} no text, so {} compared as pictures rather than read.",
            if scanned_pairs == 1 { " has" } else { "s have" },
            if scanned_pairs == 1 { "it was" } else { "they were" }
        ));
    }
    let total_lines: usize =
        a_text.iter().map(Vec::len).sum::<usize>() + b_text.iter().map(Vec::len).sum::<usize>();
    if total_lines == 0 {
        report.notes.push(
            "Neither document has a text layer, so only the pictures could be compared. \
             Reading the words would need OCR, which isn't built yet."
                .into(),
        );
    }

    Ok(report)
}

fn load(bytes: &[u8], which: &str) -> Result<Document> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read {which} PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported(format!(
            "{which} PDF is password-protected — unlock it first"
        )));
    }
    Ok(doc)
}

/// Normalised page text, for deciding which pages correspond.
fn fingerprint(lines: &[String]) -> String {
    lines.iter().map(|line| normalise(line)).collect::<Vec<_>>().join("\n")
}

/// Whitespace and case are not changes worth reporting.
fn normalise(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn compare_lines(
    report: &mut CompareReport,
    before: &[String],
    after: &[String],
    page_before: u32,
    page_after: u32,
) {
    if before.len() > ALIGN_LIMIT || after.len() > ALIGN_LIMIT {
        report.notes.push(format!(
            "Page {page_before} is too long to line up exactly, so it is summarised by counts."
        ));
        report.lines_removed += before.len() as u32;
        report.lines_added += after.len() as u32;
        return;
    }

    let a: Vec<String> = before.iter().map(|l| normalise(l)).collect();
    let b: Vec<String> = after.iter().map(|l| normalise(l)).collect();
    for op in diff(&a, &b) {
        match op {
            Op::Equal(..) => {}
            Op::Delete(i) => {
                report.lines_removed += 1;
                report.changes.push(Change {
                    page: page_before,
                    kind: ChangeKind::Removed,
                    text: before[i].clone(),
                });
            }
            Op::Insert(j) => {
                report.lines_added += 1;
                report.changes.push(Change {
                    page: page_after,
                    kind: ChangeKind::Added,
                    text: after[j].clone(),
                });
            }
        }
    }
}

/// How different two pages look, 0.0 to 1.0.
fn visual_difference(
    a: &Document,
    a_page: ObjectId,
    b: &Document,
    b_page: ObjectId,
) -> Option<f32> {
    let left = super::pages::page_image(a, a_page)?;
    let right = super::pages::page_image(b, b_page)?;

    // Compared at a common small size: the question is whether the content
    // differs, not whether one was scanned at a higher resolution.
    const SIZE: u32 = 192;
    let filter = image::imageops::FilterType::Triangle;
    let left = left.resize_exact(SIZE, SIZE, filter).to_luma8();
    let right = right.resize_exact(SIZE, SIZE, filter).to_luma8();

    let total: u64 = left
        .pixels()
        .zip(right.pixels())
        .map(|(a, b)| (a.0[0] as i32 - b.0[0] as i32).unsigned_abs() as u64)
        .sum();
    Some(total as f32 / (SIZE * SIZE) as f32 / 255.0)
}

/* ---------- diff ---------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

/// Longest-common-subsequence diff, with the common ends trimmed first.
///
/// Trimming matters more than the algorithm: two revisions of a document
/// usually share long identical runs, and taking those out first keeps the
/// quadratic part small enough to be free.
fn diff<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Op> {
    let mut ops = Vec::new();

    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        ops.push(Op::Equal(start, start));
        start += 1;
    }

    let mut end = 0;
    while end < a.len() - start
        && end < b.len() - start
        && a[a.len() - 1 - end] == b[b.len() - 1 - end]
    {
        end += 1;
    }

    let a_mid = &a[start..a.len() - end];
    let b_mid = &b[start..b.len() - end];

    if a_mid.is_empty() {
        for j in 0..b_mid.len() {
            ops.push(Op::Insert(start + j));
        }
    } else if b_mid.is_empty() {
        for i in 0..a_mid.len() {
            ops.push(Op::Delete(start + i));
        }
    } else if a_mid.len() > ALIGN_LIMIT || b_mid.len() > ALIGN_LIMIT {
        // Too big to line up; report it as a wholesale replacement rather
        // than spending minutes on a table nobody will read.
        for i in 0..a_mid.len() {
            ops.push(Op::Delete(start + i));
        }
        for j in 0..b_mid.len() {
            ops.push(Op::Insert(start + j));
        }
    } else {
        ops.extend(lcs_ops(a_mid, b_mid, start));
    }

    for k in 0..end {
        let i = a.len() - end + k;
        let j = b.len() - end + k;
        ops.push(Op::Equal(i, j));
    }
    ops
}

fn lcs_ops<T: PartialEq>(a: &[T], b: &[T], offset: usize) -> Vec<Op> {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Equal(offset + i, offset + j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            ops.push(Op::Delete(offset + i));
            i += 1;
        } else {
            ops.push(Op::Insert(offset + j));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Delete(offset + i));
        i += 1;
    }
    while j < m {
        ops.push(Op::Insert(offset + j));
        j += 1;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn doc(lines: &[&str]) -> Vec<u8> {
        let sized: Vec<(f32, &str)> = lines.iter().map(|l| (11.0, *l)).collect();
        testkit::pdf_with_sized_text(&sized)
    }

    #[test]
    fn identical_documents_report_no_changes() {
        let a = doc(&["The quick brown fox", "jumps over the lazy dog"]);
        let report = compare(&a, &a).unwrap();
        assert!(report.identical);
        assert_eq!(report.lines_added, 0);
        assert_eq!(report.lines_removed, 0);
        assert!(report.summary().contains("same text on every page"));
    }

    #[test]
    fn a_changed_line_is_reported_as_one_change() {
        let a = doc(&["Opening line stays", "The amount is 100 pounds", "Closing line stays"]);
        let b = doc(&["Opening line stays", "The amount is 250 pounds", "Closing line stays"]);
        let report = compare(&a, &b).unwrap();

        assert!(!report.identical);
        assert_eq!(report.lines_added, 1, "{:?}", report.changes);
        assert_eq!(report.lines_removed, 1);
        assert!(report.changes.iter().any(|c| c.text.contains("100")));
        assert!(report.changes.iter().any(|c| c.text.contains("250")));
        // The unchanged lines must not be reported.
        assert!(!report.changes.iter().any(|c| c.text.contains("Opening")));
    }

    #[test]
    fn whitespace_and_case_are_not_changes() {
        let a = doc(&["The Total  Amount is due", "Second line here"]);
        let b = doc(&["the total amount is due", "Second line here"]);
        let report = compare(&a, &b).unwrap();
        assert!(report.identical, "{:?}", report.changes);
    }

    #[test]
    fn an_inserted_page_does_not_rewrite_the_rest() {
        // This is the behaviour that makes naive comparers useless.
        let page_one = doc(&["Page one content here", "and a second line"]);
        let page_two = doc(&["Page two content here", "with its own second line"]);
        let inserted = doc(&["Brand new page in the middle", "with new content"]);

        let (a, _) = super::super::merge(&[page_one.clone(), page_two.clone()]).unwrap();
        let (b, _) = super::super::merge(&[page_one, inserted, page_two]).unwrap();

        let report = compare(&a, &b).unwrap();
        assert_eq!(report.pages_before, 2);
        assert_eq!(report.pages_after, 3);
        assert_eq!(report.pages_added, vec![2], "the new page should be the only addition");
        assert!(report.pages_removed.is_empty());
        assert_eq!(report.lines_removed, 0, "nothing was removed: {:?}", report.changes);
        assert_eq!(report.lines_added, 2, "only the new page's lines are new");
    }

    #[test]
    fn a_removed_page_is_reported_as_removed() {
        let one = doc(&["First page text", "second line of it"]);
        let two = doc(&["Second page text", "second line of it too"]);
        let (a, _) = super::super::merge(&[one.clone(), two]).unwrap();
        let report = compare(&a, &one).unwrap();
        assert_eq!(report.pages_removed, vec![2]);
        assert_eq!(report.lines_added, 0);
    }

    #[test]
    fn scanned_pages_are_compared_as_pictures() {
        let a = testkit::pdf_scan(1, 240, 300, 80);
        let b = testkit::pdf_scan(1, 240, 300, 80);
        let report = compare(&a, &b).unwrap();

        assert_eq!(report.visual.len(), 1, "a page with no text should be looked at");
        assert!(
            report.visual[0].difference < 0.02,
            "the same scan twice should look the same, got {}",
            report.visual[0].difference
        );
        assert!(report.notes.iter().any(|n| n.contains("compared as pictures")));
        assert!(report.summary().contains("Scanned pages"));
    }

    #[test]
    fn a_different_scan_shows_a_difference() {
        let a = testkit::pdf_scan(1, 240, 300, 80);
        // A different picture entirely.
        let mut canvas = image::RgbImage::from_pixel(240, 300, image::Rgb([255, 255, 255]));
        for y in 40..260 {
            for x in 30..210 {
                canvas.put_pixel(x, y, image::Rgb([10, 10, 10]));
            }
        }
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88)
            .encode_image(&canvas)
            .unwrap();
        let b = testkit::pdf_with_jpeg(&jpeg, 240, 300, 612.0, 792.0);

        let report = compare(&a, &b).unwrap();
        assert_eq!(report.visual.len(), 1);
        assert!(
            report.visual[0].difference > 0.1,
            "a different picture should read as different, got {}",
            report.visual[0].difference
        );
    }

    #[test]
    fn the_summary_reads_as_a_diff() {
        let a = doc(&["Keep this line", "Remove this line"]);
        let b = doc(&["Keep this line", "Add this line instead"]);
        let summary = compare(&a, &b).unwrap().summary();
        assert!(summary.contains("```diff"), "{summary}");
        assert!(summary.contains("- Remove this line"), "{summary}");
        assert!(summary.contains("+ Add this line instead"), "{summary}");
        assert!(!summary.contains("Keep this line"), "unchanged lines stay out");
    }

    #[test]
    fn a_protected_file_says_which_one() {
        let plain = doc(&["Some text here", "and more of it"]);
        let locked = super::super::security::protect(
            &plain,
            &super::super::security::Protection {
                open_password: Some("pw".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let err = compare(&plain, &locked).unwrap_err().to_string();
        assert!(err.contains("the second"), "should say which file: {err}");
    }

    #[test]
    fn the_diff_handles_the_edges() {
        let a = vec![1, 2, 3];
        let b = vec![1, 2, 3];
        assert!(diff(&a, &b).iter().all(|op| matches!(op, Op::Equal(..))));

        let empty: Vec<i32> = vec![];
        assert_eq!(diff(&empty, &b).len(), 3);
        assert!(diff(&empty, &b).iter().all(|op| matches!(op, Op::Insert(_))));
        assert!(diff(&a, &empty).iter().all(|op| matches!(op, Op::Delete(_))));
        assert!(diff(&empty, &empty).is_empty());
    }
}
