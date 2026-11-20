//! Run a face detector and report what it gives back, before writing any
//! decoding around it.
use rten_tensor::prelude::*;
use rten_tensor::NdTensor;

fn main() {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().expect("usage: face_probe <model.onnx> <image>");
    let image_path = args.next().expect("an image");
    let model = rten::Model::load(std::fs::read(&model_path).unwrap()).unwrap();

    for id in model.input_ids() {
        let info = model.node_info(*id);
        println!(
            "input  {:?} {:?}",
            info.as_ref().and_then(|i| i.name()),
            info.as_ref().and_then(|i| i.shape())
        );
    }
    for id in model.output_ids() {
        let info = model.node_info(*id);
        println!(
            "output {:?} {:?}",
            info.as_ref().and_then(|i| i.name()),
            info.as_ref().and_then(|i| i.shape())
        );
    }

    const W: usize = 320;
    const H: usize = 240;
    let image = image::open(&image_path).unwrap();
    let small = image.resize_exact(W as u32, H as u32, image::imageops::FilterType::Triangle);
    let rgb = small.to_rgb8();
    let mut input = NdTensor::zeros([1, 3, H, W]);
    for y in 0..H {
        for x in 0..W {
            let px = rgb.get_pixel(x as u32, y as u32).0;
            for c in 0..3 {
                // UltraFace's documented preprocessing.
                input[[0, c, y, x]] = (px[c] as f32 - 127.0) / 128.0;
            }
        }
    }

    let ids: Vec<_> = model.output_ids().to_vec();
    let started = std::time::Instant::now();
    match model.run(vec![(model.input_ids()[0], input.view().into())], &ids, None) {
        Ok(values) => {
            println!("ran in {:?}", started.elapsed());
            for (i, v) in values.into_iter().enumerate() {
                let t: NdTensor<f32, 3> = match v.try_into() {
                    Ok(t) => t,
                    Err(_) => {
                        println!("  output {i}: not a 3-d float tensor");
                        continue;
                    }
                };
                let (lo, hi) =
                    t.iter().fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
                println!("  output {i}: shape {:?} range {lo:.3}..{hi:.3}", t.shape());
                // If this looks like scores, how many are confident?
                if t.shape()[2] == 2 {
                    let n = (0..t.shape()[1]).filter(|&j| t[[0, j, 1]] > 0.7).count();
                    println!("    {n} entries with face-confidence > 0.7");
                }
            }
        }
        Err(e) => println!("COULD NOT RUN: {e}"),
    }
}
