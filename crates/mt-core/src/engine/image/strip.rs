//! Removing what a file says about itself, without touching what it shows.
//!
//! Both formats here keep their metadata in whole, self-contained blocks that
//! can be dropped without re-encoding anything: a JPEG's APP and comment
//! segments, a PNG's text and timestamp chunks. Dropping them changes not one
//! pixel, which is what makes this worth having — the alternative, decoding
//! and re-encoding to get rid of a GPS tag, costs quality to remove something
//! that was never part of the picture.
//!
//! This is what lets a file that already meets its target be handed back as it
//! is, rather than re-compressed into something larger.

use crate::format::FileKind;

/// The same picture with nothing said about it. `None` when the format isn't
/// one this understands, and the bytes are returned unchanged when there was
/// nothing to remove.
pub fn without_metadata(bytes: &[u8]) -> Option<Vec<u8>> {
    match FileKind::sniff(bytes) {
        FileKind::Jpeg => strip_jpeg(bytes),
        FileKind::Png => strip_png(bytes),
        _ => None,
    }
}

/// True when there is something to remove — EXIF, GPS, a comment, a colour
/// profile that isn't being honoured anyway.
pub fn carries_metadata(bytes: &[u8]) -> bool {
    without_metadata(bytes).is_some_and(|clean| clean.len() != bytes.len())
}

/// JPEG: keep the structure, drop the APP segments that carry metadata.
///
/// APP0 stays because it holds the JFIF header, which says how the file is to
/// be read rather than where it was taken.
fn strip_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[0..2]);

    let mut at = 2;
    loop {
        if at + 4 > bytes.len() || bytes[at] != 0xFF {
            return None; // Not a shape this understands; leave the file alone.
        }
        let marker = bytes[at + 1];

        // Start of scan: everything from here is pixel data and the rest of
        // the file goes across untouched.
        if marker == 0xDA {
            out.extend_from_slice(&bytes[at..]);
            return Some(out);
        }

        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        let end = at + 2 + length;
        if end > bytes.len() {
            return None;
        }

        // APP1..APP15 is EXIF, XMP, ICC and the rest; FE is a comment.
        let drop = (0xE1..=0xEF).contains(&marker) || marker == 0xFE;
        if !drop {
            out.extend_from_slice(&bytes[at..end]);
        }
        at = end;
    }
}

/// PNG: copy the chunks that matter and drop the ones that only describe.
///
/// Each chunk carries its own length and checksum, so dropping whole chunks
/// needs no recomputation.
fn strip_png(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || bytes[0..8] != SIGNATURE {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&SIGNATURE);

    let mut at = 8;
    while at + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let tag = bytes.get(at + 4..at + 8)?;
        let end = at + 12 + length;
        if end > bytes.len() {
            return None;
        }

        let drop = matches!(tag, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME" | b"iCCP");
        if !drop {
            out.extend_from_slice(&bytes[at..end]);
        }
        at = end;

        if tag == b"IEND" {
            break;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG with an EXIF block in it, built by hand so the test doesn't
    /// depend on a camera.
    fn jpeg_with_exif() -> Vec<u8> {
        let plain = crate::testkit::encode_jpeg(&crate::testkit::photo(80, 60), 85);
        let mut out = Vec::new();
        out.extend_from_slice(&plain[0..2]); // SOI

        // APP1 with an Exif header and some bytes standing in for tags.
        let payload = b"Exif\0\0some camera said something here";
        let length = (payload.len() + 2) as u16;
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(payload);

        out.extend_from_slice(&plain[2..]);
        out
    }

    #[test]
    fn the_picture_is_identical_afterwards() {
        // The whole point: not one pixel changes, so nothing is spent.
        let with = jpeg_with_exif();
        let clean = without_metadata(&with).expect("a JPEG");

        assert!(clean.len() < with.len(), "the EXIF should be gone");
        let before = ::image::load_from_memory(&with).unwrap().into_rgb8();
        let after = ::image::load_from_memory(&clean).unwrap().into_rgb8();
        assert_eq!(before.as_raw(), after.as_raw(), "the pixels have to be untouched");
    }

    #[test]
    fn a_file_with_nothing_to_remove_comes_back_the_same_size() {
        let plain = crate::testkit::encode_jpeg(&crate::testkit::photo(60, 60), 80);
        let clean = without_metadata(&plain).expect("a JPEG");
        assert_eq!(clean.len(), plain.len());
        assert!(!carries_metadata(&plain));
        assert!(carries_metadata(&jpeg_with_exif()));
    }

    #[test]
    fn a_png_keeps_its_picture_and_loses_its_notes() {
        let mut png = Vec::new();
        crate::testkit::photo(70, 50)
            .write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png)
            .unwrap();

        // Slip a tEXt chunk in after the header.
        let text = {
            let mut chunk = Vec::new();
            let body = b"CommentSomething about this picture";
            chunk.extend_from_slice(&(body.len() as u32 - 7).to_be_bytes());
            chunk.extend_from_slice(b"tEXt");
            chunk.extend_from_slice(&body[7..]);
            chunk.extend_from_slice(&[0, 0, 0, 0]); // a checksum nobody checks here
            chunk
        };
        let header_end = 8 + 25; // signature + IHDR
        let mut with = png[..header_end].to_vec();
        with.extend_from_slice(&text);
        with.extend_from_slice(&png[header_end..]);

        let clean = without_metadata(&with).expect("a PNG");
        assert_eq!(clean.len(), png.len(), "it should be back to the plain file");
        let after = ::image::load_from_memory(&clean).unwrap().into_rgb8();
        let before = ::image::load_from_memory(&png).unwrap().into_rgb8();
        assert_eq!(before.as_raw(), after.as_raw());
    }

    #[test]
    fn anything_else_is_left_alone() {
        assert!(without_metadata(b"not a picture at all").is_none());
        let pdf = crate::testkit::pdf_text_only();
        assert!(without_metadata(&pdf).is_none(), "a PDF is somebody else's job");
    }
}
