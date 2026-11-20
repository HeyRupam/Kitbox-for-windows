//! Copy a picture, press a key, get a file small enough to upload.
//!
//! The right-click menu covers files on disk. This covers the other half of
//! how images actually arrive: a screenshot, a crop from a browser, something
//! pasted out of a chat — things that live on the clipboard and have no path
//! at all. Press the key and Kitbox takes what's on the clipboard, lands it
//! inside a size, writes it to your output folder and puts **the file** back
//! on the clipboard.
//!
//! That last part is the bit that matters. A Windows clipboard picture is an
//! uncompressed bitmap, so handing one back would throw away the compression
//! the moment anything re-encoded it. A file on the clipboard pastes into the
//! File name box of an upload dialog, and into Explorer as a copy.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the key does, and whether it is listening.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotkey {
    pub enabled: bool,
    /// A Tauri accelerator, e.g. `CommandOrControl+Alt+K`.
    pub combo: String,
    /// The size to land inside, as the size parser reads it: `100kb`,
    /// `20-50kb`.
    pub target: String,
}

impl Default for Hotkey {
    fn default() -> Self {
        Hotkey {
            // Off until asked for: a program that silently takes a key
            // combination away from everything else on the machine is not a
            // good guest.
            enabled: false,
            // Ctrl+Alt+K is the least likely of the obvious choices to be
            // taken: Ctrl+Shift+K is a browser console, Ctrl+Shift+S is a
            // screenshot tool in several apps.
            combo: "CommandOrControl+Alt+K".into(),
            target: "100kb".into(),
        }
    }
}

impl Hotkey {
    pub fn size_target(&self) -> Result<mt_core::SizeTarget, String> {
        mt_core::parse_target(&self.target)
            .map_err(|e| format!("'{}' isn't a size this understands: {e}", self.target))
    }
}

/// What one press did, for the toast and for the screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pressed {
    pub ok: bool,
    /// One line, as a person would say it.
    pub summary: String,
    pub before: u64,
    pub after: u64,
    pub path: Option<String>,
}

/// The picture on the clipboard, as bytes something can decode.
///
/// Windows keeps clipboard pictures as a device-independent bitmap, which the
/// crate hands back wrapped as a BMP file — exactly what the image decoder
/// wants.
#[cfg(windows)]
pub fn picture_on_clipboard() -> Result<Vec<u8>, String> {
    use clipboard_win::{formats, get_clipboard};

    get_clipboard(formats::Bitmap).map_err(|_| {
        "there's no picture on the clipboard — copy one first, then press the key".to_string()
    })
}

#[cfg(not(windows))]
pub fn picture_on_clipboard() -> Result<Vec<u8>, String> {
    Err("the clipboard key is a Windows thing".into())
}

/// Put a file on the clipboard, so it can be pasted where a file is wanted.
#[cfg(windows)]
pub fn put_file_on_clipboard(path: &Path) -> Result<(), String> {
    use clipboard_win::{formats, raw, Clipboard, Setter};

    // The convenience wrapper can't carry a list, so this is the long way
    // round: open the clipboard, clear it, write the one file, let the guard
    // close it on the way out.
    let files = [path.display().to_string()];
    let _clipboard =
        Clipboard::new_attempts(10).map_err(|e| format!("the clipboard was busy: {e}"))?;
    raw::empty().map_err(|e| format!("the clipboard could not be cleared: {e}"))?;
    formats::FileList
        .write_clipboard(&files[..])
        .map_err(|e| format!("the file could not be put on the clipboard: {e}"))
}

#[cfg(not(windows))]
pub fn put_file_on_clipboard(_path: &Path) -> Result<(), String> {
    Err("the clipboard key is a Windows thing".into())
}

