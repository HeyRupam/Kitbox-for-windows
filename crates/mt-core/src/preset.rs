//! Form presets: the constraints a form states, as data you can check against.
//!
//! The problem these solve is not "make this file smaller". It is "this portal
//! will reject my application and not say why". So a preset is read two ways:
//!
//! * [`check`] measures a file against it and reports **every** check it made,
//!   passed or failed, because the only way to trust a verdict against a form
//!   you can see is to read what was compared.
//! * [`fix`] produces a file that conforms, then checks its own work and says
//!   what it could not fix rather than claiming success.
//!
//! A preset is plain JSON, because the rules change. Boards revise them between
//! cycles, two portals for the same exam disagree, and the person filling the
//! form can see the real numbers on screen while this crate cannot. So the
//! bundled set is a starting point that is meant to be edited, and nothing here
//! treats it as authoritative — [`Preset::note`] exists to say where a number
//! came from.

use serde::{Deserialize, Serialize};

use crate::engine::image::{self, CompressOptions, OutputFormat};
use crate::engine::pdf::{self, PdfCompressOptions};
use crate::format::FileKind;
use crate::size::format_size;
use crate::{Bias, Error, Outcome, Result, SizeTarget};

/// The presets that ship with the app, as JSON.
///
/// Kept as text rather than Rust values so the file the app writes to disk on
/// first run is the same file this crate parses — one format, one parser, and
/// a user edit cannot produce something the bundled set couldn't.
pub const BUNDLED: &str = include_str!("bundled_presets.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Pdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Jpeg,
    Png,
    Pdf,
    /// The form doesn't care, or doesn't say.
    Any,
}

impl Format {
    fn accepts(self, kind: FileKind) -> bool {
        match self {
            Format::Jpeg => kind == FileKind::Jpeg,
            Format::Png => kind == FileKind::Png,
            Format::Pdf => kind == FileKind::Pdf,
            Format::Any => kind.is_image() || kind == FileKind::Pdf,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Format::Jpeg => "JPG",
            Format::Png => "PNG",
            Format::Pdf => "PDF",
            Format::Any => "any format",
        }
    }
}

/// How the stated pixel size is meant to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// "200 × 230 px" — exactly that, nothing else accepted.
    Exact,
    /// "at least 350 × 350 px" — bigger is fine.
    AtLeast,
    /// "no larger than 1000 × 1000 px".
    AtMost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
    #[serde(default = "exact")]
    pub fit: Fit,
}

fn exact() -> Fit {
    Fit::Exact
}

/// A form's constraints, as written on the form.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    #[serde(default = "other")]
    pub group: String,
    pub kind: Kind,
    pub target: SizeTarget,
    #[serde(default = "any_format")]
    pub format: Format,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Dimensions>,
    /// Minimum resolution the form asks for, when it says one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpi: Option<u16>,
    /// Where the numbers came from, or anything the form says that these
    /// fields can't hold. Shown with the preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn other() -> String {
    "Other".into()
}

fn any_format() -> Format {
    Format::Any
}

impl Preset {
    /// Read a whole file of presets, saying which entry is wrong when one is.
    pub fn parse_all(json: &str) -> Result<Vec<Preset>> {
        let presets: Vec<Preset> = serde_json::from_str(json).map_err(|e| {
            // The mistake nearly everyone makes editing one of these by hand,
            // worth naming instead of passing on "invalid type: map".
            if json.trim_start().starts_with('{') {
                Error::invalid(
                    "a preset file holds a list, even for one preset — wrap it in [ ]".to_string(),
                )
            } else {
                Error::invalid(format!("this isn't a preset file: {e}"))
            }
        })?;
        for preset in &presets {
            preset.check_itself()?;
        }
        Ok(presets)
    }

    /// The bundled set, parsed.
    pub fn bundled() -> Vec<Preset> {
        // A failure here is a broken build, not a broken install: the file is
        // compiled in and covered by a test.
        Preset::parse_all(BUNDLED).expect("the bundled presets parse")
    }

