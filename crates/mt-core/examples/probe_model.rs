//! Run a segmentation model on an image and report what came back, to find
//! out whether the runtime can actually execute it before any code is written
//! around it.
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;

fn main() {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().expect("usage: probe_model <model.onnx> <image> [mask.png]");
    let image_path = args.next().expect("usage: probe_model <model.onnx> <image> [mask.png]");
    let out = args.next();

    let model = rten::Model::load(std::fs::read(&model_path).unwrap()).expect("loading");
    let image = image::open(&image_path).unwrap();
    println!("image {}x{}", image.width(), image.height());

    const SIDE: usize = 320;
    let small = image.resize_exact(SIDE as u32, SIDE as u32, image::imageops::FilterType::Triangle);
    let rgb = small.to_rgb8();

    // NCHW, normalised the way U^2-Net was trained: scale to 0..1 then
    // standardise per channel.
    let plain = std::env::var("PLAIN").is_ok();
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let mut input = NdTensor::zeros([1, 3, SIDE, SIDE]);
    let peak = rgb.pixels().map(|p| p.0.iter().copied().max().unwrap_or(0)).max().unwrap_or(255);
    let peak = (peak as f32 / 255.0).max(1e-6);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let px = rgb.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                let v = px[c] as f32 / 255.0;
                input[[0, c, y, x]] =
                    if plain { v } else { (v / peak - MEAN[c]) / STD[c] };
            }
        }
    }

    let started = std::time::Instant::now();
    match model.run_one(input.view().into(), None) {
        Ok(value) => {
            let mask: NdTensor<f32, 4> = value.try_into().expect("a float mask");
            let (lo, hi) = mask
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            let above = mask.iter().filter(|v| **v > 0.5).count();
            println!(
                "ran in {:?}: shape {:?}, range {lo:.3}..{hi:.3}, {:.1}% above 0.5",
                started.elapsed(),
                mask.shape(),
                above as f32 / mask.len() as f32 * 100.0
            );

            if let Some(out) = out {
                let mut grey = image::GrayImage::new(SIDE as u32, SIDE as u32);
                let span = (hi - lo).max(1e-6);
                for y in 0..SIDE {
                    for x in 0..SIDE {
                        let v = (mask[[0, 0, y, x]] - lo) / span;
                        grey.put_pixel(x as u32, y as u32, image::Luma([(v * 255.0) as u8]));
                    }
                }
                grey.save(&out).unwrap();
                println!("mask -> {out}");
            }
        }
        Err(e) => println!("COULD NOT RUN: {e}"),
    }
}
