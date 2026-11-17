//! What fonts a PDF uses, and whether their programs are embedded.
//!
//! Font embedding is the requirement that usually decides whether a document
//! can become PDF/A at all, so it is worth knowing before anything else.
use lopdf::{Document, Object};

fn main() {
    let path = std::env::args().nth(1).expect("usage: font_probe <file.pdf>");
    let doc = Document::load(&path).unwrap();
    println!("{path}");
    let mut seen = 0;
    for (index, page_id) in doc.page_iter().enumerate() {
        let Ok(fonts) = doc.get_page_fonts(page_id) else { continue };
        for (name, dict) in fonts {
            seen += 1;
            let base = dict.get(b"BaseFont").ok().and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).to_string()).unwrap_or_default();
            let subtype = dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).to_string()).unwrap_or_default();
            // The font program lives in the descriptor, directly or via a
            // descendant for composite fonts.
            let descriptor = dict.get(b"FontDescriptor").ok().and_then(|o| match o {
                Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
                Object::Dictionary(d) => Some(d.clone()),
                _ => None,
            });
            let embedded = descriptor.as_ref().map(|d| {
                d.has(b"FontFile") || d.has(b"FontFile2") || d.has(b"FontFile3")
            }).unwrap_or(false);
            println!(
                "  page {} /{}  {base}  {subtype}  {}",
                index + 1,
                String::from_utf8_lossy(&name),
                if embedded { "EMBEDDED" } else { "not embedded" }
            );
        }
    }
    if seen == 0 {
        println!("  (no fonts — an image-only document)");
    }
}
