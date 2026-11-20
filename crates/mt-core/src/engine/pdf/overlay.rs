//! Drawing on top of an existing page.
//!
//! Shared by the watermark, page-number and signing tools, because the fiddly part is
//! the same for both and getting it wrong is quiet rather than loud:
//!
//! * The page's own content is fenced inside `q`/`Q`. A content stream that
//!   leaves the graphics state unbalanced — common in files from older
//!   software — would otherwise drag the overlay's transform with it and put
//!   the text somewhere unpredictable.
//! * Resources are *merged* under `Kbx`-prefixed names. Replacing a page's
//!   `/Resources` is how a stamping tool turns a scan into a blank page with
//!   text on it.
//! * Text uses Helvetica, which every reader supplies, so nothing has to be
//!   embedded. The price is that the advance widths have to come from the
//!   specification in order to centre or right-align anything.

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

use crate::error::{Error, Result};

pub const FONT_NAME: &str = "KbxWmFont";
pub const STATE_NAME: &str = "KbxWmGs";
pub const IMAGE_NAME: &str = "KbxWmImg";

/// Append `content` over a page, after fencing off whatever was already there.
pub fn stamp_page(
    doc: &mut Document,
    page_id: ObjectId,
    content: &str,
    opacity: f32,
    image_id: Option<ObjectId>,
) -> Result<()> {
    stamp_page_with(doc, page_id, content, opacity, image_id, &[])
}

/// The same, plus extra graphics states the content can switch between.
///
/// One `ca`/`CA` pair covers a watermark, which is one mark at one opacity.
/// Annotating a page is several marks at once — a highlight that multiplies
/// into the paper, an opaque shape, a half-transparent one — and each blend
/// mode and alpha has to exist as its own named state before the content can
/// name it.
pub fn stamp_page_with(
    doc: &mut Document,
    page_id: ObjectId,
    content: &str,
    opacity: f32,
    image_id: Option<ObjectId>,
    extra_states: &[(String, Dictionary)],
) -> Result<()> {
    let open = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let close = doc.add_object(Stream::new(dictionary! {}, format!("Q\n{content}").into_bytes()));

    let page = doc.get_dictionary(page_id).map_err(|_| Error::other("page disappeared"))?.clone();

    let mut contents: Vec<Object> = match page.get(b"Contents") {
        Ok(Object::Reference(id)) => vec![Object::Reference(*id)],
        Ok(Object::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    contents.insert(0, Object::Reference(open));
    contents.push(Object::Reference(close));

    let mut resources = sub_dictionary(doc, &page, b"Resources");

    let mut ext_g_state = sub_dictionary(doc, &resources, b"ExtGState");
    ext_g_state.set(
        STATE_NAME,
        dictionary! {
            "Type" => "ExtGState",
            // Fill and stroke alpha both, or a stroked mark stays opaque.
            "ca" => opacity as f64,
            "CA" => opacity as f64,
        },
    );
    for (name, state) in extra_states {
        ext_g_state.set(name.clone(), state.clone());
    }
    resources.set("ExtGState", ext_g_state);

    let mut fonts = sub_dictionary(doc, &resources, b"Font");
    fonts.set(
        FONT_NAME,
        dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        },
    );
    resources.set("Font", fonts);

    if let Some(image_id) = image_id {
        let mut xobjects = sub_dictionary(doc, &resources, b"XObject");
        xobjects.set(IMAGE_NAME, Object::Reference(image_id));
        resources.set("XObject", xobjects);
    }

    let page = doc.get_dictionary_mut(page_id).map_err(|_| Error::other("page disappeared"))?;
    page.set("Contents", Object::Array(contents));
    page.set("Resources", resources);
    Ok(())
}

/// A nested dictionary, whether it is inline or behind a reference.
pub fn sub_dictionary(doc: &Document, parent: &Dictionary, key: &[u8]) -> Dictionary {
    match parent.get(key) {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    }
}

/* ---------- the page as the reader sees it ---------- */

/// A page's size and its `/Rotate`, together.
///
/// A scanned page is often stored upright and rotated for display. Anything
/// stamped onto it has to be placed in *displayed* space and transformed back,
/// or it lands sideways in a corner nobody picked. Page numbers and signatures
/// both need this, and two copies of it would drift.
pub struct Frame {
    pub media_w: f32,
    pub media_h: f32,
    pub rotation: i32,
}

impl Frame {
    /// Width and height as displayed — swapped on a quarter-turned page.
    pub fn displayed(&self) -> (f32, f32) {
        if self.rotation.rem_euclid(180) == 90 {
            (self.media_h, self.media_w)
        } else {
            (self.media_w, self.media_h)
        }
    }

    /// Where displayed `(dx, dy)` actually is in the page's own coordinates.
    ///
    /// The matrix form below is what you want for *drawing* something that
    /// should read upright. This is for the other direction: working out which
    /// part of the page a reader's rectangle covers.
    pub fn point_in_page_space(&self, dx: f32, dy: f32) -> (f32, f32) {
        match self.rotation.rem_euclid(360) {
            90 => (self.media_w - dy, dx),
            180 => (self.media_w - dx, self.media_h - dy),
            270 => (dy, self.media_h - dx),
            _ => (dx, dy),
        }
    }

    /// A text matrix that puts the baseline at displayed `(dx, dy)` and keeps
    /// it running left to right *for the reader*, undoing `/Rotate`.
    pub fn to_page_space(&self, dx: f32, dy: f32) -> String {
        let (a, b, c, d, e, f) = match self.rotation.rem_euclid(360) {
            90 => (0.0, 1.0, -1.0, 0.0, self.media_w - dy, dx),
            180 => (-1.0, 0.0, 0.0, -1.0, self.media_w - dx, self.media_h - dy),
            270 => (0.0, -1.0, 1.0, 0.0, dy, self.media_h - dx),
            _ => (1.0, 0.0, 0.0, 1.0, dx, dy),
        };
        format!("{a} {b} {c} {d} {e:.2} {f:.2}")
    }
}

pub fn rotation_of(doc: &Document, page_id: ObjectId) -> i32 {
    doc.get_dictionary(page_id)
        .ok()
        .and_then(|d| d.get(b"Rotate").ok())
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0) as i32
}