    /// Catch the mistakes a hand-edited file makes, while the editor can still
    /// be told which preset to look at.
    pub fn check_itself(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(Error::invalid("a preset needs an id"));
        }
        if self.name.trim().is_empty() {
            return Err(Error::invalid(format!("preset '{}' needs a name", self.id)));
        }
        if self.target.max_bytes() == 0 {
            return Err(Error::invalid(format!(
                "preset '{}' allows a maximum of nothing — check the size",
                self.id
            )));
        }
        if self.target.min_bytes() > self.target.max_bytes() {
            return Err(Error::invalid(format!(
                "preset '{}' asks for at least {} and at most {}, which nothing can satisfy",
                self.id,
                format_size(self.target.min_bytes()),
                format_size(self.target.max_bytes())
            )));
        }
        if let Some(d) = self.dimensions {
            if d.width == 0 || d.height == 0 {
                return Err(Error::invalid(format!(
                    "preset '{}' has a pixel size of zero",
                    self.id
                )));
            }
        }
        if self.kind == Kind::Pdf && self.dimensions.is_some() {
            return Err(Error::invalid(format!(
                "preset '{}' is for PDFs, which have no pixel size",
                self.id
            )));
        }
        Ok(())
    }

    /// One line describing what this demands, for a card or a log.
    pub fn describe(&self) -> String {
        let mut parts = vec![self.target.describe()];
        if let Some(d) = self.dimensions {
            parts.push(match d.fit {
                Fit::Exact => format!("{}×{} px", d.width, d.height),
                Fit::AtLeast => format!("at least {}×{} px", d.width, d.height),
                Fit::AtMost => format!("at most {}×{} px", d.width, d.height),
            });
        }
        if self.format != Format::Any {
            parts.push(self.format.label().to_string());
        }
        if let Some(dpi) = self.dpi {
            parts.push(format!("{dpi} DPI"));
        }
        parts.join(", ")
    }
}

/// How one requirement came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Wrong,
    /// The file doesn't say, so neither will this.
    Unknown,
}

/// One requirement, and what the file actually is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// "Size", "Pixels", "Format", "Resolution".
    pub what: String,
    pub verdict: Verdict,
    /// What the form asks for.
    pub wants: String,
    /// What the file is.
    pub has: String,
}

/// Everything that was compared, and whether the file would pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub checks: Vec<Check>,
    /// True only when nothing is `Wrong`. An `Unknown` does not fail a file —
    /// a photo that doesn't state its resolution is not a photo with the wrong
    /// resolution, and saying otherwise would send people chasing a fault that
    /// isn't there.
    pub fits: bool,
}

impl Report {
    fn new(checks: Vec<Check>) -> Self {
        let fits = checks.iter().all(|c| c.verdict != Verdict::Wrong);
        Report { checks, fits }
    }

    /// The failures, in the order they were checked.
    pub fn problems(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.verdict == Verdict::Wrong)
    }
}

