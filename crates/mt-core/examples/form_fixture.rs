//! Write the test form out, for trying the form tools by hand.
//!
//! Run with: cargo run -p mt-core --features testkit --example form_fixture -- out.pdf
fn main() {
    let path = std::env::args().nth(1).expect("usage: form_fixture <out.pdf>");
    std::fs::write(&path, mt_core::testkit::pdf_form()).unwrap();
    println!("wrote {path}");
}
