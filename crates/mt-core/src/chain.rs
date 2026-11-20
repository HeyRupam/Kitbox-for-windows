//! A saved sequence of operations: crop, resize, compress, as one thing.
//!
//! Every other part of this crate does one job to one file. A chain is how the
//! job someone actually has gets expressed — "crop to 3.5 × 4.5, make it
//! 200 × 230, land it between 20 and 50 KB" is one task to the person doing it
//! and three operations to the engine. Saved once, it becomes a drop.
//!
//! Two rules it enforces rather than hopes for:
//!
//! * **Compression goes last.** Every step after it re-encodes the file, which
//!   throws away the size that was just solved for. A chain that puts it
//!   anywhere else is refused when it is saved, not quietly obeyed.
//! * **A step says what it needs.** The watermark needs a font and this crate
//!   reads no files, so the caller passes one; without it the chain refuses
//!   rather than silently skipping the step.

use serde::{Deserialize, Serialize};

use crate::engine::image::{
    self, CompressOptions, Flip, ImageMark, ImageWatermarkOptions, OutputFormat, ResizeMode,
    TransformOptions,
};
use crate::engine::pdf::{self, PdfCompressOptions};
use crate::format::FileKind;
use crate::{Bias, Error, Outcome, Result, SizeTarget};

/// One operation in a chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Step {
    /// Cap the longest edge.
    Resize {
        longest: u32,
    },
    /// Exactly these pixels, by covering the box and trimming the overflow —
    /// how a form photo gets to 200 × 230 without being squashed.
    Fit {
        width: u32,
        height: u32,
    },
    /// Trim to a shape, keeping the middle.
    Aspect {
        ratio: f32,
    },
    Rotate {
        degrees: i32,
        #[serde(default)]
        flip: Flip,
    },
    /// Levels and colour.
    Adjust(image::AdjustOptions),
    Watermark {
        text: String,
        #[serde(default)]
        options: ImageWatermarkOptions,
    },
    Convert {
        format: OutputFormat,
    },
    /// Land inside a size. Always the last step.
    Compress {
        target: SizeTarget,
        #[serde(default)]
        bias: Bias,
    },
}

impl Step {
    /// What this step does, in a few words.
    pub fn describe(&self) -> String {
        match self {
            Step::Resize { longest } => format!("Resize to {longest} px on the longest edge"),
            Step::Fit { width, height } => format!("Crop and fit to {width}×{height}"),
            Step::Aspect { ratio } => format!("Crop to a {ratio:.2} shape"),
            Step::Rotate { degrees, flip } => match (degrees, flip) {
                (0, Flip::None) => "Leave it as it is".into(),
                (0, f) => format!("Flip {}", flip_word(*f)),
                (d, Flip::None) => format!("Turn {d}°"),
                (d, f) => format!("Turn {d}° and flip {}", flip_word(*f)),
            },
            Step::Adjust(_) => "Correct levels and colour".into(),
            Step::Watermark { text, .. } => format!("Stamp \"{text}\" on it"),
            Step::Convert { format } => format!("Save as {}", format_word(*format)),
            Step::Compress { target, .. } => format!("Compress to {}", target.describe()),
        }
    }

    fn needs_font(&self) -> bool {
        matches!(self, Step::Watermark { .. })
    }
}

fn flip_word(flip: Flip) -> &'static str {
    match flip {
        Flip::None => "not at all",
        Flip::Horizontal => "left to right",
        Flip::Vertical => "top to bottom",
    }
}

fn format_word(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Jpeg => "JPG",
        OutputFormat::Png => "PNG",
        OutputFormat::Keep => "it is",
    }
}

/// A named sequence, saved and re-run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    pub id: String,
    pub name: String,
    pub steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Chain {
    /// Catch what cannot work before it is saved.
    pub fn check(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::invalid("a recipe needs a name"));
        }
        if self.steps.is_empty() {
            return Err(Error::invalid(format!("'{}' has no steps in it", self.name)));
        }
        if let Some(at) = self.steps.iter().position(|s| matches!(s, Step::Compress { .. })) {
            if at != self.steps.len() - 1 {
                // Everything after a compress re-encodes the file, which
                // undoes the size it just solved for.
                return Err(Error::invalid(
                    "compressing has to be the last step — anything after it would undo the size"
                        .to_string(),
                ));
            }
            if self.steps.iter().filter(|s| matches!(s, Step::Compress { .. })).count() > 1 {
                return Err(Error::invalid("a recipe compresses once, at the end".to_string()));
            }
        }
        Ok(())
    }

    /// True when the only step is one a PDF can take.
    pub fn works_on_pdfs(&self) -> bool {
        self.steps.iter().all(|s| matches!(s, Step::Compress { .. }))
    }

    pub fn needs_font(&self) -> bool {
        self.steps.iter().any(Step::needs_font)
    }

    pub fn describe(&self) -> Vec<String> {
        self.steps.iter().map(Step::describe).collect()
    }
}

