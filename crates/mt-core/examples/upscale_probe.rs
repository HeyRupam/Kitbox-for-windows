//! Enlarge one picture, for checking the result by eye.
//!
//! cargo run -p mt-core --example upscale_probe -- in.jpg out.png <sharpen|reconstruct> [model.onnx]
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("an input image");
    let out = args.next().expect("an output path");
    let how = args.next().unwrap_or_else(|| "sharpen".into());
    let model_path = args.next();

    let model = model_path
        .map(|p| mt_core::engine::image::UpscaleModel::load(std::fs::read(&p).unwrap()).unwrap());
    let options = mt_core::engine::image::UpscaleOptions {
        method: if how == "reconstruct" {
            mt_core::engine::image::UpscaleMethod::Reconstruct
        } else {
            mt_core::engine::image::UpscaleMethod::Sharpen
        },
        scale: 4,
        sharpen: 0.4,
    };

    match mt_core::engine::image::upscale(&std::fs::read(&input).unwrap(), model.as_ref(), &options)
    {
        Ok((image, report)) => {
            image.save(&out).unwrap();
            println!(
                "{:?}×{:?} -> {:?}  {:?}  {} tiles  {:.1}s  -> {out}",
                report.from.0,
                report.from.1,
                report.to,
                report.method,
                report.tiles,
                report.seconds
            );
            for w in &report.warnings {
                println!("  warning: {w}");
            }
        }
        Err(e) => println!("refused: {e}"),
    }
}