/// Measure a file against a preset.
pub fn check(bytes: &[u8], preset: &Preset) -> Report {
    let kind = FileKind::sniff(bytes);
    let size = bytes.len() as u64;
    let mut checks = Vec::new();

    checks.push(Check {
        what: "Format".into(),
        verdict: if preset.format.accepts(kind) { Verdict::Ok } else { Verdict::Wrong },
        wants: preset.format.label().to_string(),
        has: kind.label().to_string(),
    });

    checks.push(Check {
        what: "Size".into(),
        verdict: if preset.target.contains(size) { Verdict::Ok } else { Verdict::Wrong },
        wants: preset.target.describe(),
        has: format_size(size),
    });

    if preset.kind == Kind::Image {
        let dimensions = ::image::load_from_memory(bytes).ok().map(|i| (i.width(), i.height()));

        if let Some(want) = preset.dimensions {
            let (verdict, has) = match dimensions {
                Some((w, h)) => {
                    let ok = match want.fit {
                        Fit::Exact => w == want.width && h == want.height,
                        Fit::AtLeast => w >= want.width && h >= want.height,
                        Fit::AtMost => w <= want.width && h <= want.height,
                    };
                    (if ok { Verdict::Ok } else { Verdict::Wrong }, format!("{w}×{h} px"))
                }
                None => (Verdict::Unknown, "couldn't be read".into()),
            };
            checks.push(Check {
                what: "Pixels".into(),
                verdict,
                wants: match want.fit {
                    Fit::Exact => format!("{}×{} px", want.width, want.height),
                    Fit::AtLeast => format!("at least {}×{} px", want.width, want.height),
                    Fit::AtMost => format!("at most {}×{} px", want.width, want.height),
                },
                has,
            });
        }

        if let Some(want) = preset.dpi {
            let (verdict, has) = match density(bytes) {
                Some(dpi) if dpi >= want => (Verdict::Ok, format!("{dpi} DPI")),
                Some(dpi) => (Verdict::Wrong, format!("{dpi} DPI")),
                // Most photos say nothing about resolution, and most forms
                // never look. Reporting that as a failure would be a lie with
                // consequences: people would resample a file that was fine.
                None => (Verdict::Unknown, "not stated in the file".into()),
            };
            checks.push(Check {
                what: "Resolution".into(),
                verdict,
                wants: format!("at least {want} DPI"),
                has,
            });
        }
    }

    Report::new(checks)
}

/// What fixing a file did, and what it couldn't.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixed {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    /// The file as it now stands, measured again.
    pub report: Report,
    /// What was done, in order.
    pub steps: Vec<String>,
    /// Set when the result still doesn't conform, saying why in a sentence.
    pub shortfall: Option<String>,
    pub extension: String,
}

/// Make a file that conforms, then check it and say so honestly.
pub fn fix(bytes: &[u8], preset: &Preset, bias: Bias) -> Result<Fixed> {
    let kind = FileKind::sniff(bytes);
    let mut steps = Vec::new();

    if preset.kind == Kind::Pdf && kind != FileKind::Pdf {
        return Err(Error::invalid(format!(
            "{} expects a PDF, and this is {}",
            preset.name,
            kind.article()
        )));
    }
    if preset.kind == Kind::Image && !kind.is_image() {
        return Err(Error::invalid(format!(
            "{} expects a picture, and this is {}",
            preset.name,
            kind.article()
        )));
    }

    let (bytes, extension) = match preset.kind {
        Kind::Pdf => {
            let outcome = pdf::compress_to_target(
                bytes,
                preset.target,
                bias,
                PdfCompressOptions { strip_metadata: true, ..Default::default() },
            )?;
            steps.push(format!("compressed to {}", preset.target.describe()));
            let (bytes, note) = unwrap(outcome);
            steps.extend(note);
            (bytes, "pdf".to_string())
        }
        Kind::Image => {
            let format = match preset.format {
                Format::Png => OutputFormat::Png,
                Format::Jpeg => OutputFormat::Jpeg,
                // "Any" and "Pdf" on an image preset both mean: don't change
                // what it is just to satisfy a rule nobody stated.
                _ => OutputFormat::Keep,
            };
            let exact = preset.dimensions.and_then(|d| match d.fit {
                Fit::Exact => Some((d.width, d.height)),
                // A floor or a ceiling leaves the solver free to use pixels,
                // which is a lever worth keeping; the check afterwards says
                // whether it stayed inside.
                _ => None,
            });
            if let Some((w, h)) = exact {
                steps.push(format!("cropped and resized to {w}×{h}"));
            }
            if let Some(cap) = preset.dimensions.filter(|d| d.fit == Fit::AtMost) {
                steps.push(format!("kept within {}×{}", cap.width, cap.height));
            }

            let outcome = image::compress_to_target(
                bytes,
                preset.target,
                bias,
                CompressOptions {
                    format,
                    strip_metadata: true,
                    max_dimension: preset
                        .dimensions
                        .filter(|d| d.fit == Fit::AtMost)
                        .map(|d| d.width.max(d.height)),
                    exact_dimensions: exact,
                },
            )?;
            steps.push(format!("compressed to {}", preset.target.describe()));
            let (fixed, note) = unwrap(outcome);
            steps.extend(note);
            let kind = FileKind::sniff(&fixed);
            let extension = kind.extension().to_string();

            // Stamping the resolution is metadata, not resampling: the pixels
            // are already what the form asked for, and this is the field a
            // portal reads when it bothers to look.
            match preset.dpi {
                Some(dpi) if kind == FileKind::Jpeg => match stamp_density(&fixed, dpi) {
                    Some(stamped) => {
                        steps.push(format!("marked as {dpi} DPI"));
                        (stamped, extension)
                    }
                    None => (fixed, extension),
                },
                _ => (fixed, extension),
            }
        }
    };

    let report = check(&bytes, preset);
    let shortfall = (!report.fits).then(|| {
        let problems: Vec<String> = report
            .problems()
            .map(|c| format!("{} is {} and needs {}", c.what.to_lowercase(), c.has, c.wants))
            .collect();
        format!("This is as close as the file gets: {}.", problems.join("; "))
    });

    Ok(Fixed { bytes, report, steps, shortfall, extension })
}

