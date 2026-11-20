//! Format sniffing from bytes. Never trust the extension — a form upload that
//! says `.jpg` is routinely a PNG, and the encoder we pick has to match reality.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Jpeg,
    Png,
    Gif,
    WebP,
    Bmp,
    Tiff,
    Heic,
    Svg,
    Pdf,
    Unknown,
}

impl FileKind {
    pub fn sniff(bytes: &[u8]) -> FileKind {
        let b = bytes;
        if b.len() >= 3 && b[..3] == [0xFF, 0xD8, 0xFF] {
            return FileKind::Jpeg;
        }
        if b.len() >= 8 && b[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
            return FileKind::Png;
        }
        if b.len() >= 6 && (&b[..6] == b"GIF87a" || &b[..6] == b"GIF89a") {
            return FileKind::Gif;
        }
        if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
            return FileKind::WebP;
        }
        if b.len() >= 2 && &b[..2] == b"BM" {
            return FileKind::Bmp;
        }
        if b.len() >= 4
            && (b[..4] == [0x49, 0x49, 0x2A, 0x00] || b[..4] == [0x4D, 0x4D, 0x00, 0x2A])
        {
            return FileKind::Tiff;
        }
        if b.len() >= 12 && &b[4..8] == b"ftyp" {
            let brand = &b[8..12];
            if brand == b"heic" || brand == b"heix" || brand == b"hevc" || brand == b"mif1" {
                return FileKind::Heic;
            }
        }
        if b.len() >= 5 && &b[..5] == b"%PDF-" {
            return FileKind::Pdf;
        }
        // SVG may be preceded by whitespace, a BOM or an XML declaration.
        let head_len = b.len().min(512);
        if let Ok(head) = std::str::from_utf8(&b[..head_len]) {
            let t = head.trim_start_matches('\u{feff}').trim_start();
            if t.starts_with("<svg") || (t.starts_with("<?xml") && head.contains("<svg")) {
                return FileKind::Svg;
            }
        }
        FileKind::Unknown
    }

    /// What a name promises, which is not the same as what a file is.
    ///
    /// Sniffing the bytes is the truth and is what every tool uses. This is
    /// for deciding whether a file is worth opening at all — a folder watcher
    /// that sniffed every `.exe` that landed would be reading whole binaries
    /// to learn what their name already said.
    pub fn from_extension(name: &str) -> Option<FileKind> {
        let extension = name.rsplit('.').next()?.to_ascii_lowercase();
        Some(match extension.as_str() {
            "jpg" | "jpeg" | "jfif" => FileKind::Jpeg,
            "png" => FileKind::Png,
            "gif" => FileKind::Gif,
            "webp" => FileKind::WebP,
            "bmp" | "dib" => FileKind::Bmp,
            "tif" | "tiff" => FileKind::Tiff,
            "heic" | "heif" => FileKind::Heic,
            "svg" => FileKind::Svg,
            "pdf" => FileKind::Pdf,
            _ => return None,
        })
    }

    pub fn is_image(self) -> bool {
        !matches!(self, FileKind::Pdf | FileKind::Unknown)
    }

    /// Extension without the dot, for naming outputs.
    pub fn extension(self) -> &'static str {
        match self {
            FileKind::Jpeg => "jpg",
            FileKind::Png => "png",
            FileKind::Gif => "gif",
            FileKind::WebP => "webp",
            FileKind::Bmp => "bmp",
            FileKind::Tiff => "tiff",
            FileKind::Heic => "heic",
            FileKind::Svg => "svg",
            FileKind::Pdf => "pdf",
            FileKind::Unknown => "bin",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            FileKind::Jpeg => "image/jpeg",
            FileKind::Png => "image/png",
            FileKind::Gif => "image/gif",
            FileKind::WebP => "image/webp",
            FileKind::Bmp => "image/bmp",
            FileKind::Tiff => "image/tiff",
            FileKind::Heic => "image/heic",
            FileKind::Svg => "image/svg+xml",
            FileKind::Pdf => "application/pdf",
            FileKind::Unknown => "application/octet-stream",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileKind::Jpeg => "JPG",
            FileKind::Png => "PNG",
            FileKind::Gif => "GIF",
            FileKind::WebP => "WEBP",
            FileKind::Bmp => "BMP",
            FileKind::Tiff => "TIFF",
            FileKind::Heic => "HEIC",
            FileKind::Svg => "SVG",
            FileKind::Pdf => "PDF",
            FileKind::Unknown => "file",
        }
    }

    /// The label with the right article in front: "a PDF", "an SVG".
    ///
    /// These are initialisms, so the article follows how the first letter is
    /// said, not whether it is a vowel — "an SVG", because it starts "ess".
    pub fn article(self) -> String {
        let label = self.label();
        // SVG is the only label here that is spelled out letter by letter;
        // HEIC is said "heek", so it keeps "a".
        let said_with_a_vowel = self == FileKind::Svg;
        format!("{} {label}", if said_with_a_vowel { "an" } else { "a" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_says_what_to_expect_even_before_the_bytes() {
        assert_eq!(FileKind::from_extension("scan.JPG"), Some(FileKind::Jpeg));
        assert_eq!(FileKind::from_extension("report.pdf"), Some(FileKind::Pdf));
        assert_eq!(FileKind::from_extension("photo.heif"), Some(FileKind::Heic));
        assert_eq!(FileKind::from_extension("notes.txt"), None);
        assert_eq!(FileKind::from_extension("noextension"), None);
    }

    #[test]
    fn sniffs_the_common_ones() {
        assert_eq!(FileKind::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), FileKind::Jpeg);
        assert_eq!(
            FileKind::sniff(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            FileKind::Png
        );
        assert_eq!(FileKind::sniff(b"%PDF-1.7\n"), FileKind::Pdf);
        assert_eq!(FileKind::sniff(b"  <svg xmlns=\"\">"), FileKind::Svg);
        assert_eq!(FileKind::sniff(b"nope"), FileKind::Unknown);
    }
}
