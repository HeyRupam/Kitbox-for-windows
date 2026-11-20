//! PDF compression driven by a size target.
//!
//! The ladder's top rung is "lossless only" — repack and strip, change nothing
//! visible. Every rung below it trades resolution and JPEG quality on the
//! embedded images. Text is never touched, so a compressed form stays
//! selectable, searchable and printable.
//!
//! What this deliberately does *not* do is rasterise the pages. Flattening a
//! document into pictures of words will always hit a smaller number, and it
//! silently destroys text selection, search and screen-reader access. When
//! downsampling can't reach the target, the engine says so.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use image::{imageops::FilterType, DynamicImage};
use lopdf::{Document, ObjectId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::images::{apply_image, decode_image, encode_jpeg_for_pdf, pixels_for_dpi};
use super::inspect::{inspect_doc, DocKind, ImageInfo, PdfReport};
use super::lossless::{lossless_pass, save_compact};
use crate::error::{Error, Result};
use crate::quality;
use crate::target::{solve, Bias, Encodable, Outcome, SizeTarget};

/// Raw pixels we're willing to hold in memory across probes. A 50-page 300 DPI
/// colour scan would be several gigabytes decoded, so beyond this budget images
/// are decoded on demand instead of cached — slower, but it won't fall over.
const DECODE_CACHE_PIXELS: usize = 64_000_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfCompressOptions {
    pub strip_metadata: bool,
    /// Let the solver drop colour when it runs out of other levers. Off by
    /// default: a greyscale ID scan can be rejected by the thing you're
    /// uploading it to.
    pub allow_grayscale: bool,
    /// Never go below this, however desperate. 50 DPI is already unpleasant.
    pub min_dpi: u16,
}

impl Default for PdfCompressOptions {
    fn default() -> Self {
        PdfCompressOptions { strip_metadata: true, allow_grayscale: false, min_dpi: 50 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfKnob {
    /// Repack only — no image is touched.
    pub lossless_only: bool,
    pub dpi: u16,
    pub quality: u8,
    pub grayscale: bool,
}

pub struct PdfCompressor {
    /// The document after the lossless pass; every probe clones this.
    baseline: Document,
    baseline_bytes: Vec<u8>,
    report: PdfReport,
    /// Images worth re-encoding, largest first.
    targets: Vec<ImageInfo>,
    decoded: Mutex<HashMap<ObjectId, Arc<DynamicImage>>>,
    cached_pixels: AtomicUsize,
    /// Similarity of the dominant image, keyed by output length so `score`
    /// can find the number computed while encoding.
    scores: Mutex<HashMap<usize, f32>>,
    opts: PdfCompressOptions,
}

impl PdfCompressor {
    pub fn new(bytes: &[u8], opts: PdfCompressOptions) -> Result<Self> {
        let mut doc = Document::load_mem(bytes)
            .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;

        if doc.is_encrypted() {
            return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
        }

        let report = inspect_doc(&doc, bytes.len() as u64)?;

        lossless_pass(&mut doc, opts.strip_metadata);
        let mut baseline_bytes = save_compact(&mut doc)?;
        // Repacking can very occasionally produce a larger file than the
        // original; never hand back something worse than we were given.
        if baseline_bytes.len() >= bytes.len() {
            baseline_bytes = bytes.to_vec();
        }

        let mut targets: Vec<ImageInfo> =
            report.images.iter().filter(|i| i.recompressible).cloned().collect();
        targets.sort_by_key(|i| std::cmp::Reverse(i.bytes));

        Ok(PdfCompressor {
            baseline: doc,
            baseline_bytes,
            report,
            targets,
            decoded: Mutex::new(HashMap::new()),
            cached_pixels: AtomicUsize::new(0),
            scores: Mutex::new(HashMap::new()),
            opts,
        })
    }

    pub fn report(&self) -> &PdfReport {
        &self.report
    }

    /// Size after the lossless pass alone — what you get for free.
    pub fn lossless_size(&self) -> u64 {
        self.baseline_bytes.len() as u64
    }

    fn image_for(&self, id: ObjectId) -> Option<Arc<DynamicImage>> {
        if let Some(found) = self.decoded.lock().ok()?.get(&id) {
            return Some(found.clone());
        }
        let decoded = Arc::new(decode_image(&self.baseline, id)?);
        let pixels = (decoded.width() as usize).saturating_mul(decoded.height() as usize);
        // Racing threads may both decode the same image; that wastes a little
        // work but keeps the lock off the slow path.
        if self.cached_pixels.load(Ordering::Relaxed) + pixels <= DECODE_CACHE_PIXELS {
            if let Ok(mut cache) = self.decoded.lock() {
                cache.insert(id, decoded.clone());
                self.cached_pixels.fetch_add(pixels, Ordering::Relaxed);
            }
        }
        Some(decoded)
    }
}

/// One image's re-encoded result, ready to write into the document.
struct Encoded {
    id: ObjectId,
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}

impl Encodable for PdfCompressor {
    type Knob = PdfKnob;

    fn ladder(&self, bias: Bias) -> Vec<PdfKnob> {
        let lossless = PdfKnob { lossless_only: true, dpi: 0, quality: 0, grayscale: false };
        if self.targets.is_empty() {
            return vec![lossless];
        }

        let dpis: Vec<u16> = [300u16, 250, 200, 165, 140, 120, 100, 85, 72, 60, 50]
            .into_iter()
            .filter(|d| *d >= self.opts.min_dpi)
            .collect();
        let qualities: Vec<u8> = (25..=92).step_by(3).map(|q| q as u8).collect();
        let grays: Vec<bool> =
            if self.opts.allow_grayscale { vec![false, true] } else { vec![false] };

        // Only the image bytes respond to these knobs; the rest of the file is
        // a floor. Folding that in makes the solver's interpolation land much
        // closer on text-heavy documents.
        let total = self.report.total_bytes.max(1) as f64;
        let image_fraction = (self.report.recompressible_bytes() as f64 / total).clamp(0.0, 1.0);
        let floor = 1.0 - image_fraction;

        let mut buckets: std::collections::BTreeMap<i64, PdfKnob> = Default::default();
        for &dpi in &dpis {
            for &quality in &qualities {
                for &grayscale in &grays {
                    let knob = PdfKnob { lossless_only: false, dpi, quality, grayscale };
                    let scale = (dpi as f64 / 300.0).min(1.0);
                    let q = quality as f64 / 100.0;
                    let gray_factor = if grayscale { 0.55 } else { 1.0 };
                    let est = floor
                        + image_fraction * scale * scale * (0.08 + 0.42 * q * q) * gray_factor;
                    let bucket = (est.max(1e-4).ln() / 1.06f64.ln()).round() as i64;
                    match buckets.get(&bucket) {
                        Some(existing) if !prefers(bias, knob, *existing) => {}
                        _ => {
                            buckets.insert(bucket, knob);
                        }
                    }
                }
            }
        }

        let mut ladder: Vec<PdfKnob> = buckets.into_values().collect();
        // Lossless is always the largest output, so it caps the ladder.
        ladder.push(lossless);
        ladder
    }

    fn encode(&self, knob: &PdfKnob) -> Result<Vec<u8>> {
        if knob.lossless_only {
            return Ok(self.baseline_bytes.clone());
        }

        let encoded: Vec<Encoded> = self
            .targets
            .par_iter()
            .filter_map(|info| {
                let source = self.image_for(info.id)?;
                let resized = match pixels_for_dpi(&source, info.dpi, knob.dpi as f32) {
                    Some((w, h)) => Arc::new(source.resize_exact(w, h, FilterType::Lanczos3)),
                    None => source,
                };
                let jpeg = encode_jpeg_for_pdf(&resized, knob.quality, knob.grayscale).ok()?;
                Some(Encoded {
                    id: info.id,
                    jpeg,
                    width: resized.width(),
                    height: resized.height(),
                })
            })
            .collect();

        let mut doc = self.baseline.clone();
        for job in encoded {
            apply_image(&mut doc, job.id, job.jpeg, job.width, job.height, knob.grayscale)?;
        }
        let out = save_compact(&mut doc)?;
        Ok(out)
    }

    fn describe(&self, knob: &PdfKnob) -> String {
        if knob.lossless_only {
            return "repacked, images untouched".into();
        }
        let gray = if knob.grayscale { ", greyscale" } else { "" };
        format!("images at {} DPI, quality {}{}", knob.dpi, knob.quality, gray)
    }

    /// How much of the picture survived, measured on the image that dominates
    /// the file.
    ///
    /// Done here rather than while encoding, which is the whole reason a scan
    /// no longer takes ten seconds: the solver asks for a score once, on the
    /// answer it chose, and this used to be computed on every attempt it threw
    /// away — about 300 ms of each 450 ms probe, nine times for nothing.
    fn score(&self, encoded: &[u8]) -> Option<f32> {
        if encoded.len() == self.baseline_bytes.len() {
            return Some(1.0);
        }
        if let Some(found) = self.scores.lock().ok()?.get(&encoded.len()).copied() {
            return Some(found);
        }

        let dominant = self.targets.first()?;
        let before = self.image_for(dominant.id)?;
        // The image keeps its object id through the rebuild, so it can be
        // found again in the finished document.
        let finished = Document::load_mem(encoded).ok()?;
        let after = decode_image(&finished, dominant.id)?;

        let score = quality::ssim(&before, &after);
        if let Ok(mut scores) = self.scores.lock() {
            scores.insert(encoded.len(), score);
        }
        Some(score)
    }

    fn suggestions(&self, _target: SizeTarget) -> Vec<String> {
        let mut out = Vec::new();
        match self.report.kind {
            DocKind::TextOnly => {
                out.push(
                    "This PDF is text and fonts — there are no images to shrink, so there is \
                     nothing left to give. Splitting it into fewer pages is the realistic option."
                        .into(),
                );
            }
            _ => {
                if !self.opts.allow_grayscale {
                    out.push(
                        "Allow greyscale — on a scan that typically saves another third.".into(),
                    );
                }
                if self.opts.min_dpi > 72 {
                    out.push(format!(
                        "Lower the minimum resolution below {} DPI.",
                        self.opts.min_dpi
                    ));
                }
                let skipped = self.report.images.iter().filter(|i| !i.recompressible).count();
                if skipped > 0 {
                    out.push(format!(
                        "{skipped} image{} can't be re-encoded safely and were left alone.",
                        if skipped == 1 { "" } else { "s" }
                    ));
                }
                out.push("Split the document, or remove pages you don't need to send.".into());
            }
        }
        out.push(
            "Kitbox won't flatten the pages into images to hit a number — that would hit it, \
             but your text would stop being text."
                .into(),
        );
        out
    }
}

fn prefers(bias: Bias, candidate: PdfKnob, incumbent: PdfKnob) -> bool {
    match bias {
        // Resolution is legibility on a scan, so hold DPI and spend quality.
        Bias::KeepSharp => candidate.dpi > incumbent.dpi,
        Bias::ShrinkPixels => candidate.quality > incumbent.quality,
        Bias::Balanced => {
            let score = |k: PdfKnob| k.dpi as u32 + k.quality as u32 * 2;
            score(candidate) > score(incumbent)
        }
    }
}

/// Bytes in, bytes at the requested size out.
pub fn compress_to_target(
    bytes: &[u8],
    target: SizeTarget,
    bias: Bias,
    opts: PdfCompressOptions,
) -> Result<Outcome> {
    let compressor = PdfCompressor::new(bytes, opts)?;
    solve(&compressor, target, bias)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::size::parse_target;
    use crate::testkit;

    fn pages(bytes: &[u8]) -> usize {
        Document::load_mem(bytes).unwrap().page_iter().count()
    }

    #[test]
    fn lossless_alone_is_used_when_it_is_enough() {
        let pdf = testkit::pdf_with_image(800, 600, 400.0, 300.0, 90);
        let generous = SizeTarget::AtMost(pdf.len() as u64 * 2);
        let out = compress_to_target(&pdf, generous, Bias::Balanced, Default::default()).unwrap();
        let solved = out.solved().expect("solvable");
        assert_eq!(solved.probes, 1, "a file that already fits costs one encode");
        assert!(solved.knob.contains("untouched"));
    }

    #[test]
    fn shrinks_a_scan_into_a_range() {
        let pdf = testkit::pdf_scan(4, 1700, 2200, 85);
        let target = parse_target("150-250kb").unwrap();
        let out = compress_to_target(&pdf, target, Bias::KeepSharp, Default::default()).unwrap();
        let solved = out.solved().expect("a scan should be compressible");
        assert!(
            target.contains(solved.size),
            "{} bytes is outside 150–250 KB ({})",
            solved.size,
            solved.knob
        );
        assert_eq!(pages(&solved.bytes), 4, "every page must survive");
        assert!(solved.probes <= 9, "took {} probes", solved.probes);
    }

    #[test]
    fn output_is_a_readable_pdf_with_its_images_intact() {
        let pdf = testkit::pdf_with_image(1600, 1200, 612.0, 459.0, 92);
        let out = compress_to_target(
            &pdf,
            SizeTarget::AtMost(80_000),
            Bias::Balanced,
            Default::default(),
        )
        .unwrap();
        let bytes = out.bytes().to_vec();
        let report = super::super::inspect::inspect(&bytes).unwrap();
        assert_eq!(report.pages, 1);
        assert_eq!(report.images.len(), 1, "the image should still be there");
        assert!(report.images[0].width < 1600, "it should have been downsampled");
        assert!(report.images[0].dpi.is_some(), "and still be placed on the page");
    }

    #[test]
    fn text_only_documents_are_refused_honestly() {
        let pdf = testkit::pdf_text_only();
        // Far below anything repacking can reach.
        let out =
            compress_to_target(&pdf, SizeTarget::AtMost(200), Bias::Balanced, Default::default())
                .unwrap();
        match out {
            Outcome::Infeasible { suggestions, .. } => {
                assert!(
                    suggestions.iter().any(|s| s.contains("no images to shrink")),
                    "should explain why: {suggestions:?}"
                );
                assert!(
                    suggestions.iter().any(|s| s.contains("stop being text")),
                    "should say what it refuses to do: {suggestions:?}"
                );
            }
            other => panic!("expected an honest refusal, got {other:?}"),
        }
    }

    #[test]
    fn keeps_text_selectable_while_shrinking_a_mixed_document() {
        let pdf = testkit::pdf_with_image(1400, 1000, 500.0, 357.0, 90);
        let out = compress_to_target(
            &pdf,
            SizeTarget::AtMost(60_000),
            Bias::Balanced,
            Default::default(),
        )
        .unwrap();
        let doc = Document::load_mem(out.bytes()).unwrap();
        // The content stream still drives the page; nothing was flattened.
        let page = doc.page_iter().next().unwrap();
        let content = doc.get_page_content(page);
        assert!(
            lopdf::content::Content::decode(&content).is_ok(),
            "page content should still be real operators"
        );
    }

    #[test]
    fn encoding_an_attempt_does_not_measure_it() {
        // The solver throws most attempts away. Measuring every one of them
        // cost about 300 ms of each 450 ms probe — two thirds of the time a
        // scan took — to produce nine numbers nobody ever saw.
        let pdf = testkit::pdf_with_image(1200, 900, 540.0, 405.0, 92);
        let compressor = PdfCompressor::new(&pdf, Default::default()).unwrap();
        let knob = PdfKnob { lossless_only: false, dpi: 150, quality: 60, grayscale: false };

        let encoded = compressor.encode(&knob).unwrap();
        assert!(
            compressor.scores.lock().unwrap().is_empty(),
            "encoding should not have measured anything"
        );

        // And the measurement still happens when the answer is asked for —
        // worked out from the finished document rather than remembered.
        let score = compressor.score(&encoded).expect("a score when it is wanted");
        assert!((0.0..=1.0).contains(&score));
        assert_eq!(compressor.scores.lock().unwrap().len(), 1, "and only once");
    }

    #[test]
    fn reports_a_quality_score_for_the_dominant_image() {
        let pdf = testkit::pdf_with_image(1200, 900, 540.0, 405.0, 92);
        let out = compress_to_target(
            &pdf,
            SizeTarget::AtMost(70_000),
            Bias::Balanced,
            Default::default(),
        )
        .unwrap();
        let score = out.solved().unwrap().score.expect("a score for the main image");
        assert!((0.0..=1.0).contains(&score), "score out of range: {score}");
    }
}
