//! Folders Kitbox keeps an eye on.
//!
//! Point it at the folder your scanner writes to, or the one your phone syncs
//! into, and anything that lands there is compressed to the size you set. The
//! app doesn't have to be in front of you, and nothing has to be dragged.
//!
//! Three rules shape everything here, all of them about not doing damage:
//!
//! * **Arrivals only.** Pointing at a folder with five hundred photos in it
//!   does not compress five hundred photos. What is already there when a watch
//!   starts is noted and left alone.
//! * **Never the original.** Results go to the output folder like every other
//!   tool. A watcher that edits files in place would be a watcher that can
//!   destroy a scanner's only copy.
//! * **Wait until it stops moving.** A file appears the instant it is created
//!   and may be written for seconds afterwards. Reading it too early gets half
//!   a photo, so a file is only taken once its size has stopped changing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// One folder, and what to do with what lands in it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub path: String,
    /// A size as the parser reads it: `100kb`, `20-50kb`.
    pub target: String,
    pub enabled: bool,
}

impl Watch {
    pub fn size_target(&self) -> Result<mt_core::SizeTarget, String> {
        mt_core::parse_target(&self.target)
            .map_err(|e| format!("'{}' isn't a size this understands: {e}", self.target))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Watches(pub Vec<Watch>);

pub fn path(dir: &Path) -> PathBuf {
    dir.join("watched-folders.json")
}

pub fn load(dir: &Path) -> Watches {
    std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, watches: &Watches) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(watches).map_err(|e| e.to_string())?;
    std::fs::write(path(dir), json).map_err(|e| e.to_string())
}

/// Whether this file is worth looking at.
///
/// The output folder is excluded by the caller; this catches the rest — part
/// files, temporary files, and anything Kitbox itself produced, so a watch
/// pointed at the output folder cannot chase its own tail.
pub fn worth_taking(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || name.starts_with('~') {
        return false;
    }
    let lower = name.to_lowercase();
    if lower.contains("-kitbox") {
        return false;
    }
    // Half-written files announce themselves: these are the suffixes browsers,
    // sync clients and scanners use while a file is still arriving.
    for suffix in [".crdownload", ".part", ".partial", ".tmp", ".temp", ".download"] {
        if lower.ends_with(suffix) {
            return false;
        }
    }
    matches!(
        mt_core::FileKind::from_extension(&lower),
        Some(kind) if kind.is_image() || kind == mt_core::FileKind::Pdf
    )
}

/// Wait for a file to stop being written to.
///
/// Returns its size once two readings in a row agree, or `None` if it never
/// settles — which usually means it was deleted again, or is being held open
/// by whatever made it.
pub fn settle(path: &Path, patience: Duration) -> Option<u64> {
    let deadline = Instant::now() + patience;
    let mut previous = None;

    while Instant::now() < deadline {
        let size = std::fs::metadata(path).ok().map(|m| m.len());
        match (size, previous) {
            // Two readings the same, and not empty: it has stopped growing.
            (Some(now), Some(before))
                if now == before && now > 0
                // One last check that it can actually be opened — on Windows a
                // file can have its final size and still be locked.
                && std::fs::File::open(path).is_ok() =>
            {
                return Some(now);
            }
            _ => {}
        }
        previous = size;
        std::thread::sleep(Duration::from_millis(400));
    }
    None
}

/// What was in a folder when the watch started.
///
/// Everything already there is "seen", so only arrivals are taken.
pub fn already_there(folder: &Path) -> HashSet<PathBuf> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pictures_and_documents_are_taken() {
        assert!(worth_taking(Path::new("C:/in/scan.jpg")));
        assert!(worth_taking(Path::new("C:/in/report.pdf")));
        assert!(!worth_taking(Path::new("C:/in/notes.txt")));
        assert!(!worth_taking(Path::new("C:/in/setup.exe")));
    }

    #[test]
    fn a_file_still_arriving_is_left_alone() {
        // Chrome, sync clients and scanners all write to a temporary name
        // first. Taking one of those gets half a file.
        for name in ["photo.jpg.crdownload", "scan.pdf.part", "image.jpg.tmp", "~draft.pdf"] {
            assert!(!worth_taking(Path::new(name)), "{name} should be skipped");
        }
    }

    #[test]
    fn kitbox_never_watches_its_own_output() {
        // Otherwise a watch on the output folder would compress its own
        // results, then compress those, forever.
        assert!(!worth_taking(Path::new("C:/out/photo-kitbox.jpg")));
        assert!(!worth_taking(Path::new("C:/out/photo-kitbox-2.jpg")));
    }

    #[test]
    fn a_file_that_is_still_growing_is_not_reported_as_ready() {
        let dir = std::env::temp_dir().join("kitbox-watch-growing");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("arriving.jpg");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();

        // A writer that keeps appending for longer than we are willing to wait.
        let writing = file.clone();
        let hand = std::thread::spawn(move || {
            for _ in 0..8 {
                let mut bytes = std::fs::read(&writing).unwrap_or_default();
                bytes.extend(std::iter::repeat_n(0u8, 4096));
                let _ = std::fs::write(&writing, bytes);
                std::thread::sleep(Duration::from_millis(200));
            }
        });

        assert!(settle(&file, Duration::from_millis(900)).is_none(), "it was still being written");
        hand.join().unwrap();

        // Once the writing stops it settles.
        assert!(settle(&file, Duration::from_secs(3)).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn what_was_already_there_is_not_an_arrival() {
        let dir = std::env::temp_dir().join("kitbox-watch-existing");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.jpg"), b"x").unwrap();

        let seen = already_there(&dir);
        assert!(seen.iter().any(|p| p.ends_with("old.jpg")));

        std::fs::write(dir.join("new.jpg"), b"y").unwrap();
        assert!(!seen.iter().any(|p| p.ends_with("new.jpg")), "added afterwards is an arrival");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn watches_survive_the_round_trip() {
        let dir = std::env::temp_dir().join("kitbox-watch-settings");
        std::fs::create_dir_all(&dir).unwrap();
        let mine = Watches(vec![Watch {
            path: "C:\\Users\\me\\Scans".into(),
            target: "20-50kb".into(),
            enabled: true,
        }]);
        save(&dir, &mine).unwrap();

        let back = load(&dir);
        assert_eq!(back.0.len(), 1);
        assert_eq!(back.0[0].size_target().unwrap().max_bytes(), 50_000);
        std::fs::remove_dir_all(&dir).ok();
    }
}
