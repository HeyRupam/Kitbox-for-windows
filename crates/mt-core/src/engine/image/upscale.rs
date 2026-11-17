//! Making a picture bigger.
//!
//! Two ways, because they answer different questions and pretending one tool
//! does both would mislead.
//!
//! **Sharpen** is Lanczos resampling with an unsharp mask. It is instant, has
//! no size limit, needs no download, and invents nothing — it resamples what
//! is already there. For a photo that is slightly too small for a form, this
//! is the right answer and the AI is overkill.
//!
//! **Reconstruct** is Real-ESRGAN, which has learned what detail tends to look
//! like and puts some back. It genuinely produces more than was there, and
//! that is the point and the catch: it is *plausible* detail, not recovered
//! detail. On a face or a licence plate, plausible is not the same as true.
//!
//! # The cost, stated up front
//!
//! Measured on this machine, 16 threads: about **125 seconds per megapixel of
//! input**. A 200×230 form photo is six seconds. A 12-megapixel phone photo
//! would be twenty-five minutes, which is not a thing to let someone start by
//! accident — so the model path refuses inputs above [`MODEL_PIXEL_LIMIT`] and
//! says to crop or shrink first, and [`estimate_seconds`] lets a caller show
//! the wait before anyone commits to it.
//!
//! # Tiling
//!
//! Output is sixteen times the pixels of the input, so a whole-image pass
//! would need a tensor far larger than the picture. The work goes tile by
//! tile, each one grown by a margin that is thrown away afterwards: a model
//! does its worst work at the edge of what it can see, and without the margin
//! every tile boundary leaves a visible seam.

use image::{imageops::FilterType, DynamicImage, GenericImageView, RgbImage};
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The loaded reconstruction model.
pub struct Model {
    inner: rten::Model,
}

