//! Doing the same thing to a pile of files.
//!
//! The single-file tools are the hard part and they already exist; this is the
//! loop around them. What makes it worth having as its own module rather than
//! a `for` in the shell is what happens when one file misbehaves.
//!
//! # One bad file does not stop the run
//!
//! Drop forty files in and one of them is a renamed `.docx`, or a JPEG that
//! was truncated mid-download. The obvious loop stops dead on it and you lose
//! the thirty-nine that would have worked. Every item here is caught
//! individually: the run continues, and the failure is reported against the
//! file it belongs to, by name, so it can be dealt with on its own.
//!
//! # And a target it cannot hit is not a failure
//!
//! Asking for 50 KB and getting a file that will not go below 180 KB is an
//! answer, not an error. The solver already distinguishes the two, so the
//! outcome carries that through: the file is written at its best, and it is
//! listed separately from the ones that worked and the ones that broke.

use serde::{Deserialize, Serialize};

use crate::engine::image::{self, CompressOptions, OutputFormat};
use crate::engine::pdf::{self, PdfCompressOptions};
use crate::format::FileKind;
use crate::job::{Cancel, JobId, Progress, ProgressSink};
use crate::size::format_size;
use crate::{Bias, Outcome, SizeTarget};

/// What a batch can do.
///
/// Deliberately only the operations that need nothing but the file itself. A
/// caption needs a font and a cutout needs a model, and threading those
/// through here would buy little — they are one-at-a-time tools in practice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BatchOp {
    /// The headline: every file brought inside a size range.
    Compress {
        target: SizeTarget,
        bias: Bias,
        format: OutputFormat,
        strip_metadata: bool,
        /// Force exact output dimensions, the way a form demands them.
        exact: Option<(u32, u32)>,
    },
    /// Cap the longest edge, keeping the proportions.
    Resize { longest: u32 },
    /// Change format without chasing a size.
    Convert { format: OutputFormat, strip_metadata: bool },
    /// Tone and colour.
    Adjust(image::AdjustOptions),
}

impl BatchOp {
    /// What to call this run in a sentence.
    pub fn describe(&self) -> String {
        match self {
            BatchOp::Compress { target, .. } => {
                format!("compress to {}", describe_target(target))
            }
            BatchOp::Resize { longest } => format!("resize to {longest}px"),
            BatchOp::Convert { format, .. } => format!("convert to {format:?}").to_lowercase(),
            BatchOp::Adjust(_) => "adjust".into(),
        }
    }

    /// Whether this operation knows what to do with a PDF.
    pub fn handles_pdfs(&self) -> bool {
        matches!(self, BatchOp::Compress { .. })
    }
}

fn describe_target(target: &SizeTarget) -> String {
    match target {
        SizeTarget::AtMost(bytes) => format!("under {}", format_size(*bytes)),
        SizeTarget::Range { min, max } => {
            format!("{}–{}", format_size(*min), format_size(*max))
        }
        SizeTarget::Approx { bytes, .. } => format!("about {}", format_size(*bytes)),
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// How one file turned out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Produced {
    pub name: String,
    pub before: u64,
    pub after: u64,
    /// What the file became, as an extension.
    pub extension: String,
    /// In the user's words — "98.1% identical, 4 attempts", or why not.
    pub note: String,
    /// The target was asked for and could not be reached. The file is still
    /// here, at its best.
    pub short: bool,
}

/// What became of one file.
///
/// Tagged on `state` rather than by variant name: the screen reads this as a
/// union discriminated by a field it can switch on, which is one fewer thing
/// to get wrong than a wrapper key whose spelling depends on a serde
/// attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ItemOutcome {
    /// It worked.
    Done(Produced),
    /// It could not be done, and why — against this file, not the run.
    Failed { name: String, why: String },
    /// The run was stopped before reaching this one.
    Skipped { name: String },
}

impl ItemOutcome {
    pub fn name(&self) -> &str {
        match self {
            ItemOutcome::Done(p) => &p.name,
            ItemOutcome::Failed { name, .. } | ItemOutcome::Skipped { name } => name,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, ItemOutcome::Done(_))
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, ItemOutcome::Failed { .. })
    }

    /// Produced, but outside the size that was asked for.
    pub fn fell_short(&self) -> bool {
        matches!(self, ItemOutcome::Done(p) if p.short)
    }
}

