//! Running a past job again, on a different file.
//!
//! Every job already records the settings that produced it. This reads one of
//! those recordings back and applies it, so "the same thing I did to last
//! month's form" is a file-picker away rather than a hunt through settings you
//! half remember.
//!
//! It lives in the desktop crate rather than the engine because a recipe is a
//! recording of what this shell was asked to do — several of the shapes are
//! this crate's own request types — and because replaying needs the things the
//! engine deliberately doesn't have: somewhere to write, a font, a model.
//!
//! Two rules it keeps:
//!
//! * **A recipe that cannot be replayed says why, in the same words every
//!   time.** Some were never recorded in full on purpose: what a redaction
//!   removed, what was typed into a form, where a signature went. Those are
//!   private, that was deliberate, and the screen says so rather than offering
//!   a button that fails.
//! * **The replay is the recorded settings, not a fresh guess.** If a recipe
//!   is missing something the tool now takes, the default is used and the
//!   summary says so — nothing is silently invented.

use mt_core::engine::image as img;
use mt_core::engine::pdf;
use mt_core::FileKind;
use serde::{Deserialize, Serialize};

use crate::TransformOp;

/// A recorded job that can be run again on a new file.
#[derive(Debug, Deserialize)]
#[serde(tag = "tool", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Recipe {
    Compress {
        target: mt_core::SizeTarget,
        /// Older rows, and everything the command line records, leave this
        /// out. A recipe missing a setting the tool has since grown should
        /// replay with the default, not refuse.
        #[serde(default)]
        bias: mt_core::Bias,
    },
    Transform {
        op: TransformOp,
        /// Older rows didn't record this; the defaults are what the screen
        /// used at the time anyway.
        #[serde(default)]
        options: Option<img::TransformOptions>,
    },
    Adjust {
        options: img::AdjustOptions,
    },
    Pages {
        ops: Vec<pdf::PageOp>,
    },
    Split {
        mode: pdf::SplitMode,
    },
    #[serde(rename = "pdf-to-images")]
    PdfToImages {
        #[serde(default)]
        max_px: Option<u32>,
        quality: u8,
    },
    #[serde(rename = "crop-pdf")]
    CropPdf {
        #[serde(default)]
        pages: Option<String>,
        spec: pdf::CropSpec,
    },
    #[serde(rename = "page-numbers")]
    PageNumbers {
        #[serde(default)]
        pages: Option<String>,
        options: pdf::PageNumberOptions,
    },
    Watermark {
        #[serde(default)]
        pages: Option<String>,
        options: pdf::WatermarkOptions,
    },
    #[serde(rename = "watermark-image")]
    WatermarkImage {
        options: img::ImageWatermarkOptions,
        mark: RecordedMark,
    },
    Meme {
        options: img::MemeOptions,
    },
    #[serde(rename = "preset-fix")]
    PresetFix {
        preset: mt_core::preset::Preset,
    },
    #[serde(rename = "pdf-archive")]
    Archive {
        /// Recorded as the part's label ("PDF/A-2b"), not the enum.
        #[serde(default)]
        part: Option<String>,
    },
    Repair,
    #[serde(rename = "pdf-to-markdown")]
    Markdown {
        options: pdf::MarkdownOptions,
    },
}

/// What the image watermark recipe kept of the mark.
///
/// A logo's bytes would be in every history row, so only the fact of it was
/// recorded — which means a logo watermark can't be replayed from here.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedMark {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub logo: bool,
}

/// Whether a recorded job can be run again, and what to call it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Replayable {
    pub can: bool,
    /// What the button should say: "Compress another file to 50 KB–100 KB".
    pub label: String,
    /// Why not, when it can't. Shown instead of the button.
    pub why: Option<String>,
    /// What the replay takes: "image", "pdf", or "either".
    pub takes: String,
}

/// The files a replay produced.
#[derive(Debug)]
pub struct Produced {
    /// Name and bytes, in the order they were made.
    pub files: Vec<(String, Vec<u8>)>,
    pub summary: String,
}

