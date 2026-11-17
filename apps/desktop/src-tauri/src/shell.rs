//! The Windows right-click menu.
//!
//! The point of this one is that the app never has to be open. A form wants a
//! photo under 100 KB: right-click it, pick the size, and the file is waiting
//! in your output folder. That is the whole workflow, without a window, a drag
//! or a file picker.
//!
//! Everything here is written under `HKEY_CURRENT_USER`, which needs no
//! administrator and touches nothing outside this account. Turning the menu
//! off removes exactly the keys it added, and the uninstaller runs the same
//! removal — a context menu that outlives the program it points at is one of
//! the nastier things a Windows app can leave behind.

use std::path::Path;

use serde::Serialize;

#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
#[cfg(windows)]
use winreg::RegKey;

/// The file types the menu appears on.
///
/// Deliberately not `*`: a toolkit for pictures and documents has no business
/// adding itself to the menu for every text file and executable on the
/// machine.
const PICTURES: &[&str] =
    &[".jpg", ".jpeg", ".png", ".webp", ".bmp", ".tif", ".tiff", ".heic", ".gif"];
const DOCUMENTS: &[&str] = &[".pdf"];

/// The key Kitbox owns under each file type.
const KEY: &str = "Kitbox";

/// One line in the menu.
struct Item {
    /// Orders the menu; Windows sorts the subkeys by name.
    order: u8,
    label: &'static str,
    /// Passed back to the app as `--shell <action>`.
    action: &'static str,
}

const FOR_PICTURES: &[Item] = &[
    Item { order: 1, label: "Compress to 20–50 KB", action: "compress:20-50kb" },
    Item { order: 2, label: "Compress to 50–100 KB", action: "compress:50-100kb" },
    Item { order: 3, label: "Compress to under 100 KB", action: "compress:100kb" },
    Item { order: 4, label: "Open in Kitbox", action: "open" },
];

