//! Letting history forget by itself.
//!
//! A gallery that keeps everything forever is a gallery nobody trusts with
//! their documents. The manual tools are already there — forget one entry,
//! tidy away what's broken — and this is the one that runs without being
//! asked, which is exactly why it needs to be careful.
//!
//! Four rules make it safe to leave on:
//!
//! * **Off until you turn it on.** Deleting someone's files on a schedule is
//!   not a default.
//! * **Pinned is pinned.** That is what pinning is for, and it is the only
//!   thing in the app that overrides everything else.
//! * **Nothing recent.** The age is the age of the stored copy, and the
//!   shortest setting offered is a month.
//! * **You can see what it would take before you let it.** The screen shows
//!   the count and the bytes of what would go *now*, so turning it on is a
//!   decision rather than a hope.
//!
//! The default scope is the one worth explaining: an **intermediate** is a
//! stored copy that something else was made from — the phone photo behind the
//! compressed one, the scan behind the signed PDF. The end of a chain is what
//! you actually wanted, so it stays.

use serde::{Deserialize, Serialize};

use mt_store::{FileRecord, Store};

/// What gets dropped when the rule runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Copies something else was made from. The finished files stay.
    #[default]
    Intermediates,
    /// Everything old enough, finished files included.
    Everything,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub enabled: bool,
    /// How old a stored copy has to be. Never less than 30.
    pub days: u32,
    pub scope: Scope,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { enabled: false, days: 90, scope: Scope::Intermediates }
    }
}

impl Policy {
    /// The shortest age this will honour, whatever is in the file.
    ///
    /// A hand-edited `0` would mean "delete everything the moment the app
    /// starts", which is not a rule anybody wants and is easy to type.
    pub fn days(&self) -> u32 {
        self.days.max(30)
    }
}

/// One stored copy, and whether the rule would take it.
pub struct Candidate<'a> {
    pub file: &'a FileRecord,
    pub why_kept: Option<&'static str>,
}

/// Decide about one file. Pure, so the decision can be tested without a disk.
pub fn judge(
    file: &FileRecord,
    is_intermediate: bool,
    policy: &Policy,
    now_ms: i64,
) -> Option<&'static str> {
    if file.pinned {
        return Some("pinned");
    }
    let age_days = (now_ms - file.created_at).max(0) / 86_400_000;
    if age_days < policy.days() as i64 {
        return Some("not old enough");
    }
    if policy.scope == Scope::Intermediates && !is_intermediate {
        return Some("the end of its chain");
    }
    None
}

/// What the rule would take right now.
pub fn plan<'a>(store: &Store, files: &'a [FileRecord], policy: &Policy, now_ms: i64) -> Vec<Candidate<'a>> {
    files
        .iter()
        .map(|file| {
            // Something was made from it, so it is a step rather than a result.
            let is_intermediate = store
                .derivatives(&file.hash)
                .map(|jobs| !jobs.is_empty())
                .unwrap_or(false);
            Candidate { file, why_kept: judge(file, is_intermediate, policy, now_ms) }
        })
        .collect()
}

/// What a run did, or would do.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Swept {
    pub forgotten: u32,
    pub freed: u64,
    /// Kept because they are pinned, so the screen can say so.
    pub pinned: u32,
}

/// Run the rule. Returns what it took.
pub fn run(store: &Store, policy: &Policy, now_ms: i64) -> Result<Swept, String> {
    if !policy.enabled {
        return Ok(Swept::default());
    }
    let files = store.all_files().map_err(|e| e.to_string())?;
    let mut swept = Swept::default();

    for candidate in plan(store, &files, policy, now_ms) {
        match candidate.why_kept {
            Some("pinned") => swept.pinned += 1,
            Some(_) => {}
            None => {
                store.forget(&candidate.file.hash).map_err(|e| e.to_string())?;
                swept.forgotten += 1;
                swept.freed += candidate.file.size;
            }
        }
    }
    Ok(swept)
}

/// What it would take, without taking it.
pub fn preview(store: &Store, policy: &Policy, now_ms: i64) -> Result<Swept, String> {
    // Deliberately ignores `enabled`: the point is to show what turning it on
    // would cost before it is on.
    let files = store.all_files().map_err(|e| e.to_string())?;
    let mut swept = Swept::default();
    for candidate in plan(store, &files, policy, now_ms) {
        match candidate.why_kept {
            Some("pinned") => swept.pinned += 1,
            Some(_) => {}
            None => {
                swept.forgotten += 1;
                swept.freed += candidate.file.size;
            }
        }
    }
    Ok(swept)
}

pub fn path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("retention.json")
}

