//! Image compression, driven by a size target rather than a quality guess.
//!
//! The encoder exposes a ladder of (scale, quality) settings to the solver,
//! which measures real encodes until it finds the largest output inside the
//! requested window. Resized copies are cached across probes, so walking the
//! ladder costs re-encodes, not re-resizes.

use std::collections::HashMap;
use std::sync::Mutex;

use image::{imageops::FilterType, DynamicImage};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::format::FileKind;
use crate::quality;
use crate::target::{solve, Bias, Encodable, Outcome, SizeTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// Keep the source format where we can encode it, otherwise fall back to JPG.
    #[default]
    Keep,
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompressOptions {
    pub format: OutputFormat,
    /// Drop EXIF, GPS and colour profiles. On by default in the UI: form uploads
    /// should not carry the location the photo was taken.
    pub strip_metadata: bool,
    /// Cap the longest edge before solving. Applied as a hard constraint.
    pub max_dimension: Option<u32>,
    /// Force exact output dimensions — how form presets pin "200×230 px".
    /// When set, the solver may only trade quality, never pixels.
    pub exact_dimensions: Option<(u32, u32)>,
}

/// One rung of the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageKnob {
    /// Percent of the (already constrained) working size, 10–100.
    pub scale: u16,
    /// Encoder quality 10–100. Ignored for PNG output, which is lossless.
    pub quality: u8,
}

pub struct ImageCompressor {
    source: DynamicImage,
    /// Source after `max_dimension` / `exact_dimensions` are applied. All scale
    /// percentages are relative to this.
    working: DynamicImage,
    output: FileKind,
    opts: CompressOptions,
    /// Resized copies, keyed by scale percent.
    resized: Mutex<HashMap<u16, DynamicImage>>,
}

impl ImageCompressor {
    pub fn new(bytes: &[u8], opts: CompressOptions) -> Result<Self> {
        let kind = FileKind::sniff(bytes);
        if !kind.is_image() {
            return Err(Error::unsupported(format!(
                "{} is not an image this tool can read",
                kind.label()
            )));
        }
        let source = image::load_from_memory(bytes)?;
        if source.width() == 0 || source.height() == 0 {
            return Err(Error::invalid("image has no pixels"));
        }

        let output = super::encode::resolve_kind(kind, opts.format, &source);

        let working = constrain(&source, &opts);

        Ok(ImageCompressor { source, working, output, opts, resized: Mutex::new(HashMap::new()) })
    }

    pub fn output_kind(&self) -> FileKind {
        self.output
    }

    pub fn source_dimensions(&self) -> (u32, u32) {
        (self.source.width(), self.source.height())
    }

    /// Dimensions the given rung produces.
    pub fn dimensions_at(&self, knob: &ImageKnob) -> (u32, u32) {
        scaled_dimensions(&self.working, knob.scale)
    }

    fn at_scale(&self, scale: u16) -> DynamicImage {
        if scale >= 100 {
            return self.working.clone();
        }
        if let Some(cached) = self.resized.lock().unwrap().get(&scale) {
            return cached.clone();
        }
        let (w, h) = scaled_dimensions(&self.working, scale);
        let resized = self.working.resize_exact(w, h, FilterType::Lanczos3);
        self.resized.lock().unwrap().insert(scale, resized.clone());
        resized
    }

    fn encode_bytes(&self, img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
        super::encode::encode(img, self.output, quality, self.opts.strip_metadata)
    }
}

impl Encodable for ImageCompressor {
    type Knob = ImageKnob;

    fn ladder(&self, bias: Bias) -> Vec<ImageKnob> {
        // Exact dimensions are a hard requirement from a form preset, so pixels
        // are off the table and only quality may move.
        let scales: Vec<u16> = if self.opts.exact_dimensions.is_some() {
            vec![100]
        } else {
            vec![100, 90, 80, 70, 60, 50, 42, 35, 28, 22, 16, 12]
        };
        let qualities: Vec<u8> = if self.output == FileKind::Png {
            // PNG is lossless; the only lever is pixel count.
            vec![100]
        } else {
            (10..=98).step_by(2).map(|q| q as u8).collect()
        };

        // Rungs that would produce near-identical file sizes are wasted probes,
        // so candidates are bucketed by predicted size and the bias picks the
        // winner within each bucket.
        let mut buckets: std::collections::BTreeMap<i64, ImageKnob> = Default::default();
        for &scale in &scales {
            for &q in &qualities {
                let knob = ImageKnob { scale, quality: q };
                let est = predicted_ratio(scale, q, self.output);
                let bucket = (est.ln() / 1.06f64.ln()).round() as i64;
                match buckets.get(&bucket) {
                    Some(existing) if !prefers(bias, knob, *existing) => {}
                    _ => {
                        buckets.insert(bucket, knob);
                    }
                }
            }
        }
        buckets.into_values().collect()
    }

