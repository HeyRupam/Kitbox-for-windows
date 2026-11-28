//! Form presets as files you can edit.
//!
//! The rules a form states change between cycles, and two portals for the same
//! exam often disagree. Whoever is filling the form can see the real numbers
//! and this app cannot, so the presets are plain JSON in a folder the person
//! owns: the bundled set is compiled in and always current, and anything in
//! `yours.json` overrides or adds to it by id.
//!
//! That split is what makes both halves work. Updating the app can improve the
//! bundled numbers without touching anybody's edits, and deleting an override
//! puts the bundled preset back — there is no state to repair and no merge to
//! get wrong.

use std::path::{Path, PathBuf};

use mt_core::preset::Preset;
use serde::{Deserialize, Serialize};

/// The file the app writes when you change a preset.
const YOURS: &str = "yours.json";

/// A copy of the bundled set, rewritten on every start, purely to read and
/// copy from. Never loaded back.
const REFERENCE: &str = "bundled-reference.json";

/// The app's own settings, which happen to live in this same folder.
///
/// None of these is a preset, and two of them are JSON objects rather than
/// lists, so the scan below read them, failed to parse them, and put two
/// permanent "couldn't be read" banners at the top of the Form presets
/// screen telling the person to go and fix files they should never touch.
/// `recipes.json` and `watched-folders.json` escaped only by being `[]`,
/// which parses as a preset list of none.
///
/// Saying so out loud when a file here cannot be read is deliberate — a
/// typo in a hand-edited preset is the fault most likely to look like "my
/// preset vanished". That only works if the app's own files are not in the
/// pile. `every_file_the_app_writes_here_is_named_above` keeps this list
/// true as the settings grow.
const OURS: [&str; 5] =
    [REFERENCE, "hotkey.json", "recipes.json", "retention.json", "watched-folders.json"];

const README: &str = "\
Form presets
============

Each preset is one entry in a JSON array. Edit `yours.json` to change a
bundled preset or add your own, then reopen Form presets in Kitbox.

  * A preset with the same `id` as a bundled one replaces it.
  * Delete an entry from `yours.json` and the bundled preset comes back.
  * `bundled-reference.json` is rewritten every time Kitbox starts. Copy
    entries out of it; editing it changes nothing.

Fields
------

  id          required, unique
  name        required, what you see on the card
  group       optional, the heading it sits under (default \"Other\")
  kind        \"image\" or \"pdf\"
  target      { \"kind\": \"at_most\", \"value\": 100000 }
              { \"kind\": \"range\", \"value\": { \"min\": 20000, \"max\": 50000 } }
              Sizes are in bytes. 100000 is 100 KB.
  format      \"jpeg\", \"png\", \"pdf\" or \"any\"
  dimensions  { \"width\": 200, \"height\": 230, \"fit\": \"exact\" }
              fit is \"exact\", \"at_least\" or \"at_most\"
  dpi         a number, when the form asks for a resolution
  note        anything the fields can't hold — where the numbers came from,
              what the form actually said

If a file here can't be read, Kitbox says so on the Form presets screen
rather than ignoring it quietly.
";

/// A preset plus where it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[serde(flatten)]
    pub preset: Preset,
    /// True when this came from `yours.json` rather than the bundled set.
    pub yours: bool,
    /// True when this replaces a bundled preset of the same id, so the screen
    /// can offer to put the original back.
    pub overrides_bundled: bool,
}

/// A file in the folder that could not be read, and why.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub file: String,
    pub why: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub presets: Vec<Entry>,
    pub folder: String,
    /// The README inside the folder. Revealing a file is how the app opens a
    /// folder without asking for permission to open arbitrary paths.
    pub readme: String,
    /// Said out loud rather than swallowed: a typo in a hand-edited file is
    /// the one fault most likely to look like "my preset vanished".
    pub problems: Vec<Problem>,
}

/// Make sure the folder exists and the reference copy is current.
pub fn prepare(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(REFERENCE), mt_core::preset::BUNDLED)?;
    std::fs::write(dir.join("README.txt"), README)?;
    Ok(())
}

/// Everything the screen should show: bundled, overridden and added.
pub fn load(dir: &Path) -> Loaded {
    let mut problems = Vec::new();
    let mut yours: Vec<Preset> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))
                    && p.file_name()
                        .is_some_and(|n| !OURS.iter().any(|ours| n.eq_ignore_ascii_case(ours)))
            })
            .collect();
        // `yours.json` first, then anything else alphabetically, so a run is
        // the same every time and the app's own file is the one that loses a
        // tie rather than the one the person made.
        files.sort();
        files.sort_by_key(|p| p.file_name().map(|n| n != YOURS));

        for file in files {
            let name = file_name(&file);
            match read_text(&file) {
                Ok(text) => match Preset::parse_all(&text) {
                    Ok(presets) => yours.extend(presets),
                    Err(e) => problems.push(Problem { file: name, why: e.to_string() }),
                },
                Err(e) => problems.push(Problem { file: name, why: e.to_string() }),
            }
        }
    }

    let bundled = Preset::bundled();
    let mut entries: Vec<Entry> = Vec::new();

    for preset in &yours {
        let overrides_bundled = bundled.iter().any(|b| b.id == preset.id);
        // A second entry with an id already taken is a mistake worth seeing
        // rather than a silent winner.
        if entries.iter().any(|e: &Entry| e.preset.id == preset.id) {
            problems.push(Problem {
                file: YOURS.into(),
                why: format!("'{}' is defined twice; the first one is being used", preset.id),
            });
            continue;
        }
        entries.push(Entry { preset: preset.clone(), yours: true, overrides_bundled });
    }

    for preset in bundled {
        if entries.iter().any(|e| e.preset.id == preset.id) {
            continue;
        }
        entries.push(Entry { preset, yours: false, overrides_bundled: false });
    }

    Loaded {
        presets: entries,
        folder: dir.display().to_string(),
        readme: dir.join("README.txt").display().to_string(),
        problems,
    }
}

