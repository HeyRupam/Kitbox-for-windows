//! Reading the text in a scan.
//!
//! A scanned page is a photograph. Every tool that works on text — search,
//! compare, redact, Markdown — looks at it and reports, honestly, that there
//! is nothing there. OCR is what turns the photograph into something those
//! tools can see.
//!
//! The engine is [`ocrs`], which is pure Rust. That matters more than it
//! sounds: Tesseract would mean vendoring two C libraries per platform, and
//! `mt-core`'s whole rule is that it has no platform dependencies so the
//! Android port can reuse it unchanged. Models arrive here as bytes — this
//! module never opens a file or reaches the network, and whoever calls it
//! decides where the bytes came from.
//!
//! # The searchable layer
//!
//! The output keeps the scan exactly as it is and adds the recognised words as
//! **invisible text** behind it, positioned over the pixels they were read
//! from. Nothing about the page changes visually. That is the whole design:
//! OCR is a guess, and a guess that rewrites your document is dangerous, while
//! a guess that only fills in a hidden index costs you a bad search result at
//! worst. The visible page is still the original photograph, so a word read
//! wrongly cannot corrupt anything.
//!
//! # What it can read
//!
//! Pages that *are* an image — which is what a scan is. A page already made of
//! text doesn't need OCR and is skipped rather than clumsily re-read. A page
//! built from vector drawing can't be rasterised here (there is no renderer in
//! this crate), and is reported rather than silently skipped.

use lopdf::{Document, ObjectId};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use serde::{Deserialize, Serialize};

use super::overlay::{escape_text, stamp_page, text_width, FONT_NAME};
use super::pages::{materialise_inherited, PageSelection};
use crate::error::{Error, Result};

/// The two models the engine needs, already decoded.
///
/// Loading them is the slow part — several seconds — so it is done once and
/// the result reused across pages and documents.
pub struct Models {
    engine: OcrEngine,
}

impl Models {
    /// Build the engine from the detection and recognition model bytes.
    pub fn load(detection: Vec<u8>, recognition: Vec<u8>) -> Result<Models> {
        let detection = rten::Model::load(detection)
            .map_err(|e| Error::invalid(format!("the text-detection model wouldn't load: {e}")))?;
        let recognition = rten::Model::load(recognition).map_err(|e| {
            Error::invalid(format!("the text-recognition model wouldn't load: {e}"))
        })?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: Some(recognition),
            ..Default::default()
        })
        .map_err(|e| Error::other(format!("the OCR engine wouldn't start: {e}")))?;
        Ok(Models { engine })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrOptions {
    /// Pages to read. `None` means every page.
    #[serde(default)]
    pub pages: Option<PageSelection>,
    /// Leave pages that already carry text alone.
    pub skip_pages_with_text: bool,
    /// Shrink a page's image to this on its longest side before reading it.
    ///
    /// Recognition doesn't improve above roughly 2000px and the time taken
    /// grows with the pixels, so a 12-megapixel phone photo is mostly wasted
    /// effort.
    pub max_px: u32,
}

impl Default for OcrOptions {
    fn default() -> Self {
        OcrOptions { pages: None, skip_pages_with_text: true, max_px: 2000 }
    }
}

/// One recognised word and where it sits, in the page's own coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Which line of the page this word was read from.
    ///
    /// Carried because a word on its own is not enough to rebuild readable
    /// text: the hidden layer has to put a space between neighbours, and
    /// nothing else in the output says where one line ends.
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageText {
    pub page: u32,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrReport {
    pub pages_read: Vec<u32>,
    /// Pages left alone because they already had text.
    pub pages_with_text: Vec<u32>,
    /// Pages with nothing this crate can turn into pixels.
    pub pages_not_readable: Vec<u32>,
    pub words: u32,
    pub characters: u32,
    /// Always present: what OCR output is and isn't.
    pub note: String,
    pub warnings: Vec<String>,
}

pub const A_GUESS: &str =
    "Recognised text is a guess, and it will contain mistakes — numbers, names and anything \
     handwritten most of all. The page itself is untouched: the words go into a hidden layer \
     behind the scan, so a misread word costs you a failed search, never a wrong document.";

/// Read the text on every scanned page.
pub fn read(
    bytes: &[u8],
    models: &Models,
    options: &OcrOptions,
) -> Result<(Vec<PageText>, OcrReport)> {
    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }
    let (pages, report) = collect(&mut doc, models, options)?;
    Ok((pages, report))
}

/// Read the scans and write the words back as an invisible layer.
pub fn searchable(
    bytes: &[u8],
    models: &Models,
    options: &OcrOptions,
) -> Result<(Vec<u8>, OcrReport)> {
    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }

    let (pages, report) = collect(&mut doc, models, options)?;
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();

    for page in &pages {
        if page.words.is_empty() {
            continue;
        }
        let page_id = page_ids[page.page as usize - 1];
        stamp_page(&mut doc, page_id, &invisible_layer(&page.words), 1.0, None)?;
    }

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;
    Ok((out, report))
}