/// Compress what's on the clipboard. The caller writes the file and puts it
/// back, because where files go is the shell's business, not this module's.
pub fn squeeze(bytes: &[u8], target: mt_core::SizeTarget) -> Result<(Vec<u8>, String), String> {
    use mt_core::engine::image::{compress_to_target, CompressOptions, OutputFormat};

    let outcome = compress_to_target(
        bytes,
        target,
        mt_core::Bias::Balanced,
        CompressOptions {
            // A clipboard bitmap has no format worth keeping, and JPEG is what
            // an upload box expects.
            format: OutputFormat::Jpeg,
            strip_metadata: true,
            max_dimension: None,
            exact_dimensions: None,
        },
    )
    .map_err(|e| e.to_string())?;

    Ok(match outcome {
        mt_core::Outcome::Hit(solved) => (solved.bytes, solved.knob),
        mt_core::Outcome::Padded(solved) => {
            (solved.bytes, format!("{}, padded up to the minimum", solved.knob))
        }
        mt_core::Outcome::Infeasible { best, why, .. } => (best, why),
    })
}

/// A name for something that never had one.
pub fn name_for(now: std::time::SystemTime) -> String {
    let stamp = now.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    format!("clipboard-{stamp}.jpg")
}

/// Where the settings live.
pub fn path(dir: &Path) -> PathBuf {
    dir.join("hotkey.json")
}

pub fn load(dir: &Path) -> Hotkey {
    std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, hotkey: &Hotkey) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(hotkey).map_err(|e| e.to_string())?;
    std::fs::write(path(dir), json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_off_until_it_is_asked_for() {
        // Taking a key combination away from the whole machine is not a thing
        // to do on someone's behalf.
        let fresh = Hotkey::default();
        assert!(!fresh.enabled);
        assert!(fresh.combo.contains("Alt"));
        assert_eq!(fresh.size_target().unwrap().max_bytes(), 100_000);
    }

    #[test]
    fn a_size_it_cannot_read_is_refused_in_words() {
        let hotkey = Hotkey { target: "smallish".into(), ..Default::default() };
        let refused = hotkey.size_target().unwrap_err();
        assert!(refused.contains("smallish"), "{refused}");
    }

    #[test]
    fn settings_survive_the_round_trip() {
        let dir = std::env::temp_dir().join("kitbox-hotkey-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mine = Hotkey {
            enabled: true,
            combo: "CommandOrControl+Shift+K".into(),
            target: "20-50kb".into(),
        };
        save(&dir, &mine).unwrap();

        let back = load(&dir);
        assert!(back.enabled);
        assert_eq!(back.combo, "CommandOrControl+Shift+K");
        assert_eq!(back.size_target().unwrap().min_bytes(), 20_000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_settings_file_that_is_nonsense_falls_back_rather_than_failing() {
        // Hand-editable files get hand-edited. Losing the key is better than
        // refusing to start.
        let dir = std::env::temp_dir().join("kitbox-hotkey-broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), "{ not json").unwrap();
        assert!(!load(&dir).enabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_clipboard_picture_is_squeezed_into_the_size_asked_for() {
        let bmp = {
            let mut bytes = Vec::new();
            mt_core::testkit::photo(1400, 1000)
                .write_to(&mut std::io::Cursor::new(&mut bytes), ::image::ImageFormat::Bmp)
                .unwrap();
            bytes
        };
        // A clipboard bitmap is big and uncompressed; that is the point.
        assert!(bmp.len() > 1_000_000, "{} bytes", bmp.len());

        let (out, note) = squeeze(&bmp, mt_core::SizeTarget::AtMost(100_000)).unwrap();
        assert!(out.len() <= 100_000, "came out at {}", out.len());
        assert_eq!(mt_core::FileKind::sniff(&out), mt_core::FileKind::Jpeg);
        assert!(!note.is_empty());
    }

    #[test]
    fn the_name_says_where_it_came_from() {
        let name = name_for(std::time::SystemTime::now());
        assert!(name.starts_with("clipboard-"));
        assert!(name.ends_with(".jpg"));
    }
}
