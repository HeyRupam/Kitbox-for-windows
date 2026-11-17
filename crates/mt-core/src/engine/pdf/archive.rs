//! PDF/A: making a document that will still open in thirty years.
//!
//! The idea behind PDF/A is that a file should carry everything needed to
//! render it, and nothing that depends on the outside world. No fonts the
//! reader has to already own, no colour that means different things on
//! different screens, no scripts, no links to files that have since moved, no
//! encryption whose keys are lost.
//!
//! # This checks; it does not certify
//!
//! Conformance is a long specification and the final word belongs to a
//! validator built for the job, [veraPDF] among them. What this module does is
//! apply the fixes it can apply correctly, check the rules it can check
//! honestly, and name every one it cannot meet. A report saying `conformant:
//! true` means *every rule listed here passed* — not that an auditor has
//! signed it off, and the note says so every time.
//!
//! Claiming more than that would be the worst kind of wrong: a file that says
//! PDF/A on the tin, is accepted into an archive, and fails validation in ten
//! years when nobody remembers why.
//!
//! [veraPDF]: https://verapdf.org
//!
//! # The one that usually decides it
//!
//! Fonts have to be embedded. A document using Helvetica without embedding it
//! is relying on the reader to have Helvetica — which is exactly the dependency
//! PDF/A exists to remove. Kitbox cannot embed a font it does not ship, so a
//! text document written with the standard fonts is reported as not
//! conformant, with the fonts named. A scanned document, having no fonts at
//! all, has no such problem — and scans are most of what anyone archives.

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    /// The 2005 original. Strictest: no transparency, no layers.
    A1b,
    /// The 2011 revision, and the sensible default — it allows transparency,
    /// which almost every modern document uses somewhere.
    #[default]
    A2b,
}