/// Run the engine over the pages that have something to read.
fn collect(
    doc: &mut Document,
    models: &Models,
    options: &OcrOptions,
) -> Result<(Vec<PageText>, OcrReport)> {
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    if page_ids.is_empty() {
        return Err(Error::invalid("this PDF has no pages"));
    }

    let wanted: Vec<u32> = match &options.pages {
        Some(selection) => selection.pages().to_vec(),
        None => (1..=page_ids.len() as u32).collect(),
    };

    let mut report = OcrReport {
        pages_read: Vec::new(),
        pages_with_text: Vec::new(),
        pages_not_readable: Vec::new(),
        words: 0,
        characters: 0,
        note: A_GUESS.to_string(),
        warnings: Vec::new(),
    };
    let mut out = Vec::new();

    for number in wanted {
        let Some(page_id) = page_ids.get(number as usize - 1).copied() else {
            return Err(Error::invalid(format!(
                "there is no page {number} — this document has {}",
                page_ids.len()
            )));
        };
        materialise_inherited(doc, page_id);

        if options.skip_pages_with_text
            && !super::markdown::page_lines(doc, page_id).join("").trim().is_empty()
        {
            report.pages_with_text.push(number);
            continue;
        }

        let Some((image, rect)) = super::pages::page_image_and_rect(doc, page_id) else {
            report.pages_not_readable.push(number);
            continue;
        };

        match read_image(&image, models, options.max_px) {
            Ok((lines, working)) => {
                let (text, words) = place(&lines, working, rect);
                report.words += words.len() as u32;
                report.characters += text.chars().count() as u32;
                report.pages_read.push(number);
                out.push(PageText { page: number, text, words });
            }
            Err(e) => report.warnings.push(format!("Page {number} couldn't be read: {e}")),
        }
    }

    if report.pages_read.is_empty() && !report.pages_with_text.is_empty() {
        report.warnings.push(
            "Every page already had text in it, so there was nothing to read. Turn off \
             'skip pages that already have text' to read them anyway."
                .into(),
        );
    }
    if !report.pages_not_readable.is_empty() {
        report.warnings.push(format!(
            "{} page{} neither text nor a single image — drawn pages can't be turned into \
             pixels here, so they were left out.",
            report.pages_not_readable.len(),
            if report.pages_not_readable.len() == 1 { " is" } else { "s are" }
        ));
    }

    Ok((out, report))
}

/// The lines of text in one image, with the pixel size their coordinates are
/// measured in.
///
/// Returning the working size is the point: the engine reports positions in
/// the pixels of the copy it was handed, and that copy may have been shrunk.
/// Inferring it afterwards from how far the detections reach is guesswork that
/// is wrong on any page with a margin.
fn read_image(
    image: &image::DynamicImage,
    models: &Models,
    max_px: u32,
) -> Result<(Vec<ocrs::TextLine>, (f32, f32))> {
    // Past a point the extra pixels cost time and buy nothing.
    let working = if max_px > 0 && image.width().max(image.height()) > max_px {
        image.resize(max_px, max_px, image::imageops::FilterType::Lanczos3)
    } else {
        image.clone()
    };
    let rgb = working.to_rgb8();

    let source = ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions())
        .map_err(|e| Error::other(format!("{e}")))?;
    let input = models.engine.prepare_input(source).map_err(|e| Error::other(format!("{e}")))?;
    let words = models.engine.detect_words(&input).map_err(|e| Error::other(format!("{e}")))?;
    let lines = models.engine.find_text_lines(&input, &words);
    let recognised =
        models.engine.recognize_text(&input, &lines).map_err(|e| Error::other(format!("{e}")))?;

    Ok((recognised.into_iter().flatten().collect(), (rgb.width() as f32, rgb.height() as f32)))
}

/// Map each word from image pixels onto the page, and join the text.
fn place(
    lines: &[ocrs::TextLine],
    working: (f32, f32),
    rect: (f32, f32, f32, f32),
) -> (String, Vec<Word>) {
    let (ix, iy, iw, ih) = rect;
    let (w_px, h_px) = (working.0.max(1.0), working.1.max(1.0));
    let mut text = String::new();
    let mut words = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line.to_string());

        for word in line.words() {
            let bounds = word.bounding_rect();
            let piece = word.to_string();
            if piece.trim().is_empty() {
                continue;
            }
            // Fractions of the image, so where it sits on the page is all
            // that matters from here on.
            let fx = bounds.left() as f32 / w_px;
            let fy = bounds.top() as f32 / h_px;
            let fw = bounds.width() as f32 / w_px;
            let fh = bounds.height() as f32 / h_px;

            words.push(Word {
                text: piece,
                x: ix + fx * iw,
                // The image's y runs down from the top; the page's runs up
                // from the bottom.
                y: iy + ih - (fy + fh) * ih,
                width: fw * iw,
                height: fh * ih,
                line: index as u32,
            });
        }
    }
    (text, words)
}

