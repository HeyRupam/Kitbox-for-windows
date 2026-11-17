//! Print the page corners found in a photograph, for checking detection by eye.
fn main() {
    let path = std::env::args().nth(1).expect("usage: quad <image>");
    let bytes = std::fs::read(&path).unwrap();
    let image = image::load_from_memory(&bytes).unwrap();
    println!("image {} x {}", image.width(), image.height());
    match mt_core::engine::image::scan::detect_page(&image) {
        Some(quad) => {
            for (name, (x, y)) in ["TL", "TR", "BR", "BL"].iter().zip(quad) {
                println!("  {name}  {x:7.1} {y:7.1}");
            }
        }
        None => println!("  no page found"),
    }
}
