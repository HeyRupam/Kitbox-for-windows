//! Lossless padding, for the forms that demand a *minimum* file size.
//!
//! "Photo must be between 20 KB and 50 KB" is common, and a clean headshot at
//! full quality can land at 14 KB. Re-encoding at higher quality can't help once
//! quality is already maxed, so instead we grow the file with data every decoder
//! is required to ignore: a JPEG comment segment, a PNG ancillary chunk, a PDF
//! comment. The image itself is bit-for-bit unchanged.
//!
//! This is always reported to the user as [`crate::Outcome::Padded`] — it is a
//! legitimate trick, not something to do silently.

use crate::format::FileKind;

const NOTE: &[u8] = b"Padding added by Kitbox so this file meets a minimum size requirement. \
The content above is unchanged. ";

/// Grow `bytes` to at least `min_size`, or return `None` if this format has no
/// safe place to put ignored data.
pub fn pad_to(bytes: &[u8], min_size: u64) -> Option<Vec<u8>> {
    let current = bytes.len() as u64;
    if current >= min_size {
        return None;
    }
    let needed = (min_size - current) as usize;

    match FileKind::sniff(bytes) {
        FileKind::Jpeg => pad_jpeg(bytes, needed),
        FileKind::Png => pad_png(bytes, needed),
        FileKind::Pdf => pad_pdf(bytes, needed),
        _ => None,
    }
}

/// Filler that reads as an explanation rather than as noise, for anyone who
/// opens the file in a hex editor.
fn filler(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let take = (len - out.len()).min(NOTE.len());
        out.extend_from_slice(&NOTE[..take]);
    }
    out
}

/// Insert COM (comment) segments straight after the SOI marker. A JPEG decoder
/// is required to skip them.
fn pad_jpeg(bytes: &[u8], needed: usize) -> Option<Vec<u8>> {
    if bytes.len() < 2 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    const OVERHEAD: usize = 4; // FF FE + two length bytes
    const MAX_PAYLOAD: usize = 65_533; // length field counts itself

    let mut segments = Vec::new();
    let mut remaining = needed;
    while remaining > 0 {
        let payload = remaining.saturating_sub(OVERHEAD).min(MAX_PAYLOAD);
        let seg_len = payload + 2;
        segments.push(0xFF);
        segments.push(0xFE);
        segments.push((seg_len >> 8) as u8);
        segments.push((seg_len & 0xFF) as u8);
        segments.extend_from_slice(&filler(payload));
        remaining = remaining.saturating_sub(OVERHEAD + payload);
    }

    let mut out = Vec::with_capacity(bytes.len() + segments.len());
    out.extend_from_slice(&bytes[..2]);
    out.extend_from_slice(&segments);
    out.extend_from_slice(&bytes[2..]);
    Some(out)
}

/// Insert a private, ancillary, safe-to-copy chunk before IEND. The lowercase
/// first letter marks it ancillary, so every decoder skips it.
fn pad_png(bytes: &[u8], needed: usize) -> Option<Vec<u8>> {
    const OVERHEAD: usize = 12; // length + type + CRC
    let iend = find_iend(bytes)?;
    let payload_len = needed.saturating_sub(OVERHEAD);
    let payload = filler(payload_len);

    let mut chunk = Vec::with_capacity(OVERHEAD + payload.len());
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"kbPd");
    chunk.extend_from_slice(&payload);
    let crc = crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());

    let mut out = Vec::with_capacity(bytes.len() + chunk.len());
    out.extend_from_slice(&bytes[..iend]);
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&bytes[iend..]);
    Some(out)
}

/// Byte offset of the IEND chunk's length field.
fn find_iend(bytes: &[u8]) -> Option<usize> {
    let mut pos = 8; // skip signature
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        if kind == b"IEND" {
            return Some(pos);
        }
        pos = pos.checked_add(12)?.checked_add(len)?;
    }
    None
}

/// Append a comment after the final `%%EOF`. Readers locate the cross-reference
/// table by scanning backwards for `startxref`, which is still found.
fn pad_pdf(bytes: &[u8], needed: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len() + needed + 8);
    out.extend_from_slice(bytes);
    if !bytes.ends_with(b"\n") {
        out.push(b'\n');
    }
    let overhead = 2; // "%" and the trailing newline
    let payload = filler(needed.saturating_sub(overhead));
    out.push(b'%');
    // A comment runs to end of line, so the filler must not contain one.
    out.extend(payload.into_iter().map(|b| if b == b'\n' || b == b'\r' { b' ' } else { b }));
    out.push(b'\n');
    Some(out)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn jpeg_padding_keeps_the_image_decodable() {
        let src = testkit::tiny_jpeg();
        let padded = pad_to(&src, 30_000).expect("jpeg should pad");
        assert!(padded.len() >= 30_000);
        let before = image::load_from_memory(&src).unwrap();
        let after = image::load_from_memory(&padded).unwrap();
        assert_eq!(before.to_rgb8(), after.to_rgb8(), "padding must not touch pixels");
    }

    #[test]
    fn png_padding_keeps_the_image_decodable() {
        let src = testkit::tiny_png();
        let padded = pad_to(&src, 20_000).expect("png should pad");
        assert!(padded.len() >= 20_000);
        let before = image::load_from_memory(&src).unwrap();
        let after = image::load_from_memory(&padded).unwrap();
        assert_eq!(before.to_rgba8(), after.to_rgba8());
    }

    #[test]
    fn pdf_padding_leaves_the_trailer_findable() {
        let src = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\nstartxref\n9\n%%EOF\n".to_vec();
        let padded = pad_to(&src, 5_000).expect("pdf should pad");
        assert!(padded.len() >= 5_000);
        assert!(padded.starts_with(b"%PDF-"));
        let tail = &padded[padded.len().saturating_sub(6_000)..];
        assert!(
            tail.windows(9).any(|w| w == b"startxref"),
            "startxref must still be reachable from the end"
        );
    }

    #[test]
    fn declines_formats_with_nowhere_safe_to_put_it() {
        assert!(pad_to(b"GIF89a....", 10_000).is_none());
    }

    #[test]
    fn does_nothing_when_already_big_enough() {
        let src = testkit::tiny_jpeg();
        assert!(pad_to(&src, 10).is_none());
    }
}