/// Read a recipe and say whether it can be replayed.
pub fn inspect(json: &str) -> Replayable {
    let tool = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("tool").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_default();

    if let Some(why) = refusal(&tool) {
        return Replayable { can: false, label: String::new(), why: Some(why), takes: "".into() };
    }

    match serde_json::from_str::<Recipe>(json) {
        Ok(recipe) => Replayable {
            can: recipe.replayable_now(),
            label: recipe.label(),
            why: recipe.blocked(),
            takes: recipe.takes().into(),
        },
        // An unknown shape is not a crash and not a silent nothing: the entry
        // simply offers no button, and says the recording is one this build
        // doesn't understand.
        Err(e) => Replayable {
            can: false,
            label: String::new(),
            why: Some(format!("this recipe isn't one this version can replay ({e})")),
            takes: "".into(),
        },
    }
}

/// The honest reason a tool's work was never replayable.
fn refusal(tool: &str) -> Option<String> {
    let why = match tool {
        "redact" => "What a redaction removed is never written down — that is the point of it. Open Redact PDF and mark the new document yourself.",
        "fill-form" => "The answers typed into a form aren't recorded, only which fields were filled. Open PDF forms to fill another copy.",
        "sign" | "signature" => "Where a signature went, and the signature itself, aren't kept in history. Open Sign PDF to place it on another document.",
        "annotate" => "The notes and shapes you drew aren't kept in history, only how many there were.",
        "merge" | "compare" | "images-to-pdf" | "scan-to-pdf" => "This one works on several files at once, so there is nothing to hand it here. Open the tool and choose them.",
        "batch" => "A batch already is the 'do it to all of them' tool. Open Batch and drop the new folder in.",
        "html-to-pdf" | "html-to-image" => "This was rendered from a web page rather than a file, so there is nothing to run it on. Open the tool and paste the address again.",
        "ocr" => "Reading a scan needs the text model, which the tool loads for you. Open OCR PDF.",
        "remove-background" | "upscale" | "blur-faces" => "This one runs a model that the tool loads when you open it. Open the tool and drop the file there.",
        "protect" | "unlock" => "Passwords are never kept, which is deliberate. Open the tool and type it again.",
        _ => return None,
    };
    Some(why.into())
}

impl Recipe {
    fn label(&self) -> String {
        match self {
            Recipe::Compress { target, .. } => {
                format!("Compress another file to {}", target.describe())
            }
            Recipe::Transform { op, .. } => match op {
                TransformOp::Resize(_) => "Resize another picture the same way".into(),
                TransformOp::Crop(_) | TransformOp::CropAspect(_) => {
                    "Crop another picture the same way".into()
                }
                TransformOp::Rotate { .. } => "Turn another picture the same way".into(),
                TransformOp::Convert => "Convert another picture the same way".into(),
            },
            Recipe::Adjust { .. } => "Adjust another picture the same way".into(),
            Recipe::Pages { .. } => "Do the same to another document's pages".into(),
            Recipe::Split { .. } => "Split another document the same way".into(),
            Recipe::PdfToImages { .. } => "Export another document's pages".into(),
            Recipe::CropPdf { .. } => "Crop another document the same way".into(),
            Recipe::PageNumbers { .. } => "Number another document the same way".into(),
            Recipe::Watermark { .. } => "Watermark another document the same way".into(),
            Recipe::WatermarkImage { .. } => "Watermark another picture the same way".into(),
            Recipe::Meme { .. } => "Caption another picture the same way".into(),
            Recipe::PresetFix { preset } => format!("Fix another file for {}", preset.name),
            Recipe::Archive { .. } => "Convert another document to PDF/A".into(),
            Recipe::Repair => "Repair another document".into(),
            Recipe::Markdown { .. } => "Read another document into Markdown".into(),
        }
    }

    /// What this takes, so the screen can filter the file picker.
    fn takes(&self) -> &'static str {
        match self {
            Recipe::Compress { .. } => "either",
            Recipe::Transform { .. }
            | Recipe::Adjust { .. }
            | Recipe::WatermarkImage { .. }
            | Recipe::Meme { .. } => "image",
            Recipe::PresetFix { preset } => match preset.kind {
                mt_core::preset::Kind::Image => "image",
                mt_core::preset::Kind::Pdf => "pdf",
            },
            _ => "pdf",
        }
    }

    fn replayable_now(&self) -> bool {
        self.blocked().is_none()
    }

    /// Set when this particular recording is missing something it needs.
    fn blocked(&self) -> Option<String> {
        match self {
            Recipe::WatermarkImage { mark, .. } if mark.logo => Some(
                "The logo itself isn't kept in history, only that there was one. Open Watermark image and choose it again."
                    .into(),
            ),
            Recipe::WatermarkImage { mark, .. } if mark.text.is_none() => {
                Some("This recording doesn't say what the mark was.".into())
            }
            _ => None,
        }
    }
}

