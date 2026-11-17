//! Recovering a damaged PDF.
//!
//! The expensive part — rebuilding a cross-reference table by scanning the file
//! for indirect objects — `lopdf` already does. What is missing is everything
//! around it: trying the gentler approaches first, rebuilding a page tree whose
//! pages survived but whose index didn't, and above all **saying what actually
//! happened**.
//!
//! That last part is the point. A repair tool that reports success on a file it
//! silently emptied is worse than one that refuses, because the person sends
//! the result somewhere before noticing. Every result here carries what was
//! wrong, what was recovered, and what is still missing.

use lopdf::{dictionary, Document, LoadOptions, Object, ObjectId};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// It opened normally. Repacking is all that happened.
    Healthy,
    /// It opened, but something had to be worked around.
    Repaired,
    /// It only opened by scanning the file for whatever objects remained.
    Salvaged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    pub severity: Severity,
    /// Pages in the finished document.
    pub pages: u32,
    /// Objects recovered.
    pub objects: u32,
    /// What was wrong, in the user's terms.
    pub findings: Vec<String>,
    /// What this result does not promise.
    pub caveats: Vec<String>,
}

impl RepairReport {
    pub fn headline(&self) -> String {
        match self.severity {
            Severity::Healthy => format!(
                "Nothing was broken — {} page{} repacked.",
                self.pages,
                if self.pages == 1 { "" } else { "s" }
            ),
            Severity::Repaired => format!(
                "Repaired: {} page{} recovered.",
                self.pages,
                if self.pages == 1 { "" } else { "s" }
            ),
            Severity::Salvaged => format!(
                "Salvaged by scanning the file: {} page{} found.",
                self.pages,
                if self.pages == 1 { "" } else { "s" }
            ),
        }
    }
}

/// Open a damaged PDF as best as possible and write a clean one.
pub fn repair(bytes: &[u8]) -> Result<(Vec<u8>, RepairReport)> {
    if bytes.len() < 8 {
        return Err(Error::invalid("that file is too small to be a PDF"));
    }
    let mut findings = Vec::new();
    let mut caveats = Vec::new();

    if !bytes.starts_with(b"%PDF-") {
        // Some files have junk before the header; the parser copes, but it is
        // worth saying rather than quietly fixing.
        if find(bytes, b"%PDF-").is_some() {
            findings.push("The file didn't start with a PDF header; one was found further in.".into());
        } else {
            return Err(Error::invalid(
                "this doesn't look like a PDF at all — there is no PDF header anywhere in it",
            ));
        }
    }
    if find(&bytes[bytes.len().saturating_sub(2048)..], b"%%EOF").is_none() {
        findings.push("The end-of-file marker is missing, so the file was probably truncated.".into());
        caveats.push("Anything the file was cut off before is gone and cannot be recovered.".into());
    }

    // Gentlest first: a normal lenient load, which already corrects small
    // cross-reference faults on its own.
    let (mut doc, mut severity) = match Document::load_mem(bytes) {
        Ok(doc) => {
            let severity = match doc.xref_recovery {
                None => Severity::Healthy,
                Some(lopdf::xref::XrefRecovery::OffsetCorrected) => {
                    findings.push(
                        "The cross-reference table pointed to the wrong place; a nearby one was used."
                            .into(),
                    );
                    Severity::Repaired
                }
                Some(lopdf::xref::XrefRecovery::Reconstructed) => {
                    findings.push(
                        "The cross-reference table could not be read, so the file was scanned for objects."
                            .into(),
                    );
                    Severity::Salvaged
                }
                // The enum can grow; an unfamiliar kind of recovery is still a
                // recovery, and saying so beats pretending nothing happened.
                Some(other) => {
                    findings.push(format!("The cross-reference data needed fixing ({other:?})."));
                    Severity::Repaired
                }
            };
            (doc, severity)
        }
        Err(first) => {
            findings.push(format!("The document would not open normally: {first}."));
            // Last resort: ignore the index entirely and take whatever objects
            // can be found in the bytes.
            let options = LoadOptions { force_reconstruct: true, ..Default::default() };
            let doc = Document::load_mem_with_options(bytes, options).map_err(|e| {
                Error::invalid(format!(
                    "this file is too damaged to recover anything from: {e}"
                ))
            })?;
            findings.push("Recovered by scanning the file for whatever objects survived.".into());
            (doc, Severity::Salvaged)
        }
    };

    if doc.is_encrypted() {
        return Err(Error::unsupported(
            "this PDF is password-protected — unlock it first, then repair it",
        ));
    }

    // A rebuilt file often has its pages but not the tree that indexes them.
    let found = doc.page_iter().count();
    if found == 0 {
        let rebuilt = rebuild_page_tree(&mut doc)?;
        if rebuilt == 0 {
            return Err(Error::invalid(
                "no pages survived in this file — there is nothing left to put in a document",
            ));
        }
        findings.push(format!(
            "The page tree was missing or unreadable; {rebuilt} loose page{} were put back in order of object number.",
            if rebuilt == 1 { "" } else { "s" }
        ));
        caveats.push("Page order is a guess, because the list that recorded it was lost.".into());
        severity = Severity::Salvaged;
    }

    if severity == Severity::Salvaged {
        caveats.push(
            "Objects from an older revision of the document may have been picked up, so check the contents."
                .into(),
        );
    }

    doc.delete_zero_length_streams();
    doc.prune_objects();
    doc.renumber_objects();
    let output = super::lossless::save_compact(&mut doc)?;

    let pages = Document::load_mem(&output).map(|d| d.page_iter().count()).unwrap_or(0) as u32;
    if pages == 0 {
        return Err(Error::other(
            "the repaired file came out with no pages, so it has not been written",
        ));
    }

    Ok((
        output,
        RepairReport {
            severity,
            pages,
            objects: doc.objects.len() as u32,
            findings,
            caveats,
        },
    ))
}