    fn encode(&self, knob: &ImageKnob) -> Result<Vec<u8>> {
        let img = self.at_scale(knob.scale);
        self.encode_bytes(&img, knob.quality)
    }

    fn describe(&self, knob: &ImageKnob) -> String {
        let (w, h) = self.dimensions_at(knob);
        let size = if knob.scale >= 100 {
            format!("{w}×{h}")
        } else {
            format!("{w}×{h} ({}% of original)", knob.scale)
        };
        if self.output == FileKind::Png {
            format!("PNG, {size}")
        } else {
            format!("quality {}, {size}", knob.quality)
        }
    }

    fn score(&self, encoded: &[u8]) -> Option<f32> {
        let decoded = image::load_from_memory(encoded).ok()?;
        // Compare against the *working* image, not the original file. When the
        // user pinned 200×230 for a form, cropping to that is what they asked
        // for, not damage — scoring against the full-size original would blame
        // the encoder for the crop. Any resizing the solver chose itself is
        // still counted, because `ssim` scales the result back up first.
        Some(quality::ssim(&self.working, &decoded))
    }

    fn suggestions(&self, _target: SizeTarget) -> Vec<String> {
        let mut out = Vec::new();
        if self.output == FileKind::Png {
            out.push("Switch the output format to JPG — PNG is lossless, so it can only get smaller by losing pixels.".into());
        }
        if self.opts.exact_dimensions.is_some() {
            out.push(
                "The dimensions are pinned by the preset, so quality is the only lever left."
                    .into(),
            );
        } else {
            out.push("Allow smaller dimensions, or crop to just the part you need.".into());
        }
        out.push("Convert to grayscale if colour isn't required.".into());
        out
    }
}

/// One-call convenience: bytes in, bytes at the requested size out.
pub fn compress_to_target(
    bytes: &[u8],
    target: SizeTarget,
    bias: Bias,
    opts: CompressOptions,
) -> Result<Outcome> {
    let compressor = ImageCompressor::new(bytes, opts)?;
    solve(&compressor, target, bias)
}

/// Rough size model used only to order and thin the ladder. Real encodes decide
/// everything that matters; this just stops the solver wasting probes on rungs
/// that would come out the same size.
fn predicted_ratio(scale: u16, quality: u8, output: FileKind) -> f64 {
    let pixels = (scale as f64 / 100.0).powi(2);
    if output == FileKind::Png {
        return pixels;
    }
    let q = quality as f64 / 100.0;
    pixels * (0.08 + 0.42 * q * q)
}

/// Within a bucket of same-size candidates, which one does this bias want?
fn prefers(bias: Bias, candidate: ImageKnob, incumbent: ImageKnob) -> bool {
    match bias {
        Bias::KeepSharp => candidate.scale > incumbent.scale,
        Bias::ShrinkPixels => candidate.quality > incumbent.quality,
        // Split the difference: favour keeping pixels, but not at any cost to
        // quality — a 20% image at quality 95 is not a good trade.
        Bias::Balanced => {
            let score = |k: ImageKnob| k.scale as u32 * 2 + k.quality as u32;
            score(candidate) > score(incumbent)
        }
    }
}

fn scaled_dimensions(img: &DynamicImage, scale: u16) -> (u32, u32) {
    let f = scale as f64 / 100.0;
    let w = ((img.width() as f64 * f).round() as u32).max(1);
    let h = ((img.height() as f64 * f).round() as u32).max(1);
    (w, h)
}

fn constrain(source: &DynamicImage, opts: &CompressOptions) -> DynamicImage {
    if let Some((w, h)) = opts.exact_dimensions {
        // Cover-fit then centre-crop, so the subject isn't stretched.
        return cover_crop(source, w.max(1), h.max(1));
    }
    if let Some(cap) = opts.max_dimension {
        let longest = source.width().max(source.height());
        if cap > 0 && longest > cap {
            return source.resize(cap, cap, FilterType::Lanczos3);
        }
    }
    source.clone()
}

