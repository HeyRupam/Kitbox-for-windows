//! Where a PDF's bytes actually go.
//!
//! "Compress PDF" tools are a black box: you press the button and either it got
//! smaller or it didn't, and you never learn why. This answers the question
//! first — images, fonts, page content, metadata — so a 1.5 MB file that won't
//! shrink can say "your two scans are 1.3 MB of it" instead of shrugging.
//!
//! Working out an image's real DPI means knowing how big it is *drawn*, not how
//! many pixels it has, so this walks each page's content stream tracking the
//! transformation matrix. That number drives the whole downsampling strategy:
//! without it, "reduce to 150 DPI" is guesswork.

use std::collections::{HashMap, HashSet};

use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// Pages are essentially one big image each — a scan or a photographed form.
    Scan,
    /// Real text plus images worth shrinking.
    TextWithImages,
    /// Text, vectors and fonts. Downsampling has nothing to bite on.
    TextOnly,
}

impl DocKind {
    pub fn describe(self) -> &'static str {
        match self {
            DocKind::Scan => "a scanned document",
            DocKind::TextWithImages => "text with images",
            DocKind::TextOnly => "text only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    #[serde(skip)]
    pub id: ObjectId,
    pub width: u32,
    pub height: u32,
    /// Stored (already-encoded) size in the file.
    pub bytes: u64,
    pub filter: String,
    pub color_space: String,
    /// Effective resolution as drawn. `None` when the image isn't placed on any
    /// page we could follow — inside a form XObject, say.
    pub dpi: Option<f32>,
    /// Whether this engine can safely re-encode it.
    pub recompressible: bool,
    /// Why not, when it isn't.
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfReport {
    pub pages: usize,
    pub total_bytes: u64,
    pub image_bytes: u64,
    pub font_bytes: u64,
    pub content_bytes: u64,
    pub metadata_bytes: u64,
    pub other_bytes: u64,
    pub images: Vec<ImageInfo>,
    pub kind: DocKind,
    pub encrypted: bool,
}

impl PdfReport {
    /// Bytes in images this engine can actually act on.
    pub fn recompressible_bytes(&self) -> u64 {
        self.images.iter().filter(|i| i.recompressible).map(|i| i.bytes).sum()
    }

    /// One line for the UI, in the user's terms.
    pub fn headline(&self) -> String {
        use crate::size::format_size;
        let biggest = self.images.iter().max_by_key(|i| i.bytes);
        match (self.kind, biggest) {
            (DocKind::TextOnly, _) => format!(
                "{} of text and fonts across {} page{}",
                format_size(self.total_bytes),
                self.pages,
                if self.pages == 1 { "" } else { "s" }
            ),
            (_, Some(img)) => format!(
                "{} of {} is images — the largest is {} at {}×{}",
                format_size(self.image_bytes),
                format_size(self.total_bytes),
                format_size(img.bytes),
                img.width,
                img.height
            ),
            _ => format_size(self.total_bytes),
        }
    }
}

/// A 2×3 PDF transformation matrix: `[a b c d e f]`.
#[derive(Debug, Clone, Copy)]
struct Matrix([f32; 6]);

impl Matrix {
    const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `self × other`, in PDF's row-vector convention.
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

    /// Size of the transformed unit square, in points.
    fn drawn_size(self) -> (f32, f32) {
        let [a, b, c, d, _, _] = self.0;
        ((a * a + b * b).sqrt(), (c * c + d * d).sqrt())
    }
}

pub fn inspect(bytes: &[u8]) -> Result<PdfReport> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| crate::Error::invalid(format!("could not read this PDF: {e}")))?;
    inspect_doc(&doc, bytes.len() as u64)
}

