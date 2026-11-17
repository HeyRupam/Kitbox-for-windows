//! Caption a picture, for checking the look by eye.
//!
//! cargo run -p mt-core --example meme_probe -- in.jpg out.png "TOP" "BOTTOM" [band]
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("an image");
    let out = args.next().expect("an output path");
    let top = args.next().unwrap_or_default();
    let bottom = args.next().unwrap_or_default();
    let band = args.next().map(|a| a == "band").unwrap_or(false);

    // Impact where it exists, else whatever heavy face is to hand.
    let font = ["C:/Windows/Fonts/impact.ttf", "C:/Windows/Fonts/arialbd.ttf"]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .expect("no font found");

    let options = mt_core::engine::image::MemeOptions { top, bottom, band, ..Default::default() };
    let started = std::time::Instant::now();
    let (image, report) =
        mt_core::engine::image::meme(&std::fs::read(&input).unwrap(), &font, &options).unwrap();
    image.save(&out).unwrap();
    println!(
        "{}x{} in {:?}  {} top line(s), {} bottom",
        report.width, report.height, started.elapsed(), report.top_lines, report.bottom_lines
    );
    for w in &report.warnings {
        println!("  warning: {w}");
    }
}