/// What came back, and the bytes to write.
pub struct BatchResult {
    pub outcomes: Vec<ItemOutcome>,
    /// The produced files, in the same order as the `Done` outcomes.
    pub files: Vec<(String, Vec<u8>)>,
}

impl BatchResult {
    pub fn done(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_done()).count()
    }
    pub fn failed(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_failure()).count()
    }
    pub fn short(&self) -> usize {
        self.outcomes.iter().filter(|o| o.fell_short()).count()
    }
}

/// Run `op` over every item.
///
/// Progress is reported per file before it starts, so a long one doesn't look
/// like a hang. Cancellation is checked between files: a single compress can
/// take a few seconds and stopping mid-encode would leave a half file.
pub fn run(
    items: &[Item],
    op: &BatchOp,
    sink: &dyn ProgressSink,
    cancel: &Cancel,
) -> BatchResult {
    let mut outcomes = Vec::with_capacity(items.len());
    let mut files = Vec::new();

    for (index, item) in items.iter().enumerate() {
        if cancel.is_cancelled() {
            outcomes.push(ItemOutcome::Skipped { name: item.name.clone() });
            continue;
        }

        sink.report(Progress {
            job: JobId(index as u64),
            stage: format!("{} ({} of {})", item.name, index + 1, items.len()),
            fraction: Some(index as f32 / items.len().max(1) as f32),
        });

        match one(item, op) {
            Ok((produced, bytes)) => {
                files.push((produced.name.clone(), bytes));
                outcomes.push(ItemOutcome::Done(produced));
            }
            Err(why) => {
                // Caught here rather than returned: the other thirty-nine
                // files are still worth doing.
                outcomes.push(ItemOutcome::Failed { name: item.name.clone(), why });
            }
        }
    }

    sink.report(Progress {
        job: JobId(items.len() as u64),
        stage: "finished".into(),
        fraction: Some(1.0),
    });

    BatchResult { outcomes, files }
}