pub fn inspect_doc(doc: &Document, total_bytes: u64) -> Result<PdfReport> {
    let pages: Vec<ObjectId> = doc.page_iter().collect();
    let drawn = drawn_sizes(doc, &pages);
    let font_streams = font_stream_ids(doc);

    let mut content_ids: HashSet<ObjectId> = HashSet::new();
    for page in &pages {
        content_ids.extend(doc.get_page_contents(*page));
    }

    let mut image_bytes = 0u64;
    let mut font_bytes = 0u64;
    let mut content_bytes = 0u64;
    let mut metadata_bytes = 0u64;
    let mut other_bytes = 0u64;
    let mut images = Vec::new();

    for (id, object) in doc.objects.iter() {
        let Object::Stream(stream) = object else { continue };
        let size = stream.content.len() as u64;
        let subtype = stream.dict.get(b"Subtype").ok().and_then(name_of);
        let kind = stream.dict.get(b"Type").ok().and_then(name_of);

        if subtype.as_deref() == Some("Image") {
            image_bytes += size;
            images.push(describe_image(doc, *id, stream, size, drawn.get(id).copied()));
        } else if font_streams.contains(id) || kind.as_deref() == Some("Font") {
            font_bytes += size;
        } else if kind.as_deref() == Some("Metadata") {
            metadata_bytes += size;
        } else if content_ids.contains(id) {
            content_bytes += size;
        } else {
            other_bytes += size;
        }
    }

    // Whatever isn't in a stream — the object graph, xref, trailer — is the
    // remainder. Reporting it as "other" keeps the bar adding up to the file.
    let accounted = image_bytes + font_bytes + content_bytes + metadata_bytes + other_bytes;
    other_bytes += total_bytes.saturating_sub(accounted);

    let kind = classify(&images, image_bytes, content_bytes, font_bytes, pages.len());

    Ok(PdfReport {
        pages: pages.len(),
        total_bytes,
        image_bytes,
        font_bytes,
        content_bytes,
        metadata_bytes,
        other_bytes,
        images,
        kind,
        encrypted: doc.is_encrypted(),
    })
}

fn classify(
    images: &[ImageInfo],
    image_bytes: u64,
    content_bytes: u64,
    font_bytes: u64,
    pages: usize,
) -> DocKind {
    if images.is_empty() {
        return DocKind::TextOnly;
    }
    let text_bytes = content_bytes + font_bytes;
    // One big image per page and almost no text is the signature of a scan.
    let roughly_one_per_page = pages > 0 && images.len() <= pages * 2;
    if roughly_one_per_page && image_bytes > text_bytes.saturating_mul(4) {
        return DocKind::Scan;
    }
    if image_bytes * 4 < text_bytes {
        return DocKind::TextOnly;
    }
    DocKind::TextWithImages
}

fn describe_image(
    doc: &Document,
    id: ObjectId,
    stream: &lopdf::Stream,
    size: u64,
    drawn: Option<(f32, f32)>,
) -> ImageInfo {
    let width = stream.dict.get(b"Width").and_then(|o| o.as_i64()).unwrap_or(0).max(0) as u32;
    let height = stream.dict.get(b"Height").and_then(|o| o.as_i64()).unwrap_or(0).max(0) as u32;
    let filters: Vec<String> = stream
        .filters()
        .map(|fs| fs.into_iter().map(|f| String::from_utf8_lossy(f).into_owned()).collect())
        .unwrap_or_default();
    let filter = if filters.is_empty() { "none".to_string() } else { filters.join("+") };
    let color_space = color_space_name(doc, stream);
    let bpc = stream.dict.get(b"BitsPerComponent").and_then(|o| o.as_i64()).unwrap_or(8);

    let dpi = drawn.and_then(|(w_pt, h_pt)| {
        if w_pt <= 0.1 || h_pt <= 0.1 || width == 0 {
            return None;
        }
        // Use the horizontal axis; the two agree for any sanely placed image.
        Some(width as f32 / (w_pt / 72.0))
    });

    let (recompressible, skip_reason) = can_recompress(&filters, &color_space, bpc, width, height);

    ImageInfo {
        id,
        width,
        height,
        bytes: size,
        filter,
        color_space,
        dpi,
        recompressible,
        skip_reason,
    }
}