/// What running a chain produced.
#[derive(Debug, Clone)]
pub struct Applied {
    pub bytes: Vec<u8>,
    /// Each step, and what it did, in order.
    pub steps: Vec<String>,
    /// Set when the last step could not reach the size it was given.
    pub short: Option<String>,
    pub extension: String,
}

/// Run a chain over one file.
///
/// `font` is only read if a step needs one; `mt-core` opens no files, so the
/// caller supplies it.
pub fn apply(bytes: &[u8], chain: &Chain, font: Option<&[u8]>) -> Result<Applied> {
    chain.check()?;
    let kind = FileKind::sniff(bytes);

    if kind == FileKind::Pdf {
        return apply_to_pdf(bytes, chain);
    }
    if !kind.is_image() {
        return Err(Error::unsupported(format!(
            "a recipe works on pictures and PDFs, and this is {}",
            kind.article()
        )));
    }
    if chain.needs_font() && font.is_none() {
        return Err(Error::invalid(
            "this recipe stamps text on the picture, and no font was supplied".to_string(),
        ));
    }

    let mut current = bytes.to_vec();
    let mut done = Vec::new();
    let mut short = None;
    let mut padded = false;

    for step in &chain.steps {
        let before = current.len() as u64;
        current = match step {
            Step::Resize { longest } => {
                image::resize(&current, ResizeMode::LongestEdge(*longest), &keep_format(&current))?
            }
            Step::Fit { width, height } => image::resize(
                &current,
                ResizeMode::Fill { width: *width, height: *height },
                &keep_format(&current),
            )?,
            Step::Aspect { ratio } => {
                image::crop_to_aspect(&current, *ratio, &keep_format(&current))?
            }
            Step::Rotate { degrees, flip } => {
                image::rotate(&current, *degrees, *flip, &keep_format(&current))?
            }
            Step::Adjust(options) => {
                let (adjusted, _) = image::adjust(&current, options)?;
                let out = if FileKind::sniff(&current) == FileKind::Png {
                    FileKind::Png
                } else {
                    FileKind::Jpeg
                };
                image::encode::encode(&adjusted, out, 92, true)?
            }
            Step::Watermark { text, options } => {
                let font = font.expect("checked above");
                let mark = ImageMark::Text { text: text.clone(), font: font.to_vec() };
                image::watermark::watermark(&current, &mark, options)?
            }
            Step::Convert { format } => image::convert(
                &current,
                &TransformOptions { format: *format, ..Default::default() },
            )?,
            Step::Compress { target, bias } => {
                let outcome = image::compress_to_target(
                    &current,
                    *target,
                    *bias,
                    CompressOptions {
                        format: OutputFormat::Keep,
                        strip_metadata: true,
                        max_dimension: None,
                        exact_dimensions: None,
                    },
                )?;
                match outcome {
                    Outcome::Hit(solved) => solved.bytes,
                    // Filler added to clear a minimum the encoder couldn't
                    // reach. It works, and the step line has to say so — the
                    // numbers alone would just look like the file got bigger.
                    Outcome::Padded(solved) => {
                        padded = true;
                        solved.bytes
                    }
                    Outcome::Infeasible { best, why, .. } => {
                        short = Some(why);
                        best
                    }
                }
            }
        };

        done.push(format!(
            "{} ({} → {}){}",
            step.describe(),
            crate::size::format_size(before),
            crate::size::format_size(current.len() as u64),
            if padded { ", padded with filler to clear the minimum" } else { "" }
        ));
    }

    let extension = FileKind::sniff(&current).extension().to_string();
    Ok(Applied { bytes: current, steps: done, short, extension })
}