/// Collect every object that looks like a page and hang a fresh tree off them.
///
/// Order comes from the object numbers, which is usually but not always the
/// order they were written in — hence the caveat on the report.
fn rebuild_page_tree(doc: &mut Document) -> Result<usize> {
    let mut pages: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| {
            let dict = match object {
                Object::Dictionary(d) => d,
                _ => return false,
            };
            dict.get(b"Type").ok().and_then(|o| o.as_name().ok()) == Some(b"Page".as_ref())
        })
        .map(|(id, _)| *id)
        .collect();
    if pages.is_empty() {
        return Ok(0);
    }
    pages.sort();

    let pages_id = doc.new_object_id();
    for id in &pages {
        if let Ok(page) = doc.get_dictionary_mut(*id) {
            page.set("Parent", Object::Reference(pages_id));
            // A page with no MediaBox anywhere will not render; Letter is a
            // better guess than nothing, and the caveat says order is uncertain
            // anyway.
            if !page.has(b"MediaBox") {
                page.set(
                    "MediaBox",
                    Object::Array(vec![
                        Object::Real(0.0),
                        Object::Real(0.0),
                        Object::Real(612.0),
                        Object::Real(792.0),
                    ]),
                );
            }
        }
    }

    let count = pages.len() as i64;
    let kids: Vec<Object> = pages.iter().map(|id| Object::Reference(*id)).collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );

    // Reuse the existing catalog if there is one, so bookmarks and metadata
    // that survived are kept.
    let catalog_id = doc
        .objects
        .iter()
        .find(|(_, object)| {
            matches!(object, Object::Dictionary(d)
                if d.get(b"Type").ok().and_then(|o| o.as_name().ok()) == Some(b"Catalog".as_ref()))
        })
        .map(|(id, _)| *id);

    let catalog_id = match catalog_id {
        Some(id) => {
            if let Ok(catalog) = doc.get_dictionary_mut(id) {
                catalog.set("Pages", Object::Reference(pages_id));
            }
            id
        }
        None => doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        }),
    };
    doc.trailer.set("Root", Object::Reference(catalog_id));

    Ok(pages.len())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// What is wrong with a file, without changing it.
pub fn diagnose(bytes: &[u8]) -> RepairReport {
    match repair(bytes) {
        Ok((_, report)) => report,
        Err(e) => RepairReport {
            severity: Severity::Salvaged,
            pages: 0,
            objects: 0,
            findings: vec![e.to_string()],
            caveats: vec!["Nothing could be recovered from this file.".into()],
        },
    }
}

/// Count of objects that still hold data, for the UI's before/after.
pub fn object_count(bytes: &[u8]) -> Option<u32> {
    Document::load_mem(bytes).ok().map(|d| d.objects.len() as u32)
}