/// Add or replace one preset in `yours.json`.
pub fn save(dir: &Path, preset: Preset) -> Result<(), String> {
    preset.check_itself().map_err(|e| e.to_string())?;
    let mut mine = read_yours(dir)?;
    match mine.iter().position(|p| p.id == preset.id) {
        Some(at) => mine[at] = preset,
        None => mine.push(preset),
    }
    write_yours(dir, &mine)
}

/// Remove one preset from `yours.json`.
///
/// For an override that means the bundled preset comes back, which is why
/// there is no separate "reset" — they are the same action.
pub fn forget(dir: &Path, id: &str) -> Result<(), String> {
    let mut mine = read_yours(dir)?;
    mine.retain(|p| p.id != id);
    write_yours(dir, &mine)
}

fn read_yours(dir: &Path) -> Result<Vec<Preset>, String> {
    let path = dir.join(YOURS);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = read_text(&path)?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Refusing to write over a file we can't parse: someone's edit is in
    // there, and overwriting it to save one preset would lose the lot.
    Preset::parse_all(&text)
        .map_err(|e| format!("{YOURS} has to be fixed before Kitbox can change it: {e}"))
}

fn write_yours(dir: &Path, presets: &[Preset]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(presets).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(YOURS), json).map_err(|e| e.to_string())
}

/// Read a file the way Windows actually writes one.
///
/// This folder is meant to be edited by hand, and the ordinary ways of saving
/// a file here do not produce plain UTF-8: Notepad and PowerShell's
/// `Out-File -Encoding utf8` both add a byte order mark, and "Save as →
/// Unicode" writes UTF-16. A JSON parser rejects all three on the first
/// character, so a preset someone typed correctly would come back as "this
/// isn't a preset file" — which is a true sentence about the bytes and a
/// useless one to read.
fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    match bytes.as_slice() {
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|e| e.to_string())
        }
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes),
        _ => String::from_utf8(bytes)
            .map_err(|e| format!("this file isn't text Kitbox can read ({e}); save it as UTF-8")),
    }
}

