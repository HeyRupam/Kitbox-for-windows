//! Filling in a PDF's form fields.
//!
//! The thing people actually want is to type into the form on screen and send
//! it, instead of printing it, writing on it, and photographing the result.
//!
//! Two traps make the difference between a form that works and one that looks
//! filled in on your machine and arrives empty.
//!
//! **A value is not an appearance.** Setting a field's `/V` records what it
//! says; it does not draw anything. Most readers render the *appearance
//! stream* and ignore the value, so a file filled by setting `/V` alone shows
//! up blank in half the world's PDF viewers and in every printer. The
//! honest-looking shortcut is `/NeedAppearances true`, which asks the reader to
//! draw it — Acrobat obliges, plenty of others don't, and no printer does. So
//! this builds the appearance streams itself.
//!
//! **A checkbox's "on" is not `/Yes`.** It is whatever key the widget's `/AP`
//! dictionary uses, which is `/Yes` often, `/On` sometimes, and `/1`, `/Ja` or
//! something the form's author invented the rest of the time. Writing `/Yes`
//! into a field whose on-state is `/1` leaves a box that is neither on nor off
//! and renders as empty.
//!
//! Flattening draws each appearance into the page and takes the form away, so
//! the values can't be edited or accidentally cleared. It is what "submit a
//! filled PDF" nearly always means, and it is one-way — which the report says.

/// A widget's box on the page: left, bottom, right, top, in points.
type Rect = (f32, f32, f32, f32);

use std::collections::BTreeMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};

use super::overlay::{escape_text, sub_dictionary, text_width};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    /// A text field that wraps.
    Multiline,
    Password,
    Checkbox,
    /// One of a set; the options are the on-states.
    Radio,
    /// A dropdown or list.
    Choice,
    /// A push button: nothing to fill in.
    Button,
    Signature,
}