const FOR_DOCUMENTS: &[Item] = &[
    Item { order: 1, label: "Compress to 50–100 KB", action: "compress:50-100kb" },
    Item { order: 2, label: "Compress to under 200 KB", action: "compress:200kb" },
    Item { order: 3, label: "Compress to under 500 KB", action: "compress:500kb" },
    Item { order: 4, label: "Open in Kitbox", action: "open" },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuState {
    pub installed: bool,
    /// What the menu offers, for the switch to describe itself honestly.
    pub items: Vec<String>,
    pub types: Vec<String>,
    /// Windows 11 hides classic entries behind "Show more options".
    pub note: String,
}

pub fn state() -> MenuState {
    let mut items: Vec<String> = FOR_PICTURES.iter().map(|i| i.label.to_string()).collect();
    for item in FOR_DOCUMENTS {
        if !items.iter().any(|existing| existing == item.label) {
            items.push(item.label.to_string());
        }
    }
    MenuState {
        installed: installed(),
        items,
        types: PICTURES.iter().chain(DOCUMENTS).map(|e| e.to_string()).collect(),
        note: "On Windows 11 this sits under \"Show more options\" in the right-click menu, \
               or press Shift while right-clicking."
            .into(),
    }
}

#[cfg(windows)]
pub fn installed() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    PICTURES
        .first()
        .map(|ext| hkcu.open_subkey_with_flags(shell_path(ext), KEY_READ).is_ok())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn installed() -> bool {
    false
}

#[cfg(windows)]
fn shell_path(extension: &str) -> String {
    format!("Software\\Classes\\SystemFileAssociations\\{extension}\\shell\\{KEY}")
}

/// Add the menu for this account.
#[cfg(windows)]
pub fn install(exe: &Path) -> Result<(), String> {
    let exe = exe.display().to_string();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    for (types, items) in [(PICTURES, FOR_PICTURES), (DOCUMENTS, FOR_DOCUMENTS)] {
        for extension in types {
            let (root, _) = hkcu
                .create_subkey_with_flags(shell_path(extension), KEY_WRITE)
                .map_err(|e| format!("could not write the menu for {extension}: {e}"))?;

            root.set_value("MUIVerb", &"Kitbox").map_err(|e| e.to_string())?;
            root.set_value("Icon", &exe).map_err(|e| e.to_string())?;
            // An empty `subcommands` is what makes Windows read the `shell`
            // subkey below as a flyout rather than looking for a COM handler.
            root.set_value("subcommands", &"").map_err(|e| e.to_string())?;

            for item in items.iter() {
                let path = format!("{}\\shell\\{:02}{}", shell_path(extension), item.order, KEY);
                let (entry, _) = hkcu
                    .create_subkey_with_flags(&path, KEY_WRITE)
                    .map_err(|e| e.to_string())?;
                entry.set_value("MUIVerb", &item.label).map_err(|e| e.to_string())?;

                let (command, _) = hkcu
                    .create_subkey_with_flags(format!("{path}\\command"), KEY_WRITE)
                    .map_err(|e| e.to_string())?;
                command
                    .set_value("", &format!("\"{exe}\" --shell {} \"%1\"", item.action))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Point an installed menu at this copy of the app.
///
/// The command line in the registry names an exe. Move the app, or install it
/// properly after trying it from a folder, and every menu entry would run a
/// program that is no longer there — a menu that does nothing when clicked is
/// worse than no menu.
#[cfg(windows)]
pub fn refresh(exe: &Path) -> Result<(), String> {
    if !installed() {
        return Ok(());
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let first = PICTURES.first().ok_or("no file types")?;
    let path = format!("{}\\shell\\01{KEY}\\command", shell_path(first));
    let current: String = hkcu
        .open_subkey_with_flags(&path, KEY_READ)
        .and_then(|k| k.get_value(""))
        .unwrap_or_default();

    if current.contains(&exe.display().to_string()) {
        return Ok(());
    }
    install(exe)
}

#[cfg(not(windows))]
pub fn refresh(_exe: &Path) -> Result<(), String> {
    Ok(())
}

/// Take the menu away again, leaving nothing behind.
#[cfg(windows)]
pub fn remove() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for extension in PICTURES.iter().chain(DOCUMENTS) {
        match hkcu.delete_subkey_all(shell_path(extension)) {
            Ok(()) => {}
            // Already gone is the desired state, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("could not remove the menu for {extension}: {e}")),
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn install(_exe: &Path) -> Result<(), String> {
    Err("the right-click menu is a Windows thing".into())
}

#[cfg(not(windows))]
pub fn remove() -> Result<(), String> {
    Ok(())
}

/// What the app was asked to do, read from `--shell <action> <file>`.
#[derive(Debug, Clone, PartialEq)]
pub enum Asked {
    /// Land the file inside a size, without opening anything.
    Compress(mt_core::SizeTarget),
    /// Put it on screen with the file loaded.
    Open,
}

/// Read the command line Explorer hands back.
///
/// Returns `None` when this isn't a shell launch, which is the ordinary case.
pub fn parse(argv: &[String]) -> Option<(Asked, Vec<std::path::PathBuf>)> {
    let at = argv.iter().position(|a| a == "--shell")?;
    let action = argv.get(at + 1)?;
    let asked = match action.strip_prefix("compress:") {
        Some(target) => Asked::Compress(mt_core::parse_target(target).ok()?),
        None if action == "open" => Asked::Open,
        None => return None,
    };

    let files: Vec<std::path::PathBuf> =
        argv.iter().skip(at + 2).filter(|a| !a.starts_with("--")).map(Into::into).collect();
    if files.is_empty() {
        return None;
    }
    Some((asked, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_launch_is_read_back_exactly_as_it_was_written() {
        // The registry writes these strings; this reads them. If the two ever
        // disagree the menu silently does nothing, which is the worst way for
        // this to fail.
        for item in FOR_PICTURES.iter().chain(FOR_DOCUMENTS) {
            let argv = vec![
                "kitbox.exe".to_string(),
                "--shell".to_string(),
                item.action.to_string(),
                "C:\\Users\\me\\photo.jpg".to_string(),
            ];
            let (asked, files) = parse(&argv).unwrap_or_else(|| panic!("{}", item.action));
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].to_string_lossy(), "C:\\Users\\me\\photo.jpg");
            if item.action == "open" {
                assert_eq!(asked, Asked::Open);
            } else {
                assert!(matches!(asked, Asked::Compress(_)), "{}", item.action);
            }
        }
    }

    #[test]
    fn the_sizes_in_the_menu_are_the_sizes_that_come_back() {
        let argv = ["x".into(), "--shell".into(), "compress:20-50kb".into(), "a.jpg".into()];
        match parse(&argv).unwrap().0 {
            Asked::Compress(target) => {
                assert_eq!(target.min_bytes(), 20_000);
                assert_eq!(target.max_bytes(), 50_000);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_ordinary_start_is_not_a_shell_launch() {
        assert!(parse(&["kitbox.exe".to_string()]).is_none());
        assert!(parse(&["kitbox.exe".into(), "--render".into(), "a.html".into()]).is_none());
        // A verb with nothing to act on is not a job.
        assert!(parse(&["x".into(), "--shell".into(), "open".into()]).is_none());
        // And a verb this build doesn't know is ignored rather than guessed at.
        assert!(parse(&["x".into(), "--shell".into(), "frobnicate".into(), "a.jpg".into()]).is_none());
    }

    #[test]
    fn several_files_arrive_together() {
        let argv = [
            "x".into(),
            "--shell".into(),
            "compress:100kb".into(),
            "one.jpg".into(),
            "two.jpg".into(),
        ];
        let (_, files) = parse(&argv).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn the_menu_only_offers_itself_on_files_it_can_open() {
        // Adding Kitbox to the menu for every file on the machine would be
        // rude and useless.
        let state = state();
        assert!(state.types.iter().all(|t| t.starts_with('.')));
        assert!(state.types.contains(&".pdf".to_string()));
        assert!(!state.types.contains(&"*".to_string()));
        assert!(state.types.len() >= 8);
    }
}