/// Take the bytes from any outcome, including one that fell short — a file
/// that missed by 2 KB is worth more than an error message — along with
/// anything about how it got there that belongs in the steps.
fn unwrap(outcome: Outcome) -> (Vec<u8>, Option<String>) {
    match outcome {
        Outcome::Hit(solved) => {
            let note = solved.score.map(|s| format!("{:.0}% identical", s * 100.0));
            (solved.bytes, note)
        }
        // Padding is filler added to clear a minimum the encoder could not
        // reach. It is lossless and it works, but a file that is part padding
        // is a surprising thing to hand someone without saying so.
        Outcome::Padded(solved) => {
            (solved.bytes, Some("padded with filler to clear the minimum the form asks for".into()))
        }
        Outcome::Infeasible { best, .. } => (best, None),
    }
}

/// The resolution a file states, if it states one.
///
/// JPEG keeps it in the JFIF header and PNG in a `pHYs` chunk. Anything else,
/// or a file that simply doesn't say, returns `None` — which is not the same
/// as zero and is not a failure.
pub fn density(bytes: &[u8]) -> Option<u16> {
    match FileKind::sniff(bytes) {
        FileKind::Jpeg => jfif_density(bytes),
        FileKind::Png => phys_density(bytes),
        _ => None,
    }
}

fn jfif_density(bytes: &[u8]) -> Option<u16> {
    // The APP0 segment is: FF E0, two length bytes, then the payload —
    // "JFIF " (5), version (2), units (1), then x and y density (2 each).
    // So units sits eleven bytes past the marker, not seven; reading it at
    // seven lands inside the word JFIF and quietly reports nothing.
    let at = find_jfif(bytes)?;
    let units = *bytes.get(at + 11)?;
    let x = u16::from_be_bytes([*bytes.get(at + 12)?, *bytes.get(at + 13)?]);
    match units {
        1 => Some(x),                                // already dots per inch
        2 => Some((x as f32 * 2.54).round() as u16), // dots per cm
        _ => None,                                   // 0 means "aspect ratio only"
    }
}

/// Offset of the APP0 marker that carries `JFIF\0`.
fn find_jfif(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        // Start of scan: pixel data from here on, no more headers.
        if marker == 0xDA {
            return None;
        }
        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if marker == 0xE0 && bytes.get(at + 4..at + 9) == Some(b"JFIF\0") {
            return Some(at);
        }
        at += 2 + length;
    }
    None
}