impl FieldKind {
    /// Whether a value can be typed into it.
    pub fn takes_a_value(self) -> bool {
        !matches!(self, FieldKind::Button | FieldKind::Signature)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// The full dotted name, which is what a filler addresses it by.
    pub name: String,
    /// The form author's own words for it, when they left any.
    pub label: Option<String>,
    pub kind: FieldKind,
    pub value: Option<String>,
    /// What a checkbox, radio or dropdown will accept.
    pub options: Vec<String>,
    pub required: bool,
    pub read_only: bool,
    pub max_length: Option<u32>,
    /// Pages this field appears on. Usually one; a signature block repeated on
    /// every page is one field with many widgets.
    pub pages: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillOptions {
    /// Field name to value. A checkbox takes one of its options, or any of
    /// "yes"/"true"/"on"/"1" for the on-state, and "off"/"no"/"false"/"" for off.
    pub values: BTreeMap<String, String>,
    /// Draw the values into the pages and remove the form afterwards.
    pub flatten: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillReport {
    pub filled: Vec<String>,
    /// Names given that this document has no field for. Reported rather than
    /// ignored: a typo in a field name is otherwise completely silent.
    pub unknown: Vec<String>,
    /// Fields the form marks as not editable, left alone.
    pub read_only: Vec<String>,
    /// Required fields still empty after filling.
    pub still_required: Vec<String>,
    pub flattened: bool,
    pub warnings: Vec<String>,
}

/// What a document with no form in it gets told.
pub const NOT_A_FORM: &str =
    "This PDF has no form fields in it. A form that was scanned or printed to PDF is a picture of \
     a form, not a fillable one — there is nothing to type into. Add the text with the signing or \
     watermark tools, or fill it by hand.";

/* ---------- reading ---------- */

/// Every field in the document, in the order the form declares them.
pub fn fields(bytes: &[u8]) -> Result<Vec<Field>> {
    let doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }
    Ok(collect(&doc))
}

fn collect(doc: &Document) -> Vec<Field> {
    let widget_pages = widget_pages(doc);
    let Some(roots) = acroform_fields(doc) else { return Vec::new() };

    let mut out = Vec::new();
    for root in roots {
        walk(doc, root, "", None, 0, &widget_pages, &mut out);
    }
    out
}

/// Which page each annotation sits on, so a field can say where it is.
fn widget_pages(doc: &Document) -> BTreeMap<ObjectId, u32> {
    let mut map = BTreeMap::new();
    for (index, page_id) in doc.page_iter().enumerate() {
        let Ok(page) = doc.get_dictionary(page_id) else { continue };
        let Ok(annots) = page.get(b"Annots") else { continue };
        let list = match annots {
            Object::Array(items) => items.clone(),
            Object::Reference(id) => doc
                .get_object(*id)
                .ok()
                .and_then(|o| o.as_array().ok().cloned())
                .unwrap_or_default(),
            _ => continue,
        };
        for entry in list {
            if let Object::Reference(id) = entry {
                map.insert(id, index as u32 + 1);
            }
        }
    }
    map
}

fn acroform_fields(doc: &Document) -> Option<Vec<ObjectId>> {
    let catalog = doc.catalog().ok()?;
    let form = sub_dictionary(doc, catalog, b"AcroForm");
    let fields = form.get(b"Fields").ok()?;
    let list = match fields {
        Object::Array(items) => items.clone(),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?.clone(),
        _ => return None,
    };
    Some(
        list.iter()
            .filter_map(|o| match o {
                Object::Reference(id) => Some(*id),
                _ => None,
            })
            .collect(),
    )
}

/// Walk the field tree, carrying down what a child inherits.
fn walk(
    doc: &Document,
    id: ObjectId,
    prefix: &str,
    inherited_kind: Option<Vec<u8>>,
    inherited_flags: i64,
    widget_pages: &BTreeMap<ObjectId, u32>,
    out: &mut Vec<Field>,
) {
    let Ok(dict) = doc.get_dictionary(id).cloned() else { return };

    let partial =
        dict.get(b"T").ok().and_then(|o| o.as_str().ok()).map(decode_text).unwrap_or_default();
    let name = match (prefix.is_empty(), partial.is_empty()) {
        (_, true) => prefix.to_string(),
        (true, false) => partial.clone(),
        (false, false) => format!("{prefix}.{partial}"),
    };

    let kind_name =
        dict.get(b"FT").ok().and_then(|o| o.as_name().ok()).map(|n| n.to_vec()).or(inherited_kind);
    let flags = dict.get(b"Ff").ok().and_then(|o| o.as_i64().ok()).unwrap_or(inherited_flags);

    // Kids are either child fields or this field's own widgets.
    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .ok()
        .and_then(|o| match o {
            Object::Array(items) => Some(items.clone()),
            Object::Reference(r) => {
                doc.get_object(*r).ok().and_then(|o| o.as_array().ok().cloned())
            }
            _ => None,
        })
        .map(|items| {
            items
                .iter()
                .filter_map(|o| match o {
                    Object::Reference(id) => Some(*id),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let child_fields: Vec<ObjectId> = kids
        .iter()
        .copied()
        .filter(|kid| doc.get_dictionary(*kid).map(|d| d.has(b"T")).unwrap_or(false))
        .collect();

    if !child_fields.is_empty() {
        for kid in child_fields {
            walk(doc, kid, &name, kind_name.clone(), flags, widget_pages, out);
        }
        return;
    }

    let Some(kind_name) = kind_name else { return };
    let kind = classify(&kind_name, flags);

    // The widgets are this node itself when it has no kids, otherwise its kids.
    let widgets: Vec<ObjectId> = if kids.is_empty() { vec![id] } else { kids };
    let mut pages: Vec<u32> = widgets.iter().filter_map(|w| widget_pages.get(w).copied()).collect();
    pages.sort_unstable();
    pages.dedup();

    let mut options = match kind {
        FieldKind::Checkbox | FieldKind::Radio => {
            let mut states: Vec<String> = widgets.iter().flat_map(|w| on_states(doc, *w)).collect();
            states.sort();
            states.dedup();
            states
        }
        FieldKind::Choice => choice_options(doc, &dict),
        _ => Vec::new(),
    };
    options.retain(|o| o != "Off");

    out.push(Field {
        name,
        label: dict.get(b"TU").ok().and_then(|o| o.as_str().ok()).map(decode_text),
        kind,
        value: current_value(doc, &dict),
        options,
        // Bit 2 is required, bit 1 is read-only, counting from 1.
        required: flags & 0b10 != 0,
        read_only: flags & 0b1 != 0,
        max_length: dict.get(b"MaxLen").ok().and_then(|o| o.as_i64().ok()).map(|n| n as u32),
        pages,
    });
}

fn classify(kind: &[u8], flags: i64) -> FieldKind {
    match kind {
        b"Tx" => {
            if flags & (1 << 12) != 0 {
                FieldKind::Multiline
            } else if flags & (1 << 13) != 0 {
                FieldKind::Password
            } else {
                FieldKind::Text
            }
        }
        b"Btn" => {
            if flags & (1 << 16) != 0 {
                FieldKind::Button
            } else if flags & (1 << 15) != 0 {
                FieldKind::Radio
            } else {
                FieldKind::Checkbox
            }
        }
        b"Ch" => FieldKind::Choice,
        b"Sig" => FieldKind::Signature,
        _ => FieldKind::Text,
    }
}

/// The names a checkbox or radio widget will actually accept.
fn on_states(doc: &Document, widget: ObjectId) -> Vec<String> {
    let Ok(dict) = doc.get_dictionary(widget) else { return Vec::new() };
    let ap = sub_dictionary(doc, dict, b"AP");
    let normal = sub_dictionary(doc, &ap, b"N");
    normal
        .iter()
        .map(|(key, _)| String::from_utf8_lossy(key).to_string())
        .filter(|key| key != "Off")
        .collect()
}

fn choice_options(doc: &Document, dict: &Dictionary) -> Vec<String> {
    let Ok(opt) = dict.get(b"Opt") else { return Vec::new() };
    let list = match opt {
        Object::Array(items) => items.clone(),
        Object::Reference(id) => {
            doc.get_object(*id).ok().and_then(|o| o.as_array().ok().cloned()).unwrap_or_default()
        }
        _ => return Vec::new(),
    };
    list.iter()
        .map(|item| match item {
            // An option can be [export, display]; the display one is what a
            // person recognises.
            Object::Array(pair) => {
                pair.last().and_then(|o| o.as_str().ok()).map(decode_text).unwrap_or_default()
            }
            other => other.as_str().ok().map(decode_text).unwrap_or_default(),
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn current_value(doc: &Document, dict: &Dictionary) -> Option<String> {
    let value = dict.get(b"V").ok()?;
    let value = match value {
        Object::Reference(id) => doc.get_object(*id).ok()?.clone(),
        other => other.clone(),
    };
    match value {
        Object::String(bytes, _) => Some(decode_text(&bytes)),
        Object::Name(name) => {
            let text = String::from_utf8_lossy(&name).to_string();
            (text != "Off").then_some(text)
        }
        Object::Array(items) => {
            let joined: Vec<String> =
                items.iter().filter_map(|o| o.as_str().ok()).map(decode_text).collect();
            (!joined.is_empty()).then(|| joined.join(", "))
        }
        _ => None,
    }
}

/// A PDF text string is either UTF-16BE with a byte-order mark or PDFDoc
/// encoding, which is Latin-1 for everything a form is likely to hold.
fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> =
            bytes[2..].chunks_exact(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|b| *b as char).collect()
    }
}

/// Values that aren't plain ASCII have to go back as UTF-16BE, or a name with
/// an accent in it arrives mangled.
fn encode_text(text: &str) -> Object {
    if text.is_ascii() {
        Object::string_literal(text)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, lopdf::StringFormat::Hexadecimal)
    }
}

/* ---------- filling ---------- */

/// Put values into the form, drawing them so every reader shows them.
pub fn fill(bytes: &[u8], options: &FillOptions) -> Result<(Vec<u8>, FillReport)> {
    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }

    let known = collect(&doc);
    if known.is_empty() {
        return Err(Error::unsupported(NOT_A_FORM));
    }

    let mut report = FillReport {
        filled: Vec::new(),
        unknown: Vec::new(),
        read_only: Vec::new(),
        still_required: Vec::new(),
        flattened: false,
        warnings: Vec::new(),
    };

    let by_name: BTreeMap<&str, &Field> = known.iter().map(|f| (f.name.as_str(), f)).collect();
    for name in options.values.keys() {
        if !by_name.contains_key(name.as_str()) {
            report.unknown.push(name.clone());
        }
    }

    // The field objects, found again by name so the tree walk isn't repeated.
    let targets = locate(&doc);

    for (name, value) in &options.values {
        let Some(field) = by_name.get(name.as_str()) else { continue };
        if field.read_only {
            report.read_only.push(name.clone());
            continue;
        }
        if !field.kind.takes_a_value() {
            report.warnings.push(format!("'{name}' is a {:?} and takes no value", field.kind));
            continue;
        }
        let Some((field_id, widgets)) = targets.get(name.as_str()) else { continue };

        match field.kind {
            FieldKind::Checkbox | FieldKind::Radio => {
                if !set_button(&mut doc, *field_id, widgets, field, value, &mut report) {
                    // It was refused and the reason recorded; saying it was
                    // filled as well would be a lie the caller acts on.
                    continue;
                }
            }
            _ => {
                if let Some(max) = field.max_length {
                    if value.chars().count() > max as usize {
                        report.warnings.push(format!(
                            "'{name}' holds at most {max} characters; the rest was cut off"
                        ));
                    }
                }
                let trimmed: String = match field.max_length {
                    Some(max) => value.chars().take(max as usize).collect(),
                    None => value.clone(),
                };
                set_text(&mut doc, *field_id, widgets, field, &trimmed);
            }
        }
        report.filled.push(name.clone());
    }

    // Anything the form insists on that is still blank.
    let after = collect(&doc);
    for field in &after {
        if field.required && field.value.as_deref().unwrap_or("").is_empty() {
            report.still_required.push(field.name.clone());
        }
    }

    // Appearances have been written, so the reader must not regenerate them —
    // left on, some readers redraw and lose the formatting that was just set.
    set_need_appearances(&mut doc, false);

    if options.flatten {
        flatten_in_place(&mut doc)?;
        report.flattened = true;
    }

    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;
    Ok((out, report))
}

/// Field name to its object and its widget objects.
fn locate(doc: &Document) -> BTreeMap<String, (ObjectId, Vec<ObjectId>)> {
    let mut out = BTreeMap::new();
    let Some(roots) = acroform_fields(doc) else { return out };
    for root in roots {
        locate_walk(doc, root, "", &mut out);
    }
    out
}

fn locate_walk(
    doc: &Document,
    id: ObjectId,
    prefix: &str,
    out: &mut BTreeMap<String, (ObjectId, Vec<ObjectId>)>,
) {
    let Ok(dict) = doc.get_dictionary(id).cloned() else { return };
    let partial =
        dict.get(b"T").ok().and_then(|o| o.as_str().ok()).map(decode_text).unwrap_or_default();
    let name = match (prefix.is_empty(), partial.is_empty()) {
        (_, true) => prefix.to_string(),
        (true, false) => partial,
        (false, false) => format!("{prefix}.{partial}"),
    };

    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .ok()
        .and_then(|o| match o {
            Object::Array(items) => Some(items.clone()),
            Object::Reference(r) => {
                doc.get_object(*r).ok().and_then(|o| o.as_array().ok().cloned())
            }
            _ => None,
        })
        .map(|items| {
            items
                .iter()
                .filter_map(|o| match o {
                    Object::Reference(id) => Some(*id),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let child_fields: Vec<ObjectId> = kids
        .iter()
        .copied()
        .filter(|kid| doc.get_dictionary(*kid).map(|d| d.has(b"T")).unwrap_or(false))
        .collect();

    if !child_fields.is_empty() {
        for kid in child_fields {
            locate_walk(doc, kid, &name, out);
        }
        return;
    }

    let widgets = if kids.is_empty() { vec![id] } else { kids };
    out.insert(name, (id, widgets));
}

fn set_text(
    doc: &mut Document,
    field_id: ObjectId,
    widgets: &[ObjectId],
    field: &Field,
    value: &str,
) {
    if let Ok(dict) = doc.get_dictionary_mut(field_id) {
        dict.set("V", encode_text(value));
    }
    for widget in widgets {
        if let Some(stream) = text_appearance(doc, *widget, field_id, field, value) {
            let id = doc.add_object(stream);
            attach_appearance(doc, *widget, id);
        }
    }
}

/// Returns whether the value was accepted.
fn set_button(
    doc: &mut Document,
    field_id: ObjectId,
    widgets: &[ObjectId],
    field: &Field,
    value: &str,
    report: &mut FillReport,
) -> bool {
    let wanted = value.trim();
    let off = matches!(
        wanted.to_ascii_lowercase().as_str(),
        "" | "off" | "no" | "false" | "0" | "unchecked"
    );

    // "yes" has to be translated into whatever this particular form calls on,
    // which is frequently not "Yes".
    let chosen = if off {
        "Off".to_string()
    } else {
        let generic =
            matches!(wanted.to_ascii_lowercase().as_str(), "yes" | "true" | "on" | "1" | "checked");
        match field.options.iter().find(|o| o.eq_ignore_ascii_case(wanted)) {
            Some(exact) => exact.clone(),
            None if generic => match field.options.first() {
                Some(only) => only.clone(),
                None => {
                    report.warnings.push(format!(
                        "'{}' has no on-state to set, so it was left alone",
                        field.name
                    ));
                    return false;
                }
            },
            None => {
                report.warnings.push(format!(
                    "'{}' doesn't accept '{wanted}' — it takes {}",
                    field.name,
                    field.options.join(", ")
                ));
                return false;
            }
        }
    };

    if let Ok(dict) = doc.get_dictionary_mut(field_id) {
        dict.set("V", Object::Name(chosen.clone().into_bytes()));
    }
    for widget in widgets {
        // A radio group's other widgets have to show Off, so only the one
        // carrying this state turns on.
        let states = on_states(doc, *widget);
        let shown = if chosen != "Off" && states.contains(&chosen) {
            chosen.clone()
        } else {
            "Off".to_string()
        };
        if let Ok(dict) = doc.get_dictionary_mut(*widget) {
            dict.set("AS", Object::Name(shown.into_bytes()));
        }
    }
    true
}

/// The widget's box, and whether it is lying on its side.
fn widget_rect(doc: &Document, widget: ObjectId) -> Option<(f32, f32, f32, f32)> {
    let dict = doc.get_dictionary(widget).ok()?;
    let rect = dict.get(b"Rect").ok()?;
    let items = match rect {
        Object::Array(items) => items.clone(),
        Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?.clone(),
        _ => return None,
    };
    if items.len() < 4 {
        return None;
    }
    let v: Vec<f32> = items.iter().map(|o| o.as_float().unwrap_or(0.0)).collect();
    Some((v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])))
}

/// Font, size and colour the form asked for, from a `/DA` string.
fn default_appearance(
    doc: &Document,
    widget: ObjectId,
    field_id: ObjectId,
) -> (String, f32, [f32; 3]) {
    let read = |id: ObjectId| -> Option<String> {
        doc.get_dictionary(id)
            .ok()?
            .get(b"DA")
            .ok()?
            .as_str()
            .ok()
            .map(|b| String::from_utf8_lossy(b).to_string())
    };
    let form_default =
        doc.catalog().ok().map(|c| sub_dictionary(doc, c, b"AcroForm")).and_then(|f| {
            f.get(b"DA")
                .ok()
                .and_then(|o| o.as_str().ok())
                .map(|b| String::from_utf8_lossy(b).to_string())
        });

    let da = read(widget).or_else(|| read(field_id)).or(form_default).unwrap_or_default();

    let mut font = "Helv".to_string();
    let mut size = 0.0f32;
    let mut colour = [0.0, 0.0, 0.0];
    let tokens: Vec<&str> = da.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        match *token {
            "Tf" if i >= 2 => {
                font = tokens[i - 2].trim_start_matches('/').to_string();
                size = tokens[i - 1].parse().unwrap_or(0.0);
            }
            "g" if i >= 1 => {
                let grey = tokens[i - 1].parse().unwrap_or(0.0);
                colour = [grey, grey, grey];
            }
            "rg" if i >= 3 => {
                for (slot, raw) in colour.iter_mut().zip(&tokens[i - 3..i]) {
                    *slot = raw.parse().unwrap_or(0.0);
                }
            }
            _ => {}
        }
    }
    (font, size, colour)
}

/// Draw the value into a form XObject the size of the widget.
fn text_appearance(
    doc: &mut Document,
    widget: ObjectId,
    field_id: ObjectId,
    field: &Field,
    value: &str,
) -> Option<Stream> {
    let (x0, y0, x1, y1) = widget_rect(doc, widget)?;
    let (width, height) = (x1 - x0, y1 - y0);
    if width <= 1.0 || height <= 1.0 {
        return None;
    }

    let (font, mut size, colour) = default_appearance(doc, widget, field_id);
    let multiline = field.kind == FieldKind::Multiline;
    // Size 0 means "fit it to the box", which is what most forms ask for.
    if size <= 0.0 {
        size = if multiline { 10.0 } else { (height * 0.62).clamp(4.0, 14.0) };
    }

    // A password field shows dots, not the password — rendering the characters
    // into the page would defeat the field's entire purpose.
    let shown: String = if field.kind == FieldKind::Password {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };

    let quadding = doc
        .get_dictionary(field_id)
        .ok()
        .and_then(|d| d.get(b"Q").ok().and_then(|o| o.as_i64().ok()))
        .unwrap_or(0);

    const PAD: f32 = 2.0;
    let mut content = String::new();
    content.push_str(&format!(
        "/Tx BMC\nq\nBT\n/{font} {size} Tf\n{:.3} {:.3} {:.3} rg\n",
        colour[0], colour[1], colour[2]
    ));

    if multiline {
        let lines = wrap(&shown, size, width - PAD * 2.0);
        let leading = size * 1.18;
        let mut y = height - PAD - size;
        for line in lines {
            if y < -size {
                break;
            }
            content.push_str(&format!(
                "1 0 0 1 {:.2} {:.2} Tm\n({}) Tj\n",
                PAD,
                y,
                escape_text(&line)
            ));
            y -= leading;
        }
    } else {
        // Vertically centred on the box, which is where a reader draws it.
        let y = (height - size * 0.72) / 2.0;
        let text_w = text_width(&shown, size);
        let x = match quadding {
            1 => (width - text_w) / 2.0,
            2 => width - text_w - PAD,
            _ => PAD,
        }
        .max(PAD);
        content.push_str(&format!(
            "1 0 0 1 {:.2} {:.2} Tm\n({}) Tj\n",
            x,
            y.max(PAD),
            escape_text(&shown)
        ));
    }
    content.push_str("ET\nQ\nEMC\n");

    // The appearance needs the font it names, so carry the form's resources in.
    let resources = doc
        .catalog()
        .ok()
        .map(|c| sub_dictionary(doc, c, b"AcroForm"))
        .map(|f| sub_dictionary(doc, &f, b"DR"))
        .unwrap_or_default();

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set("FormType", 1i64);
    dict.set("BBox", Object::Array(vec![0.into(), 0.into(), width.into(), height.into()]));
    dict.set("Resources", Object::Dictionary(resources));
    Some(Stream::new(dict, content.into_bytes()))
}

/// Break a line to fit, on spaces where it can and mid-word where it must.
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate =
                if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(&candidate, size) <= width || line.is_empty() {
                line = candidate;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
            // A single word longer than the box still has to go somewhere.
            while text_width(&line, size) > width && line.chars().count() > 1 {
                let mut cut = line.chars().count() - 1;
                while cut > 1
                    && text_width(&line.chars().take(cut).collect::<String>(), size) > width
                {
                    cut -= 1;
                }
                out.push(line.chars().take(cut).collect());
                line = line.chars().skip(cut).collect();
            }
        }
        out.push(line);
    }
    out
}

fn attach_appearance(doc: &mut Document, widget: ObjectId, stream: ObjectId) {
    if let Ok(dict) = doc.get_dictionary_mut(widget) {
        let mut normal = Dictionary::new();
        normal.set("N", Object::Reference(stream));
        dict.set("AP", Object::Dictionary(normal));
    }
}

fn set_need_appearances(doc: &mut Document, value: bool) {
    let Ok(catalog) = doc.catalog() else { return };
    let Ok(form) = catalog.get(b"AcroForm") else { return };
    match form.clone() {
        Object::Reference(id) => {
            if let Ok(dict) = doc.get_dictionary_mut(id) {
                dict.set("NeedAppearances", Object::Boolean(value));
            }
        }
        Object::Dictionary(mut dict) => {
            dict.set("NeedAppearances", Object::Boolean(value));
            let root = doc.trailer.get(b"Root").ok().and_then(|o| match o {
                Object::Reference(id) => Some(*id),
                _ => None,
            });
            if let Some(root) = root {
                if let Ok(catalog) = doc.get_dictionary_mut(root) {
                    catalog.set("AcroForm", Object::Dictionary(dict));
                }
            }
        }
        _ => {}
    }
}

/* ---------- flattening ---------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlattenReport {
    pub widgets_drawn: u32,
    pub pages_changed: Vec<u32>,
    pub warnings: Vec<String>,
    /// Always said: this cannot be undone.
    pub note: String,
}

pub const ONE_WAY: &str =
    "The values are now part of the page. The form is gone, so nothing can be typed into it or \
     cleared by accident — and nothing can be changed back either. Keep the unflattened copy if \
     you may need to edit it.";

/// Draw anything that has a value but nothing to show for it.
fn ensure_appearances(doc: &mut Document) {
    let known = collect(doc);
    let targets = locate(doc);

    for field in &known {
        let Some(value) = field.value.clone().filter(|v| !v.is_empty()) else { continue };
        let Some((field_id, widgets)) = targets.get(&field.name) else { continue };

        for widget in widgets {
            let Ok(dict) = doc.get_dictionary(*widget).cloned() else { continue };
            if appearance_stream(doc, &dict).is_some() {
                continue;
            }
            match field.kind {
                FieldKind::Checkbox | FieldKind::Radio => {
                    // The appearance exists per state; it just isn't being
                    // pointed at because /AS was never set.
                    let states = on_states(doc, *widget);
                    let shown = if states.contains(&value) { value.clone() } else { "Off".into() };
                    if let Ok(dict) = doc.get_dictionary_mut(*widget) {
                        dict.set("AS", Object::Name(shown.into_bytes()));
                    }
                }
                kind if kind.takes_a_value() => {
                    if let Some(stream) = text_appearance(doc, *widget, *field_id, field, &value) {
                        let id = doc.add_object(stream);
                        attach_appearance(doc, *widget, id);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Draw every field's appearance into its page and remove the form.
pub fn flatten(bytes: &[u8]) -> Result<(Vec<u8>, FlattenReport)> {
    let mut doc = Document::load_mem(bytes)
        .map_err(|e| Error::invalid(format!("could not read this PDF: {e}")))?;
    if doc.is_encrypted() {
        return Err(Error::unsupported("this PDF is password-protected — unlock it first"));
    }
    if collect(&doc).is_empty() {
        return Err(Error::unsupported(NOT_A_FORM));
    }
    let report = flatten_in_place(&mut doc)?;
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| Error::other(format!("could not write the PDF: {e}")))?;
    Ok((out, report))
}

fn flatten_in_place(doc: &mut Document) -> Result<FlattenReport> {
    // Anything holding a value has to be drawable before the form goes away,
    // or it silently disappears. A field can carry a value and no appearance:
    // forms that rely on /NeedAppearances ship none at all, and a value set by
    // another tool often has none either. Flattening those drops the answer
    // from the page while the report happily says it flattened six fields.
    ensure_appearances(doc);

    let mut report = FlattenReport {
        widgets_drawn: 0,
        pages_changed: Vec::new(),
        warnings: Vec::new(),
        note: ONE_WAY.to_string(),
    };

    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    for (index, page_id) in page_ids.iter().enumerate() {
        let number = index as u32 + 1;
        let annots: Vec<ObjectId> =
            match doc.get_dictionary(*page_id).and_then(|p| p.get(b"Annots")) {
                Ok(Object::Array(items)) => items
                    .iter()
                    .filter_map(|o| match o {
                        Object::Reference(id) => Some(*id),
                        _ => None,
                    })
                    .collect(),
                Ok(Object::Reference(id)) => doc
                    .get_object(*id)
                    .ok()
                    .and_then(|o| o.as_array().ok().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|o| match o {
                        Object::Reference(id) => Some(*id),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
        if annots.is_empty() {
            continue;
        }

        let mut drawn: Vec<(usize, ObjectId, Rect)> = Vec::new();
        let mut keep: Vec<Object> = Vec::new();

        for widget in annots {
            let Ok(dict) = doc.get_dictionary(widget).cloned() else { continue };
            let is_widget = dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| n == b"Widget")
                .unwrap_or(false);
            if !is_widget {
                // Comments and links aren't part of the form; leave them.
                keep.push(Object::Reference(widget));
                continue;
            }
            // Hidden fields shouldn't suddenly appear when flattened.
            let hidden = dict.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0) & 0b10 != 0;
            let Some(rect) = widget_rect(doc, widget) else { continue };
            if hidden {
                continue;
            }

            if let Some(appearance) = appearance_stream(doc, &dict) {
                drawn.push((drawn.len(), appearance, rect));
            }
        }

        if drawn.is_empty() {
            if let Ok(page) = doc.get_dictionary_mut(*page_id) {
                page.remove(b"Annots");
                if !keep.is_empty() {
                    page.set("Annots", Object::Array(keep));
                }
            }
            continue;
        }

        let mut ops: Vec<Operation> = Vec::new();
        let mut extra_resources = Dictionary::new();
        for (index, appearance, rect) in &drawn {
            match inline(doc, *appearance, *index, *rect, &mut extra_resources) {
                Some(mut inner) => ops.append(&mut inner),
                None => report.warnings.push(format!(
                    "A field on page {number} had an appearance that couldn't be read, so it \
                     isn't in the flattened page."
                )),
            }
        }

        let encoded = Content { operations: ops }
            .encode()
            .map_err(|e| Error::other(format!("could not draw the fields: {e}")))?;
        let addition = doc.add_object(Stream::new(Dictionary::new(), encoded));

        // The page's own drawing is fenced first, so an unbalanced stream
        // can't drag the fields somewhere unexpected.
        let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
        let close = doc.add_object(Stream::new(Dictionary::new(), b"Q\n".to_vec()));

        let page = doc.get_dictionary(*page_id).cloned().unwrap_or_default();
        let mut contents: Vec<Object> = match page.get(b"Contents") {
            Ok(Object::Reference(id)) => vec![Object::Reference(*id)],
            Ok(Object::Array(items)) => items.clone(),
            _ => Vec::new(),
        };
        contents.insert(0, Object::Reference(open));
        contents.push(Object::Reference(close));
        contents.push(Object::Reference(addition));

        let mut resources = sub_dictionary(doc, &page, b"Resources");
        for (category, entries) in extra_resources.iter() {
            let Object::Dictionary(entries) = entries else { continue };
            let mut existing = sub_dictionary(doc, &resources, category);
            for (key, value) in entries.iter() {
                existing.set(key.to_vec(), value.clone());
            }
            resources.set(category.to_vec(), Object::Dictionary(existing));
        }

        report.widgets_drawn += drawn.len() as u32;
        report.pages_changed.push(number);

        if let Ok(page) = doc.get_dictionary_mut(*page_id) {
            page.set("Contents", Object::Array(contents));
            page.set("Resources", Object::Dictionary(resources));
            page.remove(b"Annots");
            if !keep.is_empty() {
                page.set("Annots", Object::Array(keep));
            }
        }
    }

    // The form itself goes, or a reader will offer to edit fields that no
    // longer exist and may blank what was just drawn.
    let root = doc.trailer.get(b"Root").ok().and_then(|o| match o {
        Object::Reference(id) => Some(*id),
        _ => None,
    });
    if let Some(root) = root {
        if let Ok(catalog) = doc.get_dictionary_mut(root) {
            catalog.remove(b"AcroForm");
        }
    }
    doc.prune_objects();

    Ok(report)
}

/// Copy an appearance's drawing straight into the page.
///
/// The obvious way is to keep the appearance as a form XObject and `Do` it,
/// which renders correctly — and makes the values unextractable, because text
/// extraction reads a page's own content stream and does not descend into
/// XObjects. A flattened form whose values can't be copied out, searched, or
/// read by a screen reader is a worse document than the one that went in.
///
/// So the operators are copied in, with the appearance's own resources carried
/// along under prefixed names. The prefix is what stops the form's `/Helv`
/// from colliding with a different `/Helv` the page already had.
fn inline(
    doc: &mut Document,
    appearance: ObjectId,
    index: usize,
    rect: (f32, f32, f32, f32),
    extra_resources: &mut Dictionary,
) -> Option<Vec<Operation>> {
    let stream = match doc.get_object(appearance) {
        Ok(Object::Stream(stream)) => stream.clone(),
        _ => return None,
    };
    let content = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
    let decoded = Content::decode(&content).ok()?;

    // The appearance's own box, after its own matrix.
    let numbers = |key: &[u8]| -> Option<Vec<f32>> {
        let value = stream.dict.get(key).ok()?;
        let items = match value {
            Object::Array(items) => items.clone(),
            Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?.clone(),
            _ => return None,
        };
        Some(items.iter().map(|o| o.as_float().unwrap_or(0.0)).collect())
    };
    let bbox = numbers(b"BBox").filter(|v| v.len() >= 4)?;
    let matrix = numbers(b"Matrix")
        .filter(|v| v.len() >= 6)
        .unwrap_or_else(|| vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    let apply = |x: f32, y: f32| -> (f32, f32) {
        (matrix[0] * x + matrix[2] * y + matrix[4], matrix[1] * x + matrix[3] * y + matrix[5])
    };
    let corners = [
        apply(bbox[0], bbox[1]),
        apply(bbox[2], bbox[1]),
        apply(bbox[2], bbox[3]),
        apply(bbox[0], bbox[3]),
    ];
    let bx0 = corners.iter().map(|c| c.0).fold(f32::MAX, f32::min);
    let bx1 = corners.iter().map(|c| c.0).fold(f32::MIN, f32::max);
    let by0 = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min);
    let by1 = corners.iter().map(|c| c.1).fold(f32::MIN, f32::max);

    let (rx0, ry0, rx1, ry1) = rect;
    let sx = if (bx1 - bx0).abs() > 0.0001 { (rx1 - rx0) / (bx1 - bx0) } else { 1.0 };
    let sy = if (by1 - by0).abs() > 0.0001 { (ry1 - ry0) / (by1 - by0) } else { 1.0 };

    // Squash the appearance's box onto the widget's box, then its own matrix.
    let a = [
        matrix[0] * sx,
        matrix[1] * sy,
        matrix[2] * sx,
        matrix[3] * sy,
        (matrix[4] - bx0) * sx + rx0,
        (matrix[5] - by0) * sy + ry0,
    ];

    // Carry the resources across under names nothing else is using.
    let prefix = format!("KbxF{index}");
    let own = sub_dictionary(doc, &stream.dict, b"Resources");
    let mut renames: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for (category, entries) in own.iter() {
        let entries = match entries {
            Object::Dictionary(d) => d.clone(),
            Object::Reference(id) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
            _ => continue,
        };
        let mut copied = match extra_resources.get(category) {
            Ok(Object::Dictionary(d)) => d.clone(),
            _ => Dictionary::new(),
        };
        for (key, value) in entries.iter() {
            let renamed = format!("{prefix}{}", String::from_utf8_lossy(key)).into_bytes();
            copied.set(renamed.clone(), value.clone());
            renames.insert(key.to_vec(), renamed);
        }
        extra_resources.set(category.to_vec(), Object::Dictionary(copied));
    }

    let mut ops = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![a[0].into(), a[1].into(), a[2].into(), a[3].into(), a[4].into(), a[5].into()],
        ),
    ];
    for mut op in decoded.operations {
        // The operators that name a resource have to point at the copy.
        let slot = match op.operator.as_str() {
            "Tf" | "Do" | "gs" | "sh" | "cs" | "CS" => Some(0),
            "scn" | "SCN" => Some(op.operands.len().saturating_sub(1)),
            _ => None,
        };
        if let Some(slot) = slot {
            if let Some(Object::Name(name)) = op.operands.get(slot) {
                if let Some(renamed) = renames.get(name.as_slice()) {
                    op.operands[slot] = Object::Name(renamed.clone());
                }
            }
        }
        ops.push(op);
    }
    ops.push(Operation::new("Q", vec![]));
    Some(ops)
}

/// The widget's current normal appearance: either a stream, or the one in its
/// state dictionary matching `/AS`.
fn appearance_stream(doc: &Document, widget: &Dictionary) -> Option<ObjectId> {
    let ap = widget.get(b"AP").ok()?;
    let ap = match ap {
        Object::Dictionary(d) => d.clone(),
        Object::Reference(id) => doc.get_dictionary(*id).ok()?.clone(),
        _ => return None,
    };
    let normal = ap.get(b"N").ok()?;
    match normal {
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Stream(_)) => Some(*id),
            // A state dictionary: pick the one the widget says it is showing.
            Ok(Object::Dictionary(states)) => {
                let shown = widget.get(b"AS").ok().and_then(|o| o.as_name().ok())?;
                match states.get(shown) {
                    Ok(Object::Reference(id)) => Some(*id),
                    _ => None,
                }
            }
            _ => None,
        },
        Object::Dictionary(states) => {
            let shown = widget.get(b"AS").ok().and_then(|o| o.as_name().ok())?;
            match states.get(shown) {
                Ok(Object::Reference(id)) => Some(*id),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn named<'a>(fields: &'a [Field], name: &str) -> &'a Field {
        fields.iter().find(|f| f.name == name).unwrap_or_else(|| {
            panic!("no field '{name}' in {:?}", fields.iter().map(|f| &f.name).collect::<Vec<_>>())
        })
    }

    fn filled(values: &[(&str, &str)], flatten: bool) -> (Vec<u8>, FillReport) {
        let options = FillOptions {
            values: values.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            flatten,
        };
        fill(&testkit::pdf_form(), &options).unwrap()
    }

    #[test]
    fn every_field_is_found_with_its_type() {
        let found = fields(&testkit::pdf_form()).unwrap();
        assert_eq!(found.len(), 6, "{:?}", found.iter().map(|f| &f.name).collect::<Vec<_>>());

        assert_eq!(named(&found, "applicant_name").kind, FieldKind::Text);
        assert_eq!(named(&found, "address").kind, FieldKind::Multiline);
        assert_eq!(named(&found, "agree").kind, FieldKind::Checkbox);
        assert_eq!(named(&found, "contact_by").kind, FieldKind::Radio);
        assert_eq!(named(&found, "country").kind, FieldKind::Choice);
        assert_eq!(named(&found, "reference").kind, FieldKind::Text);
    }

    #[test]
    fn a_field_reports_what_it_will_accept() {
        let found = fields(&testkit::pdf_form()).unwrap();
        // The checkbox's on-state is "On", not "Yes" — a filler has to be told.
        assert_eq!(named(&found, "agree").options, vec!["On"]);
        let radio = named(&found, "contact_by");
        assert_eq!(radio.options, vec!["email", "post"]);
        assert_eq!(named(&found, "country").options, vec!["India", "United Kingdom", "Singapore"]);
    }

    #[test]
    fn the_form_authors_own_words_come_through() {
        let found = fields(&testkit::pdf_form()).unwrap();
        let name = named(&found, "applicant_name");
        assert_eq!(name.label.as_deref(), Some("Full name as on your ID"));
        assert!(name.required, "the fixture marks it required");
        assert!(!name.read_only);
        assert_eq!(name.pages, vec![1]);
    }

    #[test]
    fn a_read_only_field_keeps_its_value_and_is_reported() {
        let found = fields(&testkit::pdf_form()).unwrap();
        let reference = named(&found, "reference");
        assert!(reference.read_only);
        assert_eq!(reference.value.as_deref(), Some("REF-2026-014"));

        let (out, report) = filled(&[("reference", "REF-9999-999")], false);
        assert_eq!(report.read_only, vec!["reference"]);
        assert!(report.filled.is_empty(), "{report:?}");
        let after = fields(&out).unwrap();
        assert_eq!(named(&after, "reference").value.as_deref(), Some("REF-2026-014"));
    }

    #[test]
    fn a_value_is_drawn_as_well_as_recorded() {
        // The failure this guards: setting /V alone, which renders blank in
        // most viewers and on every printer.
        let (out, report) = filled(&[("applicant_name", "Jane Q. Doe")], false);
        assert_eq!(report.filled, vec!["applicant_name"]);

        let after = fields(&out).unwrap();
        assert_eq!(named(&after, "applicant_name").value.as_deref(), Some("Jane Q. Doe"));

        let doc = Document::load_mem(&out).unwrap();
        let drawn = doc.objects.values().any(|o| match o {
            Object::Stream(s) => String::from_utf8_lossy(&s.content).contains("(Jane Q. Doe) Tj"),
            _ => false,
        });
        assert!(drawn, "the value was recorded but never drawn");
    }

    #[test]
    fn readers_are_told_not_to_redraw_what_was_just_drawn() {
        let (out, _) = filled(&[("applicant_name", "Jane")], false);
        let doc = Document::load_mem(&out).unwrap();
        let form = sub_dictionary(&doc, doc.catalog().unwrap(), b"AcroForm");
        assert_eq!(form.get(b"NeedAppearances").ok().and_then(|o| o.as_bool().ok()), Some(false));
    }

    #[test]
    fn yes_is_translated_into_whatever_this_form_calls_on() {
        // "Yes" written literally would leave a box that is neither on nor off.
        let (out, report) = filled(&[("agree", "yes")], false);
        assert_eq!(report.filled, vec!["agree"]);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let after = fields(&out).unwrap();
        assert_eq!(named(&after, "agree").value.as_deref(), Some("On"));

        let doc = Document::load_mem(&out).unwrap();
        let showing = doc.objects.values().any(|o| match o {
            Object::Dictionary(d) => {
                d.get(b"AS").ok().and_then(|o| o.as_name().ok()) == Some(b"On".as_ref())
            }
            _ => false,
        });
        assert!(showing, "the widget should be showing its on-state");
    }

    #[test]
    fn a_checkbox_can_be_turned_off_again() {
        let (on, _) = filled(&[("agree", "yes")], false);
        let options =
            FillOptions { values: [("agree".into(), "no".into())].into(), flatten: false };
        let (off, _) = fill(&on, &options).unwrap();
        assert_eq!(named(&fields(&off).unwrap(), "agree").value, None);
    }

    #[test]
    fn only_the_chosen_radio_turns_on() {
        let (out, report) = filled(&[("contact_by", "email")], false);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(named(&fields(&out).unwrap(), "contact_by").value.as_deref(), Some("email"));

        let doc = Document::load_mem(&out).unwrap();
        let states: Vec<Vec<u8>> = doc
            .objects
            .values()
            .filter_map(|o| match o {
                Object::Dictionary(d) if d.has(b"AS") && d.has(b"Parent") => {
                    d.get(b"AS").ok().and_then(|o| o.as_name().ok()).map(|n| n.to_vec())
                }
                _ => None,
            })
            .collect();
        assert!(states.contains(&b"email".to_vec()), "{states:?}");
        assert!(states.contains(&b"Off".to_vec()), "the other one must be off: {states:?}");
    }

    #[test]
    fn a_value_the_field_cannot_take_is_refused_rather_than_guessed() {
        let (out, report) = filled(&[("contact_by", "carrier pigeon")], false);
        assert!(report.filled.is_empty(), "{report:?}");
        assert!(
            report.warnings.iter().any(|w| w.contains("accept") && w.contains("email")),
            "{:?}",
            report.warnings
        );
        assert_eq!(named(&fields(&out).unwrap(), "contact_by").value, None);
    }

    #[test]
    fn a_name_the_form_does_not_have_is_reported_not_ignored() {
        let (_, report) = filled(&[("applicant_nmae", "typo"), ("applicant_name", "Jane")], false);
        assert_eq!(report.unknown, vec!["applicant_nmae"]);
        assert_eq!(report.filled, vec!["applicant_name"]);
    }

    #[test]
    fn required_fields_still_empty_are_listed() {
        let (_, report) = filled(&[("agree", "yes")], false);
        assert_eq!(report.still_required, vec!["applicant_name"]);

        let (_, done) = filled(&[("applicant_name", "Jane")], false);
        assert!(done.still_required.is_empty(), "{done:?}");
    }

    #[test]
    fn a_long_answer_wraps_inside_a_multiline_box() {
        let address = "17 Somewhat Long Street, A Rather Lengthy Suburb Name, Metropolis 110001";
        let (out, _) = filled(&[("address", address)], false);
        let doc = Document::load_mem(&out).unwrap();
        let appearance = doc
            .objects
            .values()
            .find_map(|o| match o {
                Object::Stream(s) => {
                    let text = String::from_utf8_lossy(&s.content).to_string();
                    text.contains("Somewhat").then_some(text)
                }
                _ => None,
            })
            .expect("the address should be drawn");
        assert!(appearance.matches(" Tj").count() > 1, "it should wrap: {appearance}");
    }

    #[test]
    fn a_password_field_shows_dots_rather_than_the_password() {
        // Drawing the characters into the page would defeat the field.
        let form = testkit::pdf_form();
        let mut doc = Document::load_mem(&form).unwrap();
        let id = *doc
            .objects
            .iter()
            .find(|(_, o)| matches!(o, Object::Dictionary(d)
                if d.get(b"T").ok().and_then(|o| o.as_str().ok()) == Some(b"applicant_name".as_ref())))
            .unwrap()
            .0;
        if let Ok(d) = doc.get_dictionary_mut(id) {
            d.set("Ff", 8192i64);
        }
        let mut patched = Vec::new();
        doc.save_to(&mut patched).unwrap();

        let options = FillOptions {
            values: [("applicant_name".into(), "hunter2".into())].into(),
            flatten: false,
        };
        let (out, _) = fill(&patched, &options).unwrap();
        let raw = String::from_utf8_lossy(&out).to_string();
        assert!(!raw.contains("(hunter2) Tj"), "the password was drawn onto the page");
    }

    #[test]
    fn flattening_draws_the_values_and_takes_the_form_away() {
        let (out, report) = filled(&[("applicant_name", "Jane Q. Doe"), ("agree", "yes")], true);
        assert!(report.flattened);

        // No form left to edit.
        let doc = Document::load_mem(&out).unwrap();
        assert!(doc.catalog().unwrap().get(b"AcroForm").is_err(), "the form should be gone");
        assert!(fields(&out).unwrap().is_empty(), "nothing should be fillable");

        // But the value is now part of the page.
        let page_id = doc.page_iter().next().unwrap();
        let content = String::from_utf8_lossy(&doc.get_page_content(page_id)).to_string();
        assert!(content.contains("Jane Q. Doe"), "the value should be in the page: {content}");
        let text = super::super::markdown::page_lines(&doc, page_id).join(" ");
        assert!(text.contains("Jane Q. Doe"), "the name should read off the page: {text}");
        // The read-only field was never filled by us, so it has a value and no
        // appearance of its own. Flattening has to draw it anyway or the
        // answer vanishes from the page.
        assert!(
            text.contains("REF-2026-014"),
            "a field with a value but no appearance was dropped: {text}"
        );
    }

    #[test]
    fn flattening_keeps_the_pages_and_the_original_drawing() {
        let (out, report) = filled(&[("applicant_name", "Jane")], true);
        assert_eq!(super::super::page_count(&out).unwrap(), 1);
        assert!(report.flattened);
        let doc = Document::load_mem(&out).unwrap();
        let text =
            super::super::markdown::page_lines(&doc, doc.page_iter().next().unwrap()).join(" ");
        assert!(text.contains("Application"), "the form's own text should survive: {text}");
    }

    #[test]
    fn flattening_on_its_own_works_too() {
        let (filled_pdf, _) = filled(&[("applicant_name", "Jane")], false);
        let (out, report) = flatten(&filled_pdf).unwrap();
        assert!(report.widgets_drawn >= 1, "{report:?}");
        assert!(report.note.contains("changed back"), "{}", report.note);
        assert!(fields(&out).unwrap().is_empty());
    }

    #[test]
    fn a_document_with_no_form_says_so_plainly() {
        let plain = testkit::pdf_with_sized_text(&[(12.0, "just a document")]);
        assert!(fields(&plain).unwrap().is_empty());

        let error = fill(&plain, &FillOptions::default()).unwrap_err().to_string();
        assert!(error.contains("no form fields"), "{error}");
        assert!(error.contains("picture of a form"), "it should explain why: {error}");

        assert!(flatten(&plain).is_err());
    }

    #[test]
    fn an_accented_name_survives_the_round_trip() {
        let (out, _) = filled(&[("applicant_name", "Zoe Muller-Odegard")], false);
        let after = fields(&out).unwrap();
        assert_eq!(named(&after, "applicant_name").value.as_deref(), Some("Zoe Muller-Odegard"));

        let (wide, _) = filled(&[("applicant_name", "Zoë Müller")], false);
        assert_eq!(
            named(&fields(&wide).unwrap(), "applicant_name").value.as_deref(),
            Some("Zoë Müller")
        );
    }

    #[test]
    fn a_value_longer_than_the_field_allows_is_cut_and_reported() {
        let form = testkit::pdf_form();
        let mut doc = Document::load_mem(&form).unwrap();
        let id = *doc
            .objects
            .iter()
            .find(|(_, o)| matches!(o, Object::Dictionary(d)
                if d.get(b"T").ok().and_then(|o| o.as_str().ok()) == Some(b"applicant_name".as_ref())))
            .unwrap()
            .0;
        if let Ok(d) = doc.get_dictionary_mut(id) {
            d.set("MaxLen", 4i64);
        }
        let mut patched = Vec::new();
        doc.save_to(&mut patched).unwrap();

        let options = FillOptions {
            values: [("applicant_name".into(), "Jonathan".into())].into(),
            flatten: false,
        };
        let (out, report) = fill(&patched, &options).unwrap();
        assert!(report.warnings.iter().any(|w| w.contains("at most 4")), "{:?}", report.warnings);
        assert_eq!(named(&fields(&out).unwrap(), "applicant_name").value.as_deref(), Some("Jona"));
    }

    #[test]
    fn wrapping_handles_a_word_wider_than_the_box() {
        let lines = wrap("Supercalifragilisticexpialidocious", 12.0, 40.0);
        assert!(lines.len() > 1, "{lines:?}");
        for line in &lines {
            assert!(text_width(line, 12.0) <= 40.0 + 0.01, "'{line}' is too wide");
        }
    }
}