/// Apply a recipe to a file.
pub fn run(recipe: &Recipe, name: &str, bytes: &[u8]) -> Result<Produced, String> {
    let kind = FileKind::sniff(bytes);
    let wants = match recipe {
        Recipe::Compress { .. } => Wants::Either,
        Recipe::Transform { .. }
        | Recipe::Adjust { .. }
        | Recipe::WatermarkImage { .. }
        | Recipe::Meme { .. } => Wants::Image,
        Recipe::PresetFix { preset } => match preset.kind {
            mt_core::preset::Kind::Image => Wants::Image,
            mt_core::preset::Kind::Pdf => Wants::Pdf,
        },
        _ => Wants::Pdf,
    };
    wants.check(kind, name)?;

    let single = |bytes: Vec<u8>, summary: String| -> Result<Produced, String> {
        let extension = FileKind::sniff(&bytes).extension().to_string();
        Ok(Produced { files: vec![(renamed(name, &extension), bytes)], summary })
    };

    match recipe {
        Recipe::Compress { target, bias } => {
            let outcome = if kind == FileKind::Pdf {
                pdf::compress_to_target(bytes, *target, *bias, pdf::PdfCompressOptions::default())
            } else {
                img::compress_to_target(bytes, *target, *bias, img::CompressOptions::default())
            }
            .map_err(|e| e.to_string())?;

            let (out, note) = match outcome {
                mt_core::Outcome::Hit(solved) => {
                    let note = format!("{}, {}", solved.knob, mt_core::format_size(solved.size));
                    (solved.bytes, note)
                }
                mt_core::Outcome::Padded(solved) => (
                    solved.bytes,
                    "padded up to clear the minimum".to_string(),
                ),
                mt_core::Outcome::Infeasible { best, why, .. } => (best, why),
            };
            single(out, note)
        }

        Recipe::Transform { op, options } => {
            let opts = options.clone().unwrap_or_default();
            let out = match op {
                TransformOp::Resize(mode) => img::resize(bytes, *mode, &opts),
                TransformOp::Crop(rect) => img::crop(bytes, *rect, &opts),
                TransformOp::CropAspect(aspect) => img::crop_to_aspect(bytes, *aspect, &opts),
                TransformOp::Rotate { degrees, flip } => img::rotate(bytes, *degrees, *flip, &opts),
                TransformOp::Convert => img::convert(bytes, &opts),
            }
            .map_err(|e| e.to_string())?;
            let dims = ::image::load_from_memory(&out)
                .map(|i| format!("{}×{}", i.width(), i.height()))
                .unwrap_or_default();
            single(out, dims)
        }

        Recipe::Adjust { options } => {
            let (adjusted, report) = img::adjust(bytes, options).map_err(|e| e.to_string())?;
            let out_kind = if kind == FileKind::Png { FileKind::Png } else { FileKind::Jpeg };
            let out = img::encode::encode(&adjusted, out_kind, 92, true)
                .map_err(|e| e.to_string())?;
            single(out, format!("{}×{}", report.width, report.height))
        }

        Recipe::Pages { ops } => {
            let (out, report) = pdf::rebuild_ops(bytes, ops).map_err(|e| e.to_string())?;
            single(out, format!("{} → {} pages", report.pages_before, report.pages_after))
        }

        Recipe::Split { mode } => {
            let parts = pdf::split(bytes, mode).map_err(|e| e.to_string())?;
            let stem = stem(name);
            let count = parts.len();
            Ok(Produced {
                // Each piece keeps the name the engine chose for it, against
                // the new file's stem: `statement-pages-1-3.pdf`.
                files: parts
                    .into_iter()
                    .map(|part| (format!("{stem}{}.pdf", part.suffix), part.bytes))
                    .collect(),
                summary: format!("split into {count} files"),
            })
        }

        Recipe::PdfToImages { max_px, quality } => {
            let pages = pdf::export_pages(bytes, *max_px, *quality).map_err(|e| e.to_string())?;
            let stem = stem(name);
            // A page that couldn't be drawn says so in the report rather than
            // producing an empty file.
            let files: Vec<(String, Vec<u8>)> = pages
                .iter()
                .filter_map(|page| {
                    page.jpeg
                        .clone()
                        .map(|jpeg| (format!("{stem}-{}.jpg", page.page), jpeg))
                })
                .collect();
            let missed = pages.len() - files.len();
            Ok(Produced {
                summary: format!(
                    "exported {} page{} as JPG{}",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" },
                    if missed > 0 { format!(", {missed} could not be drawn") } else { String::new() }
                ),
                files,
            })
        }

        Recipe::CropPdf { pages, spec } => {
            let selection = selection(bytes, pages.as_deref())?;
            let (out, _report) = pdf::crop(bytes, selection.as_ref(), spec)
                .map_err(|e| e.to_string())?;
            single(out, "cropped".into())
        }

        Recipe::PageNumbers { pages, options } => {
            let selection = selection(bytes, pages.as_deref())?;
            let (out, numbered) = pdf::add_page_numbers(bytes, selection.as_ref(), options)
                .map_err(|e| e.to_string())?;
            single(out, format!("numbered {numbered} pages"))
        }

        Recipe::Watermark { pages, options } => {
            let selection = selection(bytes, pages.as_deref())?;
            let out =
                pdf::watermark(bytes, selection.as_ref(), options).map_err(|e| e.to_string())?;
            single(out, "watermarked".into())
        }

        Recipe::WatermarkImage { options, mark } => {
            let text = mark
                .text
                .clone()
                .ok_or("this recording doesn't say what the mark was")?;
            let font = crate::system_font()?;
            let out = img::watermark::watermark(bytes, &img::ImageMark::Text { text, font }, options)
                .map_err(|e| e.to_string())?;
            single(out, "watermarked".into())
        }

        Recipe::Meme { options } => {
            let font = crate::caption_font()?;
            let (image, report) = img::meme(bytes, &font, options).map_err(|e| e.to_string())?;
            let out = img::encode::encode(&image, FileKind::Jpeg, 92, true)
                .map_err(|e| e.to_string())?;
            single(out, format!("captioned, {}×{}", report.width, report.height))
        }

        Recipe::PresetFix { preset } => {
            let fixed = mt_core::preset::fix(bytes, preset, mt_core::Bias::Balanced)
                .map_err(|e| e.to_string())?;
            let summary = if fixed.report.fits {
                format!("{}, ready to upload", mt_core::format_size(fixed.bytes.len() as u64))
            } else {
                fixed.shortfall.clone().unwrap_or_else(|| "still short".into())
            };
            single(fixed.bytes, summary)
        }

        Recipe::Archive { part } => {
            let options = pdf::ArchiveOptions {
                // The profile belongs to this machine, not to the recording.
                icc: crate::system_srgb_profile(),
                part: match part.as_deref() {
                    Some(label) if label.contains("1") => pdf::Part::A1b,
                    _ => pdf::Part::A2b,
                },
                ..Default::default()
            };
            let (out, report) = pdf::to_archive(bytes, &options).map_err(|e| e.to_string())?;
            single(
                out,
                format!(
                    "{}{}",
                    report.part,
                    if report.conformant { "" } else { ", with what it couldn't fix listed" }
                ),
            )
        }

        Recipe::Repair => {
            let (out, report) = pdf::repair(bytes).map_err(|e| e.to_string())?;
            single(out, report.headline())
        }

        Recipe::Markdown { options } => {
            let (markdown, report) = pdf::to_markdown(bytes, options).map_err(|e| e.to_string())?;
            if report.pages_with_text == 0 {
                return Err("there is no text in this one to read".into());
            }
            Ok(Produced {
                files: vec![(renamed(name, "md"), markdown.into_bytes())],
                summary: format!("{} pages read", report.pages_with_text),
            })
        }
    }
}

