//! Find and cover faces in one picture, for checking by eye.
//!
//! cargo run -p mt-core --example faces_probe -- model.onnx in.jpg out.jpg [blur|pixelate|box|boxes-only]
fn main() {
    let mut args = std::env::args().skip(1);
    let model = args.next().expect("a model");
    let input = args.next().expect("an input image");
    let out = args.next().expect("an output path");
    let how = args.next().unwrap_or_else(|| "blur".into());

    let model = mt_core::engine::image::FaceModel::load(std::fs::read(&model).unwrap()).unwrap();
    let bytes = std::fs::read(&input).unwrap();
    let options = mt_core::engine::image::BlurOptions {
        cover: match how.as_str() {
            "pixelate" => mt_core::engine::image::Cover::Pixelate,
            "box" => mt_core::engine::image::Cover::Box([20, 20, 24]),
            _ => mt_core::engine::image::Cover::Blur,
        },
        ..Default::default()
    };

    let started = std::time::Instant::now();
    let (image, report) = mt_core::engine::image::blur_faces(&bytes, &model, &options).unwrap();
    println!("{} faces in {:?}", report.covered, started.elapsed());
    for f in &report.faces {
        println!("  {}x{} at {},{}  confidence {:.2}", f.width, f.height, f.x, f.y, f.confidence);
    }

    if how == "boxes-only" {
        // Draw outlines instead, to check placement.
        let mut rgb = image::open(&input).unwrap().to_rgb8();
        for f in &report.faces {
            for t in 0..3u32 {
                for x in f.x..(f.x + f.width).min(rgb.width()) {
                    for y in [f.y + t, (f.y + f.height).saturating_sub(t).min(rgb.height() - 1)] {
                        if y < rgb.height() {
                            rgb.put_pixel(x, y, image::Rgb([255, 60, 0]));
                        }
                    }
                }
                for y in f.y..(f.y + f.height).min(rgb.height()) {
                    for x in [f.x + t, (f.x + f.width).saturating_sub(t).min(rgb.width() - 1)] {
                        if x < rgb.width() {
                            rgb.put_pixel(x, y, image::Rgb([255, 60, 0]));
                        }
                    }
                }
            }
        }
        image::DynamicImage::ImageRgb8(rgb).save(&out).unwrap();
    } else {
        image.save(&out).unwrap();
    }
    println!("-> {out}");
}