fn decode_utf16(bytes: &[u8], order: fn([u8; 2]) -> u16) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("this file looks like UTF-16 but ends mid-character".into());
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| order([pair[0], pair[1]])).collect();
    String::from_utf16(&units).map_err(|e| e.to_string())
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join("kitbox-preset-tests").join(stamp.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn one(id: &str, name: &str) -> String {
        format!(
            r#"[{{ "id": "{id}", "name": "{name}", "kind": "image",
                   "target": {{ "kind": "at_most", "value": 50000 }} }}]"#
        )
    }

    #[test]
    fn with_no_files_you_get_exactly_the_bundled_set() {
        let dir = temp();
        let loaded = load(&dir);
        assert_eq!(loaded.presets.len(), Preset::bundled().len());
        assert!(loaded.presets.iter().all(|e| !e.yours));
        assert!(loaded.problems.is_empty());
    }

    #[test]
    fn your_version_of_a_bundled_preset_wins_and_says_so() {
        let dir = temp();
        std::fs::write(dir.join(YOURS), one("ssc-photo", "SSC photo, my numbers")).unwrap();

        let loaded = load(&dir);
        let entry = loaded.presets.iter().find(|e| e.preset.id == "ssc-photo").unwrap();
        assert_eq!(entry.preset.name, "SSC photo, my numbers");
        assert!(entry.yours && entry.overrides_bundled);
        assert_eq!(
            loaded.presets.iter().filter(|e| e.preset.id == "ssc-photo").count(),
            1,
            "it replaces the bundled one rather than sitting beside it"
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_is_reported_not_ignored() {
        // The failure that would otherwise look like "my preset vanished".
        let dir = temp();
        std::fs::write(dir.join("mine.json"), "{ not json at all").unwrap();

        let loaded = load(&dir);
        assert_eq!(loaded.problems.len(), 1);
        assert_eq!(loaded.problems[0].file, "mine.json");
        assert!(!loaded.presets.is_empty(), "the bundled ones still load");
    }

    #[test]
    fn deleting_your_version_brings_the_bundled_one_back() {
        let dir = temp();
        std::fs::write(dir.join(YOURS), one("pan-photo", "Mine")).unwrap();
        assert!(load(&dir).presets.iter().any(|e| e.preset.name == "Mine"));

        forget(&dir, "pan-photo").unwrap();
        let entry = load(&dir).presets.into_iter().find(|e| e.preset.id == "pan-photo").unwrap();
        assert!(!entry.yours);
        assert_eq!(entry.preset.name, "PAN application photo");
    }

    #[test]
    fn saving_keeps_the_other_presets_in_the_file() {
        let dir = temp();
        let mut first = Preset::bundled()[0].clone();
        first.name = "First, edited".into();
        save(&dir, first).unwrap();

        let mut second = Preset::bundled()[1].clone();
        second.name = "Second, edited".into();
        save(&dir, second).unwrap();

        let text = std::fs::read_to_string(dir.join(YOURS)).unwrap();
        let saved = Preset::parse_all(&text).unwrap();
        assert_eq!(saved.len(), 2, "the first edit has to survive the second");
    }

    #[test]
    fn a_preset_that_makes_no_sense_is_refused_before_it_is_written() {
        let dir = temp();
        let mut broken = Preset::bundled()[0].clone();
        broken.name = "   ".into();
        assert!(save(&dir, broken).is_err());
        assert!(!dir.join(YOURS).exists(), "nothing should have been written");
    }

    #[test]
    fn an_unreadable_yours_file_is_never_overwritten() {
        // Someone's edits are in there. Losing them to save one preset would
        // be the worst thing this module could do.
        let dir = temp();
        std::fs::write(dir.join(YOURS), "[ this is broken").unwrap();

        let refused = save(&dir, Preset::bundled()[0].clone()).unwrap_err();
        assert!(refused.contains("fixed"), "{refused}");
        assert_eq!(std::fs::read_to_string(dir.join(YOURS)).unwrap(), "[ this is broken");
    }

    #[test]
    fn a_file_saved_the_way_windows_saves_one_still_loads() {
        // Notepad and PowerShell both write a byte order mark. Rejecting that
        // would mean a correctly typed preset comes back as "this isn't a
        // preset file", which is the least helpful true sentence available.
        let dir = temp();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(one("with-bom", "Typed in Notepad").as_bytes());
        std::fs::write(dir.join("mine.json"), bytes).unwrap();

        let loaded = load(&dir);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert!(loaded.presets.iter().any(|e| e.preset.id == "with-bom"));
    }

    #[test]
    fn a_file_saved_as_unicode_loads_too() {
        let dir = temp();
        let mut bytes = vec![0xFF, 0xFE];
        for unit in one("utf16", "Saved as Unicode").encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(dir.join("mine.json"), bytes).unwrap();

        let loaded = load(&dir);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert!(loaded.presets.iter().any(|e| e.preset.name == "Saved as Unicode"));
    }

    #[test]
    fn saving_over_a_file_with_a_mark_keeps_what_was_in_it() {
        // The mark must not make the file look broken, or the refusal to
        // overwrite would fire and nothing could be saved.
        let dir = temp();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(one("kept", "Kept").as_bytes());
        std::fs::write(dir.join(YOURS), bytes).unwrap();

        save(&dir, Preset::bundled()[0].clone()).unwrap();
        let saved = Preset::parse_all(&read_text(&dir.join(YOURS)).unwrap()).unwrap();
        assert_eq!(saved.len(), 2, "the hand-typed one has to survive");
    }

    #[test]
    fn the_reference_copy_is_not_loaded_as_presets() {
        // It holds the whole bundled set; loading it too would double every
        // preset and look like a bug in the merge.
        let dir = temp();
        prepare(&dir).unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.presets.len(), Preset::bundled().len());
        assert!(loaded.problems.is_empty());
        assert!(dir.join("README.txt").exists(), "the folder explains itself");
    }

    #[test]
    fn the_apps_own_settings_are_not_read_as_presets() {
        // Each of these is what the matching module actually writes. Two are
        // objects, so before `OURS` existed they produced a "couldn't be
        // read" banner on the Form presets screen on every machine.
        let dir = temp();
        prepare(&dir).unwrap();
        std::fs::write(dir.join("hotkey.json"), r#"{"enabled":true,"key":"K"}"#).unwrap();
        std::fs::write(dir.join("retention.json"), r#"{"keepDays":30}"#).unwrap();
        std::fs::write(dir.join("recipes.json"), "[]").unwrap();
        std::fs::write(dir.join("watched-folders.json"), "[]").unwrap();
        std::fs::write(dir.join(YOURS), one("mine", "Mine")).unwrap();

        let loaded = load(&dir);
        assert!(
            loaded.problems.is_empty(),
            "the app's own settings are not the person's presets: {:?}",
            loaded.problems
        );
        assert!(loaded.presets.iter().any(|e| e.preset.id == "mine"));
    }

    #[test]
    fn a_broken_preset_is_still_reported() {
        // The point of the filter is to stop crying wolf, not to stop crying.
        let dir = temp();
        prepare(&dir).unwrap();
        std::fs::write(dir.join("hotkey.json"), r#"{"enabled":true}"#).unwrap();
        std::fs::write(dir.join(YOURS), "{ not a list }").unwrap();

        let loaded = load(&dir);
        assert_eq!(loaded.problems.len(), 1, "{:?}", loaded.problems);
        assert_eq!(loaded.problems[0].file, YOURS);
    }

    /// `OURS` is a hand-written list, and the way it goes stale is that a new
    /// setting gets its own file in this folder and nobody thinks of the
    /// preset screen. So read the sibling modules and check.
    #[test]
    fn every_file_the_app_writes_here_is_named_above() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        for module in ["hotkey.rs", "retention.rs", "watch.rs", "lib.rs"] {
            let text = std::fs::read_to_string(src.join(module)).expect("own source");
            // `dir.join("something.json")` — how each module names its file in
            // the preset folder.
            for piece in text.split("dir.join(\"").skip(1) {
                let Some(name) = piece.split('"').next() else { continue };
                if name.ends_with(".json") {
                    found.push((module, name.to_string()));
                }
            }
        }
        assert!(
            !found.is_empty(),
            "found no `dir.join(\"….json\")` at all — the parser is wrong, not the code"
        );
        for (module, name) in found {
            assert!(
                OURS.contains(&name.as_str()),
                "{module} writes {name} into the preset folder but OURS does not list it, \
                 so the Form presets screen will report it as a broken preset"
            );
        }
    }
}
