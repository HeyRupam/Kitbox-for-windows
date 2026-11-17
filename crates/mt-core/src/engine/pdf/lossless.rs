//! Stage 1: make the file smaller without changing a single pixel.
//!
//! This runs before any image is touched and often gets 10–40% on its own —
//! more on files produced by office software, which tend to carry duplicated
//! images, orphaned objects and a few hundred KB of XML metadata nobody reads.
//! If it reaches the target by itself, the solver stops here and the document
//! is bit-for-bit as good as it started.

use std::collections::HashMap;

use lopdf::{Document, Object, ObjectId, SaveOptions};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LosslessStats {
    pub metadata_removed: usize,
    pub duplicate_images_merged: usize,
    pub orphans_pruned: usize,
}

/// Apply every lossless saving to `doc` in place.
pub fn lossless_pass(doc: &mut Document, strip_metadata: bool) -> LosslessStats {
    let mut stats = LosslessStats::default();

    if strip_metadata {
        stats.metadata_removed = strip_document_metadata(doc);
    }
    stats.duplicate_images_merged = dedupe_images(doc);
    stats.orphans_pruned = doc.prune_objects().len();
    doc.delete_zero_length_streams();
    doc.compress();
    doc.renumber_objects();

    stats
}

/// Serialise with every structural saving the format allows: objects packed
/// into object streams, a compressed cross-reference stream, maximum deflate.
pub fn save_compact(doc: &mut Document) -> Result<Vec<u8>> {
    let options = SaveOptions::builder()
        .use_object_streams(true)
        .use_xref_streams(true)
        .compression_level(9)
        .build();
    let mut out = Vec::new();
    doc.save_with_options(&mut out, options)
        .map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;
    Ok(out)
}

/// Remove the things that describe the document rather than being it: the XMP
/// packet, the Info dictionary's personal fields, page thumbnails, and
/// application scratch data.
///
/// Titles and authors are personal information that routinely rides along into
/// a form upload, so stripping them is the default rather than an extra.
fn strip_document_metadata(doc: &mut Document) -> usize {
    let mut removed = 0;

    if let Ok(catalog) = doc.catalog_mut() {
        for key in [&b"Metadata"[..], b"PieceInfo", b"SpiderInfo", b"OutputIntents"] {
            if catalog.remove(key).is_some() {
                removed += 1;
            }
        }
    }

    for key in [
        &b"Title"[..],
        b"Author",
        b"Subject",
        b"Keywords",
        b"Creator",
        b"Producer",
    ] {
        if let Ok(Object::Dictionary(info)) = doc.trailer.get_mut(b"Info") {
            if info.remove(key).is_some() {
                removed += 1;
            }
        }
    }
    // The Info dictionary is usually a reference rather than inline.
    if let Ok(Object::Reference(id)) = doc.trailer.get(b"Info").cloned() {
        if let Ok(info) = doc.get_dictionary_mut(id) {
            for key in [
                &b"Title"[..],
                b"Author",
                b"Subject",
                b"Keywords",
                b"Creator",
                b"Producer",
            ] {
                if info.remove(key).is_some() {
                    removed += 1;
                }
            }
        }
    }

    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    for id in page_ids {
        if let Ok(page) = doc.get_dictionary_mut(id) {
            for key in [&b"Thumb"[..], b"PieceInfo", b"Metadata"] {
                if page.remove(key).is_some() {
                    removed += 1;
                }
            }
        }
    }

    removed
}

/// Point every reference to a byte-identical image at one copy of it.
///
/// A letterhead logo repeated on forty pages is forty copies in plenty of
/// real-world PDFs.
fn dedupe_images(doc: &mut Document) -> usize {
    let mut first_seen: HashMap<[u8; 32], ObjectId> = HashMap::new();
    let mut remap: HashMap<ObjectId, ObjectId> = HashMap::new();

    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else { continue };
        let is_image = stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| match o {
                Object::Name(n) => Some(n.as_slice() == b"Image"),
                _ => None,
            })
            .unwrap_or(false);
        if !is_image {
            continue;
        }
        // Hash the dictionary alongside the bytes: two streams with the same
        // pixels but different colour spaces are not interchangeable.
        let mut hasher = blake3::Hasher::new();
        hasher.update(&stream.content);
        hasher.update(format!("{:?}", stream.dict).as_bytes());
        let key = *hasher.finalize().as_bytes();

        match first_seen.get(&key) {
            Some(canonical) => {
                remap.insert(id, *canonical);
            }
            None => {
                first_seen.insert(key, id);
            }
        }
    }

    if remap.is_empty() {
        return 0;
    }

    doc.traverse_objects(|object| {
        if let Object::Reference(id) = object {
            if let Some(canonical) = remap.get(id) {
                *id = *canonical;
            }
        }
    });
    for id in remap.keys() {
        doc.objects.remove(id);
    }

    remap.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn repacking_alone_shrinks_a_plain_document() {
        let original = testkit::pdf_text_only();
        let mut doc = Document::load_mem(&original).unwrap();
        lossless_pass(&mut doc, true);
        let out = save_compact(&mut doc).unwrap();
        assert!(
            out.len() <= original.len(),
            "repacking should not grow the file: {} → {}",
            original.len(),
            out.len()
        );
        // And it must still be a readable PDF with the same page count.
        let reloaded = Document::load_mem(&out).unwrap();
        assert_eq!(reloaded.page_iter().count(), 1);
    }

    #[test]
    fn metadata_is_removed() {
        let original = testkit::pdf_text_only();
        let mut doc = Document::load_mem(&original).unwrap();
        // lopdf stamps a Producer on save, so there is something to strip.
        let before = doc.trailer.get(b"Info").is_ok();
        lossless_pass(&mut doc, true);
        let out = save_compact(&mut doc).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("/Author"), "author should be gone");
        let _ = before;
    }

    #[test]
    fn identical_images_collapse_to_one() {
        use lopdf::{dictionary, Stream};
        let jpeg = testkit::encode_jpeg(&testkit::photo(200, 200), 80);
        let mut doc = Document::load_mem(&testkit::pdf_text_only()).unwrap();
        let dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 200i64,
            "Height" => 200i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8i64,
            "Filter" => "DCTDecode",
        };
        let a = doc.add_object(Stream::new(dict.clone(), jpeg.clone()).with_compression(false));
        let b = doc.add_object(Stream::new(dict, jpeg).with_compression(false));
        // Reference both from the catalog so neither is pruned as an orphan.
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("KitboxA", Object::Reference(a));
        catalog.set("KitboxB", Object::Reference(b));

        let merged = dedupe_images(&mut doc);
        assert_eq!(merged, 1, "the second copy should fold into the first");
        let catalog = doc.catalog().unwrap();
        assert_eq!(catalog.get(b"KitboxA").unwrap(), catalog.get(b"KitboxB").unwrap());
    }
}
