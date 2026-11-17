//! Page operations: merge, split, extract, remove, reorder, rotate.
//!
//! All of them are the same primitive underneath — [`rebuild`] produces a
//! document containing the pages you name, in the order you name them, with the
//! rotations you ask for. Extract is a rebuild with a subset; remove is a
//! rebuild with the complement; reorder is a permutation; split is several
//! rebuilds. Writing it once means a fix to the page-tree handling fixes every
//! tool at once.
//!
//! Two things get reported rather than done quietly. Flattening the page tree
//! means inherited attributes have to be copied down onto each page first, or
//! pages silently lose their size and resources. And once the page set changes,
//! the outline, the tag tree and named destinations point at pages that may not
//! exist, so they are dropped — and [`PageOpReport`] says so.

use std::collections::BTreeMap;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Attributes a page can inherit from its parent in the page tree.
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Catalog entries that stop making sense once pages move.
const PAGE_DEPENDENT: [&[u8]; 4] = [b"Outlines", b"StructTreeRoot", b"Names", b"Dests"];

/// A set of 1-based page numbers, in the order the user gave them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PageSelection(Vec<u32>);

impl PageSelection {
    /// Parse the shapes people actually type: `1-3,5,8-`, `all`, `odd`, `even`,
    /// `last`. Page numbers are 1-based and a range may be open-ended.
    pub fn parse(spec: &str, total: u32) -> Result<PageSelection> {
        let spec = spec.trim().to_ascii_lowercase();
        if total == 0 {
            return Err(Error::invalid("the document has no pages"));
        }
        let all = || (1..=total).collect::<Vec<u32>>();
        match spec.as_str() {
            "" | "all" => return Ok(PageSelection(all())),
            "odd" => return Ok(PageSelection(all().into_iter().filter(|p| p % 2 == 1).collect())),
            "even" => return Ok(PageSelection(all().into_iter().filter(|p| p % 2 == 0).collect())),
            "last" => return Ok(PageSelection(vec![total])),
            _ => {}
        }

        let mut pages = Vec::new();
        for part in spec.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (from, to) = match part.split_once('-') {
                Some((a, b)) => {
                    let from = parse_page(a, total, 1)?;
                    let to = parse_page(b, total, total)?;
                    (from, to)
                }
                None => {
                    let n = parse_page(part, total, 0)?;
                    (n, n)
                }
            };
            if from == 0 || to == 0 {
                return Err(Error::invalid(format!("'{part}' is not a page number")));
            }
            if from > total || to > total {
                return Err(Error::invalid(format!(
                    "page {} is past the end — this document has {total}",
                    from.max(to)
                )));
            }
            // A backwards range is almost always a typo, but reading it as
            // "these pages, reversed" is what the user meant if they wrote it.
            if from <= to {
                pages.extend(from..=to);
            } else {
                pages.extend((to..=from).rev());
            }
        }
        if pages.is_empty() {
            return Err(Error::invalid("that selection matched no pages"));
        }
        Ok(PageSelection(pages))
    }

    pub fn from_pages(pages: Vec<u32>) -> PageSelection {
        PageSelection(pages)
    }

    pub fn pages(&self) -> &[u32] {
        &self.0
    }

    pub fn contains(&self, page: u32) -> bool {
        self.0.contains(&page)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every page *not* in this selection, in document order.
    pub fn complement(&self, total: u32) -> PageSelection {
        PageSelection((1..=total).filter(|p| !self.contains(*p)).collect())
    }
}

fn parse_page(text: &str, total: u32, default: u32) -> Result<u32> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(default);
    }
    if text == "last" || text == "n" {
        return Ok(total);
    }
    text.parse::<u32>()
        .map_err(|_| Error::invalid(format!("'{text}' is not a page number")))
}

/// What a page looks like, without rendering it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub number: u32,
    pub width_pt: f32,
    pub height_pt: f32,
    /// Clockwise rotation applied on display: 0, 90, 180 or 270.
    pub rotation: i32,
    /// "A4 portrait", "Letter landscape", or the raw size when it's unusual.
    pub label: String,
    /// Whether [`page_preview`] can produce a picture of this page.
    pub has_preview: bool,
}

/// What an operation changed, including what it had to throw away.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOpReport {
    pub pages_before: u32,
    pub pages_after: u32,
    /// Document features dropped because they referenced pages that moved.
    pub dropped: Vec<String>,
}