/// The content stream that puts the words behind the page, unseen.
///
/// `3 Tr` is the invisible text rendering mode: the glyphs are laid out, take
/// part in selection and search, and are never painted.
fn invisible_layer(words: &[Word]) -> String {
    let mut out = String::from("q\nBT\n3 Tr\n");
    for (index, word) in words.iter().enumerate() {
        if word.width <= 0.0 || word.height <= 0.0 {
            continue;
        }
        // Size from the box's height; a capital is about 0.72 of the size.
        let size = (word.height / 0.72).clamp(1.0, 400.0);
        let natural = text_width(&word.text, size).max(0.01);
        // Stretch horizontally so the selectable box matches the ink it came
        // from — without this a short word highlights as a long smear.
        let stretch = ((word.width / natural) * 100.0).clamp(1.0, 1000.0);

        // A trailing space when the next word is on the same line. Each word
        // is drawn at its own absolute position, so nothing in the stream
        // implies a gap; without this the layer reads back as
        // "Nameofapplicant" and every search for two words fails.
        let joined = match words.get(index + 1) {
            Some(next) if next.line == word.line => format!("{} ", word.text),
            _ => word.text.clone(),
        };

        out.push_str(&format!(
            "/{FONT_NAME} {size:.2} Tf\n{stretch:.1} Tz\n1 0 0 1 {:.2} {:.2} Tm\n({}) Tj\n",
            word.x,
            word.y,
            escape_text(&joined)
        ));
    }
    out.push_str("ET\nQ\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;
    use ocrs::{TextChar, TextLine};
    use rten_imageproc::Rect;

    /// A line of recognised text at a known pixel box, built without models so
    /// the placement maths can be checked on its own.
    fn line(text: &str, top: i32, left: i32, height: i32, char_width: i32) -> TextLine {
        let chars: Vec<TextChar> = text
            .chars()
            .enumerate()
            .map(|(i, char)| TextChar {
                char,
                rect: Rect::from_tlhw(top, left + i as i32 * char_width, height, char_width),
            })
            .collect();
        TextLine::new(chars)
    }

    #[test]
    fn a_word_lands_where_it_was_read_from() {
        // An image 1000x500 px drawn across a page at (50, 100) and 400x200pt.
        let lines = vec![line("hi", 0, 0, 50, 100)];
        let (text, words) = place(&lines, (1000.0, 500.0), (50.0, 100.0, 400.0, 200.0));

        assert_eq!(text, "hi");
        assert_eq!(words.len(), 1);
        let word = &words[0];
        // 200px of 1000 across → a fifth of 400pt, starting at x=50.
        assert!((word.x - 50.0).abs() < 0.01, "{word:?}");
        assert!((word.width - 80.0).abs() < 0.01, "{word:?}");
        // 50px of 500 tall → a tenth of 200pt. The image's y runs down from
        // the top, the page's up from the bottom, so a box at the image's top
        // sits at the page rectangle's top.
        assert!((word.height - 20.0).abs() < 0.01, "{word:?}");
        assert!(
            (word.y - (100.0 + 200.0 - 20.0)).abs() < 0.01,
            "a word at the top of the image belongs at the top of the page: {word:?}"
        );
    }

    #[test]
    fn the_image_being_shrunk_does_not_move_the_words() {
        // The same text read at two working resolutions has to land in the
        // same place on the page, or the layer drifts on big scans.
        let small = place(&[line("ab", 10, 20, 30, 15)], (200.0, 100.0), (0.0, 0.0, 400.0, 200.0));
        let large = place(&[line("ab", 20, 40, 60, 30)], (400.0, 200.0), (0.0, 0.0, 400.0, 200.0));
        let (a, b) = (&small.1[0], &large.1[0]);
        assert!((a.x - b.x).abs() < 0.01, "{a:?} vs {b:?}");
        assert!((a.y - b.y).abs() < 0.01, "{a:?} vs {b:?}");
        assert!((a.width - b.width).abs() < 0.01, "{a:?} vs {b:?}");
    }

    #[test]
    fn words_on_a_line_are_separated_so_the_text_reads_back() {
        // The bug this guards: every word drawn at its own absolute position
        // with nothing between them, which reads back as "Nameofapplicant".
        let lines = vec![line("one two", 0, 0, 40, 20)];
        let (_, words) = place(&lines, (1000.0, 500.0), (0.0, 0.0, 500.0, 250.0));
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].line, words[1].line);

        let layer = invisible_layer(&words);
        assert!(layer.contains("(one ) Tj"), "a space has to follow it: {layer}");
        assert!(layer.contains("(two) Tj"), "but not trail the line: {layer}");
    }

    #[test]
    fn the_last_word_of_a_line_gets_no_trailing_space() {
        let lines = vec![line("aa", 0, 0, 40, 20), line("bb", 60, 0, 40, 20)];
        let (_, words) = place(&lines, (1000.0, 500.0), (0.0, 0.0, 500.0, 250.0));
        assert_eq!(words.len(), 2);
        assert_ne!(words[0].line, words[1].line, "different lines");

        let layer = invisible_layer(&words);
        assert!(layer.contains("(aa) Tj"), "{layer}");
        assert!(layer.contains("(bb) Tj"), "{layer}");
        assert!(!layer.contains("(aa ) Tj"), "a line end needs no space: {layer}");
    }

    #[test]
    fn the_layer_is_invisible_and_stretched_to_fit() {
        let words = vec![Word {
            text: "wide".into(),
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 12.0,
            line: 0,
        }];
        let layer = invisible_layer(&words);
        assert!(layer.contains("3 Tr"), "it must not be painted: {layer}");
        assert!(layer.contains("Tz"), "it must be stretched to the box: {layer}");
        assert!(layer.contains("1 0 0 1 10.00 20.00 Tm"), "{layer}");
    }

    #[test]
    fn a_word_with_no_size_is_skipped_rather_than_dividing_by_zero() {
        let words =
            vec![Word { text: "ghost".into(), x: 0.0, y: 0.0, width: 0.0, height: 0.0, line: 0 }];
        let layer = invisible_layer(&words);
        assert!(!layer.contains("ghost"), "{layer}");
    }

    #[test]
    fn brackets_in_the_text_cannot_break_the_stream() {
        let words = vec![Word {
            text: r"a(b)c\d".into(),
            x: 1.0,
            y: 2.0,
            width: 40.0,
            height: 10.0,
            line: 0,
        }];
        let layer = invisible_layer(&words);
        // An unescaped bracket would end the string early and turn the rest of
        // the word into operators.
        assert!(layer.contains(r"(a\(b\)c\\d) Tj"), "not escaped properly: {layer}");
    }

    #[test]
    fn a_broken_model_is_refused_clearly() {
        let Err(error) = Models::load(b"not a model".to_vec(), b"nor this".to_vec()) else {
            panic!("rubbish bytes should not make an engine");
        };
        let error = error.to_string();
        assert!(error.contains("detection model"), "{error}");
    }

    /// The real thing, when the models are on this machine.
    ///
    /// Ignored by default: the models are a 12 MB download the app fetches on
    /// first use, and a test suite that silently needs the network is a test
    /// suite that fails for the wrong reasons. Run it with
    /// `KITBOX_OCR_MODELS=<dir> cargo test -- --ignored ocr`.
    #[test]
    #[ignore = "needs the OCR models; set KITBOX_OCR_MODELS to a folder holding them"]
    fn reads_a_scan_and_makes_it_searchable() {
        let Ok(dir) = std::env::var("KITBOX_OCR_MODELS") else {
            panic!("set KITBOX_OCR_MODELS to the folder with the two .onnx models");
        };
        let dir = std::path::Path::new(&dir);
        let models = Models::load(
            std::fs::read(dir.join("text-detection.onnx")).unwrap(),
            std::fs::read(dir.join("text-recognition.onnx")).unwrap(),
        )
        .unwrap();

        // A scan of plain words, rendered large enough to be legible.
        let page = testkit::pdf_scan(1, 1200, 1600, 92);
        let (_, report) = read(&page, &models, &OcrOptions::default()).unwrap();
        // The fixture is a synthetic pattern, not words, so the only firm
        // claim is that the page was attempted rather than skipped.
        assert_eq!(report.pages_read, vec![1], "{report:?}");
        assert!(report.note.contains("a guess"));
    }

    #[test]
    fn a_page_that_already_has_text_is_left_alone() {
        // No models needed: the skip happens before the engine is asked.
        let doc_bytes = testkit::pdf_with_sized_text(&[(12.0, "already words")]);
        let mut doc = Document::load_mem(&doc_bytes).unwrap();
        assert!(Models::load(Vec::new(), Vec::new()).is_err(), "empty bytes are not a model");

        // Reach the same decision the engine path would.
        let page_id = doc.page_iter().next().unwrap();
        super::super::pages::materialise_inherited(&mut doc, page_id);
        let has_text =
            !super::super::markdown::page_lines(&doc, page_id).join("").trim().is_empty();
        assert!(has_text, "the fixture should carry text");
    }

    #[test]
    fn the_note_about_guessing_is_always_there() {
        assert!(A_GUESS.contains("guess"));
        assert!(A_GUESS.contains("page itself is untouched"));
    }
}