fn phys_density(bytes: &[u8]) -> Option<u16> {
    let mut at = 8; // past the PNG signature
    while at + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let tag = bytes.get(at + 4..at + 8)?;
        if tag == b"pHYs" {
            let x = u32::from_be_bytes(bytes.get(at + 8..at + 12)?.try_into().ok()?);
            let unit = *bytes.get(at + 16)?;
            // 1 means metres; 0 means the numbers are a ratio and say nothing.
            return (unit == 1).then(|| (x as f32 * 0.0254).round() as u16);
        }
        at += 12 + length;
    }
    None
}

/// Write a resolution into a JPEG's JFIF header, leaving the pixels alone.
///
/// Returns `None` when the file has no JFIF header to write into, rather than
/// inventing one — a file this doesn't understand is one to leave untouched.
pub fn stamp_density(bytes: &[u8], dpi: u16) -> Option<Vec<u8>> {
    let at = find_jfif(bytes)?;
    if bytes.len() < at + 16 {
        return None;
    }
    let mut out = bytes.to_vec();
    out[at + 11] = 1; // units: dots per inch
    out[at + 12..at + 14].copy_from_slice(&dpi.to_be_bytes());
    out[at + 14..at + 16].copy_from_slice(&dpi.to_be_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    fn photo_preset() -> Preset {
        Preset {
            id: "test-photo".into(),
            name: "Test photo".into(),
            group: "Exams".into(),
            kind: Kind::Image,
            target: SizeTarget::Range { min: 20_000, max: 50_000 },
            format: Format::Jpeg,
            dimensions: Some(Dimensions { width: 200, height: 230, fit: Fit::Exact }),
            dpi: None,
            note: None,
        }
    }

    #[test]
    fn the_bundled_presets_are_all_sane() {
        // They ship compiled in, so a typo here is a broken app rather than a
        // broken file someone can fix.
        let presets = Preset::bundled();
        assert!(presets.len() >= 15, "got {}", presets.len());
        for preset in &presets {
            preset.check_itself().expect(&preset.id);
        }
        let mut ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "two presets share an id");
    }

    #[test]
    fn a_file_that_fits_is_reported_check_by_check() {
        let mut preset = photo_preset();
        let bytes = testkit::encode_jpeg(&testkit::photo(200, 230), 70);
        // The range is set around the file rather than the other way round:
        // this test is about what a passing report looks like.
        preset.target = SizeTarget::Range { min: 1_000, max: 200_000 };
        assert!(preset.target.contains(bytes.len() as u64), "{} bytes", bytes.len());

        let report = check(&bytes, &preset);
        assert!(report.fits, "{:?}", report.checks);
        assert_eq!(report.checks.len(), 3, "format, size and pixels");
        assert!(report.checks.iter().all(|c| c.verdict == Verdict::Ok));
    }

    #[test]
    fn every_failure_says_what_it_wanted_and_what_it_got() {
        // The point of the checklist: you hold it against the form on screen.
        let preset = photo_preset();
        let bytes = testkit::encode_jpeg(&testkit::photo(800, 600), 95);

        let report = check(&bytes, &preset);
        assert!(!report.fits);
        let pixels = report.checks.iter().find(|c| c.what == "Pixels").unwrap();
        assert_eq!(pixels.verdict, Verdict::Wrong);
        assert_eq!(pixels.wants, "200×230 px");
        assert_eq!(pixels.has, "800×600 px");
    }

    #[test]
    fn a_png_where_jpeg_is_demanded_fails_on_format_alone() {
        let preset = photo_preset();
        let mut png = Vec::new();
        testkit::photo(200, 230)
            .write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png)
            .unwrap();

        let report = check(&png, &preset);
        let format = report.checks.iter().find(|c| c.what == "Format").unwrap();
        assert_eq!(format.verdict, Verdict::Wrong);
        assert_eq!(format.has, "PNG");
    }

    #[test]
    fn fixing_an_awkward_photo_produces_one_that_passes() {
        // The whole promise: a phone photo in, a conforming file out.
        let preset = photo_preset();
        let bytes = testkit::encode_jpeg(&testkit::photo(3000, 2000), 95);

        let fixed = fix(&bytes, &preset, Bias::Balanced).expect("a fix");
        assert!(fixed.report.fits, "{:?}", fixed.report.checks);
        assert!(fixed.shortfall.is_none());
        assert_eq!(fixed.extension, "jpg");

        let again = check(&fixed.bytes, &preset);
        assert!(again.fits, "the fix has to survive being checked again");
    }

    #[test]
    fn a_file_padded_up_to_the_minimum_says_so() {
        // A form that demands at least 20 KB gets 20 KB, but part of it is
        // filler, and handing that over quietly would be the wrong kind of
        // success.
        let mut preset = photo_preset();
        preset.target = SizeTarget::Range { min: 400_000, max: 500_000 };
        let bytes = testkit::encode_jpeg(&testkit::photo(900, 900), 90);

        let fixed = fix(&bytes, &preset, Bias::Balanced).expect("a fix");
        assert!(fixed.report.fits, "{:?}", fixed.report.checks);
        assert!(
            fixed.steps.iter().any(|s| s.contains("padded")),
            "the padding has to be in the steps: {:?}",
            fixed.steps
        );
    }

    #[test]
    fn a_target_nothing_can_reach_is_said_plainly_not_hidden() {
        let mut preset = photo_preset();
        // 200×230 pixels cannot be squeezed under 300 bytes as a JPEG.
        preset.target = SizeTarget::AtMost(300);
        let bytes = testkit::encode_jpeg(&testkit::photo(3000, 2000), 95);

        let fixed = fix(&bytes, &preset, Bias::Balanced).expect("still returns a file");
        assert!(!fixed.bytes.is_empty(), "the best attempt is still worth having");
        assert!(!fixed.report.fits);
        let why = fixed.shortfall.expect("it has to say why");
        assert!(why.contains("size"), "{why}");
    }

    #[test]
    fn a_floor_on_pixels_leaves_the_solver_its_lever() {
        let mut preset = photo_preset();
        preset.dimensions = Some(Dimensions { width: 350, height: 350, fit: Fit::AtLeast });
        preset.target = SizeTarget::AtMost(60_000);
        let bytes = testkit::encode_jpeg(&testkit::photo(1200, 1200), 95);

        let fixed = fix(&bytes, &preset, Bias::Balanced).expect("a fix");
        let (w, h) =
            ::image::load_from_memory(&fixed.bytes).map(|i| (i.width(), i.height())).unwrap();
        assert!(w >= 350 && h >= 350, "{w}×{h} fell below the floor");
        assert!(fixed.report.fits, "{:?}", fixed.report.checks);
    }

    #[test]
    fn a_resolution_the_file_never_states_is_unknown_not_wrong() {
        let mut preset = photo_preset();
        preset.dpi = Some(200);
        preset.target = SizeTarget::Range { min: 1_000, max: 200_000 };
        let bytes = testkit::encode_jpeg(&testkit::photo(200, 230), 70);

        let report = check(&bytes, &preset);
        let dpi = report.checks.iter().find(|c| c.what == "Resolution").unwrap();
        assert!(
            matches!(dpi.verdict, Verdict::Unknown | Verdict::Ok),
            "a silent file must not be called wrong: {dpi:?}"
        );
        assert!(report.fits, "and it must not fail the file");
    }

    #[test]
    fn fixing_stamps_the_resolution_a_form_asks_for() {
        let mut preset = photo_preset();
        preset.dpi = Some(300);
        let bytes = testkit::encode_jpeg(&testkit::photo(1000, 1000), 92);

        let fixed = fix(&bytes, &preset, Bias::Balanced).expect("a fix");
        assert_eq!(density(&fixed.bytes), Some(300), "it should say 300 DPI now");
        assert!(fixed.steps.iter().any(|s| s.contains("300 DPI")), "{:?}", fixed.steps);
        assert!(fixed.report.fits);
    }

    #[test]
    fn stamping_leaves_the_picture_alone() {
        let bytes = testkit::encode_jpeg(&testkit::photo(300, 300), 90);
        let stamped = stamp_density(&bytes, 200).expect("a JFIF header");
        assert_eq!(bytes.len(), stamped.len(), "only the header changes");
        let before = ::image::load_from_memory(&bytes).unwrap().into_rgb8();
        let after = ::image::load_from_memory(&stamped).unwrap().into_rgb8();
        assert_eq!(before.as_raw(), after.as_raw(), "not a single pixel");
    }

    #[test]
    fn a_pdf_preset_fixes_a_pdf() {
        let preset = Preset {
            id: "doc".into(),
            name: "Document under 100 KB".into(),
            group: "Documents".into(),
            kind: Kind::Pdf,
            target: SizeTarget::AtMost(100_000),
            format: Format::Pdf,
            dimensions: None,
            dpi: None,
            note: None,
        };
        let pdf = testkit::pdf_scan(2, 800, 1000, 92);
        let fixed = fix(&pdf, &preset, Bias::Balanced).expect("a fix");
        assert!(fixed.report.fits, "{:?}", fixed.report.checks);
        assert_eq!(fixed.extension, "pdf");
    }

    #[test]
    fn the_wrong_sort_of_file_is_refused_before_anything_is_done_to_it() {
        let preset = photo_preset();
        let pdf = testkit::pdf_scan(1, 200, 200, 80);
        let refused = fix(&pdf, &preset, Bias::Balanced).unwrap_err().to_string();
        assert!(refused.contains("picture"), "{refused}");
    }

    #[test]
    fn a_broken_preset_file_names_the_preset_that_is_broken() {
        let json = r#"[
            { "id": "fine", "name": "Fine", "kind": "image",
              "target": { "kind": "at_most", "value": 50000 } },
            { "id": "backwards", "name": "Backwards", "kind": "image",
              "target": { "kind": "range", "value": { "min": 90000, "max": 10000 } } }
        ]"#;
        let refused = Preset::parse_all(json).unwrap_err().to_string();
        assert!(refused.contains("backwards"), "{refused}");
    }

    #[test]
    fn one_preset_without_the_brackets_is_told_what_to_do_about_it() {
        let json = r#"{ "id": "alone", "name": "Alone", "kind": "image",
                        "target": { "kind": "at_most", "value": 50000 } }"#;
        let refused = Preset::parse_all(json).unwrap_err().to_string();
        assert!(refused.contains("[ ]"), "{refused}");
    }

    #[test]
    fn a_preset_survives_the_round_trip_to_json_and_back() {
        // Presets are edited on disk, so what the app writes has to be what it
        // can read.
        let preset = photo_preset();
        let json = serde_json::to_string_pretty(&vec![preset.clone()]).unwrap();
        let back = Preset::parse_all(&json).expect("re-read");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, preset.id);
        assert_eq!(back[0].dimensions.unwrap().width, 200);
        assert_eq!(back[0].target.max_bytes(), 50_000);
    }

    #[test]
    fn the_fields_a_form_rarely_states_may_be_left_out() {
        // Hand-editing has to be forgiving: group, format and note are all
        // optional, and a file without them still loads.
        let json = r#"[{ "id": "bare", "name": "Bare", "kind": "image",
                         "target": { "kind": "at_most", "value": 100000 } }]"#;
        let presets = Preset::parse_all(json).expect("a bare preset");
        assert_eq!(presets[0].group, "Other");
        assert_eq!(presets[0].format, Format::Any);
        assert!(presets[0].dimensions.is_none());
    }
}