impl Part {
    fn number(self) -> i64 {
        match self {
            Part::A1b => 1,
            Part::A2b => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::A1b => "PDF/A-1b",
            Part::A2b => "PDF/A-2b",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveOptions {
    pub part: Part,
    /// An ICC profile for the output intent.
    ///
    /// Supplied by the caller rather than carried here: `mt-core` ships no
    /// binaries and reads no files, and the machine running this already has a
    /// perfectly good sRGB profile of its own.
    #[serde(skip)]
    pub icc: Option<Vec<u8>>,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// A rule the document does not meet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    /// Short name of the rule, as the specification talks about it.
    pub rule: String,
    /// What is wrong with this particular document.
    pub detail: String,
    /// Whether Kitbox could fix it given something it hasn't got.
    pub fixable_elsewhere: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveReport {
    /// Every rule checked here passed. Not a certification.
    pub conformant: bool,
    pub part: String,
    /// What was changed.
    pub applied: Vec<String>,
    /// What stands in the way, if anything.
    pub blockers: Vec<Blocker>,
    pub note: String,
}

pub const NOT_A_CERTIFICATE: &str =
    "Kitbox checked the rules it knows how to check and fixed the ones it can fix. That is not a \
     certification — if this is going somewhere that will reject a non-conformant file, run it \
     past a validator built for the job, such as veraPDF, before you rely on it.";

/// Look at a document without changing it.
pub fn check(bytes: &[u8], part: Part) -> Result<ArchiveReport> {
    let doc = load(bytes)?;
    let blockers = inspect(&doc, part, true);
    Ok(ArchiveReport {
        conformant: blockers.is_empty(),
        part: part.label().to_string(),
        applied: Vec::new(),
        blockers,
        note: NOT_A_CERTIFICATE.to_string(),
    })
}

/// Convert as far as it can go, and say where it stopped.
pub fn to_archive(bytes: &[u8], options: &ArchiveOptions) -> Result<(Vec<u8>, ArchiveReport)> {
    let mut doc = load(bytes)?;
    let mut applied = Vec::new();

    // 1. The things PDF/A forbids outright, taken out.
    let removed = strip_forbidden(&mut doc);
    if !removed.is_empty() {
        applied.push(format!("Removed {}", removed.join(", ")));
    }

    // 2. A file identifier, which an archive uses to tell two versions apart.
    if ensure_id(&mut doc, bytes) {
        applied.push("Added a file identifier".into());
    }

    // 3. The output intent, so a colour means one definite thing.
    match &options.icc {
        Some(icc) if !icc.is_empty() => {
            add_output_intent(&mut doc, icc)?;
            applied.push("Added an sRGB output intent".into());
        }
        _ => {}
    }

    // 4. The metadata that says what this file claims to be.
    add_xmp(&mut doc, options)?;
    applied.push(format!("Wrote {} metadata", options.part.label()));

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;

    // Checked on the finished bytes, not on the document in memory: what a
    // reader will open is what should be judged.
    let finished = load(&out)?;
    let blockers = inspect(&finished, options.part, options.icc.is_none());

    Ok((
        out,
        ArchiveReport {
            conformant: blockers.is_empty(),
            part: options.part.label().to_string(),
            applied,
            blockers,
            note: NOT_A_CERTIFICATE.to_string(),
        },
    ))
}

fn load(bytes: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        // Not a blocker to report — it is a refusal. An archival format whose
        // whole purpose is still opening in thirty years cannot be locked.
        return Err(Error::unsupported(
            "this PDF is encrypted, and PDF/A does not allow encryption — unlock it first",
        ));
    }
    Ok(doc)
}

/* ---------- checking ---------- */

/// Every rule this module knows how to test.
fn inspect(doc: &Document, part: Part, missing_intent: bool) -> Vec<Blocker> {
    let mut blockers = Vec::new();

    let loose = unembedded_fonts(doc);
    if !loose.is_empty() {
        blockers.push(Blocker {
            rule: "Fonts must be embedded".into(),
            detail: format!(
                "{} rel{} on the reader having {} already. Kitbox has no font files to embed, so \
                 it cannot fix this.",
                loose.join(", "),
                if loose.len() == 1 { "ies" } else { "y" },
                if loose.len() == 1 { "it" } else { "them" }
            ),
            fixable_elsewhere: true,
        });
    }

    if missing_intent && uses_device_colour(doc) {
        blockers.push(Blocker {
            rule: "Colour must be device-independent".into(),
            detail: "This document paints in device colour and has no output intent, so a colour \
                     means whatever the screen decides. An ICC profile is needed."
                .into(),
            fixable_elsewhere: true,
        });
    }

    for filter in forbidden_filters(doc) {
        blockers.push(Blocker {
            rule: "Only permitted compression".into(),
            detail: format!("A stream uses {filter}, which PDF/A does not allow."),
            fixable_elsewhere: true,
        });
    }

    if part == Part::A1b {
        if let Some(what) = transparency(doc) {
            blockers.push(Blocker {
                rule: "No transparency (PDF/A-1 only)".into(),
                detail: format!(
                    "{what} PDF/A-1 forbids transparency; PDF/A-2 allows it, so converting to \
                     that instead would clear this."
                ),
                fixable_elsewhere: false,
            });
        }
    }

    blockers
}

/// Fonts whose programs are not in the file.
fn unembedded_fonts(doc: &Document) -> Vec<String> {
    let mut loose: Vec<String> = Vec::new();
    for page_id in doc.page_iter() {
        let Ok(fonts) = doc.get_page_fonts(page_id) else { continue };
        for (_, dict) in fonts {
            let base = dict
                .get(b"BaseFont")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_else(|| "an unnamed font".into());

            if embedded(doc, &dict) {
                continue;
            }
            // The subset prefix is noise when naming the font to a person.
            let shown = base.split_once('+').map(|(_, rest)| rest).unwrap_or(&base).to_string();
            if !loose.contains(&shown) {
                loose.push(shown);
            }
        }
    }
    loose.sort();
    loose
}

fn embedded(doc: &Document, font: &Dictionary) -> bool {
    let resolve = |object: &Object| -> Option<Dictionary> {
        match object {
            Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
            Object::Dictionary(d) => Some(d.clone()),
            _ => None,
        }
    };

    // A composite font keeps its program one level down, in the descendant.
    let descriptors: Vec<Dictionary> = match font.get(b"DescendantFonts") {
        Ok(value) => {
            let list = match value {
                Object::Array(items) => items.clone(),
                Object::Reference(id) => doc
                    .get_object(*id)
                    .ok()
                    .and_then(|o| o.as_array().ok().cloned())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            list.iter()
                .filter_map(|o| resolve(o))
                .filter_map(|d| d.get(b"FontDescriptor").ok().and_then(resolve))
                .collect()
        }
        Err(_) => font.get(b"FontDescriptor").ok().and_then(resolve).into_iter().collect(),
    };

    descriptors.iter().any(|d| {
        d.has(b"FontFile") || d.has(b"FontFile2") || d.has(b"FontFile3")
    })
}

/// Whether anything paints in a colour space that means nothing on its own.
fn uses_device_colour(doc: &Document) -> bool {
    // In practice every document does, but a page that is purely an ICC-based
    // or indexed image genuinely doesn't, and saying so is cheap.
    doc.objects.values().any(|object| match object {
        Object::Stream(stream) => {
            let space = stream.dict.get(b"ColorSpace").ok().and_then(|o| o.as_name().ok());
            matches!(space, Some(b"DeviceRGB") | Some(b"DeviceGray") | Some(b"DeviceCMYK"))
        }
        _ => false,
    }) || doc.page_iter().next().is_some()
}

/// Compression PDF/A will not accept.
fn forbidden_filters(doc: &Document) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for object in doc.objects.values() {
        let Object::Stream(stream) = object else { continue };
        let names: Vec<String> = match stream.dict.get(b"Filter") {
            Ok(Object::Name(name)) => vec![String::from_utf8_lossy(name).to_string()],
            Ok(Object::Array(items)) => items
                .iter()
                .filter_map(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .collect(),
            _ => Vec::new(),
        };
        for name in names {
            // LZW is out on patent-era grounds; JPX only from PDF/A-2 and is
            // treated as out here to stay on the safe side.
            if (name == "LZWDecode" || name == "JPXDecode") && !found.contains(&name) {
                found.push(name);
            }
        }
    }
    found
}

/// Anything that makes the page see-through, which PDF/A-1 forbids.
fn transparency(doc: &Document) -> Option<String> {
    for object in doc.objects.values() {
        let dict = match object {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        if dict.has(b"SMask") {
            return Some("An image carries a soft mask.".into());
        }
        if dict.get(b"S").ok().and_then(|o| o.as_name().ok()) == Some(b"Transparency".as_ref()) {
            return Some("A page or form uses a transparency group.".into());
        }
        for key in [&b"ca"[..], b"CA"] {
            if let Ok(value) = dict.get(key) {
                if value.as_float().map(|v| v < 1.0).unwrap_or(false) {
                    return Some("Something is drawn part-transparent.".into());
                }
            }
        }
        if let Ok(mode) = dict.get(b"BM") {
            if mode.as_name().map(|n| n != b"Normal" && n != b"Compatible").unwrap_or(false) {
                return Some("Something uses a blend mode.".into());
            }
        }
    }
    None
}

/* ---------- fixing ---------- */

/// Take out what PDF/A does not allow to be there at all.
fn strip_forbidden(doc: &mut Document) -> Vec<String> {
    let mut removed = Vec::new();
    let root = doc.trailer.get(b"Root").ok().and_then(|o| match o {
        Object::Reference(id) => Some(*id),
        _ => None,
    });

    if let Some(root) = root {
        if let Ok(catalog) = doc.get_dictionary_mut(root) {
            // Names holds the JavaScript and embedded-file trees.
            for (key, label) in [
                (&b"Names"[..], "embedded files and scripts"),
                (b"OpenAction", "an action on open"),
                (b"AA", "automatic actions"),
                (b"AcroForm", "form scripting"),
            ] {
                if catalog.has(key) {
                    catalog.remove(key);
                    removed.push(label.to_string());
                }
            }
        }
    }

    // Per-page and per-annotation actions, which can carry JavaScript.
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let mut page_actions = 0;
    for page_id in page_ids {
        if let Ok(page) = doc.get_dictionary_mut(page_id) {
            for key in [&b"AA"[..], b"JS"] {
                if page.has(key) {
                    page.remove(key);
                    page_actions += 1;
                }
            }
        }
    }
    if page_actions > 0 {
        removed.push(format!("{page_actions} page actions"));
    }

    // Anything the catalog no longer points at.
    doc.prune_objects();
    removed
}

/// Give the file an identifier if it hasn't got one.
fn ensure_id(doc: &mut Document, source: &[u8]) -> bool {
    if doc.trailer.has(b"ID") {
        return false;
    }
    // Derived from the content, so converting the same document twice gives
    // the same identifier rather than a new one each time.
    let digest = blake3::hash(source);
    let id = digest.as_bytes()[..16].to_vec();
    doc.trailer.set(
        "ID",
        Object::Array(vec![
            Object::String(id.clone(), lopdf::StringFormat::Hexadecimal),
            Object::String(id, lopdf::StringFormat::Hexadecimal),
        ]),
    );
    true
}

/// Attach the ICC profile that says what the colours mean.
fn add_output_intent(doc: &mut Document, icc: &[u8]) -> Result<()> {
    let profile = doc.add_object(Stream::new(
        dictionary! {
            // Three components: this is an RGB profile.
            "N" => 3i64,
        },
        icc.to_vec(),
    ));

    let intent = doc.add_object(dictionary! {
        "Type" => "OutputIntent",
        "S" => "GTS_PDFA1",
        "OutputConditionIdentifier" => Object::string_literal("sRGB"),
        "Info" => Object::string_literal("sRGB IEC61966-2.1"),
        "DestOutputProfile" => profile,
    });

    let root = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| match o {
            Object::Reference(id) => Some(*id),
            _ => None,
        })
        .ok_or_else(|| Error::invalid("this PDF has no catalogue"))?;
    let catalog = doc
        .get_dictionary_mut(root)
        .map_err(|_| Error::invalid("this PDF's catalogue is unreadable"))?;
    catalog.set("OutputIntents", Object::Array(vec![Object::Reference(intent)]));
    Ok(())
}

/// Write the XMP packet that states the claim.
///
/// It has to be a plain, uncompressed stream: a reader is expected to be able
/// to find and parse it without decoding anything, which is the point of
/// metadata that has to outlive its software.
fn add_xmp(doc: &mut Document, options: &ArchiveOptions) -> Result<()> {
    let title = options.title.clone().unwrap_or_default();
    let author = options.author.clone().unwrap_or_default();
    let part = options.part.number();

    let xmp = format!(
        r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/">
   <pdfaid:part>{part}</pdfaid:part>
   <pdfaid:conformance>B</pdfaid:conformance>
  </rdf:Description>
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">{}</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>
  </rdf:Description>
  <rdf:Description rdf:about=""
    xmlns:pdf="http://ns.adobe.com/pdf/1.3/">
   <pdf:Producer>Kitbox</pdf:Producer>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#,
        escape_xml(&title),
        escape_xml(&author),
    );

    let metadata = doc.add_object(
        Stream::new(
            dictionary! {
                "Type" => "Metadata",
                "Subtype" => "XML",
            },
            xmp.into_bytes(),
        )
        // Never compressed: see above.
        .with_compression(false),
    );

    let root = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| match o {
            Object::Reference(id) => Some(*id),
            _ => None,
        })
        .ok_or_else(|| Error::invalid("this PDF has no catalogue"))?;

    // The Info dictionary has to agree with the XMP, or a validator calls it
    // a contradiction. Easiest agreement is to keep only what is stated here.
    let info = doc.add_object(dictionary! {
        "Title" => Object::string_literal(options.title.clone().unwrap_or_default()),
        "Author" => Object::string_literal(options.author.clone().unwrap_or_default()),
        "Producer" => Object::string_literal("Kitbox"),
    });
    doc.trailer.set("Info", Object::Reference(info));

    let catalog = doc
        .get_dictionary_mut(root)
        .map_err(|_| Error::invalid("this PDF's catalogue is unreadable"))?;
    catalog.set("Metadata", Object::Reference(metadata));
    Ok(())
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// Something standing in for a colour profile. The module embeds whatever
    /// it is handed; judging the profile itself is not its job.
    fn fake_icc() -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[36..40].copy_from_slice(b"acsp");
        bytes
    }

    fn with_icc(part: Part) -> ArchiveOptions {
        ArchiveOptions {
            part,
            icc: Some(fake_icc()),
            title: Some("A document".into()),
            author: Some("Someone".into()),
        }
    }

    #[test]
    fn a_scan_has_no_fonts_to_embed_and_so_can_be_archived() {
        // The case that actually matters: scans are most of what gets
        // archived, and having no fonts they clear the rule that stops
        // everything else.
        let scan = testkit::pdf_scan(2, 200, 260, 70);
        let (out, report) = to_archive(&scan, &with_icc(Part::A2b)).unwrap();

        assert!(report.conformant, "blocked by {:?}", report.blockers);
        assert_eq!(report.part, "PDF/A-2b");
        assert_eq!(super::super::page_count(&out).unwrap(), 2, "the pages survive");
        assert!(report.applied.iter().any(|a| a.contains("output intent")));
        assert!(report.applied.iter().any(|a| a.contains("identifier")));
    }

    #[test]
    fn a_document_using_a_font_it_does_not_carry_is_refused_by_name() {
        // The honest failure. Kitbox has no font files, so it says which font
        // and why rather than writing PDF/A on something that isn't.
        let text = testkit::pdf_with_sized_text(&[(12.0, "Hello")]);
        let (_, report) = to_archive(&text, &with_icc(Part::A2b)).unwrap();

        assert!(!report.conformant);
        let blocker = report
            .blockers
            .iter()
            .find(|b| b.rule.contains("embedded"))
            .expect("the font rule should be the one that bites");
        assert!(blocker.detail.contains("Helvetica"), "name it: {}", blocker.detail);
        assert!(blocker.fixable_elsewhere, "it is fixable, just not here");
    }

    #[test]
    fn without_a_colour_profile_it_says_so_rather_than_claiming_conformance() {
        let scan = testkit::pdf_scan(1, 120, 160, 70);
        let bare = ArchiveOptions { part: Part::A2b, icc: None, ..Default::default() };
        let (_, report) = to_archive(&scan, &bare).unwrap();

        assert!(!report.conformant);
        assert!(
            report.blockers.iter().any(|b| b.rule.contains("device-independent")),
            "{:?}",
            report.blockers
        );
    }

    #[test]
    fn the_claim_is_written_where_a_reader_will_find_it() {
        let scan = testkit::pdf_scan(1, 120, 160, 70);
        let (out, _) = to_archive(&scan, &with_icc(Part::A2b)).unwrap();
        let raw = String::from_utf8_lossy(&out);

        assert!(raw.contains("<pdfaid:part>2</pdfaid:part>"), "the part has to be stated");
        assert!(raw.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
        assert!(raw.contains("GTS_PDFA1"), "the output intent has to be there");
        // Uncompressed on purpose: a reader is meant to find this without
        // decoding anything.
        assert!(raw.contains("pdfaid"), "the XMP must not be compressed away");
    }

    #[test]
    fn part_one_and_part_two_say_different_things() {
        let scan = testkit::pdf_scan(1, 120, 160, 70);
        let (one, _) = to_archive(&scan, &with_icc(Part::A1b)).unwrap();
        let (two, _) = to_archive(&scan, &with_icc(Part::A2b)).unwrap();
        assert!(String::from_utf8_lossy(&one).contains("<pdfaid:part>1</pdfaid:part>"));
        assert!(String::from_utf8_lossy(&two).contains("<pdfaid:part>2</pdfaid:part>"));
        assert_eq!(Part::A1b.label(), "PDF/A-1b");
    }

    #[test]
    fn transparency_blocks_part_one_and_not_part_two() {
        // A signature is the everyday case: it carries an alpha channel, which
        // PDF/A-1 forbids outright and PDF/A-2 is fine with.
        let mut doc = Document::load_mem(&testkit::pdf_scan(1, 80, 100, 70)).unwrap();
        let mut masked = Dictionary::new();
        masked.set("Type", Object::Name(b"XObject".to_vec()));
        masked.set("Subtype", Object::Name(b"Image".to_vec()));
        masked.set("SMask", Object::Reference((99, 0)));
        doc.add_object(Object::Dictionary(masked));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let strict = check(&bytes, Part::A1b).unwrap();
        assert!(
            strict.blockers.iter().any(|b| b.rule.contains("transparency")),
            "{:?}",
            strict.blockers
        );
        let relaxed = check(&bytes, Part::A2b).unwrap();
        assert!(
            !relaxed.blockers.iter().any(|b| b.rule.contains("transparency")),
            "part 2 allows it: {:?}",
            relaxed.blockers
        );
    }

    #[test]
    fn checking_changes_nothing() {
        let scan = testkit::pdf_scan(1, 120, 160, 70);
        let report = check(&scan, Part::A2b).unwrap();
        assert!(report.applied.is_empty(), "a check applies nothing");
        assert!(report.note.contains("not a certification"));
    }

    #[test]
    fn scripts_and_attachments_are_taken_out() {
        let mut doc = Document::load_mem(&testkit::pdf_scan(1, 80, 100, 70)).unwrap();
        let root = match doc.trailer.get(b"Root").unwrap() {
            Object::Reference(id) => *id,
            _ => panic!("no catalogue"),
        };
        let catalog = doc.get_dictionary_mut(root).unwrap();
        catalog.set("Names", Object::Dictionary(Dictionary::new()));
        catalog.set("OpenAction", Object::Dictionary(Dictionary::new()));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let (out, report) = to_archive(&bytes, &with_icc(Part::A2b)).unwrap();
        assert!(
            report.applied.iter().any(|a| a.contains("Removed")),
            "{:?}",
            report.applied
        );
        let after = Document::load_mem(&out).unwrap();
        let root = match after.trailer.get(b"Root").unwrap() {
            Object::Reference(id) => *id,
            _ => panic!("no catalogue"),
        };
        let catalog = after.get_dictionary(root).unwrap();
        assert!(!catalog.has(b"Names"), "scripts and attachments should be gone");
        assert!(!catalog.has(b"OpenAction"));
    }

    #[test]
    fn an_encrypted_document_is_refused_rather_than_reported_on() {
        // Not a blocker to list — a format whose whole point is still opening
        // in thirty years cannot be locked, so there is nothing to convert.
        let locked = crate::engine::pdf::protect(
            &testkit::pdf_scan(1, 80, 100, 70),
            &crate::engine::pdf::Protection {
                open_password: Some("secret".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let error = to_archive(&locked, &with_icc(Part::A2b)).unwrap_err().to_string();
        assert!(error.contains("encrypt"), "{error}");
        assert!(error.contains("unlock it first"), "say what to do: {error}");
    }

    #[test]
    fn the_same_document_twice_gets_the_same_identifier() {
        // Derived from the content, so re-archiving doesn't look like a
        // different file every time.
        let scan = testkit::pdf_scan(1, 120, 160, 70);
        let (a, _) = to_archive(&scan, &with_icc(Part::A2b)).unwrap();
        let (b, _) = to_archive(&scan, &with_icc(Part::A2b)).unwrap();
        let id = |bytes: &[u8]| {
            let doc = Document::load_mem(bytes).unwrap();
            format!("{:?}", doc.trailer.get(b"ID").unwrap())
        };
        assert_eq!(id(&a), id(&b));
    }

    #[test]
    fn every_report_says_it_is_not_a_certificate() {
        assert!(NOT_A_CERTIFICATE.contains("not a certification"));
        assert!(NOT_A_CERTIFICATE.contains("veraPDF"), "point at a real validator");
    }

    #[test]
    fn a_title_with_markup_in_it_cannot_break_the_metadata() {
        let scan = testkit::pdf_scan(1, 80, 100, 70);
        let options = ArchiveOptions {
            part: Part::A2b,
            icc: Some(fake_icc()),
            title: Some("Smith & Sons <Ltd>".into()),
            author: None,
        };
        let (out, _) = to_archive(&scan, &options).unwrap();
        let raw = String::from_utf8_lossy(&out);
        assert!(raw.contains("Smith &amp; Sons &lt;Ltd&gt;"), "XML has to be escaped");
    }
}