/// Scale to cover the target box, then crop the overflow evenly.
fn cover_crop(src: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    let scale = (w as f64 / src.width() as f64).max(h as f64 / src.height() as f64);
    let sw = ((src.width() as f64 * scale).ceil() as u32).max(w);
    let sh = ((src.height() as f64 * scale).ceil() as u32).max(h);
    let resized = src.resize_exact(sw, sh, FilterType::Lanczos3);
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    resized.crop_imm(x, y, w, h)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::size::parse_target;
    use crate::testkit;
    use image::ImageEncoder;

    fn source_jpeg() -> Vec<u8> {
        testkit::encode_jpeg(&testkit::photo(1600, 1200), 95)
    }

    #[test]
    fn hits_an_upper_bound() {
        let src = source_jpeg();
        let target = SizeTarget::AtMost(100_000);
        let out =
            compress_to_target(&src, target, Bias::Balanced, CompressOptions::default()).unwrap();
        let s = out.solved().expect("should be solvable");
        assert!(target.contains(s.size), "got {} bytes", s.size);
        assert!(s.probes <= 10, "took {} probes", s.probes);
    }

    #[test]
    fn hits_a_range_the_way_a_form_asks_for_it() {
        let src = source_jpeg();
        let target = parse_target("50-100kb").unwrap();
        let out =
            compress_to_target(&src, target, Bias::Balanced, CompressOptions::default()).unwrap();
        assert!(out.is_success());
        let s = out.solved().unwrap();
        assert!(target.contains(s.size), "{} bytes is outside 50–100 KB", s.size);
        // The point of taking the largest fitting rung is that we don't end up
        // scraping the bottom of the window.
        assert!(s.size > 80_000, "should use most of the window, got {}", s.size);
    }

    #[test]
    fn returns_the_untouched_file_when_it_already_fits() {
        let src = testkit::encode_jpeg(&testkit::photo(200, 200), 80);
        let target = SizeTarget::AtMost(10_000_000);
        let out =
            compress_to_target(&src, target, Bias::Balanced, CompressOptions::default()).unwrap();
        let s = out.solved().unwrap();
        assert_eq!(s.probes, 1);
        assert!(s.score.unwrap_or(0.0) > 0.95);
    }

    #[test]
    fn keeps_quality_high_when_it_can() {
        let src = source_jpeg();
        let out = compress_to_target(
            &src,
            SizeTarget::AtMost(400_000),
            Bias::Balanced,
            CompressOptions::default(),
        )
        .unwrap();
        let solved = out.solved().unwrap();
        let score = solved.score.unwrap();
        assert!(
            score > 0.93,
            "a generous target should barely degrade the image, got {score} at {}",
            solved.knob
        );
    }

    #[test]
    fn keep_sharp_holds_resolution_where_shrink_pixels_does_not() {
        let src = source_jpeg();
        let target = SizeTarget::AtMost(40_000);
        let sharp = ImageCompressor::new(&src, CompressOptions::default()).unwrap();
        let sharp_out = solve(&sharp, target, Bias::KeepSharp).unwrap();
        let shrink = ImageCompressor::new(&src, CompressOptions::default()).unwrap();
        let shrink_out = solve(&shrink, target, Bias::ShrinkPixels).unwrap();

        let sharp_desc = sharp_out.solved().unwrap().knob.clone();
        let shrink_desc = shrink_out.solved().unwrap().knob.clone();
        assert_ne!(sharp_desc, shrink_desc, "the two biases should diverge under pressure");
    }

    #[test]
    fn exact_dimensions_are_honoured_for_form_presets() {
        let src = source_jpeg();
        let opts = CompressOptions {
            format: OutputFormat::Jpeg,
            exact_dimensions: Some((200, 230)),
            strip_metadata: true,
            ..Default::default()
        };
        let target = parse_target("20-50kb").unwrap();
        let out = compress_to_target(&src, target, Bias::Balanced, opts).unwrap();
        assert!(out.is_success());
        let decoded = image::load_from_memory(out.bytes()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (200, 230));
        assert!(target.contains(out.size()), "{} bytes", out.size());
    }

    #[test]
    fn says_so_instead_of_mangling_an_impossible_target() {
        let src = source_jpeg();
        // 200 bytes cannot hold a photograph.
        let out = compress_to_target(
            &src,
            SizeTarget::AtMost(200),
            Bias::Balanced,
            CompressOptions::default(),
        )
        .unwrap();
        match out {
            Outcome::Infeasible { why, suggestions, .. } => {
                assert!(why.contains("smallest"));
                assert!(!suggestions.is_empty());
            }
            other => panic!("expected an honest refusal, got {other:?}"),
        }
    }

    #[test]
    fn png_with_transparency_stays_png() {
        let mut rgba = image::RgbaImage::new(64, 64);
        for (x, y, p) in rgba.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 4) as u8, (y * 4) as u8, 128, if x < 32 { 0 } else { 255 }]);
        }
        let mut src = Vec::new();
        image::codecs::png::PngEncoder::new(&mut src)
            .write_image(rgba.as_raw(), 64, 64, image::ExtendedColorType::Rgba8)
            .unwrap();

        let c = ImageCompressor::new(&src, CompressOptions::default()).unwrap();
        assert_eq!(c.output_kind(), FileKind::Png);
    }
}