/// Which images this engine will touch.
///
/// The conservative list is deliberate: re-encoding an Indexed or Separation
/// image through RGB shifts its colours, and JPEG2000 or JBIG2 data isn't
/// something we can decode and put back faithfully. Leaving those alone costs
/// some compression and avoids wrecking the document.
fn can_recompress(
    filters: &[String],
    color_space: &str,
    bpc: i64,
    width: u32,
    height: u32,
) -> (bool, Option<String>) {
    if width < 8 || height < 8 {
        return (false, Some("too small to be worth re-encoding".into()));
    }
    for f in filters {
        match f.as_str() {
            "DCTDecode" | "FlateDecode" | "LZWDecode" => {}
            other => {
                return (false, Some(format!("{other} data can't be re-encoded safely")));
            }
        }
    }
    match color_space {
        "DeviceRGB" | "DeviceGray" | "CalRGB" | "CalGray" | "ICCBased" => {}
        other => return (false, Some(format!("{other} colour would shift if re-encoded"))),
    }
    if !filters.iter().any(|f| f == "DCTDecode") && bpc != 8 {
        return (false, Some(format!("{bpc}-bit samples aren't supported yet")));
    }
    (true, None)
}

fn color_space_name(doc: &Document, stream: &lopdf::Stream) -> String {
    let Ok(cs) = stream.dict.get(b"ColorSpace") else {
        return "unknown".into();
    };
    let cs = doc.dereference(cs).map(|(_, o)| o).unwrap_or(cs);
    match cs {
        Object::Name(n) => String::from_utf8_lossy(n).to_string(),
        Object::Array(items) => items
            .first()
            .and_then(name_of)
            .unwrap_or_else(|| "array".into()),
        _ => "unknown".into(),
    }
}

fn name_of(object: &Object) -> Option<String> {
    match object {
        Object::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
        _ => None,
    }
}

