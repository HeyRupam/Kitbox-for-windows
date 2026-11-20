//! Deterministic fixtures for tests.
//!
//! Synthetic images need real high-frequency detail or JPEG compresses them to
//! almost nothing and the solver tests become meaningless, so `photo` layers a
//! gradient, hard edges and reproducible noise.

use image::{DynamicImage, ImageEncoder, RgbImage};

/// A plausible photo: smooth gradient, geometric edges, fine grain. Same output
/// every run.
pub fn photo(w: u32, h: u32) -> DynamicImage {
    let mut rng: u32 = 0x1234_5678;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    let img = RgbImage::from_fn(w, h, |x, y| {
        let fx = x as f32 / w.max(1) as f32;
        let fy = y as f32 / h.max(1) as f32;

        let mut r = (40.0 + 180.0 * fx) as i32;
        let mut g = (60.0 + 150.0 * fy) as i32;
        let mut b = (200.0 - 120.0 * (fx + fy) * 0.5) as i32;

        // Hard-edged shapes, which is where compression artefacts show up.
        let cx = w as f32 * 0.35;
        let cy = h as f32 * 0.55;
        let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
        if d < w as f32 * 0.22 {
            r = 240 - r / 3;
            g = 220 - g / 4;
        }
        // Broad bands, not a fine checker: real photos carry their energy at
        // low frequencies, and a high-frequency pattern would make every codec
        // look far worse than it is.
        if (x / 24 + y / 32) % 3 == 0 {
            b = (b + 45).min(255);
        }

        // Enough grain to be realistic, not so much that JPEG is judged on
        // its ability to preserve pure noise.
        let n = (next() % 9) as i32 - 4;
        image::Rgb([
            (r + n).clamp(0, 255) as u8,
            (g + n).clamp(0, 255) as u8,
            (b + n).clamp(0, 255) as u8,
        ])
    });
    DynamicImage::ImageRgb8(img)
}

pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality)
        .encode_image(&img.to_rgb8())
        .expect("jpeg encode");
    buf
}

pub fn reencode_jpeg(img: &DynamicImage, quality: u8) -> DynamicImage {
    image::load_from_memory(&encode_jpeg(img, quality)).expect("jpeg decode")
}

pub fn tiny_jpeg() -> Vec<u8> {
    encode_jpeg(&photo(48, 48), 70)
}

/// A one-page PDF holding a single JPEG, drawn into a box of the given size in
/// points. Lets a test state the DPI it expects: a 600 px image in a 144 pt box
/// is 300 DPI.
pub fn pdf_with_image(
    px_w: u32,
    px_h: u32,
    draw_w_pt: f32,
    draw_h_pt: f32,
    quality: u8,
) -> Vec<u8> {
    let jpeg = encode_jpeg(&photo(px_w, px_h), quality);
    pdf_with_jpeg(&jpeg, px_w, px_h, draw_w_pt, draw_h_pt)
}

/// The same, from JPEG bytes a test made itself — for cases that need specific
/// pixels rather than the generic photo.
pub fn pdf_with_jpeg(jpeg: &[u8], px_w: u32, px_h: u32, draw_w_pt: f32, draw_h_pt: f32) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};

    let jpeg = jpeg.to_vec();

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let image = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => px_w as i64,
            "Height" => px_h as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8i64,
            "Filter" => "DCTDecode",
        },
        jpeg,
    )
    .with_compression(false);
    let image_id = doc.add_object(image);

    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    Object::Real(draw_w_pt),
                    Object::Real(0.0),
                    Object::Real(0.0),
                    Object::Real(draw_h_pt),
                    Object::Real(0.0),
                    Object::Real(0.0),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! {
            "XObject" => dictionary! { "Im0" => image_id },
        },
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });

    finish(doc, pages_id, vec![page_id])
}