/// Escape the three characters a PDF literal string cannot carry raw.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            // WinAnsiEncoding is single-byte; anything outside it would draw
            // as nonsense, so it becomes a question mark rather than silently
            // corrupting the stream.
            c if (c as u32) < 256 => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// Width of a string in Helvetica, in points, at the given size.
pub fn text_width(text: &str, font_size: f32) -> f32 {
    let thousandths: u32 = text
        .chars()
        .map(|c| {
            let index = c as u32;
            if (32..127).contains(&index) {
                HELVETICA_WIDTHS[(index - 32) as usize] as u32
            } else {
                // A reasonable stand-in for anything off the table.
                556
            }
        })
        .sum();
    thousandths as f32 / 1000.0 * font_size
}

/// Standard Helvetica advance widths for printable ASCII, in 1/1000 em.
/// Readers supply the font, so these come from the specification rather than
/// from measuring one.
pub const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278,
    278, // space../
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 0..?
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // @..O
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // P.._
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // `..o
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // p..~
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helvetica_widths_match_the_specification() {
        assert_eq!(text_width(" ", 1000.0).round(), 278.0);
        assert_eq!(text_width("M", 1000.0).round(), 833.0);
        assert_eq!(text_width("i", 1000.0).round(), 222.0);
        assert_eq!(text_width("~", 1000.0).round(), 584.0);
        assert_eq!(HELVETICA_WIDTHS.len(), 95, "exactly printable ASCII");
    }

    #[test]
    fn escaping_covers_what_breaks_a_stream() {
        assert_eq!(escape_text(r"(c) \ x"), r"\(c\) \\ x");
        // Outside WinAnsi, so it must not reach the stream raw.
        assert_eq!(escape_text("日"), "?");
    }
}