enum Wants {
    Image,
    Pdf,
    Either,
}

impl Wants {
    fn check(&self, kind: FileKind, name: &str) -> Result<(), String> {
        let ok = match self {
            Wants::Image => kind.is_image(),
            Wants::Pdf => kind == FileKind::Pdf,
            Wants::Either => kind.is_image() || kind == FileKind::Pdf,
        };
        if ok {
            return Ok(());
        }
        Err(match self {
            Wants::Image => format!("{name} isn't a picture, and this job works on pictures"),
            Wants::Pdf => format!("{name} isn't a PDF, and this job works on PDFs"),
            Wants::Either => format!("{name} is neither a picture nor a PDF"),
        })
    }
}

fn selection(bytes: &[u8], spec: Option<&str>) -> Result<Option<pdf::PageSelection>, String> {
    match spec.filter(|s| !s.trim().is_empty()) {
        Some(spec) => {
            let total = pdf::page_count(bytes).map_err(|e| e.to_string())?;
            // The pages were chosen for a different document, so a range that
            // ran off the end of this one is worth saying rather than quietly
            // doing less than was asked.
            let selection = pdf::PageSelection::parse(spec, total).map_err(|e| {
                format!("the pages this job used ({spec}) don't fit this document: {e}")
            })?;
            Ok(Some(selection))
        }
        None => Ok(None),
    }
}