/// A PDF with nothing but text — nothing for a downsampler to bite on.
pub fn pdf_text_only() -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });

    let mut operations = Vec::new();
    operations.push(Operation::new("BT", vec![]));
    operations.push(Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), Object::Real(14.0)]));
    for line in 0..40 {
        operations.push(Operation::new(
            "Td",
            vec![
                Object::Real(if line == 0 { 72.0 } else { 0.0 }),
                Object::Real(if line == 0 { 720.0 } else { -16.0 }),
            ],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::string_literal(
                "Declaration: the information given above is true to the best of my knowledge.",
            )],
        ));
    }
    operations.push(Operation::new("ET", vec![]));

    let content = Content { operations };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });

    finish(doc, pages_id, vec![page_id])
}

/// A multi-page scan: one full-page image per page.
pub fn pdf_scan(pages: usize, px_w: u32, px_h: u32, quality: u8) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut page_ids = Vec::new();

    for page in 0..pages {
        // Vary the content so pages don't deduplicate into one object.
        let jpeg = encode_jpeg(&photo(px_w, px_h + page as u32), quality);
        let image_id = doc.add_object(
            Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Image",
                    "Width" => px_w as i64,
                    "Height" => (px_h + page as u32) as i64,
                    "ColorSpace" => "DeviceRGB",
                    "BitsPerComponent" => 8i64,
                    "Filter" => "DCTDecode",
                },
                jpeg,
            )
            .with_compression(false),
        );
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        Object::Real(612.0),
                        Object::Real(0.0),
                        Object::Real(0.0),
                        Object::Real(792.0),
                        Object::Real(0.0),
                        Object::Real(0.0),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        page_ids.push(doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }));
    }

    finish(doc, pages_id, page_ids)
}

/// A page of Helvetica lines at given sizes, one per line, top to bottom.
///
/// The Markdown extractor infers headings from relative size, so a fixture for
/// it has to carry real sizes rather than a flat block of text.
pub fn pdf_with_sized_text(lines: &[(f32, &str)]) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream, StringFormat};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });

    let mut operations = vec![Operation::new("BT", vec![])];
    let mut y = 720.0f32;
    for (size, text) in lines {
        operations
            .push(Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), Object::Real(*size)]));
        operations.push(Operation::new(
            "Tm",
            vec![
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(72.0),
                Object::Real(y),
            ],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(to_winansi(text), StringFormat::Literal)],
        ));
        y -= size * 1.7;
    }
    operations.push(Operation::new("ET", vec![]));

    let content = Content { operations };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });

    finish(doc, pages_id, vec![page_id])
}

/// Single-byte WinAnsi, so the reader decodes back what was written. Writing
/// UTF-8 into a WinAnsi font is how fixtures end up testing mojibake.
fn to_winansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c {
            c if (c as u32) < 128 => c as u8,
            '\u{2022}' => 0x95,
            '\u{2019}' => 0x92,
            '\u{2014}' => 0x97,
            _ => b'?',
        })
        .collect()
}

fn finish(
    mut doc: lopdf::Document,
    pages_id: lopdf::ObjectId,
    page_ids: Vec<lopdf::ObjectId>,
) -> Vec<u8> {
    use lopdf::{dictionary, Object};
    let count = page_ids.len() as i64;
    let kids: Vec<Object> = page_ids.into_iter().map(Object::Reference).collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save pdf fixture");
    out
}

pub fn tiny_png() -> Vec<u8> {
    let img = photo(48, 48).to_rgba8();
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
        .expect("png encode");
    buf
}