impl PageOpReport {
    pub fn note(&self) -> Option<String> {
        if self.dropped.is_empty() {
            return None;
        }
        Some(format!(
            "Changing the pages invalidated the {}, so {} removed.",
            self.dropped.join(", the "),
            if self.dropped.len() == 1 { "it was" } else { "they were" }
        ))
    }
}

pub fn page_count(bytes: &[u8]) -> Result<u32> {
    let doc = load(bytes)?;
    Ok(doc.page_iter().count() as u32)
}

pub fn page_summaries(bytes: &[u8]) -> Result<Vec<PageInfo>> {
    let doc = load(bytes)?;
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let previewable = previewable_pages(&doc, &page_ids);

    Ok(page_ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let (w, h) = media_box(&doc, *id).unwrap_or((612.0, 792.0));
            let rotation = inherited_number(&doc, *id, b"Rotate").unwrap_or(0.0) as i32;
            // A rotated page is presented with its sides swapped.
            let (dw, dh) = if rotation.rem_euclid(180) == 90 { (h, w) } else { (w, h) };
            PageInfo {
                number: i as u32 + 1,
                width_pt: dw,
                height_pt: dh,
                rotation: rotation.rem_euclid(360),
                label: describe_size(dw, dh),
                has_preview: previewable.get(i).copied().unwrap_or(false),
            }
        })
        .collect())
}

