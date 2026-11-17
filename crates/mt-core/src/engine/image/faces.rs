//! Finding faces and covering them up.
//!
//! The detector is [UltraFace](https://github.com/onnx/models) (RFB-320, MIT),
//! run through `rten` — the same runtime OCR, the background remover and the
//! upscaler already use. 1.2 MB and about seven milliseconds a picture.
//!
//! # A missed face is the whole failure
//!
//! Everything else in this crate fails visibly: a bad crop looks wrong, a bad
//! OCR reads wrong. This one fails *invisibly and in the direction that
//! matters*. You publish the photo believing the faces are covered, and one of
//! them isn't, because it was in profile or too small or half behind someone's
//! shoulder.
//!
//! So this module is built to be checked rather than trusted. [`find`] returns
//! what it found with confidences so a caller can show the boxes before
//! covering anything, the report always says how many were found, and the note
//! says plainly that a count is not a guarantee. The screen shows the boxes.
//!
//! # Covering up
//!
//! A solid box destroys the pixels underneath and nothing brings them back.
//! Blur and pixelation are softer-looking and weaker: a light blur can be
//! partly undone, and pixelation at a coarse block size has been reversed
//! before now by searching for an image that pixelates to the same blocks. The
//! defaults here are heavy for that reason, and the report says which of the
//! three is the one that actually destroys.

use image::{imageops::FilterType, DynamicImage, GenericImageView, RgbImage};
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub struct Model {
    inner: rten::Model,
}

impl Model {
    pub fn load(bytes: Vec<u8>) -> Result<Model> {
        let inner = rten::Model::load(bytes)
            .map_err(|e| Error::invalid(format!("the face-detection model wouldn't load: {e}")))?;
        Ok(Model { inner })
    }
}

/// A face, in the picture's own pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Face {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// 0.0–1.0. Shown so a caller can tell a certain face from a maybe.
    pub confidence: f32,
}

/// What goes over a face.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Cover {
    /// Heavy blur. Looks natural; the weakest of the three.
    Blur,
    /// Mosaic blocks. Also recoverable in principle.
    Pixelate,
    /// A filled rectangle. The only one that certainly destroys what was
    /// underneath.
    Box(#[serde(default)] [u8; 3]),
}