/// One file, start to finish.
fn one(item: &Item, op: &BatchOp) -> Result<(Produced, Vec<u8>), String> {
    let kind = FileKind::sniff(&item.bytes);
    let before = item.bytes.len() as u64;

    if kind == FileKind::Pdf && !op.handles_pdfs() {
        return Err(format!("{} is a PDF, and this only works on pictures", item.name));
    }
    if kind != FileKind::Pdf && !kind.is_image() {
        // Worth spelling out rather than saying "unsupported": the usual cause
        // is a file whose name promises one thing and whose bytes say another,
        // and knowing that is what tells you to go and look at it.
        return Err(match kind {
            FileKind::Unknown => format!(
                "{} isn't a picture or a PDF — whatever its name says, the bytes aren't either",
                item.name
            ),
            other => format!("{} is {} file, which this can't open", item.name, other.article()),
        });
    }

    match op {
        BatchOp::Compress { target, bias, format, strip_metadata, exact } => {
            let outcome = if kind == FileKind::Pdf {
                pdf::compress_to_target(
                    &item.bytes,
                    *target,
                    *bias,
                    PdfCompressOptions { strip_metadata: *strip_metadata, ..Default::default() },
                )
            } else {
                image::compress_to_target(
                    &item.bytes,
                    *target,
                    *bias,
                    CompressOptions {
                        format: *format,
                        strip_metadata: *strip_metadata,
                        max_dimension: None,
                        exact_dimensions: *exact,
                    },
                )
            }
            .map_err(|e| e.to_string())?;

            let (bytes, note, short) = unwrap_outcome(outcome);
            let extension = FileKind::sniff(&bytes).extension().to_string();
            Ok((
                Produced {
                    name: rename(&item.name, &extension),
                    before,
                    after: bytes.len() as u64,
                    extension,
                    note,
                    short,
                },
                bytes,
            ))
        }

        BatchOp::Resize { longest } => {
            // Asking for 800px is not asking for a different format. The
            // encoder's "keep" turns an opaque PNG into a JPEG, which is the
            // right trade when you are chasing a size and the wrong one here:
            // a screenshot would come back with soft text nobody asked to pay
            // for. Compress is the only op allowed to change format.
            let format =
                if kind == FileKind::Png { OutputFormat::Png } else { OutputFormat::Keep };
            let bytes = image::resize(
                &item.bytes,
                image::ResizeMode::LongestEdge(*longest),
                &image::TransformOptions { format, ..Default::default() },
            )
            .map_err(|e| e.to_string())?;
            let extension = FileKind::sniff(&bytes).extension().to_string();
            let (w, h) = ::image::load_from_memory(&bytes)
                .map(|i| (i.width(), i.height()))
                .unwrap_or((0, 0));
            Ok((
                Produced {
                    name: rename(&item.name, &extension),
                    before,
                    after: bytes.len() as u64,
                    extension,
                    note: format!("{w}×{h}"),
                    short: false,
                },
                bytes,
            ))
        }

        BatchOp::Convert { format, strip_metadata } => {
            let bytes = image::convert(
                &item.bytes,
                &image::TransformOptions {
                    format: *format,
                    strip_metadata: *strip_metadata,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            let extension = FileKind::sniff(&bytes).extension().to_string();
            Ok((
                Produced {
                    name: rename(&item.name, &extension),
                    before,
                    after: bytes.len() as u64,
                    extension: extension.clone(),
                    note: format!("now {extension}"),
                    short: false,
                },
                bytes,
            ))
        }

        BatchOp::Adjust(options) => {
            let (adjusted, report) =
                image::adjust(&item.bytes, options).map_err(|e| e.to_string())?;
            let out_kind = if kind == FileKind::Png { FileKind::Png } else { FileKind::Jpeg };
            let bytes = image::encode::encode(&adjusted, out_kind, 92, true)
                .map_err(|e| e.to_string())?;
            Ok((
                Produced {
                    name: rename(&item.name, out_kind.extension()),
                    before,
                    after: bytes.len() as u64,
                    extension: out_kind.extension().to_string(),
                    note: report
                        .warnings
                        .first()
                        .cloned()
                        .unwrap_or_else(|| format!("{}×{}", report.width, report.height)),
                    short: false,
                },
                bytes,
            ))
        }
    }
}

/// Flatten a solver outcome into bytes, a sentence, and whether it fell short.
fn unwrap_outcome(outcome: Outcome) -> (Vec<u8>, String, bool) {
    match outcome {
        Outcome::Hit(solved) | Outcome::Padded(solved) => {
            let quality = solved
                .score
                .map(|v| format!("{:.1}% identical, ", v * 100.0))
                .unwrap_or_default();
            let extra = solved.note.map(|n| format!(" — {n}")).unwrap_or_default();
            (
                solved.bytes,
                format!("{quality}{} attempts{extra}", solved.probes),
                false,
            )
        }
        // Not an error: an answer. The file is written at its best and listed
        // apart from the ones that landed.
        Outcome::Infeasible { best, why, .. } => (best, why, true),
    }
}

/// `photo.heic` becomes `photo.jpg` when the format changed.
fn rename(name: &str, extension: &str) -> String {
    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name);
    format!("{stem}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::image::OutputFormat;
    use crate::job::Silent;
    use crate::testkit;
    use std::sync::{Arc, Mutex};

    fn photo(name: &str) -> Item {
        Item { name: name.into(), bytes: testkit::encode_jpeg(&testkit::photo(400, 300), 92) }
    }

    fn rubbish(name: &str) -> Item {
        Item { name: name.into(), bytes: b"this is not a picture at all".to_vec() }
    }

    fn compress(max: u64) -> BatchOp {
        BatchOp::Compress {
            target: SizeTarget::AtMost(max),
            bias: Bias::Balanced,
            format: OutputFormat::Keep,
            strip_metadata: true,
            exact: None,
        }
    }

    #[test]
    fn one_bad_file_does_not_cost_you_the_others() {
        // The reason this module exists rather than a loop in the shell: a
        // renamed .docx in the middle of forty holiday photos should cost you
        // that one file, not the run.
        let items = vec![photo("a.jpg"), rubbish("broken.jpg"), photo("c.jpg")];
        let result = run(&items, &compress(60_000), &Silent, &Cancel::new());

        assert_eq!(result.outcomes.len(), 3);
        assert_eq!(result.done(), 2, "{:?}", result.outcomes);
        assert_eq!(result.failed(), 1);
        assert_eq!(result.files.len(), 2, "only the good ones produce files");

        let failure = result
            .outcomes
            .iter()
            .find(|o| matches!(o, ItemOutcome::Failed { .. }))
            .unwrap();
        assert_eq!(failure.name(), "broken.jpg", "named against its own file");
    }

    #[test]
    fn a_target_that_cannot_be_reached_is_reported_apart_from_a_breakage() {
        // Asking for the impossible is an answer, not an error. The file is
        // still produced, at its best.
        //
        // Impossible has to be constructed with care: left free, the solver
        // will shrink the pixels until almost any size is reachable. Pinning
        // the dimensions is what makes it genuinely unreachable — and is
        // exactly the real case, a form demanding 200×230 *and* a size the
        // picture cannot make at those dimensions.
        let items = vec![photo("stubborn.jpg")];
        let impossible = BatchOp::Compress {
            target: SizeTarget::AtMost(300),
            bias: Bias::Balanced,
            format: OutputFormat::Keep,
            strip_metadata: true,
            exact: Some((400, 300)),
        };
        let result = run(&items, &impossible, &Silent, &Cancel::new());

        assert_eq!(result.failed(), 0, "nothing broke");
        assert_eq!(result.done(), 1, "a file still came out");
        assert_eq!(result.short(), 1, "but it fell short, and says so");
        assert_eq!(result.files.len(), 1);

        match &result.outcomes[0] {
            ItemOutcome::Done(p) => {
                assert!(p.short);
                assert!(!p.note.is_empty(), "it should say why");
            }
            other => panic!("expected a file, got {other:?}"),
        }
    }

    #[test]
    fn everything_that_fits_actually_fits() {
        let items = vec![photo("a.jpg"), photo("b.jpg")];
        let result = run(&items, &compress(40_000), &Silent, &Cancel::new());
        for outcome in &result.outcomes {
            match outcome {
                ItemOutcome::Done(p) if !p.short => {
                    assert!(p.after <= 40_000, "{} came out at {}", p.name, p.after);
                    assert!(p.before > 0);
                }
                ItemOutcome::Done(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn cancelling_stops_the_run_and_says_what_was_left() {
        let items = vec![photo("a.jpg"), photo("b.jpg"), photo("c.jpg")];
        let cancel = Cancel::new();
        cancel.cancel();

        let result = run(&items, &compress(50_000), &Silent, &cancel);
        assert_eq!(result.done(), 0);
        assert_eq!(result.outcomes.len(), 3, "every file is still accounted for");
        assert!(
            result.outcomes.iter().all(|o| matches!(o, ItemOutcome::Skipped { .. })),
            "{:?}",
            result.outcomes
        );
    }

    #[test]
    fn progress_names_each_file_as_it_starts() {
        // A long file shouldn't look like a hang, which means reporting
        // before the work rather than after it.
        let seen = Arc::new(Mutex::new(Vec::new()));
        let collector = {
            let seen = Arc::clone(&seen);
            move |p: Progress| seen.lock().unwrap().push(p.stage)
        };

        let items = vec![photo("first.jpg"), photo("second.jpg")];
        run(&items, &compress(50_000), &collector, &Cancel::new());

        let stages = seen.lock().unwrap().clone();
        assert!(stages.iter().any(|s| s.contains("first.jpg")), "{stages:?}");
        assert!(stages.iter().any(|s| s.contains("second.jpg")), "{stages:?}");
        assert!(stages.iter().any(|s| s.contains("1 of 2")), "{stages:?}");
        assert_eq!(stages.last().map(String::as_str), Some("finished"));
    }

    #[test]
    fn a_pdf_goes_through_compression_and_is_turned_away_by_the_rest() {
        let pdf = Item { name: "scan.pdf".into(), bytes: testkit::pdf_scan(1, 300, 400, 85) };

        let compressed = run(std::slice::from_ref(&pdf), &compress(80_000), &Silent, &Cancel::new());
        assert_eq!(compressed.failed(), 0, "{:?}", compressed.outcomes);

        let resized = run(
            std::slice::from_ref(&pdf),
            &BatchOp::Resize { longest: 800 },
            &Silent,
            &Cancel::new(),
        );
        assert_eq!(resized.failed(), 1);
        match &resized.outcomes[0] {
            ItemOutcome::Failed { why, .. } => {
                assert!(why.contains("PDF"), "it should say why: {why}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn converting_renames_the_file_to_match() {
        let items = vec![photo("holiday.jpg")];
        let op = BatchOp::Convert { format: OutputFormat::Png, strip_metadata: true };
        let result = run(&items, &op, &Silent, &Cancel::new());

        match &result.outcomes[0] {
            ItemOutcome::Done(p) => {
                assert_eq!(p.name, "holiday.png", "the extension has to follow the format");
                assert_eq!(p.extension, "png");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(result.files[0].0, "holiday.png");
    }

    #[test]
    fn resizing_caps_the_longest_edge() {
        let items = vec![photo("big.jpg")];
        let result = run(&items, &BatchOp::Resize { longest: 120 }, &Silent, &Cancel::new());
        match &result.outcomes[0] {
            ItemOutcome::Done(p) => assert!(p.note.starts_with("120×"), "{}", p.note),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn adjusting_a_whole_folder_works_the_same_as_one() {
        let items = vec![photo("a.jpg"), photo("b.jpg")];
        let op = BatchOp::Adjust(crate::engine::image::AdjustOptions {
            auto_levels: true,
            ..Default::default()
        });
        let result = run(&items, &op, &Silent, &Cancel::new());
        assert_eq!(result.done(), 2, "{:?}", result.outcomes);
    }

    #[test]
    fn resizing_a_png_leaves_it_a_png() {
        // The encoder's "keep" means JPEG for an opaque PNG, which is right
        // when chasing a size and wrong when the only request was pixels.
        let png = {
            let mut bytes = Vec::new();
            testkit::photo(400, 300)
                .write_to(&mut std::io::Cursor::new(&mut bytes), ::image::ImageFormat::Png)
                .unwrap();
            bytes
        };
        let items = vec![Item { name: "screenshot.png".into(), bytes: png }];
        let result = run(&items, &BatchOp::Resize { longest: 120 }, &Silent, &Cancel::new());

        match &result.outcomes[0] {
            ItemOutcome::Done(p) => assert_eq!(p.name, "screenshot.png", "the format has to survive"),
            other => panic!("{other:?}"),
        }
        assert_eq!(FileKind::sniff(&result.files[0].1), FileKind::Png);
    }

    #[test]
    fn an_empty_run_is_not_an_error() {
        let result = run(&[], &compress(50_000), &Silent, &Cancel::new());
        assert!(result.outcomes.is_empty());
        assert!(result.files.is_empty());
    }

    #[test]
    fn each_operation_can_say_what_it_is() {
        assert!(compress(100_000).describe().contains("under"));
        assert!(BatchOp::Resize { longest: 800 }.describe().contains("800"));
        assert!(BatchOp::Compress {
            target: SizeTarget::Range { min: 50_000, max: 100_000 },
            bias: Bias::Balanced,
            format: OutputFormat::Keep,
            strip_metadata: true,
            exact: None,
        }
        .describe()
        .contains('–'));
    }

    #[test]
    fn an_outcome_crosses_as_the_screen_reads_it() {
        // This is what a blank window looked like once: the screen expected a
        // wrapper key, serde sent a tag, and rendering the first result threw.
        let outcomes = vec![
            ItemOutcome::Done(Produced {
                name: "a.jpg".into(),
                before: 400_000,
                after: 90_000,
                extension: "jpg".into(),
                note: "97% identical".into(),
                short: false,
            }),
            ItemOutcome::Failed { name: "b.jpg".into(), why: "not a picture".into() },
            ItemOutcome::Skipped { name: "c.jpg".into() },
        ];
        let json: serde_json::Value = serde_json::to_value(&outcomes).unwrap();
        assert_eq!(json[0]["state"], "done");
        assert_eq!(json[0]["name"], "a.jpg", "the fields sit beside the tag, not under a key");
        assert_eq!(json[0]["short"], false);
        assert_eq!(json[1]["state"], "failed");
        assert_eq!(json[1]["why"], "not a picture");
        assert_eq!(json[2]["state"], "skipped");
        assert_eq!(json[2]["name"], "c.jpg");
    }

    #[test]
    fn the_operation_crosses_as_the_ui_writes_it() {
        let op: BatchOp = serde_json::from_str(
            r#"{ "kind": "compress",
                  "target": { "kind": "range", "value": { "min": 50000, "max": 100000 } },
                  "bias": "balanced", "format": "keep",
                  "stripMetadata": true, "exact": null }"#,
        )
        .expect("a compress op");
        match op {
            BatchOp::Compress { target, strip_metadata, .. } => {
                assert_eq!(target.max_bytes(), 100_000);
                assert!(strip_metadata, "the switch has to arrive");
            }
            other => panic!("{other:?}"),
        }

        let resize: BatchOp =
            serde_json::from_str(r#"{ "kind": "resize", "longest": 1200 }"#).expect("a resize");
        assert!(matches!(resize, BatchOp::Resize { longest: 1200 }));
    }
}