pub fn load(dir: &std::path::Path) -> Policy {
    std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(dir: &std::path::Path, policy: &Policy) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(policy).map_err(|e| e.to_string())?;
    std::fs::write(path(dir), json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    fn file(age_days: i64, pinned: bool) -> FileRecord {
        FileRecord {
            hash: format!("hash-{age_days}-{pinned}"),
            size: 1_000_000,
            name: "scan.jpg".into(),
            kind: "jpeg".into(),
            width: None,
            height: None,
            pages: None,
            source_path: None,
            thumb_hash: None,
            created_at: 1_000 * DAY - age_days * DAY,
            pinned,
        }
    }

    fn now() -> i64 {
        1_000 * DAY
    }

    #[test]
    fn pinned_beats_everything() {
        // The one thing in the app that overrides every other rule.
        let policy = Policy { enabled: true, days: 30, scope: Scope::Everything };
        assert_eq!(judge(&file(3650, true), true, &policy, now()), Some("pinned"));
    }

    #[test]
    fn nothing_recent_is_touched() {
        let policy = Policy { enabled: true, days: 90, scope: Scope::Everything };
        assert_eq!(judge(&file(89, false), true, &policy, now()), Some("not old enough"));
        assert_eq!(judge(&file(91, false), true, &policy, now()), None);
    }

    #[test]
    fn a_hand_edited_zero_cannot_mean_delete_everything_now() {
        // The settings are a file people can edit. "0 days" is one keystroke
        // away from "wipe it all on the next start".
        let policy = Policy { enabled: true, days: 0, scope: Scope::Everything };
        assert_eq!(policy.days(), 30);
        assert_eq!(judge(&file(10, false), true, &policy, now()), Some("not old enough"));
    }

    #[test]
    fn by_default_the_end_of_a_chain_survives() {
        // The compressed copy you were making is the point; the phone photo
        // behind it is the step.
        let policy = Policy { enabled: true, days: 90, scope: Scope::Intermediates };
        let old = file(200, false);
        assert_eq!(judge(&old, true, &policy, now()), None, "a step goes");
        assert_eq!(
            judge(&old, false, &policy, now()),
            Some("the end of its chain"),
            "a result stays"
        );
    }

    #[test]
    fn everything_means_everything_old() {
        let policy = Policy { enabled: true, days: 90, scope: Scope::Everything };
        assert_eq!(judge(&file(200, false), false, &policy, now()), None);
    }

    #[test]
    fn a_rule_that_is_off_takes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let policy = Policy { enabled: false, days: 30, scope: Scope::Everything };
        assert_eq!(run(&store, &policy, now()).unwrap().forgotten, 0);
    }

    #[test]
    fn the_steps_go_and_the_result_stays() {
        // The whole promise of the default scope, over a real chain: a scan
        // was compressed, so the scan is a step and the compressed copy is
        // what you were making.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let scan = store
            .put(
                b"the original scan, large",
                mt_store::FileMeta { name: "scan.jpg".into(), kind: "jpeg".into(), ..Default::default() },
            )
            .unwrap();
        let small = store
            .put(
                b"the compressed copy",
                mt_store::FileMeta { name: "scan-kitbox.jpg".into(), kind: "jpeg".into(), ..Default::default() },
            )
            .unwrap();
        store
            .record_job(mt_store::NewJob {
                tool: "compress".into(),
                recipe: "{}".into(),
                input_hash: scan.hash.clone(),
                output_hash: small.hash.clone(),
                summary: "squeezed".into(),
                duration_ms: 10,
            })
            .unwrap();

        for hash in [&scan.hash, &small.hash] {
            store.set_created_at(hash, now() - 200 * DAY).unwrap();
        }

        let policy = Policy { enabled: true, days: 90, scope: Scope::Intermediates };
        let swept = run(&store, &policy, now()).unwrap();

        assert_eq!(swept.forgotten, 1, "only the step should go");
        assert!(store.get(&scan.hash).unwrap().is_none(), "the scan was the step");
        assert!(
            store.get(&small.hash).unwrap().is_some(),
            "the compressed copy is what the person wanted"
        );
    }

    #[test]
    fn pinning_a_step_keeps_it_through_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let kept = store
            .put(
                b"an old file somebody cares about",
                mt_store::FileMeta { name: "keep.jpg".into(), kind: "jpeg".into(), ..Default::default() },
            )
            .unwrap();
        store.set_created_at(&kept.hash, now() - 900 * DAY).unwrap();
        store.set_pinned(&kept.hash, true).unwrap();

        let policy = Policy { enabled: true, days: 30, scope: Scope::Everything };
        let swept = run(&store, &policy, now()).unwrap();

        assert_eq!(swept.forgotten, 0);
        assert_eq!(swept.pinned, 1, "and it should say why it was kept");
        assert!(store.get(&kept.hash).unwrap().is_some());
    }

    #[test]
    fn the_preview_shows_what_turning_it_on_would_cost() {
        // It has to answer for a policy that isn't enabled yet — that is the
        // moment someone needs to know.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let old = store
            .put(
                b"an old scan",
                mt_store::FileMeta { name: "old.jpg".into(), kind: "jpeg".into(), ..Default::default() },
            )
            .unwrap();
        store.set_created_at(&old.hash, now() - 200 * DAY).expect("backdate it");

        let policy = Policy { enabled: false, days: 90, scope: Scope::Everything };
        let seen = preview(&store, &policy, now()).unwrap();
        assert_eq!(seen.forgotten, 1, "it should count the old file");
        assert!(seen.freed > 0);

        // And nothing was actually taken.
        assert!(store.get(&old.hash).unwrap().is_some());
    }
}