/// A picture of a page, as JPEG bytes — *if* the page is essentially one big
/// image, which is what a scan is.
///
/// Kitbox has no PDF renderer, so a text page can't be drawn. Rather than ship
/// a 10 MB rendering engine for thumbnails, this recovers the image that is
/// already there, which covers scanned forms — the case where seeing the page
/// actually matters. Text pages report `has_preview: false` and the UI shows a
/// page card instead of pretending.
pub fn page_preview(bytes: &[u8], page: u32, max_px: u32) -> Result<Option<Vec<u8>>> {
    let doc = load(bytes)?;
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let Some(page_id) = page_ids.get(page.saturating_sub(1) as usize).copied() else {
        return Err(Error::invalid(format!("there is no page {page}")));
    };
    let Some(image_id) = dominant_image(&doc, page_id) else { return Ok(None) };
    let Some(decoded) = super::images::decode_image(&doc, image_id) else { return Ok(None) };

    let scaled = if decoded.width().max(decoded.height()) > max_px {
        decoded.resize(max_px, max_px, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    Ok(Some(super::images::encode_jpeg_for_pdf(&scaled, 72, false)?))
}

/// Previews for every page in one pass.
///
/// The per-page entry point reloads the document each time, which is fine for
/// one thumbnail and wasteful for fifty, so the page grid uses this.
pub fn page_previews(bytes: &[u8], max_px: u32) -> Result<Vec<Option<Vec<u8>>>> {
    Ok(export_pages(bytes, Some(max_px), 72)?.into_iter().map(|p| p.jpeg).collect())
}

/// One page, exported as an image — or the reason it couldn't be.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedPage {
    pub page: u32,
    #[serde(skip)]
    pub jpeg: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub reason: Option<String>,
}

/// Export each page as a JPEG.
///
/// This recovers the image a page is made of rather than rendering the page, so
/// it is exact for scans — which is the case that matters for forms — and
/// impossible for pages built from text and vectors. Those come back with a
/// reason rather than a blank image, because handing someone an empty JPEG and
/// calling it a conversion is worse than saying no.
pub fn export_pages(
    bytes: &[u8],
    max_px: Option<u32>,
    quality: u8,
) -> Result<Vec<ExportedPage>> {
    let doc = load(bytes)?;
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();

    page_ids
        .iter()
        .enumerate()
        .map(|(i, page_id)| {
            let number = i as u32 + 1;
            let not_possible = |reason: &str| ExportedPage {
                page: number,
                jpeg: None,
                width: 0,
                height: 0,
                reason: Some(reason.to_string()),
            };

            let Some(image_id) = dominant_image(&doc, *page_id) else {
                return Ok(not_possible(
                    "this page is text or vector graphics, not a scan — exporting it needs a \
                     page renderer, which isn't built yet",
                ));
            };
            let Some(decoded) = super::images::decode_image(&doc, image_id) else {
                return Ok(not_possible(
                    "this page's image is in a format that can't be decoded safely",
                ));
            };

            let scaled = match max_px {
                Some(max) if decoded.width().max(decoded.height()) > max => {
                    decoded.resize(max, max, image::imageops::FilterType::Triangle)
                }
                _ => decoded,
            };
            Ok(ExportedPage {
                page: number,
                width: scaled.width(),
                height: scaled.height(),
                jpeg: Some(super::images::encode_jpeg_for_pdf(&scaled, quality, false)?),
                reason: None,
            })
        })
        .collect()
}

fn previewable_pages(doc: &Document, page_ids: &[ObjectId]) -> Vec<bool> {
    page_ids.iter().map(|id| dominant_image(doc, *id).is_some()).collect()
}

/// The image a page is essentially made of, decoded. `None` for a page built
/// from text and vectors, which has no single picture to return.
pub(super) fn page_image(doc: &Document, page_id: ObjectId) -> Option<image::DynamicImage> {
    super::images::decode_image(doc, dominant_image(doc, page_id)?)
}

/// The page's image together with where it is drawn, in page coordinates.
///
/// OCR needs both: the pixels to read, and the rectangle they occupy, so a
/// word found at some pixel can be placed at the right spot on the page.
pub(super) fn page_image_and_rect(
    doc: &Document,
    page_id: ObjectId,
) -> Option<(image::DynamicImage, (f32, f32, f32, f32))> {
    let id = dominant_image(doc, page_id)?;
    let image = super::images::decode_image(doc, id)?;
    let rect = *super::inspect::drawn_rects(doc, &[page_id]).get(&id)?;
    Some((image, rect))
}

/// Whether a page is essentially one big image — the signature of a scan.
pub(super) fn page_has_image(doc: &Document, page_id: ObjectId) -> bool {
    dominant_image(doc, page_id).is_some()
}

/// The image that covers most of a page, if one does.
fn dominant_image(doc: &Document, page_id: ObjectId) -> Option<ObjectId> {
    let (page_w, page_h) = media_box(doc, page_id)?;
    let page_area = (page_w * page_h).max(1.0);
    let drawn = super::inspect::drawn_sizes(doc, &[page_id]);

    drawn
        .into_iter()
        .filter(|(id, (w, h))| {
            // Must cover at least half the page, and actually be an image.
            if (w * h) / page_area < 0.5 {
                return false;
            }
            matches!(doc.get_object(*id), Ok(Object::Stream(s))
                if s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image".as_ref()))
        })
        .max_by(|a, b| {
            let area = |v: &(ObjectId, (f32, f32))| v.1 .0 * v.1 .1;
            area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(id, _)| id)
}

/// Build a document from the given pages, in the given order.
///
/// `order` is 1-based page numbers; repeating one duplicates the page.
/// `rotate_by` is added to each kept page's existing rotation.
pub fn rebuild(bytes: &[u8], order: &[u32], rotate_by: i32) -> Result<(Vec<u8>, PageOpReport)> {
    let ops: Vec<PageOp> = order.iter().map(|p| PageOp { page: *p, rotate: rotate_by }).collect();
    rebuild_ops(bytes, &ops)
}

/// One page in the result: which source page it is, and how much to turn it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOp {
    /// 1-based page number in the source document.
    pub page: u32,
    /// Quarter turns clockwise to add to whatever rotation it already has.
    #[serde(default)]
    pub rotate: i32,
}

/// The general form: reorder, drop, duplicate and rotate in a single rebuild.
pub fn rebuild_ops(bytes: &[u8], ops: &[PageOp]) -> Result<(Vec<u8>, PageOpReport)> {
    let mut doc = load(bytes)?;
    let before = doc.page_iter().count() as u32;
    if ops.is_empty() {
        return Err(Error::invalid("that would leave the document with no pages"));
    }
    if let Some(bad) = ops.iter().find(|o| o.rotate % 90 != 0) {
        return Err(Error::invalid(format!(
            "pages can only turn in quarter turns, not {}",
            bad.rotate
        )));
    }

    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    for page in ops.iter().map(|o| &o.page) {
        if !pages.contains_key(page) {
            return Err(Error::invalid(format!(
                "there is no page {page} — this document has {before}"
            )));
        }
    }

    // Inherited attributes live on the parent nodes we are about to discard, so
    // copy them onto the pages while the tree is still intact.
    for id in pages.values() {
        materialise_inherited(&mut doc, *id);
    }

    let root_pages = root_pages_id(&doc)?;

    // Build the new kid list, cloning any page that appears more than once.
    let mut seen: Vec<ObjectId> = Vec::new();
    let mut kids: Vec<Object> = Vec::new();
    for op in ops {
        let original = pages[&op.page];
        let id = if seen.contains(&original) {
            clone_page(&mut doc, original)?
        } else {
            seen.push(original);
            original
        };
        if op.rotate != 0 {
            add_rotation(&mut doc, id, op.rotate);
        }
        if let Ok(dict) = doc.get_dictionary_mut(id) {
            dict.set("Parent", Object::Reference(root_pages));
        }
        kids.push(Object::Reference(id));
    }

    let after = kids.len() as u32;
    if let Ok(node) = doc.get_dictionary_mut(root_pages) {
        node.set("Kids", kids);
        node.set("Count", after as i64);
        node.set("Type", Object::Name(b"Pages".to_vec()));
        // The root inherits nothing now that every page carries its own.
        for key in INHERITABLE {
            node.remove(key);
        }
    }

    let changed = ops.len() as u32 != before
        || ops.iter().enumerate().any(|(i, o)| o.page != i as u32 + 1);
    let rotated = ops.iter().any(|o| o.rotate != 0);
    let mut dropped = Vec::new();
    if changed || rotated {
        if let Ok(catalog) = doc.catalog_mut() {
            for key in PAGE_DEPENDENT {
                if catalog.remove(key).is_some() {
                    dropped.push(friendly_name(key));
                }
            }
        }
    }

    doc.prune_objects();
    doc.renumber_objects();
    let bytes = super::lossless::save_compact(&mut doc)?;

    Ok((bytes, PageOpReport { pages_before: before, pages_after: after, dropped }))
}

pub fn extract(bytes: &[u8], selection: &PageSelection) -> Result<(Vec<u8>, PageOpReport)> {
    rebuild(bytes, selection.pages(), 0)
}

pub fn remove(bytes: &[u8], selection: &PageSelection) -> Result<(Vec<u8>, PageOpReport)> {
    let total = page_count(bytes)?;
    let keep = selection.complement(total);
    if keep.is_empty() {
        return Err(Error::invalid("that would remove every page"));
    }
    rebuild(bytes, keep.pages(), 0)
}

pub fn reorder(bytes: &[u8], order: &[u32]) -> Result<(Vec<u8>, PageOpReport)> {
    rebuild(bytes, order, 0)
}

/// Rotate the selected pages by a multiple of 90 degrees, clockwise.
pub fn rotate(
    bytes: &[u8],
    selection: &PageSelection,
    degrees: i32,
) -> Result<(Vec<u8>, PageOpReport)> {
    if degrees % 90 != 0 {
        return Err(Error::invalid("pages can only turn in quarter turns"));
    }
    let mut doc = load(bytes)?;
    let before = doc.page_iter().count() as u32;
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    for id in pages.values() {
        materialise_inherited(&mut doc, *id);
    }
    for (number, id) in &pages {
        if selection.contains(*number) {
            add_rotation(&mut doc, *id, degrees);
        }
    }
    let bytes = super::lossless::save_compact(&mut doc)?;
    // Rotating changes no page's identity, so nothing has to be dropped.
    Ok((bytes, PageOpReport { pages_before: before, pages_after: before, dropped: Vec::new() }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SplitMode {
    /// One file per page.
    EveryPage,
    /// Chunks of N pages.
    EveryN(u32),
    /// Explicit 1-based inclusive ranges.
    Ranges(Vec<(u32, u32)>),
}

#[derive(Debug, Clone)]
pub struct SplitPart {
    /// Appended to the original name: `report-pages-1-3.pdf`.
    pub suffix: String,
    pub pages: Vec<u32>,
    pub bytes: Vec<u8>,
}

pub fn split(bytes: &[u8], mode: &SplitMode) -> Result<Vec<SplitPart>> {
    let total = page_count(bytes)?;
    let groups: Vec<Vec<u32>> = match mode {
        SplitMode::EveryPage => (1..=total).map(|p| vec![p]).collect(),
        SplitMode::EveryN(n) => {
            let n = (*n).max(1);
            (1..=total).collect::<Vec<_>>().chunks(n as usize).map(<[u32]>::to_vec).collect()
        }
        SplitMode::Ranges(ranges) => {
            if ranges.is_empty() {
                return Err(Error::invalid("no ranges given"));
            }
            let mut out = Vec::new();
            for (from, to) in ranges {
                if *from == 0 || *to == 0 || *from > total || *to > total {
                    return Err(Error::invalid(format!(
                        "range {from}-{to} is outside this document's {total} pages"
                    )));
                }
                out.push(if from <= to {
                    (*from..=*to).collect()
                } else {
                    (*to..=*from).rev().collect()
                });
            }
            out
        }
    };

    groups
        .into_iter()
        .map(|pages| {
            let (bytes, _) = rebuild(bytes, &pages, 0)?;
            let suffix = match pages.as_slice() {
                [one] => format!("page-{one}"),
                [first, .., last] => format!("pages-{first}-{last}"),
                [] => "empty".to_string(),
            };
            Ok(SplitPart { suffix, pages, bytes })
        })
        .collect()
}

/// Combine documents end to end, keeping every page.
pub fn merge(inputs: &[Vec<u8>]) -> Result<(Vec<u8>, PageOpReport)> {
    if inputs.is_empty() {
        return Err(Error::invalid("nothing to merge"));
    }
    if inputs.len() == 1 {
        let (bytes, report) = rebuild(&inputs[0], &all_pages(&inputs[0])?, 0)?;
        return Ok((bytes, report));
    }

    let mut merged = Document::with_version("1.7");
    let mut next_id = 1u32;
    let mut all_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut page_ids: Vec<ObjectId> = Vec::new();
    let mut dropped_any = false;

    for input in inputs {
        let mut doc = load(input)?;
        if doc.is_encrypted() {
            return Err(Error::unsupported(
                "one of these PDFs is password-protected — unlock it first",
            ));
        }
        let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
        for id in pages.values() {
            materialise_inherited(&mut doc, *id);
        }
        if doc.catalog().map(|c| PAGE_DEPENDENT.iter().any(|k| c.has(k))).unwrap_or(false) {
            dropped_any = true;
        }

        // Shift this document's object numbers clear of everything already
        // collected, so two inputs can't claim the same id.
        doc.renumber_objects_with(next_id);
        next_id = doc.max_id + 1;

        page_ids.extend(doc.get_pages().into_values());
        all_objects.extend(doc.objects.clone());
    }

    merged.objects = all_objects.into_iter().collect();
    merged.max_id = next_id;

    let pages_id = merged.new_object_id();
    for id in &page_ids {
        if let Ok(dict) = merged.get_dictionary_mut(*id) {
            dict.set("Parent", Object::Reference(pages_id));
        }
    }
    let count = page_ids.len() as i64;
    let kids: Vec<Object> = page_ids.into_iter().map(Object::Reference).collect();
    merged.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog = merged.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    merged.trailer.set("Root", catalog);

    merged.prune_objects();
    merged.renumber_objects();
    let bytes = super::lossless::save_compact(&mut merged)?;

    let dropped = if dropped_any {
        vec!["bookmarks and tagging from the source files".to_string()]
    } else {
        Vec::new()
    };
    Ok((bytes, PageOpReport { pages_before: 0, pages_after: count as u32, dropped }))
}

/* ---------- helpers ---------- */

fn load(bytes: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }
    Ok(doc)
}

fn all_pages(bytes: &[u8]) -> Result<Vec<u32>> {
    Ok((1..=page_count(bytes)?).collect())
}

fn root_pages_id(doc: &Document) -> Result<ObjectId> {
    match doc.catalog().and_then(|c| c.get(b"Pages")) {
        Ok(Object::Reference(id)) => Ok(*id),
        _ => Err(Error::invalid("this PDF has no page tree")),
    }
}

/// Copy any attribute the page is relying on inheriting down onto the page
/// itself, so flattening the tree can't silently change how it renders.
pub(super) fn materialise_inherited(doc: &mut Document, page_id: ObjectId) {
    let mut found: Vec<(Vec<u8>, Object)> = Vec::new();
    for key in INHERITABLE {
        if doc.get_dictionary(page_id).map(|d| d.has(key)).unwrap_or(false) {
            continue;
        }
        let mut current = doc.get_dictionary(page_id).ok().and_then(parent_of);
        let mut guard = 0;
        while let Some(parent) = current {
            guard += 1;
            if guard > 32 {
                break;
            }
            let Ok(dict) = doc.get_dictionary(parent) else { break };
            if let Ok(value) = dict.get(key) {
                found.push((key.to_vec(), value.clone()));
                break;
            }
            current = parent_of(dict);
        }
    }
    if let Ok(page) = doc.get_dictionary_mut(page_id) {
        for (key, value) in found {
            page.set(key, value);
        }
    }
}

fn parent_of(dict: &Dictionary) -> Option<ObjectId> {
    match dict.get(b"Parent") {
        Ok(Object::Reference(id)) => Some(*id),
        _ => None,
    }
}

fn add_rotation(doc: &mut Document, page_id: ObjectId, degrees: i32) {
    let current = inherited_number(doc, page_id, b"Rotate").unwrap_or(0.0) as i32;
    let next = (current + degrees).rem_euclid(360);
    if let Ok(page) = doc.get_dictionary_mut(page_id) {
        page.set("Rotate", next as i64);
    }
}

fn clone_page(doc: &mut Document, page_id: ObjectId) -> Result<ObjectId> {
    let dict = doc
        .get_dictionary(page_id)
        .map_err(|_| Error::other("page disappeared while duplicating"))?
        .clone();
    // A shallow clone is right: the copy shares its content and resources with
    // the original, which is what duplicating a page means.
    Ok(doc.add_object(Object::Dictionary(dict)))
}

pub(super) fn media_box(doc: &Document, page_id: ObjectId) -> Option<(f32, f32)> {
    let value = inherited_value(doc, page_id, b"MediaBox")?;
    let Object::Array(items) = value else { return None };
    if items.len() < 4 {
        return None;
    }
    let n = |i: usize| items[i].as_float().ok();
    let (x0, y0, x1, y1) = (n(0)?, n(1)?, n(2)?, n(3)?);
    Some(((x1 - x0).abs(), (y1 - y0).abs()))
}

fn inherited_number(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<f32> {
    inherited_value(doc, page_id, key)?.as_float().ok()
}

fn inherited_value(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut current = Some(page_id);
    let mut guard = 0;
    while let Some(id) = current {
        guard += 1;
        if guard > 32 {
            break;
        }
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(value) = dict.get(key) {
            return doc.dereference(value).ok().map(|(_, o)| o.clone());
        }
        current = parent_of(dict);
    }
    None
}

fn describe_size(w: f32, h: f32) -> String {
    const KNOWN: [(&str, f32, f32); 6] = [
        ("A4", 595.0, 842.0),
        ("A3", 842.0, 1191.0),
        ("A5", 420.0, 595.0),
        ("Letter", 612.0, 792.0),
        ("Legal", 612.0, 1008.0),
        ("Tabloid", 792.0, 1224.0),
    ];
    let orientation = if w > h { "landscape" } else { "portrait" };
    let (short, long) = if w < h { (w, h) } else { (h, w) };
    for (name, kw, kh) in KNOWN {
        if (short - kw).abs() < 6.0 && (long - kh).abs() < 6.0 {
            return format!("{name} {orientation}");
        }
    }
    format!("{:.0} × {:.0} pt", w, h)
}

fn friendly_name(key: &[u8]) -> String {
    match key {
        b"Outlines" => "bookmarks",
        b"StructTreeRoot" => "accessibility tagging",
        b"Names" => "named destinations",
        b"Dests" => "destinations",
        _ => "document structure",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn scan(pages: usize) -> Vec<u8> {
        testkit::pdf_scan(pages, 400, 520, 70)
    }

    #[test]
    fn per_page_rotation_in_one_rebuild() {
        let src = scan(3);
        let (out, report) = rebuild_ops(
            &src,
            &[
                PageOp { page: 3, rotate: 90 },
                PageOp { page: 1, rotate: 0 },
                PageOp { page: 2, rotate: 180 },
            ],
        )
        .unwrap();
        assert_eq!(report.pages_after, 3);
        assert_eq!(page_image_heights(&out), vec![522, 520, 521]);
        let rotations: Vec<i32> =
            page_summaries(&out).unwrap().iter().map(|p| p.rotation).collect();
        assert_eq!(rotations, vec![90, 0, 180]);
    }

    #[test]
    fn parses_the_selections_people_type() {
        let p = |s: &str| PageSelection::parse(s, 10).unwrap().pages().to_vec();
        assert_eq!(p("1-3"), vec![1, 2, 3]);
        assert_eq!(p("1,4,7"), vec![1, 4, 7]);
        assert_eq!(p("8-"), vec![8, 9, 10]);
        assert_eq!(p("-3"), vec![1, 2, 3]);
        assert_eq!(p("all").len(), 10);
        assert_eq!(p("odd"), vec![1, 3, 5, 7, 9]);
        assert_eq!(p("even"), vec![2, 4, 6, 8, 10]);
        assert_eq!(p("last"), vec![10]);
        assert_eq!(p("3-1"), vec![3, 2, 1], "a backwards range reverses");
        assert!(PageSelection::parse("11", 10).is_err(), "past the end should fail");
        assert!(PageSelection::parse("banana", 10).is_err());
    }

    #[test]
    fn complement_is_everything_else() {
        let sel = PageSelection::parse("2,4", 5).unwrap();
        assert_eq!(sel.complement(5).pages(), &[1, 3, 5]);
    }

    #[test]
    fn extracts_a_range() {
        let src = scan(6);
        let sel = PageSelection::parse("2-4", 6).unwrap();
        let (out, report) = extract(&src, &sel).unwrap();
        assert_eq!(report.pages_before, 6);
        assert_eq!(report.pages_after, 3);
        assert_eq!(page_count(&out).unwrap(), 3);
        assert_eq!(page_image_heights(&out), vec![521, 522, 523], "pages 2, 3 and 4");
    }

    #[test]
    fn removes_pages_and_keeps_the_rest() {
        let src = scan(5);
        let sel = PageSelection::parse("2,4", 5).unwrap();
        let (out, report) = remove(&src, &sel).unwrap();
        assert_eq!(report.pages_after, 3);
        assert_eq!(page_count(&out).unwrap(), 3);
        assert_eq!(page_image_heights(&out), vec![520, 522, 524], "pages 1, 3 and 5");
    }

    #[test]
    fn refuses_to_empty_the_document() {
        let src = scan(3);
        let sel = PageSelection::parse("all", 3).unwrap();
        assert!(remove(&src, &sel).is_err());
    }

    /// The height of each page's own image, in page order. `pdf_scan` gives
    /// page N an image of height `base + N`, so this reads back the page
    /// sequence — which object ids alone would not, since they don't follow it.
    fn page_image_heights(bytes: &[u8]) -> Vec<i64> {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|page| {
                let id = dominant_image(&doc, page).expect("each scanned page has an image");
                let Object::Stream(stream) = doc.get_object(id).unwrap() else {
                    panic!("not a stream")
                };
                stream.dict.get(b"Height").unwrap().as_i64().unwrap()
            })
            .collect()
    }

    #[test]
    fn reorders_pages() {
        let src = scan(3);
        assert_eq!(page_image_heights(&src), vec![520, 521, 522]);
        let (out, _) = reorder(&src, &[3, 1, 2]).unwrap();
        assert_eq!(page_image_heights(&out), vec![522, 520, 521]);
    }

    #[test]
    fn reversing_a_document_reverses_it() {
        let src = scan(4);
        let (out, _) = reorder(&src, &[4, 3, 2, 1]).unwrap();
        assert_eq!(page_image_heights(&out), vec![523, 522, 521, 520]);
    }

    #[test]
    fn duplicating_a_page_is_allowed() {
        let src = scan(2);
        let (out, report) = rebuild(&src, &[1, 1, 2], 0).unwrap();
        assert_eq!(report.pages_after, 3);
        assert_eq!(page_count(&out).unwrap(), 3);
    }

    #[test]
    fn rotation_accumulates_and_wraps() {
        let src = scan(2);
        let sel = PageSelection::parse("1", 2).unwrap();
        let (once, _) = rotate(&src, &sel, 90).unwrap();
        assert_eq!(page_summaries(&once).unwrap()[0].rotation, 90);
        let (twice, _) = rotate(&once, &sel, 270).unwrap();
        assert_eq!(page_summaries(&twice).unwrap()[0].rotation, 0, "360 is back to square one");
        // And the untouched page stayed untouched.
        assert_eq!(page_summaries(&twice).unwrap()[1].rotation, 0);
    }

    #[test]
    fn rotating_swaps_the_reported_dimensions() {
        let src = scan(1);
        let before = page_summaries(&src).unwrap()[0].clone();
        let sel = PageSelection::parse("1", 1).unwrap();
        let (out, _) = rotate(&src, &sel, 90).unwrap();
        let after = page_summaries(&out).unwrap()[0].clone();
        assert!((before.width_pt - after.height_pt).abs() < 0.1);
        assert!((before.height_pt - after.width_pt).abs() < 0.1);
    }

    #[test]
    fn rejects_rotation_that_is_not_a_quarter_turn() {
        let src = scan(1);
        let sel = PageSelection::parse("1", 1).unwrap();
        assert!(rotate(&src, &sel, 45).is_err());
    }

    #[test]
    fn splits_every_page() {
        let src = scan(4);
        let parts = split(&src, &SplitMode::EveryPage).unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].suffix, "page-1");
        for (i, part) in parts.iter().enumerate() {
            assert_eq!(page_count(&part.bytes).unwrap(), 1);
            assert_eq!(
                page_image_heights(&part.bytes),
                vec![520 + i as i64],
                "part {i} should hold page {}",
                i + 1
            );
        }
    }

    #[test]
    fn splits_into_chunks_and_ranges() {
        let src = scan(5);
        let chunks = split(&src, &SplitMode::EveryN(2)).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].pages, vec![1, 2]);
        assert_eq!(chunks[2].pages, vec![5]);
        assert_eq!(chunks[0].suffix, "pages-1-2");

        let ranges = split(&src, &SplitMode::Ranges(vec![(1, 2), (4, 5)])).unwrap();
        assert_eq!(ranges.len(), 2);
        assert_eq!(page_count(&ranges[1].bytes).unwrap(), 2);
        assert!(split(&src, &SplitMode::Ranges(vec![(1, 9)])).is_err());
    }

    #[test]
    fn merges_documents_end_to_end() {
        let a = scan(2);
        let b = testkit::pdf_text_only();
        let c = scan(3);
        let (out, report) = merge(&[a, b, c]).unwrap();
        assert_eq!(report.pages_after, 6);
        assert_eq!(page_count(&out).unwrap(), 6);
        // The images from both scans must survive the renumbering.
        let inspected = super::super::inspect::inspect(&out).unwrap();
        assert_eq!(inspected.images.len(), 5, "every scanned page keeps its image");
    }

    #[test]
    fn merged_pages_keep_their_own_size() {
        let a = scan(1);
        let b = testkit::pdf_text_only();
        let (out, _) = merge(&[a, b]).unwrap();
        let summaries = page_summaries(&out).unwrap();
        assert_eq!(summaries.len(), 2);
        for page in summaries {
            assert!(page.width_pt > 100.0, "a page lost its MediaBox: {page:?}");
        }
    }

    #[test]
    fn a_scanned_page_can_be_previewed_without_a_renderer() {
        let src = scan(2);
        let jpeg = page_preview(&src, 1, 200).unwrap().expect("a scan should preview");
        let img = image::load_from_memory(&jpeg).unwrap();
        assert!(img.width() <= 200 && img.height() <= 200);
        assert!(page_summaries(&src).unwrap()[0].has_preview);
    }

    #[test]
    fn exports_scanned_pages_and_explains_the_rest() {
        // Two scanned pages and one of text, merged into one document.
        let (mixed, _) = merge(&[scan(2), testkit::pdf_text_only()]).unwrap();
        let exported = export_pages(&mixed, Some(300), 80).unwrap();
        assert_eq!(exported.len(), 3);

        for page in &exported[..2] {
            let jpeg = page.jpeg.as_ref().expect("a scan exports");
            let img = image::load_from_memory(jpeg).unwrap();
            assert_eq!((img.width(), img.height()), (page.width, page.height));
            assert!(page.reason.is_none());
        }

        let text_page = &exported[2];
        assert!(text_page.jpeg.is_none(), "a text page cannot be exported yet");
        let reason = text_page.reason.as_deref().unwrap();
        assert!(reason.contains("renderer"), "should say why: {reason}");
    }

    #[test]
    fn a_text_page_honestly_reports_no_preview() {
        let src = testkit::pdf_text_only();
        assert_eq!(page_preview(&src, 1, 200).unwrap(), None);
        assert!(!page_summaries(&src).unwrap()[0].has_preview);
    }

    #[test]
    fn page_sizes_are_named() {
        let src = testkit::pdf_text_only();
        let info = &page_summaries(&src).unwrap()[0];
        assert_eq!(info.label, "Letter portrait");
        assert_eq!(info.number, 1);
    }

    #[test]
    fn says_what_it_had_to_throw_away() {
        let mut doc = Document::load_mem(&scan(3)).unwrap();
        let outline = doc.add_object(dictionary! { "Type" => "Outlines", "Count" => 0i64 });
        doc.catalog_mut().unwrap().set("Outlines", Object::Reference(outline));
        let mut src = Vec::new();
        doc.save_to(&mut src).unwrap();

        let sel = PageSelection::parse("1-2", 3).unwrap();
        let (_, report) = extract(&src, &sel).unwrap();
        assert!(report.dropped.iter().any(|d| d == "bookmarks"), "{:?}", report.dropped);
        assert!(report.note().unwrap().contains("bookmarks"));
    }
}