impl Model {
    pub fn load(bytes: Vec<u8>) -> Result<Model> {
        let inner = rten::Model::load(bytes)
            .map_err(|e| Error::invalid(format!("the upscaling model wouldn't load: {e}")))?;
        Ok(Model { inner })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Resample and sharpen. Instant, any size, invents nothing.
    #[default]
    Sharpen,
    /// Real-ESRGAN. Slow, limited in size, puts detail back.
    Reconstruct,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpscaleOptions {
    pub method: Method,
    /// How many times bigger, 2–4.
    pub scale: u32,
    /// Extra sharpening for the resampled path, 0.0–1.0.
    pub sharpen: f32,
}

impl Default for UpscaleOptions {
    fn default() -> Self {
        UpscaleOptions { method: Method::Sharpen, scale: 2, sharpen: 0.4 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpscaleReport {
    pub from: (u32, u32),
    pub to: (u32, u32),
    pub method: Method,
    /// How many passes the model made. Zero for the resampled path.
    pub tiles: u32,
    pub seconds: f32,
    pub note: String,
    pub warnings: Vec<String>,
}

/// The most input the model path will accept, in pixels.
///
/// About a megapixel, which is roughly two minutes of work. Past that the wait
/// stops being a wait and starts being a hang.
pub const MODEL_PIXEL_LIMIT: u32 = 1_100_000;

/// Measured cost of the model, in seconds per megapixel of input.
const SECONDS_PER_MEGAPIXEL: f32 = 125.0;

/// What the model's extra detail is and isn't.
pub const INVENTED: &str =
    "Reconstruction adds detail that was not in the picture. It is a plausible guess at what was \
     probably there, which is excellent for a portrait or a logo and misleading for anything \
     being read as evidence — a face, a plate, a serial number. Sharpen only resamples what is \
     already there.";

/// Roughly how long the model path will take on this many pixels.
pub fn estimate_seconds(width: u32, height: u32) -> f32 {
    (width as f32 * height as f32) / 1_000_000.0 * SECONDS_PER_MEGAPIXEL
}

/// Side of each tile fed to the model, in input pixels.
const TILE: u32 = 128;
/// Margin grown around each tile and discarded afterwards.
const MARGIN: u32 = 16;
/// The model's fixed factor.
const MODEL_SCALE: u32 = 4;

/// Enlarge `bytes`.
///
/// `model` is only needed for [`Method::Reconstruct`]; the resampled path
/// ignores it, so a caller with no model downloaded can still offer that.
pub fn upscale(
    bytes: &[u8],
    model: Option<&Model>,
    options: &UpscaleOptions,
) -> Result<(DynamicImage, UpscaleReport)> {
    if !(2..=4).contains(&options.scale) {
        return Err(Error::invalid("the scale has to be 2, 3 or 4"));
    }
    if !(0.0..=1.0).contains(&options.sharpen) {
        return Err(Error::invalid("sharpening has to be between 0 and 1"));
    }
    let source = image::load_from_memory(bytes)?;
    enlarge(&source, model, options)
}

/// The same, on an image already decoded.
pub fn enlarge(
    source: &DynamicImage,
    model: Option<&Model>,
    options: &UpscaleOptions,
) -> Result<(DynamicImage, UpscaleReport)> {
    let (w, h) = (source.width(), source.height());
    if w == 0 || h == 0 {
        return Err(Error::invalid("that image has no pixels"));
    }
    let target = (w.saturating_mul(options.scale), h.saturating_mul(options.scale));
    if target.0 > 20_000 || target.1 > 20_000 {
        return Err(Error::invalid(
            "that would come out over 20,000 pixels on a side — shrink it or use a smaller scale",
        ));
    }

    let started = std::time::Instant::now();
    let mut warnings = Vec::new();

    let (out, tiles) = match options.method {
        Method::Sharpen => (sharpen(source, target, options.sharpen), 0),
        Method::Reconstruct => {
            // Size before model: someone with a huge picture and nothing
            // downloaded is better told it is too big anyway than sent to
            // fetch 70 MB that would refuse it too.
            if w * h > MODEL_PIXEL_LIMIT {
                return Err(Error::unsupported(format!(
                    "this picture is {:.1} megapixels, and reconstruction would take about {:.0} \
                     minutes. Crop it, or shrink it below {:.1} megapixels first — or use \
                     Sharpen, which has no limit.",
                    (w as f32 * h as f32) / 1_000_000.0,
                    estimate_seconds(w, h) / 60.0,
                    MODEL_PIXEL_LIMIT as f32 / 1_000_000.0,
                )));
            }
            let model = model.ok_or_else(|| {
                Error::unsupported("the upscaling model isn't downloaded yet")
            })?;

            let (big, tiles) = reconstruct(source, model)?;
            // The model is fixed at 4×; anything else is resampled back from
            // there, which still beats resampling from the original.
            let out = if options.scale == MODEL_SCALE {
                big
            } else {
                big.resize_exact(target.0, target.1, FilterType::Lanczos3)
            };
            (out, tiles)
        }
    };

    if options.scale == 4 && options.method == Method::Sharpen {
        warnings.push(
            "Four times is a lot to ask of resampling — it will be smooth rather than detailed. \
             Reconstruct is the one that adds detail."
                .into(),
        );
    }

    let report = UpscaleReport {
        from: (w, h),
        to: (out.width(), out.height()),
        method: options.method,
        tiles,
        seconds: started.elapsed().as_secs_f32(),
        note: INVENTED.to_string(),
        warnings,
    };
    Ok((out, report))
}

/// Lanczos, then an unsharp mask to put back the bite resampling takes out.
fn sharpen(source: &DynamicImage, target: (u32, u32), amount: f32) -> DynamicImage {
    let bigger = source.resize_exact(target.0, target.1, FilterType::Lanczos3);
    if amount <= 0.0 {
        return bigger;
    }
    // Radius scaled to the new size, so the effect looks the same whatever
    // the picture started at.
    let radius = (target.0.max(target.1) as f32 / 1000.0).clamp(0.6, 2.4);
    let threshold = 2;
    DynamicImage::ImageRgb8(image::imageops::unsharpen(
        &bigger.to_rgb8(),
        radius,
        (amount * 24.0).round() as i32 + threshold,
    ))
}

/// Run the model over the picture, tile by tile, at 4×.
fn reconstruct(source: &DynamicImage, model: &Model) -> Result<(DynamicImage, u32)> {
    let rgb = source.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let mut out = RgbImage::new(w * MODEL_SCALE, h * MODEL_SCALE);
    let mut tiles = 0u32;

    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            // The tile, grown by a margin that is cut off again afterwards.
            let x0 = x.saturating_sub(MARGIN);
            let y0 = y.saturating_sub(MARGIN);
            let x1 = (x + TILE + MARGIN).min(w);
            let y1 = (y + TILE + MARGIN).min(h);
            let patch = rgb.view(x0, y0, x1 - x0, y1 - y0).to_image();

            let enlarged = run(model, &patch)?;
            tiles += 1;

            // Where the useful middle of this tile sits inside the result.
            let keep_x = (x - x0) * MODEL_SCALE;
            let keep_y = (y - y0) * MODEL_SCALE;
            let keep_w = ((x + TILE).min(w) - x) * MODEL_SCALE;
            let keep_h = ((y + TILE).min(h) - y) * MODEL_SCALE;

            for ty in 0..keep_h {
                for tx in 0..keep_w {
                    let px = *enlarged.get_pixel(keep_x + tx, keep_y + ty);
                    out.put_pixel(x * MODEL_SCALE + tx, y * MODEL_SCALE + ty, px);
                }
            }
            x += TILE;
        }
        y += TILE;
    }

    Ok((DynamicImage::ImageRgb8(out), tiles))
}

/// One pass of the model over one patch.
fn run(model: &Model, patch: &RgbImage) -> Result<RgbImage> {
    let (pw, ph) = (patch.width() as usize, patch.height() as usize);
    let mut input = NdTensor::zeros([1, 3, ph, pw]);
    // Plain 0..1 RGB: this model takes no mean-and-standard-deviation
    // normalisation, and giving it any produces values outside 0..1 that
    // clip to a washed-out picture.
    for y in 0..ph {
        for x in 0..pw {
            let px = patch.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                input[[0, c, y, x]] = px[c] as f32 / 255.0;
            }
        }
    }

    let output = model
        .inner
        .run_one(input.view().into(), None)
        .map_err(|e| Error::other(format!("the model couldn't be run: {e}")))?;
    let tensor: NdTensor<f32, 4> = output
        .try_into()
        .map_err(|_| Error::other("the model returned something unexpected"))?;

    let [_, channels, oh, ow] = tensor.shape();
    if channels < 3 {
        return Err(Error::other("the model returned a picture with no colour"));
    }
    if ow != pw * MODEL_SCALE as usize || oh != ph * MODEL_SCALE as usize {
        return Err(Error::other(format!(
            "the model enlarged {pw}×{ph} to {ow}×{oh}, which isn't the {MODEL_SCALE}× this \
             expects"
        )));
    }

    let mut out = RgbImage::new(ow as u32, oh as u32);
    for y in 0..oh {
        for x in 0..ow {
            let mut px = [0u8; 3];
            for (c, slot) in px.iter_mut().enumerate() {
                *slot = (tensor[[0, c, y, x]].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn small() -> DynamicImage {
        testkit::photo(60, 40)
    }

    #[test]
    fn sharpening_needs_no_model_and_lands_on_the_right_size() {
        // The whole point of offering this path: it works with nothing
        // downloaded, which is the state every new install is in.
        let options = UpscaleOptions { method: Method::Sharpen, scale: 3, sharpen: 0.4 };
        let (out, report) = enlarge(&small(), None, &options).unwrap();
        assert_eq!((out.width(), out.height()), (180, 120));
        assert_eq!(report.to, (180, 120));
        assert_eq!(report.from, (60, 40));
        assert_eq!(report.tiles, 0, "no model, no tiles");
    }

    #[test]
    fn sharpening_at_zero_is_plain_resampling() {
        let plain = UpscaleOptions { method: Method::Sharpen, scale: 2, sharpen: 0.0 };
        let (out, _) = enlarge(&small(), None, &plain).unwrap();
        assert_eq!((out.width(), out.height()), (120, 80));
    }

    #[test]
    fn asking_to_reconstruct_without_the_model_says_so_plainly() {
        let options = UpscaleOptions { method: Method::Reconstruct, scale: 4, sharpen: 0.0 };
        let error = enlarge(&small(), None, &options).unwrap_err().to_string();
        assert!(error.contains("isn't downloaded"), "{error}");
    }

    #[test]
    fn a_picture_too_big_to_reconstruct_is_refused_with_the_arithmetic() {
        // Refusing is kinder than a twenty-five minute wait nobody chose. The
        // message has to carry the numbers, or it is just a no.
        let big = DynamicImage::ImageRgb8(RgbImage::new(2000, 1500));
        let options = UpscaleOptions { method: Method::Reconstruct, scale: 4, sharpen: 0.0 };
        // Checked before the model is consulted, so None is enough to reach it.
        let error = enlarge(&big, None, &options).unwrap_err().to_string();
        assert!(error.contains("megapixels"), "{error}");
        assert!(error.contains("minutes"), "it should say how long: {error}");
        assert!(error.contains("Sharpen"), "it should name the way out: {error}");
    }

    #[test]
    fn the_estimate_is_proportional_to_the_pixels() {
        let one = estimate_seconds(1000, 1000);
        let four = estimate_seconds(2000, 2000);
        assert!((one - SECONDS_PER_MEGAPIXEL).abs() < 1.0, "{one}");
        assert!((four / one - 4.0).abs() < 0.01, "four times the pixels, four times the wait");
    }

    #[test]
    fn four_times_by_resampling_admits_what_it_is() {
        let options = UpscaleOptions { method: Method::Sharpen, scale: 4, sharpen: 0.4 };
        let (_, report) = enlarge(&small(), None, &options).unwrap();
        assert!(
            report.warnings.iter().any(|w| w.contains("smooth rather than detailed")),
            "{:?}",
            report.warnings
        );

        let modest = UpscaleOptions { scale: 2, ..options };
        let (_, report) = enlarge(&small(), None, &modest).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn every_report_distinguishes_invented_detail_from_recovered_detail() {
        let (_, report) = enlarge(&small(), None, &UpscaleOptions::default()).unwrap();
        assert!(report.note.contains("plausible guess"), "{}", report.note);
        assert!(report.note.contains("evidence"), "{}", report.note);
    }

    #[test]
    fn nonsense_scales_are_refused() {
        let encoded = testkit::encode_jpeg(&small(), 90);
        for scale in [0, 1, 5, 99] {
            let options = UpscaleOptions { scale, ..Default::default() };
            assert!(upscale(&encoded, None, &options).is_err(), "scale {scale}");
        }
        let loud = UpscaleOptions { sharpen: 4.0, ..Default::default() };
        assert!(upscale(&encoded, None, &loud).is_err());
    }

    #[test]
    fn an_enormous_result_is_refused_before_the_memory_is_asked_for() {
        let wide = DynamicImage::ImageRgb8(RgbImage::new(9000, 10));
        let options = UpscaleOptions { method: Method::Sharpen, scale: 4, sharpen: 0.0 };
        let error = enlarge(&wide, None, &options).unwrap_err().to_string();
        assert!(error.contains("20,000"), "{error}");
    }

    #[test]
    fn a_broken_model_is_refused_clearly() {
        let Err(error) = Model::load(b"not a model".to_vec()) else {
            panic!("rubbish bytes are not a model");
        };
        assert!(error.to_string().contains("upscaling model"), "{error}");
    }

    #[test]
    fn the_options_cross_as_the_ui_writes_them() {
        let options: UpscaleOptions =
            serde_json::from_str(r#"{ "method": "reconstruct", "scale": 4, "sharpen": 0.4 }"#)
                .expect("options");
        assert_eq!(options.method, Method::Reconstruct);
        assert_eq!(options.scale, 4);

        let plain: UpscaleOptions =
            serde_json::from_str(r#"{ "method": "sharpen", "scale": 2, "sharpen": 0 }"#)
                .expect("options");
        assert_eq!(plain.method, Method::Sharpen);
    }

    /// The real thing, when the model is on this machine.
    ///
    /// Ignored by default: it is a 70 MB download and about six seconds of
    /// work, neither of which belongs in an ordinary test run.
    #[test]
    #[ignore = "needs the upscaling model; set KITBOX_UPSCALE_MODEL to the .onnx file"]
    fn reconstructs_without_leaving_a_seam_between_tiles() {
        let Ok(path) = std::env::var("KITBOX_UPSCALE_MODEL") else {
            panic!("set KITBOX_UPSCALE_MODEL to the model file");
        };
        let model = Model::load(std::fs::read(path).unwrap()).unwrap();

        // Wider than one tile, so the seam logic is actually exercised.
        let source = testkit::photo(200, 60);
        let options = UpscaleOptions { method: Method::Reconstruct, scale: 4, sharpen: 0.0 };
        let (out, report) = enlarge(&source, Some(&model), &options).unwrap();

        assert_eq!((out.width(), out.height()), (800, 240));
        assert!(report.tiles >= 2, "more than one tile: {report:?}");

        // A seam shows up as a column that differs far more from its
        // neighbour than the columns around it do.
        let rgb = out.to_rgb8();
        let column_gap = |x: u32| -> f32 {
            (0..rgb.height())
                .map(|y| {
                    let a = rgb.get_pixel(x, y).0;
                    let b = rgb.get_pixel(x + 1, y).0;
                    (0..3).map(|c| (a[c] as f32 - b[c] as f32).abs()).sum::<f32>()
                })
                .sum::<f32>()
                / rgb.height() as f32
        };
        let boundary = TILE * MODEL_SCALE - 1;
        let at_seam = column_gap(boundary);
        let nearby: f32 =
            (1..=6).map(|d| column_gap(boundary - d * 7)).sum::<f32>() / 6.0;
        assert!(
            at_seam <= nearby * 3.0 + 6.0,
            "tile boundary stands out: {at_seam} against {nearby} nearby"
        );
    }
}