/// Whether the file is missing its end marker, which usually means it was cut
/// off mid-write.
pub fn looks_truncated(bytes: &[u8]) -> bool {
    find(&bytes[bytes.len().saturating_sub(2048)..], b"%%EOF").is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn a_healthy_file_is_reported_as_such() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        let (out, report) = repair(&src).unwrap();
        assert_eq!(report.severity, Severity::Healthy);
        assert_eq!(report.pages, 3);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert!(report.headline().contains("Nothing was broken"));
        assert_eq!(Document::load_mem(&out).unwrap().page_iter().count(), 3);
    }

    #[test]
    fn a_broken_cross_reference_offset_is_recovered() {
        let mut src = testkit::pdf_scan(2, 200, 260, 70);
        // Point startxref somewhere useless.
        let at = src
            .windows(9)
            .rposition(|w| w == b"startxref")
            .expect("fixtures have a startxref");
        let tail = String::from_utf8_lossy(&src[at..]).to_string();
        let line_end = tail.find('\n').unwrap() + 1;
        let number_end = tail[line_end..].find('\n').unwrap() + line_end;
        let replacement = format!("startxref\n999999{}", &tail[number_end..]);
        src.truncate(at);
        src.extend_from_slice(replacement.as_bytes());

        let (out, report) = repair(&src).unwrap();
        assert_ne!(report.severity, Severity::Healthy, "a bad offset should be noticed");
        assert_eq!(report.pages, 2, "both pages should come back");
        assert!(!report.findings.is_empty());
        assert_eq!(Document::load_mem(&out).unwrap().page_iter().count(), 2);
    }

    #[test]
    fn a_truncated_file_is_flagged_and_salvaged() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        // Cut off the trailer entirely.
        let cut = src.len() * 3 / 4;
        let damaged = src[..cut].to_vec();

        match repair(&damaged) {
            Ok((out, report)) => {
                assert_ne!(report.severity, Severity::Healthy);
                assert!(
                    report.findings.iter().any(|f| f.contains("truncated")),
                    "should say it was cut off: {:?}",
                    report.findings
                );
                assert!(
                    report.caveats.iter().any(|c| c.contains("cannot be recovered")),
                    "should admit what is gone: {:?}",
                    report.caveats
                );
                assert!(report.pages >= 1, "at least one page should survive");
                assert!(Document::load_mem(&out).is_ok(), "the result must be readable");
            }
            // Losing a quarter of a small file can leave nothing usable; the
            // refusal has to be clear rather than a silent empty document.
            Err(e) => {
                let message = e.to_string();
                assert!(
                    message.contains("damaged") || message.contains("no pages"),
                    "{message}"
                );
            }
        }
    }

    #[test]
    fn loose_pages_are_put_back_into_a_tree() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        let mut doc = Document::load_mem(&src).unwrap();

        // Sever the catalog from the page tree, as a damaged index would.
        let pages_id = match doc.catalog().unwrap().get(b"Pages").unwrap() {
            Object::Reference(id) => *id,
            _ => panic!("expected a reference"),
        };
        doc.objects.remove(&pages_id);
        doc.catalog_mut().unwrap().remove(b"Pages");
        let mut damaged = Vec::new();
        doc.save_to(&mut damaged).unwrap();

        let (out, report) = repair(&damaged).unwrap();
        assert_eq!(report.pages, 3, "every loose page should be found");
        assert!(
            report.findings.iter().any(|f| f.contains("loose page")),
            "{:?}",
            report.findings
        );
        assert!(
            report.caveats.iter().any(|c| c.contains("Page order is a guess")),
            "should admit the order is uncertain: {:?}",
            report.caveats
        );
        assert_eq!(Document::load_mem(&out).unwrap().page_iter().count(), 3);
    }

    #[test]
    fn something_that_is_not_a_pdf_is_refused() {
        let err = repair(b"this is just text, not a document at all").unwrap_err().to_string();
        assert!(err.contains("PDF header"), "{err}");
        assert!(repair(b"abc").is_err(), "a tiny file has nothing to work with");
    }

    #[test]
    fn a_protected_file_is_sent_to_unlock_first() {
        let locked = super::super::security::protect(
            &testkit::pdf_scan(1, 200, 260, 70),
            &super::super::security::Protection {
                open_password: Some("pw".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let err = repair(&locked).unwrap_err().to_string();
        assert!(err.contains("unlock it first"), "{err}");
    }

    #[test]
    fn diagnose_never_panics_on_rubbish() {
        let report = diagnose(b"%PDF-1.4 and then nothing useful");
        assert_eq!(report.pages, 0);
        assert!(!report.findings.is_empty());
    }
}