/// A PDF can only be compressed here, and `check` has already said so.
fn apply_to_pdf(bytes: &[u8], chain: &Chain) -> Result<Applied> {
    let [Step::Compress { target, bias }] = chain.steps.as_slice() else {
        return Err(Error::unsupported(format!(
            "'{}' has picture steps in it, so it can't run on a PDF — compressing is the \
             only step a document can take here",
            chain.name
        )));
    };

    let outcome = pdf::compress_to_target(
        bytes,
        *target,
        *bias,
        PdfCompressOptions { strip_metadata: true, ..Default::default() },
    )?;
    let (out, short) = match outcome {
        Outcome::Hit(solved) | Outcome::Padded(solved) => (solved.bytes, None),
        Outcome::Infeasible { best, why, .. } => (best, Some(why)),
    };

    let steps = vec![format!(
        "Compress to {} ({} → {})",
        target.describe(),
        crate::size::format_size(bytes.len() as u64),
        crate::size::format_size(out.len() as u64)
    )];
    Ok(Applied { bytes: out, steps, short, extension: "pdf".into() })
}

/// Keep whatever the file already is between steps.
///
/// The format is decided once, by the last step that cares; converting in the
/// middle would mean re-encoding a JPEG several times over, and every one of
/// those costs quality for nothing.
fn keep_format(bytes: &[u8]) -> TransformOptions {
    TransformOptions {
        format: if FileKind::sniff(bytes) == FileKind::Png {
            OutputFormat::Png
        } else {
            OutputFormat::Keep
        },
        quality: 95,
        strip_metadata: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn form_photo() -> Chain {
        // The job this feature exists for.
        Chain {
            id: "form".into(),
            name: "Form photo".into(),
            steps: vec![
                Step::Fit { width: 200, height: 230 },
                Step::Compress {
                    target: SizeTarget::Range { min: 20_000, max: 50_000 },
                    bias: Bias::Balanced,
                },
            ],
            note: None,
        }
    }

    #[test]
    fn a_chain_does_each_step_in_order_and_says_so() {
        let photo = testkit::encode_jpeg(&testkit::photo(3000, 2000), 95);
        let done = apply(&photo, &form_photo(), None).expect("a run");

        let (w, h) =
            ::image::load_from_memory(&done.bytes).map(|i| (i.width(), i.height())).unwrap();
        assert_eq!((w, h), (200, 230), "the fit step decides the pixels");
        assert!(
            (20_000..=50_000).contains(&(done.bytes.len() as u64)),
            "came out at {}",
            done.bytes.len()
        );
        assert_eq!(done.steps.len(), 2);
        assert!(done.steps[0].contains("200×230"), "{:?}", done.steps);
        assert!(done.short.is_none());
    }

    #[test]
    fn compressing_anywhere_but_last_is_refused_when_it_is_saved() {
        // Resizing after a compress re-encodes the file and throws the size
        // away. Better to refuse than to obey and disappoint.
        let chain = Chain {
            id: "wrong".into(),
            name: "Backwards".into(),
            steps: vec![
                Step::Compress { target: SizeTarget::AtMost(50_000), bias: Bias::Balanced },
                Step::Resize { longest: 800 },
            ],
            note: None,
        };
        let refused = chain.check().unwrap_err().to_string();
        assert!(refused.contains("last step"), "{refused}");
    }

    #[test]
    fn a_recipe_compresses_once() {
        let chain = Chain {
            id: "twice".into(),
            name: "Twice".into(),
            steps: vec![
                Step::Compress { target: SizeTarget::AtMost(90_000), bias: Bias::Balanced },
                Step::Compress { target: SizeTarget::AtMost(50_000), bias: Bias::Balanced },
            ],
            note: None,
        };
        assert!(chain.check().is_err());
    }

    #[test]
    fn an_empty_recipe_is_not_a_recipe() {
        let chain = Chain { id: "x".into(), name: "Nothing".into(), steps: vec![], note: None };
        assert!(chain.check().unwrap_err().to_string().contains("no steps"));
    }

    #[test]
    fn a_document_can_run_a_compress_only_recipe() {
        let chain = Chain {
            id: "doc".into(),
            name: "Under 100 KB".into(),
            steps: vec![Step::Compress {
                target: SizeTarget::AtMost(100_000),
                bias: Bias::Balanced,
            }],
            note: None,
        };
        assert!(chain.works_on_pdfs());

        let pdf = testkit::pdf_scan(2, 800, 1000, 92);
        let done = apply(&pdf, &chain, None).expect("a run");
        assert!(done.bytes.len() <= 100_000, "{} bytes", done.bytes.len());
        assert_eq!(done.extension, "pdf");
    }

    #[test]
    fn a_picture_recipe_turns_a_pdf_away_in_words() {
        let pdf = testkit::pdf_scan(1, 400, 500, 85);
        let refused = apply(&pdf, &form_photo(), None).unwrap_err().to_string();
        assert!(refused.contains("picture steps"), "{refused}");
        assert!(!form_photo().works_on_pdfs());
    }

    #[test]
    fn a_stamp_without_a_font_is_refused_rather_than_skipped() {
        // Quietly dropping a step would produce a file that looks right and
        // isn't.
        let chain = Chain {
            id: "stamp".into(),
            name: "Draft".into(),
            steps: vec![Step::Watermark {
                text: "DRAFT".into(),
                options: ImageWatermarkOptions::default(),
            }],
            note: None,
        };
        let photo = testkit::encode_jpeg(&testkit::photo(600, 400), 90);
        let refused = apply(&photo, &chain, None).unwrap_err().to_string();
        assert!(refused.contains("font"), "{refused}");
        assert!(chain.needs_font());
    }

    #[test]
    fn a_size_the_recipe_cannot_reach_is_reported_not_hidden() {
        let mut chain = form_photo();
        chain.steps = vec![
            Step::Fit { width: 200, height: 230 },
            Step::Compress { target: SizeTarget::AtMost(200), bias: Bias::Balanced },
        ];
        let photo = testkit::encode_jpeg(&testkit::photo(1200, 900), 92);

        let done = apply(&photo, &chain, None).expect("still a file");
        assert!(!done.bytes.is_empty());
        assert!(done.short.is_some(), "it has to say it fell short");
    }

    #[test]
    fn padding_up_to_a_minimum_is_said_in_the_step() {
        // A file that comes out under the floor is padded to clear it. The
        // numbers alone would just look like it got bigger for no reason.
        let chain = Chain {
            id: "pad".into(),
            name: "Padded".into(),
            steps: vec![
                Step::Fit { width: 120, height: 120 },
                Step::Compress {
                    target: SizeTarget::Range { min: 60_000, max: 90_000 },
                    bias: Bias::Balanced,
                },
            ],
            note: None,
        };
        let photo = testkit::encode_jpeg(&testkit::photo(900, 900), 92);
        let done = apply(&photo, &chain, None).expect("a run");

        assert!(done.bytes.len() >= 60_000, "{} bytes", done.bytes.len());
        assert!(done.steps.last().unwrap().contains("padded"), "{:?}", done.steps);
    }

    #[test]
    fn every_step_reads_as_a_sentence() {
        // These are what the screen shows; a step that describes itself badly
        // is a recipe nobody can check.
        let chain = Chain {
            id: "all".into(),
            name: "Everything".into(),
            steps: vec![
                Step::Resize { longest: 1600 },
                Step::Aspect { ratio: 0.777 },
                Step::Rotate { degrees: 90, flip: Flip::Horizontal },
                Step::Adjust(image::AdjustOptions::default()),
                Step::Convert { format: OutputFormat::Png },
                Step::Compress { target: SizeTarget::AtMost(100_000), bias: Bias::Balanced },
            ],
            note: None,
        };
        let lines = chain.describe();
        assert_eq!(lines.len(), 6);
        assert!(lines[0].contains("1600"));
        assert!(lines[2].contains("90°"), "{}", lines[2]);
        assert!(lines[4].contains("PNG"));
        assert!(lines[5].contains("100 KB"));
    }

    #[test]
    fn a_chain_survives_the_round_trip_to_json() {
        // They are saved as files people can edit.
        let chain = form_photo();
        let json = serde_json::to_string_pretty(&chain).unwrap();
        let back: Chain = serde_json::from_str(&json).expect("re-read");
        assert_eq!(back.steps.len(), 2);
        match &back.steps[0] {
            Step::Fit { width, height } => assert_eq!((*width, *height), (200, 230)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_format_is_not_churned_between_steps() {
        // Re-encoding a JPEG at every step would cost quality for nothing.
        let png = {
            let mut bytes = Vec::new();
            testkit::photo(500, 400)
                .write_to(&mut std::io::Cursor::new(&mut bytes), ::image::ImageFormat::Png)
                .unwrap();
            bytes
        };
        let chain = Chain {
            id: "png".into(),
            name: "Keep PNG".into(),
            steps: vec![
                Step::Resize { longest: 300 },
                Step::Rotate { degrees: 90, flip: Flip::None },
            ],
            note: None,
        };
        let done = apply(&png, &chain, None).expect("a run");
        assert_eq!(FileKind::sniff(&done.bytes), FileKind::Png, "it should still be a PNG");
        assert_eq!(done.extension, "png");
    }
}
