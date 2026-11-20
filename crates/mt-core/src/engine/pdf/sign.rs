//! Placing a signature on a PDF.
//!
//! **What this is not.** This draws a picture of a signature onto a page. It is
//! not a cryptographic signature: it does not prove who signed, and it does not
//! make the document tamper-evident. Anyone can take the image out again, and
//! anyone who has the image can put it on anything. That is true of every tool
//! that does this, including the ones that charge for it — but it is rarely
//! said out loud, so every [`SignReport`] says it, and the screen repeats it.
//!
//! It is still the right tool for what most people need, which is a form that
//! asks for a signature and accepts a scan of one. A legally binding signature
//! needs PKCS#12 or a DSC token, which is a different mechanism entirely.
//!
//! The placement is given as fractions of the page rather than points, so a
//! position chosen on a preview means the same thing on A4 and on Letter, and
//! on a page whose `/Rotate` says it is displayed sideways.

use lopdf::{Document, ObjectId};
use serde::{Deserialize, Serialize};

use super::overlay::{
    escape_text, rotation_of, stamp_page, text_width, Frame, FONT_NAME, IMAGE_NAME, STATE_NAME,
};
use super::pages::{materialise_inherited, media_box};
use crate::error::{Error, Result};

/// Where one copy of the signature goes.
///
/// Coordinates are fractions of the page **as displayed**, with `(0,0)` at the
/// top left — the same frame a preview image uses, so a click on a thumbnail
/// maps across without the caller doing any arithmetic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureSpot {
    /// 1-based.
    pub page: u32,
    /// Centre of the signature, 0.0–1.0 across the page.
    pub x: f32,
    /// Centre of the signature, 0.0–1.0 down the page.
    pub y: f32,
    /// Width as a fraction of the page width. The height follows from the
    /// signature's own proportions.
    pub width: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Caption {
    /// Drawn under the signature. `{date}` is replaced with `date`.
    pub text: String,
    /// Written out by the caller, because the core has no clock and a date
    /// invented here would be the machine's idea of today, not the signer's.
    pub date: String,
    pub size: f32,
    pub colour: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOptions {
    pub spots: Vec<SignatureSpot>,
    pub caption: Option<Caption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignReport {
    pub pages_signed: Vec<u32>,
    pub placements: u32,
    /// Always present, always the same: what a drawn signature is and isn't.
    pub note: String,
    pub warnings: Vec<String>,
}

/// What every report says. Stated once, here, so it cannot drift.
pub const NOT_CRYPTOGRAPHIC: &str =
    "This draws your signature onto the page. It is not a digital certificate: it does not prove \
     who signed and it does not make the file tamper-evident. Forms that accept a scanned \
     signature accept this; anything that demands a DSC or PKCS#12 token does not.";

/// Longest side the embedded signature is allowed to be, in pixels.
///
/// A signature is drawn a couple of inches wide. A 12-megapixel photograph of
/// one carries detail no printer will ever resolve and no reader will ever
/// show, at a hundred times the bytes.
const MAX_PX: u32 = 1400;

/// Draw `signature` onto `pdf` at each spot.
pub fn sign(pdf: &[u8], signature: &[u8], options: &SignOptions) -> Result<(Vec<u8>, SignReport)> {
    if options.spots.is_empty() {
        return Err(Error::invalid("no place to put the signature"));
    }

    let image = image::load_from_memory(signature)
        .map_err(|e| Error::invalid(format!("could not read the signature image: {e}")))?;
    let image = if image.width().max(image.height()) > MAX_PX {
        image.resize(MAX_PX, MAX_PX, image::imageops::FilterType::Lanczos3)
    } else {
        image
    };
    let aspect = image.height() as f32 / image.width().max(1) as f32;

    let mut doc = Document::load_mem(pdf)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }

    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    if page_ids.is_empty() {
        return Err(Error::invalid("this PDF has no pages"));
    }

    let mut warnings = Vec::new();
    for spot in &options.spots {
        if spot.page == 0 || spot.page as usize > page_ids.len() {
            return Err(Error::invalid(format!(
                "there is no page {} — this document has {}",
                spot.page,
                page_ids.len()
            )));
        }
        if !(0.0..=1.0).contains(&spot.x) || !(0.0..=1.0).contains(&spot.y) {
            return Err(Error::invalid("the signature's position is off the page"));
        }
        if !(0.01..=1.0).contains(&spot.width) {
            return Err(Error::invalid(
                "the signature's width has to be between 1% and 100% of the page",
            ));
        }
    }

    // One image object however many times it is placed: signing twenty pages
    // should cost one signature.
    let image_id = super::images::rgba_image_stream(&mut doc, &image.to_rgba8());

    // Group the spots by page, so a page is stamped once with everything that
    // lands on it rather than once per spot.
    let mut by_page: std::collections::BTreeMap<u32, Vec<&SignatureSpot>> = Default::default();
    for spot in &options.spots {
        by_page.entry(spot.page).or_default().push(spot);
    }

    let mut pages_signed = Vec::new();
    let mut placements = 0u32;

    for (page_number, spots) in by_page {
        let page_id = page_ids[page_number as usize - 1];
        materialise_inherited(&mut doc, page_id);
        // A page with no /MediaBox anywhere up its tree is malformed. Letter
        // is the reader's own fallback, so it is the right guess — but the
        // placement is a fraction of a size that was guessed, so say so.
        let (media_w, media_h) = match media_box(&doc, page_id) {
            Some(size) => size,
            None => {
                warnings.push(format!(
                    "Page {page_number} doesn't say how big it is, so the signature was placed as \
                     if it were Letter. Check where it landed."
                ));
                (612.0, 792.0)
            }
        };
        let frame = Frame { media_w, media_h, rotation: rotation_of(&doc, page_id) };
        let (page_w, page_h) = frame.displayed();

        let mut content = String::new();
        for spot in spots {
            let width = spot.width * page_w;
            let height = width * aspect;
            if height > page_h {
                warnings.push(format!(
                    "On page {page_number} the signature was taller than the page at that width."
                ));
            }
            // Displayed coordinates, y measured down, converted to the
            // bottom-left corner the drawing operator wants.
            let centre_x = spot.x * page_w;
            let centre_y = page_h - spot.y * page_h;
            let x = centre_x - width / 2.0;
            let y = centre_y - height / 2.0;

            content.push_str(&format!(
                "q\n/{STATE_NAME} gs\n{} cm\n{width:.2} 0 0 {height:.2} 0 0 cm\n/{IMAGE_NAME} Do\nQ\n",
                frame.to_page_space(x, y)
            ));
            placements += 1;

            if let Some(caption) = &options.caption {
                let text = caption.text.replace("{date}", &caption.date);
                if !text.trim().is_empty() {
                    let size = caption.size.max(1.0);
                    // Under the signature, left-aligned with it, far enough
                    // down not to touch the descenders of the stroke.
                    let tx = x;
                    let ty = y - size * 1.25;
                    let [r, g, b] = caption.colour;
                    content.push_str(&format!(
                        "q\nBT\n/{FONT_NAME} {size} Tf\n{:.3} {:.3} {:.3} rg\n{} Tm\n({}) Tj\nET\nQ\n",
                        r as f32 / 255.0,
                        g as f32 / 255.0,
                        b as f32 / 255.0,
                        frame.to_page_space(tx, ty),
                        escape_text(&text)
                    ));
                    if text_width(&text, size) > width * 2.5 {
                        warnings.push(format!(
                            "The caption on page {page_number} is much wider than the signature."
                        ));
                    }
                }
            }
        }

        stamp_page(&mut doc, page_id, &content, 1.0, Some(image_id))?;
        pages_signed.push(page_number);
    }

    // Deflates the signature's colour and alpha streams, which go in raw.
    doc.compress();

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;

    Ok((
        out,
        SignReport { pages_signed, placements, note: NOT_CRYPTOGRAPHIC.to_string(), warnings },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdf::{page_count, rotate, PageSelection};
    use crate::testkit;

    fn signature_png() -> Vec<u8> {
        // A wide, mostly transparent mark, as the extractor would produce.
        let mut image = image::RgbaImage::new(400, 120);
        for x in 20..380 {
            let y = 60.0 + 30.0 * ((x as f32) / 40.0).sin();
            for dy in -4i32..=4 {
                let py = (y as i32 + dy).clamp(0, 119) as u32;
                image.put_pixel(x, py, image::Rgba([20, 40, 140, 255]));
            }
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn spot(page: u32) -> SignatureSpot {
        SignatureSpot { page, x: 0.3, y: 0.85, width: 0.28 }
    }

    fn content_of(bytes: &[u8]) -> Vec<String> {
        let doc = Document::load_mem(bytes).unwrap();
        doc.page_iter()
            .map(|p| String::from_utf8_lossy(&doc.get_page_content(p)).to_string())
            .collect()
    }

    #[test]
    fn the_signature_lands_on_the_page_it_was_asked_for() {
        let src = testkit::pdf_scan(3, 200, 260, 70);
        let options = SignOptions { spots: vec![spot(2)], caption: None };
        let (out, report) = sign(&src, &signature_png(), &options).unwrap();

        assert_eq!(page_count(&out).unwrap(), 3);
        assert_eq!(report.pages_signed, vec![2]);
        assert_eq!(report.placements, 1);

        let drawn: Vec<bool> =
            content_of(&out).iter().map(|c| c.contains(&format!("/{IMAGE_NAME} Do"))).collect();
        assert_eq!(drawn, vec![false, true, false], "only page 2 should carry it");
    }

    #[test]
    fn every_report_says_it_is_not_a_digital_certificate() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = SignOptions { spots: vec![spot(1)], caption: None };
        let (_, report) = sign(&src, &signature_png(), &options).unwrap();
        assert!(report.note.contains("not a digital certificate"), "{}", report.note);
        assert!(report.note.contains("DSC"), "it should name what does count");
    }

    #[test]
    fn transparency_survives_into_the_pdf() {
        // Without an /SMask the signature is a white box over the form's line.
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = SignOptions { spots: vec![spot(1)], caption: None };
        let (out, _) = sign(&src, &signature_png(), &options).unwrap();

        let doc = Document::load_mem(&out).unwrap();
        let has_mask = doc
            .objects
            .values()
            .any(|o| matches!(o, lopdf::Object::Stream(s) if s.dict.has(b"SMask")));
        assert!(has_mask, "the signature must carry its alpha channel");
    }

    #[test]
    fn one_image_serves_however_many_placements() {
        let src = testkit::pdf_scan(6, 200, 260, 70);
        let options = SignOptions { spots: (1..=6).map(spot).collect(), caption: None };
        let (out, report) = sign(&src, &signature_png(), &options).unwrap();
        assert_eq!(report.placements, 6);
        assert_eq!(report.pages_signed, vec![1, 2, 3, 4, 5, 6]);

        let doc = Document::load_mem(&out).unwrap();
        // The colour stream and its mask: two, not twelve.
        let images = doc
            .objects
            .values()
            .filter(|o| {
                matches!(o, lopdf::Object::Stream(s)
                    if s.dict.get(b"Subtype").ok().and_then(|x| x.as_name().ok())
                        == Some(b"Image".as_ref()))
            })
            .count();
        // Six scanned pages bring six images of their own.
        assert_eq!(images, 6 + 2, "the signature should be embedded once");
    }

    #[test]
    fn two_signatures_on_one_page_are_stamped_together() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        let options = SignOptions {
            spots: vec![
                SignatureSpot { page: 1, x: 0.25, y: 0.8, width: 0.2 },
                SignatureSpot { page: 1, x: 0.75, y: 0.8, width: 0.2 },
            ],
            caption: None,
        };
        let (out, report) = sign(&src, &signature_png(), &options).unwrap();
        assert_eq!(report.pages_signed, vec![1], "one page, twice");
        assert_eq!(report.placements, 2);
        let drawn = content_of(&out)[0].matches(&format!("/{IMAGE_NAME} Do")).count();
        assert_eq!(drawn, 2);
    }

    #[test]
    fn the_caption_fills_in_the_date() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = SignOptions {
            spots: vec![spot(1)],
            caption: Some(Caption {
                text: "Signed on {date}".into(),
                date: "11 November 2026".into(),
                size: 9.0,
                colour: [40, 40, 48],
            }),
        };
        let (out, _) = sign(&src, &signature_png(), &options).unwrap();
        let content = &content_of(&out)[0];
        assert!(
            content.contains("(Signed on 11 November 2026) Tj"),
            "the date should be filled in: {content}"
        );
    }

    #[test]
    fn a_rotated_page_gets_an_upright_signature() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let selection = PageSelection::parse("1", 1).unwrap();
        let (turned, _) = rotate(&src, &selection, 90).unwrap();
        let options = SignOptions { spots: vec![spot(1)], caption: None };
        let (out, _) = sign(&turned, &signature_png(), &options).unwrap();

        // The placement matrix must counter the rotation, or the signature
        // runs up the side of the page instead of along the line.
        let content = &content_of(&out)[0];
        assert!(content.contains("0 1 -1 0"), "expected a rotated matrix: {content}");
    }

    #[test]
    fn the_page_content_survives() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        let before = content_of(&src)[0].clone();
        let options = SignOptions { spots: vec![spot(1)], caption: None };
        let (out, _) = sign(&src, &signature_png(), &options).unwrap();
        // The original drawing is still in there, fenced rather than replaced.
        let after = content_of(&out)[0].clone();
        for line in before.lines().filter(|l| l.contains("Do") || l.contains("cm")) {
            assert!(after.contains(line.trim()), "lost '{line}' from the page");
        }
    }

    #[test]
    fn a_page_that_does_not_exist_is_refused() {
        let src = testkit::pdf_scan(2, 200, 260, 70);
        let options = SignOptions { spots: vec![spot(9)], caption: None };
        let error = sign(&src, &signature_png(), &options).unwrap_err().to_string();
        assert!(error.contains("no page 9"), "{error}");
        assert!(error.contains("has 2"), "it should say how many there are: {error}");
    }

    #[test]
    fn nonsense_placements_are_refused() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let off_page = SignOptions {
            spots: vec![SignatureSpot { page: 1, x: 1.4, y: 0.5, width: 0.2 }],
            caption: None,
        };
        assert!(sign(&src, &signature_png(), &off_page).is_err());

        let too_small = SignOptions {
            spots: vec![SignatureSpot { page: 1, x: 0.5, y: 0.5, width: 0.0 }],
            caption: None,
        };
        assert!(sign(&src, &signature_png(), &too_small).is_err());

        let nowhere = SignOptions { spots: vec![], caption: None };
        assert!(sign(&src, &signature_png(), &nowhere).is_err());
    }

    #[test]
    fn a_signature_that_is_not_an_image_is_refused() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        let options = SignOptions { spots: vec![spot(1)], caption: None };
        let error = sign(&src, b"not an image", &options).unwrap_err().to_string();
        assert!(error.contains("signature image"), "{error}");
    }

    #[test]
    fn an_oversized_signature_is_warned_about() {
        let src = testkit::pdf_scan(1, 200, 260, 70);
        // Full page width, on a tall narrow signature, overflows the height.
        let tall = {
            let mut image = image::RgbaImage::new(100, 900);
            for y in 0..900 {
                image.put_pixel(50, y, image::Rgba([0, 0, 0, 255]));
            }
            let mut out = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let options = SignOptions {
            spots: vec![SignatureSpot { page: 1, x: 0.5, y: 0.5, width: 1.0 }],
            caption: None,
        };
        let (_, report) = sign(&src, &tall, &options).unwrap();
        assert!(
            report.warnings.iter().any(|w| w.contains("taller than the page")),
            "{:?}",
            report.warnings
        );
    }
}