impl Default for Cover {
    fn default() -> Self {
        Cover::Blur
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurOptions {
    pub cover: Cover,
    /// Ignore anything the detector is less sure of than this, 0.0–1.0.
    pub confidence: f32,
    /// Grow each face box by this fraction before covering, so an ear or a
    /// chin at the edge doesn't survive.
    pub grow: f32,
    /// How hard to blur or how coarse to pixelate, 0.0–1.0.
    pub strength: f32,
}

impl Default for BlurOptions {
    fn default() -> Self {
        BlurOptions { cover: Cover::Blur, confidence: 0.7, grow: 0.15, strength: 0.8 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurReport {
    pub faces: Vec<Face>,
    pub covered: u32,
    /// Always present: what a count does and doesn't promise.
    pub note: String,
    pub warnings: Vec<String>,
}

pub const NOT_A_GUARANTEE: &str =
    "Finding faces is a guess, and the one it misses is the one that matters. Profiles, small \
     faces, and faces partly hidden are all missed routinely. Check every face is boxed before \
     you share the picture — the count is not a promise.";

pub const ONLY_A_BOX_DESTROYS: &str =
    "A solid box destroys the pixels underneath. Blur and pixelation only disguise them, and \
     both have been undone before — use a box where it really matters.";

/// The size the detector was built for.
const NET_W: usize = 320;
const NET_H: usize = 240;

/// Find the faces, without changing anything.
pub fn find(bytes: &[u8], model: &Model, confidence: f32) -> Result<Vec<Face>> {
    let image = image::load_from_memory(bytes)?;
    detect(&image, model, confidence)
}

/// Find the faces and cover them.
pub fn blur_faces(
    bytes: &[u8],
    model: &Model,
    options: &BlurOptions,
) -> Result<(DynamicImage, BlurReport)> {
    if !(0.05..=0.99).contains(&options.confidence) {
        return Err(Error::invalid("the confidence has to be between 0.05 and 0.99"));
    }
    if !(0.0..=1.0).contains(&options.strength) {
        return Err(Error::invalid("strength has to be between 0 and 1"));
    }
    if !(0.0..=1.0).contains(&options.grow) {
        return Err(Error::invalid("the margin has to be between 0 and 1"));
    }

    let source = image::load_from_memory(bytes)?;
    let faces = detect(&source, model, options.confidence)?;
    let covered = cover(&source, &faces, options);

    let mut warnings = Vec::new();
    if faces.is_empty() {
        warnings.push(
            "No faces were found, so nothing was covered. If there is a face in this picture, \
             lower the confidence — and check the result either way."
                .into(),
        );
    }
    if matches!(options.cover, Cover::Blur | Cover::Pixelate) {
        warnings.push(ONLY_A_BOX_DESTROYS.into());
    }

    let report = BlurReport {
        covered: faces.len() as u32,
        faces,
        note: NOT_A_GUARANTEE.to_string(),
        warnings,
    };
    Ok((covered, report))
}

/* ---------- detection ---------- */

/// Run the detector over one picture.
///
/// The network takes a fixed 320×240, so the picture is letterboxed into that
/// rather than stretched: squashing a portrait into 4:3 distorts every face in
/// it and the detector starts missing them.
fn detect(source: &DynamicImage, model: &Model, confidence: f32) -> Result<Vec<Face>> {
    let (w, h) = (source.width(), source.height());
    if w < 16 || h < 16 {
        return Err(Error::invalid("that image is too small to find a face in"));
    }

    let scale = (NET_W as f32 / w as f32).min(NET_H as f32 / h as f32);
    let (fit_w, fit_h) =
        (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    let resized = source.resize_exact(fit_w, fit_h, FilterType::Triangle).to_rgb8();

    // Grey padding rather than black: a hard black border reads as an edge
    // and the detector finds things along it.
    let mut canvas = RgbImage::from_pixel(NET_W as u32, NET_H as u32, image::Rgb([127, 127, 127]));
    let (pad_x, pad_y) = ((NET_W as u32 - fit_w) / 2, (NET_H as u32 - fit_h) / 2);
    image::imageops::replace(&mut canvas, &resized, pad_x as i64, pad_y as i64);

    let mut input = NdTensor::zeros([1, 3, NET_H, NET_W]);
    for y in 0..NET_H {
        for x in 0..NET_W {
            let px = canvas.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                // The preprocessing this model was trained with.
                input[[0, c, y, x]] = (px[c] as f32 - 127.0) / 128.0;
            }
        }
    }

    let outputs = model
        .inner
        .run(
            vec![(model.inner.input_ids()[0], input.view().into())],
            model.inner.output_ids(),
            None,
        )
        .map_err(|e| Error::other(format!("the face detector couldn't be run: {e}")))?;
    if outputs.len() < 2 {
        return Err(Error::other("the face detector returned less than it should"));
    }

    let mut parts = outputs.into_iter();
    let scores: NdTensor<f32, 3> = parts
        .next()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| Error::other("the detector's confidences weren't numbers"))?;
    let boxes: NdTensor<f32, 3> = parts
        .next()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| Error::other("the detector's boxes weren't numbers"))?;

    let count = scores.shape()[1].min(boxes.shape()[1]);
    let mut found: Vec<Face> = Vec::new();
    for i in 0..count {
        // Column 1 is "this is a face"; column 0 is "this is not".
        let score = scores[[0, i, 1]];
        if score < confidence {
            continue;
        }
        // Already in 0..1 of the letterboxed canvas, and able to spill past
        // the edges, so they are pulled back in.
        let x0 = boxes[[0, i, 0]].clamp(0.0, 1.0) * NET_W as f32;
        let y0 = boxes[[0, i, 1]].clamp(0.0, 1.0) * NET_H as f32;
        let x1 = boxes[[0, i, 2]].clamp(0.0, 1.0) * NET_W as f32;
        let y1 = boxes[[0, i, 3]].clamp(0.0, 1.0) * NET_H as f32;

        // Back out of the letterbox into the original picture.
        let to_x = |v: f32| ((v - pad_x as f32) / scale).clamp(0.0, w as f32);
        let to_y = |v: f32| ((v - pad_y as f32) / scale).clamp(0.0, h as f32);
        let (left, top, right, bottom) = (to_x(x0), to_y(y0), to_x(x1), to_y(y1));
        if right <= left || bottom <= top {
            continue;
        }

        found.push(Face {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
            confidence: score,
        });
    }

    Ok(suppress(found))
}

/// Keep the best of each cluster of overlapping boxes.
///
/// The detector offers thousands of candidates and a real face wins several of
/// them at once; without this one face gets covered eight times over, and the
/// count in the report is nonsense.
fn suppress(mut faces: Vec<Face>) -> Vec<Face> {
    faces.sort_by(|a, b| {
        b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut kept: Vec<Face> = Vec::new();
    for face in faces {
        if kept.iter().any(|other| overlap(&face, other) > 0.4) {
            continue;
        }
        kept.push(face);
    }
    kept
}

/// Intersection over union of two boxes.
fn overlap(a: &Face, b: &Face) -> f32 {
    let ax1 = (a.x + a.width) as f32;
    let ay1 = (a.y + a.height) as f32;
    let bx1 = (b.x + b.width) as f32;
    let by1 = (b.y + b.height) as f32;

    let ix = (ax1.min(bx1) - (a.x.max(b.x)) as f32).max(0.0);
    let iy = (ay1.min(by1) - (a.y.max(b.y)) as f32).max(0.0);
    let overlap = ix * iy;
    let union = (a.width * a.height) as f32 + (b.width * b.height) as f32 - overlap;
    if union <= 0.0 {
        0.0
    } else {
        overlap / union
    }
}

/* ---------- covering ---------- */

/// Paint over each face.
pub fn cover(source: &DynamicImage, faces: &[Face], options: &BlurOptions) -> DynamicImage {
    let mut out = source.to_rgb8();
    let (w, h) = (out.width(), out.height());

    for face in faces {
        // Grown first: a box drawn tightly round the features leaves the ears,
        // the chin and the hairline, which is plenty to recognise someone by.
        let grow_x = (face.width as f32 * options.grow) as u32;
        let grow_y = (face.height as f32 * options.grow) as u32;
        let x0 = face.x.saturating_sub(grow_x);
        let y0 = face.y.saturating_sub(grow_y);
        let x1 = (face.x + face.width + grow_x).min(w);
        let y1 = (face.y + face.height + grow_y).min(h);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (bw, bh) = (x1 - x0, y1 - y0);

        match options.cover {
            Cover::Box(colour) => {
                for y in y0..y1 {
                    for x in x0..x1 {
                        out.put_pixel(x, y, image::Rgb(colour));
                    }
                }
            }
            Cover::Pixelate => {
                // How many blocks across the face — not how big a block is —
                // so the effect is the same whatever size the face was.
                //
                // The range is deliberately coarse. The first version ran from
                // 26 blocks down to 6 and the faces were still perfectly
                // recognisable at the default: a face needs to be reduced to a
                // handful of blocks before pixelation hides anything at all,
                // which is the same reason this is the weak option.
                let blocks = (12.0 - options.strength * 8.0).max(3.0);
                let size = ((bw.min(bh) as f32 / blocks).round() as u32).max(2);
                for by in (y0..y1).step_by(size as usize) {
                    for bx in (x0..x1).step_by(size as usize) {
                        let ex = (bx + size).min(x1);
                        let ey = (by + size).min(y1);
                        let mut total = [0u32; 3];
                        let mut n = 0u32;
                        for y in by..ey {
                            for x in bx..ex {
                                let px = out.get_pixel(x, y).0;
                                for c in 0..3 {
                                    total[c] += px[c] as u32;
                                }
                                n += 1;
                            }
                        }
                        if n == 0 {
                            continue;
                        }
                        let mean =
                            image::Rgb([(total[0] / n) as u8, (total[1] / n) as u8, (total[2] / n) as u8]);
                        for y in by..ey {
                            for x in bx..ex {
                                out.put_pixel(x, y, mean);
                            }
                        }
                    }
                }
            }
            Cover::Blur => {
                // Radius from the face's size: a fixed radius leaves a big
                // face perfectly readable and turns a small one to mush.
                let radius = (bw.min(bh) as f32 * (0.08 + options.strength * 0.22)).max(2.0);
                let patch = DynamicImage::ImageRgb8(out.view(x0, y0, bw, bh).to_image());
                let blurred = patch.blur(radius).to_rgb8();
                for y in 0..bh {
                    for x in 0..bw {
                        out.put_pixel(x0 + x, y0 + y, *blurred.get_pixel(x, y));
                    }
                }
            }
        }
    }

    DynamicImage::ImageRgb8(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x: u32, y: u32, size: u32, confidence: f32) -> Face {
        Face { x, y, width: size, height: size, confidence }
    }

    /// A picture with a loud patch where a "face" is, so covering it is
    /// measurable.
    fn picture() -> DynamicImage {
        let mut rgb = RgbImage::from_pixel(200, 200, image::Rgb([30, 120, 60]));
        for y in 40..120 {
            for x in 40..120 {
                // A chequer, so a blur or a mosaic has something to flatten.
                let on = ((x / 4) + (y / 4)) % 2 == 0;
                rgb.put_pixel(
                    x,
                    y,
                    if on { image::Rgb([250, 250, 250]) } else { image::Rgb([5, 5, 5]) },
                );
            }
        }
        DynamicImage::ImageRgb8(rgb)
    }

    #[test]
    fn one_face_winning_several_boxes_is_counted_once() {
        // The detector offers thousands of candidates and a real face wins a
        // cluster of them. Without this the same face is covered eight times
        // and the count in the report is nonsense.
        let cluster = vec![
            face(100, 100, 50, 0.99),
            face(103, 98, 52, 0.95),
            face(98, 104, 48, 0.91),
            face(400, 400, 50, 0.88),
        ];
        let kept = suppress(cluster);
        assert_eq!(kept.len(), 2, "one per face: {kept:?}");
        assert!((kept[0].confidence - 0.99).abs() < 1e-6, "the best of the cluster survives");
    }

    #[test]
    fn two_faces_side_by_side_stay_two() {
        let pair = vec![face(0, 0, 50, 0.9), face(60, 0, 50, 0.9)];
        assert_eq!(suppress(pair).len(), 2);
    }

    #[test]
    fn overlap_is_nothing_for_separate_boxes_and_everything_for_identical_ones() {
        assert!((overlap(&face(0, 0, 10, 1.0), &face(0, 0, 10, 1.0)) - 1.0).abs() < 1e-6);
        assert_eq!(overlap(&face(0, 0, 10, 1.0), &face(100, 100, 10, 1.0)), 0.0);
        let half = overlap(&face(0, 0, 10, 1.0), &face(5, 0, 10, 1.0));
        assert!(half > 0.3 && half < 0.4, "{half}");
    }

    #[test]
    fn a_box_replaces_every_pixel_under_it() {
        let options = BlurOptions { cover: Cover::Box([0, 0, 0]), grow: 0.0, ..Default::default() };
        let out = cover(&picture(), &[face(40, 40, 80, 0.9)], &options).to_rgb8();
        for y in 40..120 {
            for x in 40..120 {
                assert_eq!(out.get_pixel(x, y).0, [0, 0, 0], "at {x},{y}");
            }
        }
        // And nothing outside it.
        assert_eq!(out.get_pixel(10, 10).0, [30, 120, 60]);
    }

    #[test]
    fn blurring_flattens_the_detail_without_touching_the_rest() {
        let options = BlurOptions { cover: Cover::Blur, grow: 0.0, ..Default::default() };
        let before = picture().to_rgb8();
        let after = cover(&picture(), &[face(40, 40, 80, 0.9)], &options).to_rgb8();

        let spread = |img: &RgbImage| {
            let values: Vec<f32> = (45..115)
                .flat_map(|y| (45..115).map(move |x| (x, y)))
                .map(|(x, y)| img.get_pixel(x, y).0[0] as f32)
                .collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32
        };
        assert!(spread(&after) < spread(&before) * 0.25, "the face should lose its detail");
        assert_eq!(after.get_pixel(10, 10).0, [30, 120, 60], "the rest is untouched");
    }

    #[test]
    fn pixelation_is_coarse_enough_to_actually_hide_a_face() {
        // The first version left ten blocks across a face and the faces were
        // perfectly recognisable, which defeats the entire tool.
        let options =
            BlurOptions { cover: Cover::Pixelate, grow: 0.0, strength: 0.8, ..Default::default() };
        let out = cover(&picture(), &[face(40, 40, 80, 0.9)], &options).to_rgb8();

        // How many distinct colours survive in the covered area: a mosaic of a
        // handful of blocks cannot have many.
        let mut seen = std::collections::HashSet::new();
        for y in 40..120 {
            for x in 40..120 {
                seen.insert(out.get_pixel(x, y).0);
            }
        }
        assert!(seen.len() <= 64, "too much detail survived: {} colours", seen.len());
    }

    #[test]
    fn the_box_is_grown_so_a_chin_does_not_survive() {
        let tight = BlurOptions { cover: Cover::Box([0, 0, 0]), grow: 0.0, ..Default::default() };
        let roomy = BlurOptions { cover: Cover::Box([0, 0, 0]), grow: 0.25, ..Default::default() };
        let black = |o: &BlurOptions| {
            cover(&picture(), &[face(60, 60, 40, 0.9)], o)
                .to_rgb8()
                .pixels()
                .filter(|p| p.0 == [0, 0, 0])
                .count()
        };
        assert!(black(&roomy) > black(&tight), "a margin has to cover more");
    }

    #[test]
    fn covering_nothing_changes_nothing() {
        let out = cover(&picture(), &[], &BlurOptions::default()).to_rgb8();
        assert_eq!(out.as_raw(), picture().to_rgb8().as_raw());
    }

    #[test]
    fn a_face_at_the_very_edge_does_not_run_off_it() {
        let options = BlurOptions { cover: Cover::Box([1, 2, 3]), grow: 0.5, ..Default::default() };
        // Deliberately against the corner, so growing it would go negative.
        let out = cover(&picture(), &[face(0, 0, 30, 0.9)], &options).to_rgb8();
        assert_eq!(out.get_pixel(0, 0).0, [1, 2, 3]);
        assert_eq!((out.width(), out.height()), (200, 200));
    }

    #[test]
    fn every_report_says_a_count_is_not_a_promise() {
        assert!(NOT_A_GUARANTEE.contains("not a promise"));
        assert!(NOT_A_GUARANTEE.contains("Profiles"), "it should name what gets missed");
        assert!(ONLY_A_BOX_DESTROYS.contains("only disguise"));
    }

    #[test]
    fn rubbish_bytes_are_not_a_model() {
        let Err(error) = Model::load(b"not a model".to_vec()) else {
            panic!("rubbish bytes are not a model");
        };
        assert!(error.to_string().contains("face-detection model"), "{error}");
    }

    #[test]
    fn the_options_cross_as_the_ui_writes_them() {
        let options: BlurOptions = serde_json::from_str(
            r#"{ "cover": { "kind": "blur" }, "confidence": 0.7, "grow": 0.15, "strength": 0.8 }"#,
        )
        .expect("options");
        assert_eq!(options.cover, Cover::Blur);

        let boxed: BlurOptions = serde_json::from_str(
            r#"{ "cover": { "kind": "box", "value": [20, 20, 24] },
                  "confidence": 0.5, "grow": 0.2, "strength": 1 }"#,
        )
        .expect("a box");
        assert_eq!(boxed.cover, Cover::Box([20, 20, 24]));

        let pixels: BlurOptions = serde_json::from_str(
            r#"{ "cover": { "kind": "pixelate" }, "confidence": 0.7, "grow": 0.15, "strength": 0.8 }"#,
        )
        .expect("pixelate");
        assert_eq!(pixels.cover, Cover::Pixelate);
    }

    /// The detector itself, when the model is on this machine.
    #[test]
    #[ignore = "needs the face model; set KITBOX_FACE_MODEL and KITBOX_FACE_PHOTO"]
    fn finds_the_faces_in_a_picture_of_faces() {
        let Ok(path) = std::env::var("KITBOX_FACE_MODEL") else {
            panic!("set KITBOX_FACE_MODEL to the model file");
        };
        let Ok(photo) = std::env::var("KITBOX_FACE_PHOTO") else {
            panic!("set KITBOX_FACE_PHOTO to a picture with faces in it");
        };
        let model = Model::load(std::fs::read(path).unwrap()).unwrap();
        let bytes = std::fs::read(photo).unwrap();

        let faces = find(&bytes, &model, 0.7).unwrap();
        assert!(!faces.is_empty(), "it should find at least one");
        for f in &faces {
            assert!(f.width > 0 && f.height > 0, "{f:?}");
            assert!(f.confidence >= 0.7, "{f:?}");
        }
        // And nothing stacked on top of anything else.
        for (i, a) in faces.iter().enumerate() {
            for b in &faces[i + 1..] {
                assert!(overlap(a, b) <= 0.4, "{a:?} and {b:?} are the same face twice");
            }
        }
    }
}