fn stem(name: &str) -> String {
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string()
}

fn renamed(name: &str, extension: &str) -> String {
    format!("{}.{extension}", stem(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compress_recipe_runs_again_on_a_new_file() {
        // The whole point: last month's settings, this month's file.
        let json = r#"{ "tool": "compress",
                        "target": { "kind": "range", "value": { "min": 20000, "max": 50000 } },
                        "bias": "balanced" }"#;
        let status = inspect(json);
        assert!(status.can, "{status:?}");
        assert!(status.label.contains("20 KB"), "{}", status.label);
        assert_eq!(status.takes, "either");

        let recipe: Recipe = serde_json::from_str(json).unwrap();
        let photo = mt_core::testkit::encode_jpeg(&mt_core::testkit::photo(1200, 900), 95);
        let produced = run(&recipe, "new-photo.jpg", &photo).expect("a replay");
        assert_eq!(produced.files.len(), 1);
        let size = produced.files[0].1.len() as u64;
        assert!((20_000..=50_000).contains(&size), "came out at {size}");
    }

    #[test]
    fn a_recipe_without_a_setting_the_tool_later_grew_still_replays() {
        // What the command line records, and what older versions of the app
        // recorded: a target and nothing else.
        let json = r#"{ "tool": "compress", "target": { "kind": "at_most", "value": 90000 } }"#;
        let status = inspect(json);
        assert!(status.can, "{status:?}");

        let recipe: Recipe = serde_json::from_str(json).unwrap();
        let photo = mt_core::testkit::encode_jpeg(&mt_core::testkit::photo(900, 700), 92);
        let produced = run(&recipe, "old.jpg", &photo).expect("a replay");
        assert!(produced.files[0].1.len() <= 90_000);
    }

    #[test]
    fn a_recipe_that_was_never_recorded_in_full_says_why() {
        // Redaction, signing and form filling leave out what they did on
        // purpose. The screen needs the reason, not a dead button.
        for tool in ["redact", "fill-form", "sign", "annotate"] {
            let status = inspect(&format!(r#"{{ "tool": "{tool}", "targets": 2 }}"#));
            assert!(!status.can, "{tool} should not be replayable");
            let why = status.why.expect(tool);
            assert!(why.len() > 30, "{tool}: {why}");
        }
    }

    #[test]
    fn a_tool_that_needs_several_files_points_at_the_tool() {
        let status = inspect(r#"{ "tool": "merge", "files": 3 }"#);
        assert!(!status.can);
        assert!(status.why.unwrap().contains("several files"));
    }

    /// The recorded shape, built the way the app builds it.
    fn recorded(tool: &str, extra: serde_json::Value) -> String {
        let mut value = serde_json::json!({ "tool": tool });
        if let (Some(object), Some(more)) = (value.as_object_mut(), extra.as_object()) {
            for (key, v) in more {
                object.insert(key.clone(), v.clone());
            }
        }
        value.to_string()
    }

    #[test]
    fn a_logo_watermark_cannot_be_replayed_because_the_logo_was_not_kept() {
        let json = recorded(
            "watermark-image",
            serde_json::json!({
                "options": img::ImageWatermarkOptions::default(),
                "mark": { "logo": true },
            }),
        );
        let status = inspect(&json);
        assert!(!status.can, "{status:?}");
        assert!(status.why.unwrap().contains("logo"));
    }

    #[test]
    fn a_text_watermark_replays_onto_another_picture() {
        let json = recorded(
            "watermark-image",
            serde_json::json!({
                "options": img::ImageWatermarkOptions::default(),
                "mark": { "text": "DRAFT" },
            }),
        );
        let status = inspect(&json);
        assert!(status.can, "{status:?}");

        let recipe: Recipe = serde_json::from_str(&json).unwrap();
        let photo = mt_core::testkit::encode_jpeg(&mt_core::testkit::photo(600, 400), 90);
        // A machine with no usable font is a fair outcome here; a wrong one
        // is not. Either way it must not panic.
        match run(&recipe, "holiday.jpg", &photo) {
            Ok(produced) => assert_eq!(produced.files.len(), 1),
            Err(why) => assert!(why.contains("font"), "{why}"),
        }
    }

    #[test]
    fn a_recipe_shape_this_build_does_not_know_is_not_a_crash() {
        let status = inspect(r#"{ "tool": "compress", "target": "nonsense" }"#);
        assert!(!status.can);
        assert!(status.why.is_some());
    }

    #[test]
    fn the_wrong_kind_of_file_is_turned_away_before_any_work() {
        let json = r#"{ "tool": "repair" }"#;
        let recipe: Recipe = serde_json::from_str(json).unwrap();
        let photo = mt_core::testkit::encode_jpeg(&mt_core::testkit::photo(40, 40), 80);
        let refused = run(&recipe, "photo.jpg", &photo).unwrap_err();
        assert!(refused.contains("isn't a PDF"), "{refused}");
    }

    #[test]
    fn a_preset_fix_replays_onto_another_photo() {
        let json = r#"{ "tool": "preset-fix", "preset":
            { "id": "ibps-photo", "name": "IBPS photo", "group": "Banking", "kind": "image",
              "target": { "kind": "range", "value": { "min": 20000, "max": 50000 } },
              "format": "jpeg",
              "dimensions": { "width": 200, "height": 230, "fit": "exact" } } }"#;
        let status = inspect(json);
        assert!(status.can);
        assert!(status.label.contains("IBPS photo"), "{}", status.label);
        assert_eq!(status.takes, "image");

        let recipe: Recipe = serde_json::from_str(json).unwrap();
        let photo = mt_core::testkit::encode_jpeg(&mt_core::testkit::photo(1800, 1200), 92);
        let produced = run(&recipe, "mine.jpg", &photo).expect("a replay");
        let (w, h) = ::image::load_from_memory(&produced.files[0].1)
            .map(|i| (i.width(), i.height()))
            .unwrap();
        assert_eq!((w, h), (200, 230));
    }

    #[test]
    fn splitting_names_every_piece_after_the_new_file() {
        let json = recorded(
            "split",
            serde_json::json!({ "mode": pdf::SplitMode::EveryPage }),
        );
        let recipe: Recipe = serde_json::from_str(&json).unwrap();
        let pdf = mt_core::testkit::pdf_scan(3, 200, 260, 80);
        let produced = run(&recipe, "statement.pdf", &pdf).expect("a replay");
        assert_eq!(produced.files.len(), 3);
        for (name, bytes) in &produced.files {
            assert!(name.starts_with("statement"), "{name} should carry the new file's name");
            assert!(name.ends_with(".pdf"));
            assert!(!bytes.is_empty());
        }
    }

    #[test]
    fn pages_chosen_for_a_longer_document_are_refused_in_words() {
        // The job numbered pages 1-9 of something; this file has three.
        let json = recorded(
            "page-numbers",
            serde_json::json!({ "pages": "8-9", "options": pdf::PageNumberOptions::default() }),
        );
        let recipe: Recipe = serde_json::from_str(&json).unwrap();
        let pdf = mt_core::testkit::pdf_scan(3, 200, 260, 80);
        let refused = run(&recipe, "short.pdf", &pdf).unwrap_err();
        assert!(refused.contains("don't fit this document"), "{refused}");
    }
}
