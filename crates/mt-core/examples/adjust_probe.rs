//! Adjust one picture, for checking the result by eye.
//!
//! cargo run -p mt-core --example adjust_probe -- in.jpg out.jpg [auto|exposure|contrast|...]
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("an input image");
    let out = args.next().expect("an output path");
    let how = args.next().unwrap_or_else(|| "auto".into());

    let mut options = mt_core::engine::image::AdjustOptions::default();
    match how.as_str() {
        "auto" => {
            options.auto_levels = true;
            options.auto_colour = true;
        }
        "levels" => options.auto_levels = true,
        "colour" => options.auto_colour = true,
        "bright" => options.exposure = 1.0,
        "punch" => {
            options.auto_levels = true;
            options.contrast = 0.3;
            options.saturation = 0.25;
            options.sharpen = 0.4;
        }
        "grey" => options.greyscale = true,
        other => panic!("unknown preset {other}"),
    }

    let bytes = std::fs::read(&input).unwrap();
    let started = std::time::Instant::now();
    let (image, report) = mt_core::engine::image::adjust(&bytes, &options).unwrap();
    image.save(&out).unwrap();
    println!(
        "{}x{} in {:?}  levels {:?}  gain {:?}",
        report.width, report.height, started.elapsed(), report.levels, report.colour_gain
    );
    for w in &report.warnings {
        println!("  warning: {w}");
    }
}