/// Font programs are streams hanging off a FontDescriptor, not objects marked
/// `/Type /Font`, so they have to be collected by walking the descriptors.
fn font_stream_ids(doc: &Document) -> HashSet<ObjectId> {
    let mut ids = HashSet::new();
    for object in doc.objects.values() {
        let dict = match object {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        for key in [&b"FontFile"[..], b"FontFile2", b"FontFile3"] {
            if let Ok(Object::Reference(id)) = dict.get(key) {
                ids.insert(*id);
            }
        }
    }
    ids
}

/// Largest drawn size, in points, for every image XObject placed on a page.
///
/// An image used twice at different scales is downsampled for the larger of the
/// two, so the bigger placement still looks right.
pub(super) fn drawn_sizes(doc: &Document, pages: &[ObjectId]) -> HashMap<ObjectId, (f32, f32)> {
    walk_placements(doc, pages, None)
}

/// Where each image is drawn on a page: `(x, y, width, height)` in page space.
///
/// The crop tool needs the position as well as the size, so the same content
/// walk serves both rather than being written twice.
pub(super) fn drawn_rects(
    doc: &Document,
    pages: &[ObjectId],
) -> HashMap<ObjectId, (f32, f32, f32, f32)> {
    let mut rects = HashMap::new();
    walk_placements(doc, pages, Some(&mut rects));
    rects
}

fn walk_placements(
    doc: &Document,
    pages: &[ObjectId],
    mut rects: Option<&mut HashMap<ObjectId, (f32, f32, f32, f32)>>,
) -> HashMap<ObjectId, (f32, f32)> {
    let mut out: HashMap<ObjectId, (f32, f32)> = HashMap::new();

    for page in pages {
        let Ok((resources, inherited)) = doc.get_page_resources(*page) else { continue };
        let mut xobjects: HashMap<Vec<u8>, ObjectId> = HashMap::new();
        let mut collect = |dict: &lopdf::Dictionary| {
            if let Ok(x) = dict.get(b"XObject") {
                if let Ok((_, Object::Dictionary(map))) = doc.dereference(x) {
                    for (name, value) in map.iter() {
                        if let Object::Reference(id) = value {
                            xobjects.insert(name.clone(), *id);
                        }
                    }
                }
            }
        };
        if let Some(dict) = resources {
            collect(dict);
        }
        for id in inherited {
            if let Ok(dict) = doc.get_dictionary(id) {
                collect(dict);
            }
        }
        if xobjects.is_empty() {
            continue;
        }

        let content = doc.get_page_content(*page);
        let Ok(ops) = lopdf::content::Content::decode(&content) else { continue };

        let mut ctm = Matrix::IDENTITY;
        let mut stack: Vec<Matrix> = Vec::new();
        for op in ops.operations {
            match op.operator.as_str() {
                "q" => stack.push(ctm),
                "Q" => ctm = stack.pop().unwrap_or(Matrix::IDENTITY),
                "cm" => {
                    if op.operands.len() >= 6 {
                        let mut m = [0f32; 6];
                        let mut ok = true;
                        for (i, slot) in m.iter_mut().enumerate() {
                            match op.operands[i].as_float() {
                                Ok(v) => *slot = v,
                                Err(_) => ok = false,
                            }
                        }
                        if ok {
                            ctm = Matrix(m).times(ctm);
                        }
                    }
                }
                "Do" => {
                    let Some(Object::Name(name)) = op.operands.first() else { continue };
                    let Some(id) = xobjects.get(name) else { continue };
                    let (w, h) = ctm.drawn_size();
                    let entry = out.entry(*id).or_insert((0.0, 0.0));
                    entry.0 = entry.0.max(w);
                    entry.1 = entry.1.max(h);
                    if let Some(rects) = rects.as_deref_mut() {
                        // Lower-left corner, from the matrix's translation.
                        rects.insert(*id, (ctm.0[4], ctm.0[5], w, h));
                    }
                }
                _ => {}
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn matrix_multiplication_matches_the_pdf_convention() {
        let scale = Matrix([200.0, 0.0, 0.0, 100.0, 0.0, 0.0]);
        let translate = Matrix([1.0, 0.0, 0.0, 1.0, 50.0, 20.0]);
        let combined = scale.times(translate);
        let (w, h) = combined.drawn_size();
        assert!((w - 200.0).abs() < 0.01);
        assert!((h - 100.0).abs() < 0.01);
        assert!((combined.0[4] - 50.0).abs() < 0.01);
    }

    #[test]
    fn finds_the_real_dpi_of_a_placed_image() {
        // A 600×600 image drawn into a 144×144 pt box is 300 DPI.
        let pdf = testkit::pdf_with_image(600, 600, 144.0, 144.0, 80);
        let report = inspect(&pdf).unwrap();
        assert_eq!(report.images.len(), 1);
        let dpi = report.images[0].dpi.expect("should resolve a DPI");
        assert!((dpi - 300.0).abs() < 1.0, "expected 300 DPI, got {dpi}");
    }

    #[test]
    fn attributes_bytes_to_images_in_a_scan() {
        let pdf = testkit::pdf_with_image(1200, 1600, 612.0, 792.0, 85);
        let report = inspect(&pdf).unwrap();
        assert_eq!(report.pages, 1);
        assert!(
            report.image_bytes > report.total_bytes / 2,
            "images should dominate: {} of {}",
            report.image_bytes,
            report.total_bytes
        );
        assert_eq!(report.kind, DocKind::Scan);
        assert!(report.images[0].recompressible);
        assert!(report.headline().contains("images"));
    }

    #[test]
    fn a_text_only_document_is_recognised() {
        let pdf = testkit::pdf_text_only();
        let report = inspect(&pdf).unwrap();
        assert_eq!(report.kind, DocKind::TextOnly);
        assert_eq!(report.image_bytes, 0);
        assert!(report.headline().contains("text"));
    }
}