/// A fillable form: a text field, a checkbox whose on-state is deliberately
/// *not* `/Yes`, a radio group and a dropdown.
///
/// The awkward on-state is the point. Real forms use `/On`, `/1`, `/Ja` and
/// whatever else their author typed, and a filler that assumes `/Yes` produces
/// a box that renders as neither ticked nor empty.
pub fn pdf_form() -> Vec<u8> {
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.6");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 11 Tf 1 0 0 1 60 700 Tm (Application) Tj ET\n".to_vec(),
    ));
    let page_id = doc.new_object_id();

    let ticked_on = |doc: &mut Document, state: &str| {
        let on = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 14.into(), 14.into()],
            },
            b"q 0 0 0 rg 2 2 10 10 re f Q\n".to_vec(),
        ));
        let off = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 14.into(), 14.into()],
            },
            b"q 0.6 0.6 0.6 RG 1 1 12 12 re S Q\n".to_vec(),
        ));
        let mut states = lopdf::Dictionary::new();
        states.set(state.to_string(), Object::Reference(on));
        states.set("Off", Object::Reference(off));
        states
    };

    // A plain text field.
    let name_field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "T" => Object::string_literal("applicant_name"),
        "TU" => Object::string_literal("Full name as on your ID"),
        "Ff" => 2i64,
        "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        "Rect" => vec![180.into(), 660.into(), 420.into(), 682.into()],
        "P" => page_id,
    });

    // A multi-line address field.
    let address_field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "T" => Object::string_literal("address"),
        "Ff" => 4096i64,
        "DA" => Object::string_literal("/Helv 9 Tf 0 g"),
        "Rect" => vec![180.into(), 580.into(), 420.into(), 650.into()],
        "P" => page_id,
    });

    // A checkbox whose on-state is "On", not "Yes".
    let agree_states = ticked_on(&mut doc, "On");
    let agree_field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Btn",
        "T" => Object::string_literal("agree"),
        "AP" => dictionary! { "N" => agree_states },
        "AS" => Object::Name(b"Off".to_vec()),
        "Rect" => vec![180.into(), 548.into(), 194.into(), 562.into()],
        "P" => page_id,
    });

    // A radio group with two kids.
    let group_id = doc.new_object_id();
    let post_states = ticked_on(&mut doc, "post");
    let email_states = ticked_on(&mut doc, "email");
    let post_kid = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Parent" => group_id,
        "AP" => dictionary! { "N" => post_states },
        "AS" => Object::Name(b"Off".to_vec()),
        "Rect" => vec![180.into(), 518.into(), 194.into(), 532.into()],
        "P" => page_id,
    });
    let email_kid = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Parent" => group_id,
        "AP" => dictionary! { "N" => email_states },
        "AS" => Object::Name(b"Off".to_vec()),
        "Rect" => vec![260.into(), 518.into(), 274.into(), 532.into()],
        "P" => page_id,
    });
    doc.objects.insert(
        group_id,
        Object::Dictionary(dictionary! {
            "FT" => "Btn",
            "T" => Object::string_literal("contact_by"),
            // Radio button flag.
            "Ff" => 32768i64,
            "Kids" => vec![post_kid.into(), email_kid.into()],
        }),
    );

    // A dropdown.
    let country_field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Ch",
        "T" => Object::string_literal("country"),
        "DA" => Object::string_literal("/Helv 10 Tf 0 g"),
        "Opt" => vec![
            Object::string_literal("India"),
            Object::string_literal("United Kingdom"),
            Object::string_literal("Singapore"),
        ],
        "Rect" => vec![180.into(), 486.into(), 330.into(), 506.into()],
        "P" => page_id,
    });

    // One the form won't let you touch.
    let reference_field = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "T" => Object::string_literal("reference"),
        "Ff" => 1i64,
        "V" => Object::string_literal("REF-2026-014"),
        "DA" => Object::string_literal("/Helv 10 Tf 0 g"),
        "Rect" => vec![180.into(), 452.into(), 330.into(), 472.into()],
        "P" => page_id,
    });

    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            "Annots" => vec![
                name_field.into(),
                address_field.into(),
                agree_field.into(),
                post_kid.into(),
                email_kid.into(),
                country_field.into(),
                reference_field.into(),
            ],
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "AcroForm" => dictionary! {
            "Fields" => vec![
                name_field.into(),
                address_field.into(),
                agree_field.into(),
                group_id.into(),
                country_field.into(),
                reference_field.into(),
            ],
            "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "Helv" => font_id } },
        },
    });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}
