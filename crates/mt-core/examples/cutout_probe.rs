//! Cut the background out of one picture, for checking the result by eye.
//!
//! cargo run -p mt-core --example cutout_probe -- model.onnx in.jpg out.png [RRGGBB]
fn main() {
    let mut args = std::env::args().skip(1);
    let model = args.next().expect("usage: cutout_probe <model> <in> <out> [RRGGBB]");
    let input = args.next().expect("an input image");
    let out = args.next().expect("an output path");
    let backdrop = args.next();

    let model = mt_core::engine::image::CutoutModel::load(std::fs::read(&model).unwrap()).unwrap();
    let options = mt_core::engine::image::CutoutOptions {
        backdrop: match backdrop {
            Some(hex) => {
                let n = u32::from_str_radix(&hex, 16).expect("RRGGBB");
                mt_core::engine::image::Backdrop::Colour([
                    (n >> 16) as u8,
                    (n >> 8) as u8,
                    n as u8,
                ])
            }
            None => mt_core::engine::image::Backdrop::Transparent,
        },
        ..Default::default()
    };

    let started = std::time::Instant::now();
    let (image, report) =
        mt_core::engine::image::remove_background(&std::fs::read(&input).unwrap(), &model, &options)
            .unwrap();
    image.save(&out).unwrap();
    println!(
        "{out}  {}x{}  {:.1}% kept  in {:?}",
        report.width,
        report.height,
        report.coverage * 100.0,
        started.elapsed()
    );
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
}
