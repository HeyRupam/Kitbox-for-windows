//! Kitbox on the command line.
//!
//! Exists for two reasons: batch work is genuinely faster to express as a
//! command, and it gives the engines an interface that can be exercised
//! end-to-end without a window — which is how the integration tests run.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use mt_core::engine::image::{self as img, CompressOptions, ImageCompressor, OutputFormat};
use mt_core::engine::pdf::{self, PageSelection, PdfCompressOptions, PdfCompressor, SplitMode};
use mt_core::size::{format_size, parse_target};
use mt_core::{Bias, FileKind, Outcome};
use mt_store::{FileMeta, NewJob, Store};

#[derive(Parser)]
#[command(name = "kitbox", version, about = "Offline PDF and image toolkit")]
struct Cli {
    /// Where history lives. Defaults to the per-user Kitbox folder.
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    /// Don't record anything in history.
    #[arg(long, global = true)]
    no_history: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compress files to a size, not a guess: `--target 50-100kb`.
    Compress {
        /// Files to compress.
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Required size: `100kb`, `under 100kb`, `50-100kb`, `exactly 1mb`.
        #[arg(short, long)]
        target: String,

        /// Where to write results. Defaults to alongside the input.
        #[arg(short, long)]
        out: Option<PathBuf>,

        #[arg(long, value_enum, default_value_t = FormatArg::Keep)]
        format: FormatArg,

        /// Which lever to spend first when something has to give.
        #[arg(long, value_enum, default_value_t = BiasArg::Balanced)]
        bias: BiasArg,

        /// Remove EXIF, GPS and camera data.
        #[arg(long, default_value_t = true)]
        strip: bool,

        /// Cap the longest edge, in pixels.
        #[arg(long)]
        max_dimension: Option<u32>,

        /// Force exact output dimensions, as `200x230`. Images only.
        #[arg(long)]
        exact: Option<String>,

        /// Let a PDF drop to greyscale if colour alone can't reach the target.
        #[arg(long)]
        grayscale: bool,

        /// Never take a PDF's images below this resolution.
        #[arg(long, default_value_t = 50)]
        min_dpi: u16,
    },

    /// Do the same thing to a pile of files, reporting each one by name.
    ///
    /// A file that can't be read costs you that file, not the run.
    Batch {
        /// Files, or folders to read one level deep.
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// What to do to all of them.
        #[arg(long, value_enum, default_value_t = BatchWhat::Compress)]
        op: BatchWhat,

        /// For `--op compress`: `100kb`, `under 100kb`, `50-100kb`.
        #[arg(long)]
        target: Option<String>,

        /// For `--op resize`: cap the longest edge, in pixels.
        #[arg(long)]
        longest: Option<u32>,

        /// For `--op convert`: the format to write.
        #[arg(long, value_enum)]
        to: Option<FormatArg>,

        /// Folder the results go into.
        #[arg(short, long)]
        out: PathBuf,
    },

    /// Report what a file is and what is taking up its space.
    Inspect {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },

    /// Combine PDFs end to end.
    Merge {
        /// Files to join, in the order given.
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Where to write the combined file.
        #[arg(short, long)]
        out: PathBuf,
    },

    /// Break a PDF into several files.
    Split {
        file: PathBuf,

        /// Chunk size. 1 means one file per page.
        #[arg(long, default_value_t = 1)]
        every: u32,

        /// Explicit ranges instead of fixed chunks: `1-3,8-12`.
        #[arg(long)]
        ranges: Option<String>,

        /// Folder for the pieces. Defaults to alongside the input.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// List, keep, drop, reorder or rotate pages.
    ///
    /// With no operation it just lists what is in the document.
    Pages {
        file: PathBuf,

        /// Keep only these pages: `1-3,7`.
        #[arg(long)]
        keep: Option<String>,

        /// Remove these pages.
        #[arg(long)]
        drop: Option<String>,

        /// New page order: `3,1,2`. Repeating a page duplicates it.
        #[arg(long)]
        order: Option<String>,

        /// Quarter turns clockwise to apply: 90, -90, 180.
        #[arg(long)]
        rotate: Option<i32>,

        /// Which pages `--rotate` applies to. Defaults to all of them.
        #[arg(long)]
        select: Option<String>,

        /// Where to write the result.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Put images into a PDF, one per page.
    ToPdf {
        #[arg(required = true)]
        images: Vec<PathBuf>,

        #[arg(short, long)]
        out: PathBuf,

        /// a4, letter, legal, or fit to shape each page to its image.
        #[arg(long, default_value = "a4")]
        page_size: String,

        /// auto, portrait or landscape.
        #[arg(long, default_value = "auto")]
        orientation: String,

        /// White space around each image, in points.
        #[arg(long, default_value_t = 18.0)]
        margin: f32,

        /// Cap the effective resolution. 0 means no cap.
        #[arg(long, default_value_t = 200)]
        max_dpi: u16,

        /// Quality used only when an image has to be re-encoded.
        #[arg(long, default_value_t = 82)]
        quality: u8,
    },

    /// Export a PDF's pages as JPEGs.
    ToImages {
        file: PathBuf,

        #[arg(short, long)]
        out: Option<PathBuf>,

        /// Longest edge in pixels. 0 means full size.
        #[arg(long, default_value_t = 0)]
        max_px: u32,

        #[arg(long, default_value_t = 88)]
        quality: u8,
    },

    /// Resize images.
    Resize {
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Fit inside a box without distorting: `800x600`.
        #[arg(long)]
        fit: Option<String>,

        /// Exactly this size, covering and trimming: `200x230`.
        #[arg(long)]
        fill: Option<String>,

        /// A percentage of the original.
        #[arg(long)]
        percent: Option<f32>,

        /// Scale so the longest edge is this many pixels.
        #[arg(long)]
        longest: Option<u32>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Crop images.
    Crop {
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Pixels: `x,y,width,height`.
        #[arg(long)]
        rect: Option<String>,

        /// Largest centred crop of this shape: `3.5:4.5`.
        #[arg(long)]
        aspect: Option<String>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Rotate or flip images.
    Rotate {
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Quarter turns clockwise: 90, 180, 270, -90.
        #[arg(long, default_value_t = 90)]
        degrees: i32,

        /// h or v to mirror as well.
        #[arg(long)]
        flip: Option<String>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Change an image's format.
    Convert {
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// jpg or png.
        #[arg(long, default_value = "jpg")]
        to: String,

        #[arg(long, default_value_t = 88)]
        quality: u8,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Add a password and permissions to a PDF.
    Protect {
        file: PathBuf,

        /// Needed to open the document. Note that anything typed here lands in
        /// your shell history.
        #[arg(long)]
        open_password: Option<String>,

        /// Controls permissions. Defaults to the open password.
        #[arg(long)]
        owner_password: Option<String>,

        #[arg(long)]
        no_print: bool,

        #[arg(long)]
        no_copy: bool,

        #[arg(long)]
        allow_modify: bool,

        #[arg(long)]
        allow_annotate: bool,

        /// aes256 (default) or aes128 for very old readers.
        #[arg(long, default_value = "aes256")]
        strength: String,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Remove protection from a PDF you can open.
    ///
    /// Supply the password, or leave it out for a document that opens freely
    /// but restricts printing or copying. Kitbox does not guess passwords.
    Unlock {
        file: PathBuf,

        #[arg(long)]
        password: Option<String>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Stamp text or a logo across a PDF's pages.
    Watermark {
        file: PathBuf,

        /// The text to stamp.
        #[arg(long)]
        text: Option<String>,

        /// An image to stamp instead of text.
        #[arg(long)]
        logo: Option<PathBuf>,

        /// Which pages, e.g. `1-3,7`. Defaults to all of them.
        #[arg(long)]
        pages: Option<String>,

        /// centre, tiled, top-left, bottom-right, and so on.
        #[arg(long, default_value = "centre")]
        placement: String,

        /// 0 to 1. Around 0.15 reads without hiding the page.
        #[arg(long, default_value_t = 0.15)]
        opacity: f32,

        /// Degrees anticlockwise. 45 is the classic diagonal.
        #[arg(long, default_value_t = 45.0)]
        rotation: f32,

        /// Text size in points. Omit to scale with the page.
        #[arg(long)]
        size: Option<f32>,

        /// Logo width as a fraction of the page width.
        #[arg(long, default_value_t = 0.35)]
        scale: f32,

        /// Colour as `#rrggbb`.
        #[arg(long, default_value = "#5a5a5a")]
        colour: String,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Put page numbers on a PDF.
    Number {
        file: PathBuf,

        /// `{n}` is the page number, `{total}` how many are numbered.
        #[arg(long, default_value = "{n}")]
        format: String,

        /// bottom-centre, bottom-right, top-left, and so on.
        #[arg(long, default_value = "bottom-centre")]
        position: String,

        /// What to call the first numbered page.
        #[arg(long, default_value_t = 1)]
        start: u32,

        #[arg(long, default_value_t = 11.0)]
        size: f32,

        /// Colour as `#rrggbb`.
        #[arg(long, default_value = "#282828")]
        colour: String,

        /// Which pages to number, e.g. `2-`. Defaults to all of them.
        #[arg(long)]
        pages: Option<String>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Crop a PDF's pages.
    ///
    /// This sets the area readers display. Anything outside it stays in the
    /// file, so it is not a way to remove information.
    CropPdf {
        file: PathBuf,

        /// Points to trim, as `top,right,bottom,left`.
        #[arg(long)]
        margins: Option<String>,

        /// Percentages to trim, as `top,right,bottom,left` (10 means 10%).
        #[arg(long)]
        percent: Option<String>,

        /// Shrink each page to the ink on it. Works on scans.
        #[arg(long)]
        auto: bool,

        /// Points of breathing room left around the content by `--auto`.
        #[arg(long, default_value_t = 6.0)]
        padding: f32,

        /// Which pages, e.g. `2-`. Defaults to all of them.
        #[arg(long)]
        pages: Option<String>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Recover a damaged PDF.
    Repair {
        file: PathBuf,

        /// Report what is wrong without writing anything.
        #[arg(long)]
        check: bool,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Turn photographs of pages into a scanned-looking PDF.
    Scan {
        /// The photographs, in page order.
        #[arg(required = true)]
        images: Vec<PathBuf>,

        #[arg(short, long, default_value = "scan.pdf")]
        out: PathBuf,

        /// colour, grey or document (even out the lighting, black on white).
        #[arg(long, default_value = "document")]
        mode: String,

        /// Use each photograph whole instead of looking for the page's edges.
        #[arg(long)]
        no_detect: bool,

        /// How hard to push the contrast in document mode, 0 to 1.
        #[arg(long, default_value_t = 0.6)]
        strength: f32,

        #[arg(long, default_value = "a4")]
        page_size: String,
    },

    /// Cut a signature out of a photograph, as a transparent PNG.
    Signature {
        /// A photograph of your signature on paper.
        photo: PathBuf,

        #[arg(short, long)]
        out: Option<PathBuf>,

        /// keep, black or blue.
        #[arg(long, default_value = "keep")]
        ink: String,

        /// How hard to cut the paper away, 0 to 1.
        #[arg(long, default_value_t = 0.5)]
        strength: f32,

        /// Keep the whole frame instead of cropping to the ink.
        #[arg(long)]
        no_trim: bool,
    },

    /// Draw a signature onto a PDF. Not a digital certificate.
    Sign {
        file: PathBuf,

        /// The signature image, ideally a transparent PNG from `signature`.
        #[arg(long)]
        signature: PathBuf,

        /// Pages to sign, e.g. "1", "2,5" or "last".
        #[arg(long, default_value = "1")]
        pages: String,

        /// Centre across the page, 0 to 1.
        #[arg(long, default_value_t = 0.3)]
        x: f32,

        /// Centre down the page, 0 to 1.
        #[arg(long, default_value_t = 0.85)]
        y: f32,

        /// Width as a fraction of the page.
        #[arg(long, default_value_t = 0.28)]
        width: f32,

        /// A line under the signature. `{date}` is replaced by --date.
        #[arg(long)]
        caption: Option<String>,

        /// What `{date}` means. Written out by you, not invented here.
        #[arg(long, default_value = "")]
        date: String,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Delete content from a PDF, rather than drawing a box over it.
    Redact {
        file: PathBuf,

        /// Text to remove everywhere it appears. Repeatable.
        #[arg(long = "text")]
        texts: Vec<String>,

        /// A box to clear: PAGE,X,Y,W,H as fractions of the page from the top
        /// left, e.g. 1,0.1,0.8,0.3,0.05. Repeatable.
        #[arg(long = "area")]
        areas: Vec<String>,

        /// Match the text exactly, including capitals.
        #[arg(long)]
        match_case: bool,

        /// Don't draw the black boxes.
        #[arg(long)]
        no_boxes: bool,

        /// Keep metadata, bookmarks and embedded files.
        #[arg(long)]
        keep_extras: bool,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// List a PDF form's fields and what they accept.
    Fields { file: PathBuf },

    /// Fill in a PDF form.
    Fill {
        file: PathBuf,

        /// NAME=VALUE, repeatable. A checkbox takes yes/no or one of its own
        /// on-states.
        #[arg(long = "set", required = true)]
        set: Vec<String>,

        /// Draw the values into the pages and remove the form. One way.
        #[arg(long)]
        flatten: bool,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Draw a filled form's values into the pages and remove the form.
    Flatten {
        file: PathBuf,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Draw on a page: text, highlights, shapes.
    ///
    /// Boxes are PAGE,X,Y,W,H as fractions of the page from the top left.
    Annotate {
        file: PathBuf,

        /// PAGE,X,Y,W,H,SIZE,TEXT — repeatable.
        #[arg(long = "text")]
        texts: Vec<String>,

        /// PAGE,X,Y,W,H — repeatable.
        #[arg(long = "highlight")]
        highlights: Vec<String>,

        /// PAGE,X,Y,W,H — repeatable. Covers; does not remove.
        #[arg(long = "whiteout")]
        whiteouts: Vec<String>,

        /// PAGE,X,Y,W,H — repeatable.
        #[arg(long = "box")]
        boxes: Vec<String>,

        /// PAGE,X,Y,W,H — repeatable.
        #[arg(long = "ellipse")]
        ellipses: Vec<String>,

        /// PAGE,X,Y,W,H, drawn corner to corner — repeatable.
        #[arg(long = "arrow")]
        arrows: Vec<String>,

        /// Ink and outlines, as RRGGBB.
        #[arg(long, default_value = "C2410C")]
        colour: String,

        /// Fill the shapes instead of outlining them.
        #[arg(long)]
        fill: bool,

        #[arg(long, default_value_t = 1.5)]
        line_width: f32,

        #[arg(long, default_value_t = 1.0)]
        opacity: f32,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Put a caption on a picture.
    Caption {
        file: PathBuf,

        #[arg(long, default_value = "")]
        top: String,

        #[arg(long, default_value = "")]
        bottom: String,

        /// Text height as a fraction of the picture, 0.03 to 0.25.
        #[arg(long, default_value_t = 0.11)]
        size: f32,

        /// Outline thickness relative to the text, 0 to 0.2.
        #[arg(long, default_value_t = 0.06)]
        outline: f32,

        /// Leave the capitals alone.
        #[arg(long)]
        no_shout: bool,

        /// Put the top caption on a white band above the picture.
        #[arg(long)]
        band: bool,

        /// A font file. Impact or another heavy face is found automatically.
        #[arg(long)]
        font: Option<PathBuf>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Prepare a PDF for long-term archiving, and say what stands in the way.
    Archive {
        file: PathBuf,

        /// Report only; change nothing.
        #[arg(long)]
        check: bool,

        /// 1 or 2. Part 1 is stricter and forbids transparency.
        #[arg(long, default_value_t = 2)]
        part: u8,

        /// An ICC profile for the output intent. On Windows, the system sRGB
        /// profile is used when this isn't given.
        #[arg(long)]
        icc: Option<PathBuf>,

        #[arg(long)]
        title: Option<String>,

        #[arg(long)]
        author: Option<String>,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Fix a photo's tone and colour.
    Adjust {
        file: PathBuf,

        /// Stretch the levels and neutralise the light, both at once.
        #[arg(long)]
        auto: bool,

        /// Stretch each channel onto the full range.
        #[arg(long)]
        levels: bool,

        /// Neutralise the colour of the light.
        #[arg(long)]
        colour: bool,

        /// Stops of exposure, -2 to 2.
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        exposure: f32,

        /// -1 flat to 1 hard.
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        contrast: f32,

        /// -1 grey to 1 vivid.
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        saturation: f32,

        /// -1 cold to 1 warm.
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        warmth: f32,

        /// 0 to 1.
        #[arg(long, default_value_t = 0.0)]
        sharpen: f32,

        #[arg(long)]
        greyscale: bool,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Find faces and cover them up.
    BlurFaces {
        file: PathBuf,

        /// Where ultraface-320.onnx lives. Defaults to the app's model folder.
        #[arg(long)]
        models: Option<PathBuf>,

        /// blur, pixelate or box.
        #[arg(long, default_value = "blur")]
        cover: String,

        /// Ignore anything the detector is less sure of than this, 0 to 1.
        #[arg(long, default_value_t = 0.7)]
        confidence: f32,

        /// Grow each face box by this fraction before covering.
        #[arg(long, default_value_t = 0.15)]
        grow: f32,

        /// How hard to blur or how coarse to pixelate, 0 to 1.
        #[arg(long, default_value_t = 0.8)]
        strength: f32,

        /// Only report what was found; change nothing.
        #[arg(long)]
        list: bool,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Make a picture bigger.
    Upscale {
        file: PathBuf,

        /// 2, 3 or 4 times.
        #[arg(long, default_value_t = 2)]
        scale: u32,

        /// Put detail back with the model instead of resampling. Slow, and
        /// limited in size.
        #[arg(long)]
        reconstruct: bool,

        /// Where realesrgan-x4.onnx lives. Defaults to the app's model folder.
        #[arg(long)]
        models: Option<PathBuf>,

        /// Extra sharpening for the resampled path, 0 to 1.
        #[arg(long, default_value_t = 0.4)]
        sharpen: f32,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Take the background out of a photo.
    Cutout {
        file: PathBuf,

        /// Where u2netp.onnx lives. Defaults to the app's model folder.
        #[arg(long)]
        models: Option<PathBuf>,

        /// A backdrop colour as RRGGBB. Left out, the background is clear.
        #[arg(long)]
        backdrop: Option<String>,

        /// How much of the uncertain edge to keep, 0 to 1.
        #[arg(long, default_value_t = 0.35)]
        softness: f32,

        #[arg(long, default_value_t = 0.5)]
        threshold: f32,

        #[arg(long, default_value_t = 1)]
        feather: u32,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Read the text in a scan and write it back as a hidden, searchable layer.
    Ocr {
        file: PathBuf,

        /// Where text-detection.onnx and text-recognition.onnx live. Defaults
        /// to the folder the desktop app downloads them into.
        #[arg(long)]
        models: Option<PathBuf>,

        /// Print the text instead of writing a searchable PDF.
        #[arg(long)]
        text: bool,

        /// Read every page, including ones that already have text.
        #[arg(long)]
        all_pages: bool,

        /// Pages to read, e.g. "1", "2-4" or "last".
        #[arg(long)]
        pages: Option<String>,

        /// Longest side, in pixels, a page is shrunk to before reading.
        #[arg(long, default_value_t = 2000)]
        max_px: u32,

        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Turn a PDF's text into Markdown.
    ToMarkdown {
        file: PathBuf,

        /// Put a rule and a comment between pages.
        #[arg(long)]
        page_breaks: bool,

        /// Treat everything as a paragraph instead of inferring headings.
        #[arg(long)]
        no_headings: bool,

        /// Don't turn bullets and numbers into lists.
        #[arg(long)]
        no_lists: bool,

        /// Where to write it. Defaults to alongside the input.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Compare two PDFs and report what changed.
    Compare {
        /// The original.
        before: PathBuf,
        /// The one to compare against it.
        after: PathBuf,

        /// Write the report as Markdown instead of printing it.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Show recent work from history.
    History {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum BatchWhat {
    /// Land every file inside a size target.
    Compress,
    /// Cap the longest edge. Pictures only.
    Resize,
    /// Rewrite in another format. Pictures only.
    Convert,
    /// Correct levels and colour automatically. Pictures only.
    Adjust,
}

#[derive(Copy, Clone, ValueEnum)]
enum FormatArg {
    Keep,
    Jpg,
    Png,
}

#[derive(Copy, Clone, ValueEnum)]
enum BiasArg {
    Balanced,
    KeepSharp,
    ShrinkPixels,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = if cli.no_history { None } else { Some(Store::open(store_root(&cli.store)?)?) };
    // How long the command took, for the history row. One clock for the whole
    // run is honest and needs no plumbing through every arm.
    let started = std::time::Instant::now();

    match cli.command {
        Command::Compress {
            files,
            target,
            out,
            format,
            bias,
            strip,
            max_dimension,
            exact,
            grayscale,
            min_dpi,
        } => {
            let target = parse_target(&target).context("could not read the target size")?;
            let opts = CompressOptions {
                format: match format {
                    FormatArg::Keep => OutputFormat::Keep,
                    FormatArg::Jpg => OutputFormat::Jpeg,
                    FormatArg::Png => OutputFormat::Png,
                },
                strip_metadata: strip,
                max_dimension,
                exact_dimensions: exact.as_deref().map(parse_dimensions).transpose()?,
            };
            let bias = match bias {
                BiasArg::Balanced => Bias::Balanced,
                BiasArg::KeepSharp => Bias::KeepSharp,
                BiasArg::ShrinkPixels => Bias::ShrinkPixels,
            };

            let pdf_opts =
                PdfCompressOptions { strip_metadata: strip, allow_grayscale: grayscale, min_dpi };

            let mut failures = 0;
            for file in &files {
                let result = compress_one(
                    file,
                    target,
                    bias,
                    &opts,
                    pdf_opts,
                    out.as_deref(),
                    store.as_ref(),
                );
                if let Err(e) = result {
                    eprintln!("{}: {e:#}", file.display());
                    failures += 1;
                }
            }
            if failures > 0 {
                bail!("{failures} of {} files could not be compressed", files.len());
            }
        }

        Command::Batch { paths, op, target, longest, to, out } => {
            let files = gather(&paths)?;
            if files.is_empty() {
                bail!("nothing to do — no readable files in what you gave me");
            }

            let op = match op {
                BatchWhat::Compress => mt_core::batch::BatchOp::Compress {
                    target: parse_target(target.as_deref().unwrap_or("100kb"))
                        .context("could not read the target size")?,
                    bias: Bias::Balanced,
                    format: OutputFormat::Keep,
                    strip_metadata: true,
                    exact: None,
                },
                BatchWhat::Resize => mt_core::batch::BatchOp::Resize {
                    longest: longest.context("--op resize needs --longest")?,
                },
                BatchWhat::Convert => mt_core::batch::BatchOp::Convert {
                    format: match to.context("--op convert needs --to")? {
                        FormatArg::Keep => bail!("--to has to be jpg or png"),
                        FormatArg::Jpg => OutputFormat::Jpeg,
                        FormatArg::Png => OutputFormat::Png,
                    },
                    strip_metadata: true,
                },
                BatchWhat::Adjust => mt_core::batch::BatchOp::Adjust(img::AdjustOptions {
                    auto_levels: true,
                    auto_colour: true,
                    ..Default::default()
                }),
            };

            let mut items = Vec::new();
            for file in &files {
                let bytes =
                    std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.display().to_string());
                items.push(mt_core::batch::Item { name, bytes });
            }

            println!("{} on {} files", op.describe(), items.len());
            let report = |p: mt_core::job::Progress| println!("  {}", p.stage);
            let result = mt_core::batch::run(&items, &op, &report, &mt_core::job::Cancel::new());

            std::fs::create_dir_all(&out).with_context(|| format!("making {}", out.display()))?;
            for (name, bytes) in &result.files {
                deliver(
                    store.as_ref(),
                    started,
                    "batch",
                    serde_json::json!({ "tool": "batch", "op": op }),
                    None,
                    &out.join(name),
                    bytes,
                    op.describe(),
                )?;
            }

            println!();
            for outcome in &result.outcomes {
                match outcome {
                    mt_core::batch::ItemOutcome::Done(produced) => println!(
                        "  {}  {} -> {}{}  ({})",
                        produced.name,
                        format_size(produced.before),
                        format_size(produced.after),
                        if produced.short { "  [short]" } else { "" },
                        produced.note
                    ),
                    mt_core::batch::ItemOutcome::Failed { name, why } => {
                        println!("  {name}  failed: {why}")
                    }
                    mt_core::batch::ItemOutcome::Skipped { name } => {
                        println!("  {name}  not reached")
                    }
                }
            }
            println!(
                "
{} done, {} fell short, {} failed — written to {}",
                result.done(),
                result.short(),
                result.failed(),
                out.display()
            );
            if result.failed() > 0 {
                std::process::exit(1);
            }
        }

        Command::Inspect { files } => {
            for file in files {
                let bytes =
                    std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
                let kind = FileKind::sniff(&bytes);
                print!("{}  {}  {}", file.display(), kind.label(), format_size(bytes.len() as u64));
                if kind.is_image() {
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        print!("  {}×{}", img.width(), img.height());
                    }
                }
                println!();
                if kind == FileKind::Pdf {
                    print_pdf_breakdown(&bytes)?;
                }
            }
        }

        Command::Merge { files, out } => {
            let mut inputs = Vec::new();
            for file in &files {
                inputs.push(
                    std::fs::read(file).with_context(|| format!("reading {}", file.display()))?,
                );
            }
            let (bytes, report) = pdf::merge(&inputs)?;
            deliver(
                store.as_ref(),
                started,
                "merge",
                serde_json::json!({ "tool": "merge", "files": files.len() }),
                files.first().zip(inputs.first()).map(|(p, b)| (p.as_path(), b.as_slice())),
                &out,
                &bytes,
                format!("{} files joined, {} pages", files.len(), report.pages_after),
            )?;
            println!(
                "{} files -> {} pages, {}  ->  {}",
                files.len(),
                report.pages_after,
                format_size(bytes.len() as u64),
                out.display()
            );
            if let Some(note) = report.note() {
                println!("  note: {note}");
            }
        }

        Command::Split { file, every, ranges, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let mode = match &ranges {
                Some(spec) => SplitMode::Ranges(parse_ranges(spec, pdf::page_count(&bytes)?)?),
                None if every <= 1 => SplitMode::EveryPage,
                None => SplitMode::EveryN(every),
            };
            let total_pages = pdf::page_count(&bytes)?;
            let parts = pdf::split(&bytes, &mode)?;
            let dir = out.unwrap_or_else(|| file.parent().unwrap_or(Path::new(".")).to_path_buf());
            std::fs::create_dir_all(&dir)?;
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
            println!("{} -> {} files", file.display(), parts.len());
            for part in parts {
                let path = dir.join(format!("{stem}-{}.pdf", part.suffix));
                deliver(
                    store.as_ref(),
                    started,
                    "split",
                    serde_json::json!({ "tool": "split", "mode": mode }),
                    Some((file.as_path(), bytes.as_slice())),
                    &path,
                    &part.bytes,
                    format!("{} of {} pages", part.pages.len(), total_pages),
                )?;
                println!(
                    "  {:>9}  {} page{}  {}",
                    format_size(part.bytes.len() as u64),
                    part.pages.len(),
                    if part.pages.len() == 1 { "" } else { "s" },
                    path.display()
                );
            }
        }

        Command::Pages { file, keep, drop, order, rotate, select, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let total = pdf::page_count(&bytes)?;

            if keep.is_none() && drop.is_none() && order.is_none() && rotate.is_none() {
                println!("{}  {} pages", file.display(), total);
                for page in pdf::page_summaries(&bytes)? {
                    let turned = if page.rotation != 0 {
                        format!("rotated {}  ", page.rotation)
                    } else {
                        String::new()
                    };
                    println!(
                        "  {:>3}  {:<18} {turned}{}",
                        page.number,
                        page.label,
                        if page.has_preview { "scan" } else { "text or vector" }
                    );
                }
                return Ok(());
            }

            // Kept because the chain needs what went in, and `working` is
            // about to become something else.
            let original = bytes.clone();
            let mut working = bytes;
            let mut notes: Vec<String> = Vec::new();

            if let Some(spec) = &keep {
                let sel = PageSelection::parse(spec, pdf::page_count(&working)?)?;
                let (next, report) = pdf::extract(&working, &sel)?;
                working = next;
                notes.extend(report.note());
            }
            if let Some(spec) = &drop {
                let sel = PageSelection::parse(spec, pdf::page_count(&working)?)?;
                let (next, report) = pdf::remove(&working, &sel)?;
                working = next;
                notes.extend(report.note());
            }
            if let Some(spec) = &order {
                let sel = PageSelection::parse(spec, pdf::page_count(&working)?)?;
                let (next, report) = pdf::reorder(&working, sel.pages())?;
                working = next;
                notes.extend(report.note());
            }
            if let Some(degrees) = rotate {
                let count = pdf::page_count(&working)?;
                let sel = PageSelection::parse(select.as_deref().unwrap_or("all"), count)?;
                let (next, report) = pdf::rotate(&working, &sel, degrees)?;
                working = next;
                notes.extend(report.note());
            }

            let path = out.unwrap_or_else(|| {
                let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
                file.with_file_name(format!("{stem}-kitbox.pdf"))
            });
            deliver(
                store.as_ref(),
                started,
                "pages",
                serde_json::json!({
                    "tool": "pages",
                    "keep": keep,
                    "drop": drop,
                    "order": order,
                    "rotate": rotate,
                }),
                Some((file.as_path(), original.as_slice())),
                &path,
                &working,
                format!("{total} pages -> {} pages", pdf::page_count(&working)?),
            )?;
            println!(
                "{} pages -> {} pages, {}  ->  {}",
                total,
                pdf::page_count(&working)?,
                format_size(working.len() as u64),
                path.display()
            );
            for note in notes {
                println!("  note: {note}");
            }
        }

        Command::ToPdf { images, out, page_size, orientation, margin, max_dpi, quality } => {
            let mut inputs = Vec::new();
            for path in &images {
                inputs.push(
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
                );
            }
            let opts = pdf::ImagesToPdfOptions {
                page_size: match page_size.to_ascii_lowercase().as_str() {
                    "fit" | "fit-image" => pdf::PageSize::FitImage,
                    "letter" => pdf::PageSize::Letter,
                    "legal" => pdf::PageSize::Legal,
                    "a4" => pdf::PageSize::A4,
                    other => bail!("unknown page size '{other}' — try a4, letter, legal or fit"),
                },
                orientation: match orientation.to_ascii_lowercase().as_str() {
                    "portrait" => pdf::Orientation::Portrait,
                    "landscape" => pdf::Orientation::Landscape,
                    "auto" => pdf::Orientation::Auto,
                    other => bail!("unknown orientation '{other}'"),
                },
                margin_pt: margin,
                jpeg_quality: quality,
                max_dpi: (max_dpi > 0).then_some(max_dpi),
            };
            let bytes = pdf::images_to_pdf(&inputs, &opts)?;
            deliver(
                store.as_ref(),
                started,
                "images-to-pdf",
                serde_json::json!({ "tool": "images-to-pdf", "options": opts }),
                None,
                &out,
                &bytes,
                format!("{} images made into {} pages", images.len(), images.len()),
            )?;
            let before: u64 = inputs.iter().map(|i| i.len() as u64).sum();
            println!(
                "{} images -> {} pages, {} -> {}  ({})",
                images.len(),
                images.len(),
                format_size(before),
                format_size(bytes.len() as u64),
                out.display()
            );
        }

        Command::ToImages { file, out, max_px, quality } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let exported = pdf::export_pages(&bytes, (max_px > 0).then_some(max_px), quality)?;
            let dir = out.unwrap_or_else(|| file.parent().unwrap_or(Path::new(".")).to_path_buf());
            std::fs::create_dir_all(&dir)?;
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("document");

            let mut written = 0;
            for page in &exported {
                match &page.jpeg {
                    Some(jpeg) => {
                        let path = dir.join(format!("{stem}-page-{}.jpg", page.page));
                        deliver(
                            store.as_ref(),
                            started,
                            "pdf-to-images",
                            serde_json::json!({
                                "tool": "pdf-to-images",
                                "maxPx": (max_px > 0).then_some(max_px),
                                "quality": quality,
                            }),
                            Some((file.as_path(), bytes.as_slice())),
                            &path,
                            jpeg,
                            format!("page {} as JPG, {}×{}", page.page, page.width, page.height),
                        )?;
                        written += 1;
                        println!(
                            "  {:>9}  {}×{}  {}",
                            format_size(jpeg.len() as u64),
                            page.width,
                            page.height,
                            path.display()
                        );
                    }
                    None => println!(
                        "  page {} skipped — {}",
                        page.page,
                        page.reason.as_deref().unwrap_or("not exportable")
                    ),
                }
            }
            println!("{written} of {} pages exported", exported.len());
        }

        Command::Resize { files, fit, fill, percent, longest, out } => {
            let mode = match (&fit, &fill, percent, longest) {
                (Some(spec), None, None, None) => {
                    let (w, h) = parse_dimensions(spec)?;
                    img::ResizeMode::Fit { width: w, height: h }
                }
                (None, Some(spec), None, None) => {
                    let (w, h) = parse_dimensions(spec)?;
                    img::ResizeMode::Fill { width: w, height: h }
                }
                (None, None, Some(pct), None) => img::ResizeMode::Percent(pct),
                (None, None, None, Some(edge)) => img::ResizeMode::LongestEdge(edge),
                _ => bail!("choose exactly one of --fit, --fill, --percent or --longest"),
            };
            for_each_image(&files, out.as_deref(), "resized", store.as_ref(), |bytes, opts| {
                img::resize(bytes, mode, opts)
            })?;
        }

        Command::Crop { files, rect, aspect, out } => {
            enum How {
                Rect(img::Rect),
                Aspect(f32),
            }
            let how = match (&rect, &aspect) {
                (Some(spec), None) => How::Rect(parse_rect(spec)?),
                (None, Some(spec)) => How::Aspect(parse_aspect(spec)?),
                _ => bail!("choose one of --rect or --aspect"),
            };
            for_each_image(&files, out.as_deref(), "cropped", store.as_ref(), |bytes, opts| {
                match how {
                    How::Rect(r) => img::crop(bytes, r, opts),
                    How::Aspect(a) => img::crop_to_aspect(bytes, a, opts),
                }
            })?;
        }

        Command::Rotate { files, degrees, flip, out } => {
            let flip = match flip.as_deref().map(str::to_ascii_lowercase).as_deref() {
                None => img::Flip::None,
                Some("h") | Some("horizontal") => img::Flip::Horizontal,
                Some("v") | Some("vertical") => img::Flip::Vertical,
                Some(other) => bail!("unknown flip '{other}' — use h or v"),
            };
            for_each_image(&files, out.as_deref(), "rotated", store.as_ref(), |bytes, opts| {
                img::rotate(bytes, degrees, flip, opts)
            })?;
        }

        Command::Convert { files, to, quality, out } => {
            let format = match to.to_ascii_lowercase().as_str() {
                "jpg" | "jpeg" => OutputFormat::Jpeg,
                "png" => OutputFormat::Png,
                other => bail!("cannot write '{other}' yet — try jpg or png"),
            };
            let opts = img::TransformOptions { format, quality, strip_metadata: true };
            for_each_image_with(
                &files,
                out.as_deref(),
                "converted",
                opts,
                store.as_ref(),
                img::convert,
            )?;
        }

        Command::Protect {
            file,
            open_password,
            owner_password,
            no_print,
            no_copy,
            allow_modify,
            allow_annotate,
            strength,
            out,
        } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let settings = pdf::Protection {
                open_password,
                owner_password,
                allow_printing: !no_print,
                allow_copying: !no_copy,
                allow_modifying: allow_modify,
                allow_annotating: allow_annotate,
                strength: match strength.to_ascii_lowercase().as_str() {
                    "aes256" | "aes-256" => pdf::Strength::Aes256,
                    "aes128" | "aes-128" => pdf::Strength::Aes128,
                    other => bail!("unknown strength '{other}' — try aes256 or aes128"),
                },
            };
            let protected = pdf::protect(&bytes, &settings)?;
            let path = out.unwrap_or_else(|| default_name(&file, "protected", "pdf"));
            write_out(&path, &protected)?;
            let info = pdf::inspect_security(&protected)?;
            println!("{} -> {}", file.display(), path.display());
            println!("  {}", info.summary());
        }

        Command::Unlock { file, password, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let (opened, kind) = pdf::unlock(&bytes, password.as_deref())?;
            let path = out.unwrap_or_else(|| default_name(&file, "unlocked", "pdf"));
            write_out(&path, &opened)?;
            println!("{} -> {}", file.display(), path.display());
            println!(
                "  {}",
                match kind {
                    pdf::UnlockKind::RestrictionsOnly =>
                        "it opened without a password; the printing and copying restrictions are gone",
                    pdf::UnlockKind::WithPassword => "the password is no longer needed to open it",
                }
            );
        }

        Command::Watermark {
            file,
            text,
            logo,
            pages,
            placement,
            opacity,
            rotation,
            size,
            scale,
            colour,
            out,
        } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let logo_bytes = match &logo {
                Some(path) => Some(
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
                ),
                None => None,
            };
            if text.is_some() == logo_bytes.is_some() {
                bail!("give exactly one of --text or --logo");
            }

            // Images take a different path: there is no built-in font to lean
            // on, so one has to come off the machine.
            if FileKind::sniff(&bytes) != FileKind::Pdf {
                let mark = match (&text, logo_bytes) {
                    (Some(t), None) => img::ImageMark::Text { text: t.clone(), font: load_font()? },
                    (None, Some(data)) => img::ImageMark::Logo(data),
                    _ => unreachable!("checked above"),
                };
                let options = img::ImageWatermarkOptions {
                    placement: parse_placement(&placement)?,
                    opacity,
                    rotation,
                    scale,
                    colour: parse_colour(&colour)?,
                    margin: 0.04,
                    format: OutputFormat::Keep,
                    quality: 90,
                };
                let marked = img::watermark::watermark(&bytes, &mark, &options)?;
                let kind = FileKind::sniff(&marked);
                let path =
                    out.unwrap_or_else(|| default_name(&file, "watermarked", kind.extension()));
                deliver(
                    store.as_ref(),
                    started,
                    "watermark-image",
                    serde_json::json!({ "tool": "watermark-image", "options": options }),
                    Some((file.as_path(), bytes.as_slice())),
                    &path,
                    &marked,
                    format!("watermarked, {}", format_size(marked.len() as u64)),
                )?;
                println!(
                    "{} -> {}  ({})",
                    file.display(),
                    path.display(),
                    format_size(marked.len() as u64)
                );
                return Ok(());
            }

            let mark = match (&text, logo_bytes) {
                (Some(t), None) => pdf::Mark::Text(t.clone()),
                (None, Some(data)) => pdf::Mark::Image(data),
                _ => unreachable!("checked above"),
            };
            let options = pdf::WatermarkOptions {
                mark,
                placement: parse_placement(&placement)?,
                opacity,
                rotation,
                font_size: size,
                image_scale: scale,
                colour: parse_colour(&colour)?,
                margin: 36.0,
            };
            let selection = match &pages {
                Some(spec) => Some(PageSelection::parse(spec, pdf::page_count(&bytes)?)?),
                None => None,
            };
            let marked = pdf::watermark(&bytes, selection.as_ref(), &options)?;
            let path = out.unwrap_or_else(|| default_name(&file, "watermarked", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "watermark",
                serde_json::json!({ "tool": "watermark", "pages": pages, "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &marked,
                format!("watermarked, {}", format_size(marked.len() as u64)),
            )?;
            println!(
                "{} -> {}  ({} pages marked, {})",
                file.display(),
                path.display(),
                selection.map(|s| s.pages().len()).unwrap_or(pdf::page_count(&bytes)? as usize),
                format_size(marked.len() as u64)
            );
        }

        Command::Number { file, format, position, start, size, colour, pages, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let options = pdf::PageNumberOptions {
                template: format,
                position: parse_number_position(&position)?,
                start_at: start,
                font_size: size,
                colour: parse_colour(&colour)?,
                margin: 28.0,
            };
            let selection = match &pages {
                Some(spec) => Some(PageSelection::parse(spec, pdf::page_count(&bytes)?)?),
                None => None,
            };
            let (numbered, count) = pdf::add_page_numbers(&bytes, selection.as_ref(), &options)?;
            let path = out.unwrap_or_else(|| default_name(&file, "numbered", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "page-numbers",
                serde_json::json!({ "tool": "page-numbers", "pages": pages, "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &numbered,
                format!("{count} pages numbered"),
            )?;
            println!(
                "{} -> {}  ({count} pages numbered, {})",
                file.display(),
                path.display(),
                format_size(numbered.len() as u64)
            );
        }

        Command::CropPdf { file, margins, percent, auto, padding, pages, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let spec = match (&margins, &percent, auto) {
                (Some(spec), None, false) => {
                    let [top, right, bottom, left] = parse_sides(spec)?;
                    pdf::CropSpec::Margins { top, right, bottom, left }
                }
                (None, Some(spec), false) => {
                    let [top, right, bottom, left] = parse_sides(spec)?;
                    pdf::CropSpec::Percent {
                        top: top / 100.0,
                        right: right / 100.0,
                        bottom: bottom / 100.0,
                        left: left / 100.0,
                    }
                }
                (None, None, true) => pdf::CropSpec::Auto { padding },
                _ => bail!("give exactly one of --margins, --percent or --auto"),
            };
            let selection = match &pages {
                Some(spec) => Some(PageSelection::parse(spec, pdf::page_count(&bytes)?)?),
                None => None,
            };
            let (cropped, report) = pdf::crop(&bytes, selection.as_ref(), &spec)?;
            let path = out.unwrap_or_else(|| default_name(&file, "cropped", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "crop-pdf",
                serde_json::json!({ "tool": "crop-pdf", "pages": pages, "spec": spec }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &cropped,
                format!("{} of {} pages cropped", report.pages_cropped, report.total_pages),
            )?;
            println!(
                "{} -> {}  ({} of {} pages cropped, {})",
                file.display(),
                path.display(),
                report.pages_cropped,
                report.total_pages,
                format_size(cropped.len() as u64)
            );
            if let Some(note) = report.note() {
                println!("  note: {note}");
            }
            println!("  {}", report.warning);
        }

        Command::Repair { file, check, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            println!("{}", file.display());

            if check {
                let report = pdf::diagnose(&bytes);
                print_repair(&report);
                return Ok(());
            }

            let (repaired, report) = pdf::repair(&bytes)?;
            print_repair(&report);
            let path = out.unwrap_or_else(|| default_name(&file, "repaired", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "repair",
                serde_json::json!({ "tool": "repair" }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &repaired,
                report.headline(),
            )?;
            println!(
                "  {} -> {}  {}",
                format_size(bytes.len() as u64),
                format_size(repaired.len() as u64),
                path.display()
            );
        }

        Command::Scan { images, out, mode, no_detect, strength, page_size } => {
            let mut inputs = Vec::new();
            for path in &images {
                inputs.push(
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
                );
            }
            let options = img::ScanOptions {
                mode: match mode.to_ascii_lowercase().as_str() {
                    "colour" | "color" => img::ScanMode::Colour,
                    "grey" | "gray" | "greyscale" | "grayscale" => img::ScanMode::Greyscale,
                    "document" | "doc" => img::ScanMode::Document,
                    other => bail!("unknown mode '{other}' — try colour, grey or document"),
                },
                detect_edges: !no_detect,
                strength,
            };
            let pdf_options = pdf::ImagesToPdfOptions {
                page_size: match page_size.to_ascii_lowercase().as_str() {
                    "fit" | "fit-image" => pdf::PageSize::FitImage,
                    "letter" => pdf::PageSize::Letter,
                    "legal" => pdf::PageSize::Legal,
                    "a4" => pdf::PageSize::A4,
                    other => bail!("unknown page size '{other}'"),
                },
                ..Default::default()
            };

            let (bytes, report) = img::scan_to_pdf(&inputs, &options, &pdf_options)?;
            deliver(
                store.as_ref(),
                started,
                "scan-to-pdf",
                serde_json::json!({ "tool": "scan-to-pdf", "options": options }),
                None,
                &out,
                &bytes,
                format!("{} pages scanned", report.pages),
            )?;
            println!(
                "{} photograph{} -> {} pages, {}  ({})",
                report.pages,
                if report.pages == 1 { "" } else { "s" },
                report.pages,
                format_size(bytes.len() as u64),
                out.display()
            );
            println!("  {} flattened from their edges", report.flattened);
            if let Some(note) = &report.note {
                println!("  note: {note}");
            }
        }

        Command::Signature { photo, out, ink, strength, no_trim } => {
            let bytes =
                std::fs::read(&photo).with_context(|| format!("reading {}", photo.display()))?;
            let options = img::SignatureOptions {
                ink: match ink.to_ascii_lowercase().as_str() {
                    "keep" => img::InkColour::Keep,
                    "black" => img::InkColour::Black,
                    "blue" => img::InkColour::Blue,
                    other => bail!("unknown ink '{other}' — try keep, black or blue"),
                },
                strength,
                trim: !no_trim,
            };
            let (cut, report) = img::extract_signature(&bytes, &options)?;
            if !report.found_ink {
                // Writing a fully transparent PNG would look like success.
                if let Some(note) = &report.note {
                    bail!("{note}");
                }
                bail!("no pen strokes were found");
            }

            let png = mt_core::engine::image::encode::encode(&cut, FileKind::Png, 100, true)?;
            let path = out.unwrap_or_else(|| default_name(&photo, "-signature", "png"));
            deliver(
                store.as_ref(),
                started,
                "signature",
                serde_json::json!({ "tool": "signature", "options": options }),
                Some((photo.as_path(), bytes.as_slice())),
                &path,
                &png,
                format!("signature cut out, {}×{}", report.width, report.height),
            )?;
            println!(
                "{} × {} px, {:.1}% ink, {}  -> {}",
                report.width,
                report.height,
                report.coverage * 100.0,
                format_size(png.len() as u64),
                path.display()
            );
            if let Some(note) = &report.note {
                println!("  note: {note}");
            }
        }

        Command::Sign { file, signature, pages, x, y, width, caption, date, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let mark = std::fs::read(&signature)
                .with_context(|| format!("reading {}", signature.display()))?;
            let total = pdf::page_count(&bytes)?;
            let selection = PageSelection::parse(&pages, total)?;

            let options = pdf::SignOptions {
                spots: selection
                    .pages()
                    .iter()
                    .map(|page| pdf::SignatureSpot { page: *page, x, y, width })
                    .collect(),
                caption: caption.map(|text| pdf::Caption {
                    text,
                    date,
                    size: 9.0,
                    colour: [45, 45, 54],
                }),
            };

            let (signed, report) = pdf::sign(&bytes, &mark, &options)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-signed", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "sign",
                serde_json::json!({ "tool": "sign" }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &signed,
                format!("signed on {} pages", report.pages_signed.len()),
            )?;
            println!(
                "{} placement{} on page{} {}  ({})",
                report.placements,
                if report.placements == 1 { "" } else { "s" },
                if report.pages_signed.len() == 1 { "" } else { "s" },
                report.pages_signed.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);
        }

        Command::Redact { file, texts, areas, match_case, no_boxes, keep_extras, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;

            let mut targets: Vec<pdf::RedactTarget> = texts
                .into_iter()
                .map(|needle| pdf::RedactTarget::Text { needle, match_case })
                .collect();
            for spec in &areas {
                let parts: Vec<&str> = spec.split(',').map(|p| p.trim()).collect();
                if parts.len() != 5 {
                    bail!("'{spec}' isn't PAGE,X,Y,W,H — five values, e.g. 1,0.1,0.8,0.3,0.05");
                }
                let number = |i: usize| -> Result<f32> {
                    parts[i]
                        .parse::<f32>()
                        .with_context(|| format!("'{}' isn't a number", parts[i]))
                };
                targets.push(pdf::RedactTarget::Area {
                    page: parts[0]
                        .parse()
                        .with_context(|| format!("'{}' isn't a page number", parts[0]))?,
                    x: number(1)?,
                    y: number(2)?,
                    width: number(3)?,
                    height: number(4)?,
                });
            }
            if targets.is_empty() {
                bail!("nothing to redact — pass --text or --area");
            }

            let options = pdf::RedactOptions {
                targets,
                draw_boxes: !no_boxes,
                box_colour: [0, 0, 0],
                strip_extras: !keep_extras,
            };
            let (redacted, report) = pdf::redact(&bytes, &options)?;

            let path = out.unwrap_or_else(|| default_name(&file, "-redacted", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "redact",
                serde_json::json!({ "tool": "redact", "targets": options.targets.len() }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &redacted,
                format!(
                    "{} glyphs removed from {} pages",
                    report.glyphs_removed,
                    report.pages_changed.len()
                ),
            )?;
            println!(
                "{} glyphs and {} image{} removed on page{} {}  ({})",
                report.glyphs_removed,
                report.images_edited,
                if report.images_edited == 1 { "" } else { "s" },
                if report.pages_changed.len() == 1 { "" } else { "s" },
                report.pages_changed.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                path.display()
            );
            if report.annotations_removed > 0 {
                println!("  {} annotations removed", report.annotations_removed);
            }
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);

            if report.verified {
                println!("  checked: the content is gone from the finished file");
            } else {
                // Exit non-zero: a redaction that didn't work is a failure,
                // however nice the file looks.
                for leak in &report.leaks {
                    println!("  LEAK: {leak}");
                }
                bail!("the redaction could not be verified — do not share this file");
            }
        }

        Command::Fields { file } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let found = pdf::fields(&bytes)?;
            if found.is_empty() {
                println!("{}", pdf::NOT_A_FORM);
                return Ok(());
            }
            println!("{}  ({} fields)", file.display(), found.len());
            for field in &found {
                let kind = format!("{:?}", field.kind).to_lowercase();
                let mut notes = Vec::new();
                if field.required {
                    notes.push("required".to_string());
                }
                if field.read_only {
                    notes.push("read-only".to_string());
                }
                if let Some(max) = field.max_length {
                    notes.push(format!("max {max}"));
                }
                if !field.options.is_empty() {
                    notes.push(format!("takes: {}", field.options.join(" | ")));
                }
                println!(
                    "  {:<24} {:<10} {}",
                    field.name,
                    kind,
                    field.value.clone().unwrap_or_default()
                );
                if let Some(label) = &field.label {
                    println!("    {label}");
                }
                if !notes.is_empty() {
                    println!("    {}", notes.join(" · "));
                }
            }
        }

        Command::Fill { file, set, flatten, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let mut values = std::collections::BTreeMap::new();
            for pair in &set {
                let Some((name, value)) = pair.split_once('=') else {
                    bail!("'{pair}' isn't NAME=VALUE");
                };
                values.insert(name.trim().to_string(), value.to_string());
            }

            let options = pdf::FillOptions { values, flatten };
            let (filled, report) = pdf::fill(&bytes, &options)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-filled", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "fill-form",
                serde_json::json!({ "tool": "fill-form", "fields": report.filled.len(), "flatten": flatten }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &filled,
                format!("{} fields filled", report.filled.len()),
            )?;

            println!(
                "{} field{} filled{}  ({})",
                report.filled.len(),
                if report.filled.len() == 1 { "" } else { "s" },
                if report.flattened { ", flattened" } else { "" },
                path.display()
            );
            for name in &report.unknown {
                println!("  no such field: {name}");
            }
            for name in &report.read_only {
                println!("  read-only, left alone: {name}");
            }
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            if !report.still_required.is_empty() {
                println!("  still required: {}", report.still_required.join(", "));
            }
        }

        Command::Flatten { file, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let (flat, report) = pdf::flatten(&bytes)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-flat", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "flatten",
                serde_json::json!({ "tool": "flatten" }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &flat,
                format!("{} fields drawn in", report.widgets_drawn),
            )?;
            println!(
                "{} field{} drawn into page{} {}  ({})",
                report.widgets_drawn,
                if report.widgets_drawn == 1 { "" } else { "s" },
                if report.pages_changed.len() == 1 { "" } else { "s" },
                report.pages_changed.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);
        }

        Command::Annotate {
            file,
            texts,
            highlights,
            whiteouts,
            boxes,
            ellipses,
            arrows,
            colour,
            fill,
            line_width,
            opacity,
            out,
        } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;

            let rgb = parse_colour(&colour)?;
            let mut marks: Vec<pdf::Annotation> = Vec::new();

            // PAGE,X,Y,W,H, with anything after that left for the caller.
            let split = |spec: &str, want: usize| -> Result<(u32, f32, f32, f32, f32, String)> {
                let parts: Vec<&str> = spec.splitn(want, ',').collect();
                if parts.len() < 5 {
                    bail!("'{spec}' isn't PAGE,X,Y,W,H");
                }
                let number = |i: usize| -> Result<f32> {
                    parts[i]
                        .trim()
                        .parse::<f32>()
                        .with_context(|| format!("'{}' isn't a number", parts[i]))
                };
                Ok((
                    parts[0]
                        .trim()
                        .parse()
                        .with_context(|| format!("'{}' isn't a page", parts[0]))?,
                    number(1)?,
                    number(2)?,
                    number(3)?,
                    number(4)?,
                    // Everything past the box, commas and all: a text
                    // mark carries its size and then the words themselves.
                    parts[5.min(parts.len())..].join(","),
                ))
            };

            let mut push = |spec: &str,
                            want: usize,
                            make: &dyn Fn(String) -> Result<pdf::AnnotationMark>|
             -> Result<()> {
                let (page, x, y, w, h, rest) = split(spec, want)?;
                let mut mark = pdf::Annotation::new(page, x, y, w, h, make(rest)?);
                mark.colour = rgb;
                mark.line_width = line_width;
                mark.opacity = opacity;
                marks.push(mark);
                Ok(())
            };

            for spec in &texts {
                push(spec, 7, &|rest: String| {
                    let (size, text) = rest
                        .split_once(',')
                        .ok_or_else(|| anyhow::anyhow!("a text mark needs SIZE,TEXT at the end"))?;
                    Ok(pdf::AnnotationMark::Text {
                        text: text.to_string(),
                        size: size.trim().parse().context("the text size isn't a number")?,
                    })
                })?;
            }
            for spec in &highlights {
                push(spec, 5, &|_| Ok(pdf::AnnotationMark::Highlight))?;
            }
            for spec in &whiteouts {
                push(spec, 5, &|_| Ok(pdf::AnnotationMark::Whiteout))?;
            }
            for spec in &boxes {
                push(spec, 5, &|_| Ok(pdf::AnnotationMark::Rectangle { filled: fill }))?;
            }
            for spec in &ellipses {
                push(spec, 5, &|_| Ok(pdf::AnnotationMark::Ellipse { filled: fill }))?;
            }
            for spec in &arrows {
                push(spec, 5, &|_| Ok(pdf::AnnotationMark::Line { arrow: true }))?;
            }

            if marks.is_empty() {
                bail!("nothing to draw — pass --text, --highlight, --box, --ellipse, --arrow or --whiteout");
            }

            let (drawn, report) = pdf::annotate(&bytes, &pdf::AnnotateOptions { marks })?;
            let path = out.unwrap_or_else(|| default_name(&file, "-marked", "pdf"));
            deliver(
                store.as_ref(),
                started,
                "annotate",
                serde_json::json!({ "tool": "annotate", "marks": report.drawn }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &drawn,
                format!("{} marks drawn", report.drawn),
            )?;
            println!(
                "{} mark{} on page{} {}  ({})",
                report.drawn,
                if report.drawn == 1 { "" } else { "s" },
                if report.pages_changed.len() == 1 { "" } else { "s" },
                report.pages_changed.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);
        }

        Command::Caption { file, top, bottom, size, outline, no_shout, band, font, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let font = match &font {
                Some(path) => {
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?
                }
                None => caption_font()?,
            };

            let options = img::MemeOptions {
                top,
                bottom,
                uppercase: !no_shout,
                size,
                outline,
                colour: [255, 255, 255],
                outline_colour: [0, 0, 0],
                band,
            };
            let (image, report) = img::meme(&bytes, &font, &options)?;
            let encoded = mt_core::engine::image::encode::encode(&image, FileKind::Png, 100, true)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-caption", "png"));
            deliver(
                store.as_ref(),
                started,
                "meme",
                serde_json::json!({ "tool": "meme", "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &encoded,
                format!("captioned, {}×{}", report.width, report.height),
            )?;

            println!(
                "{} × {}, {}  ({})",
                report.width,
                report.height,
                format_size(encoded.len() as u64),
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
        }

        Command::Archive { file, check, part, icc, title, author, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let part = match part {
                1 => pdf::Part::A1b,
                2 => pdf::Part::A2b,
                other => bail!("part {other} isn't one this understands — try 1 or 2"),
            };

            let report = if check {
                pdf::check_archive(&bytes, part)?
            } else {
                let profile = match &icc {
                    Some(path) => Some(std::fs::read(path)?),
                    None => system_srgb_profile(),
                };
                if profile.is_none() {
                    eprintln!(
                        "no colour profile found, so the output intent will be missing —                          pass --icc to supply one"
                    );
                }
                let options = pdf::ArchiveOptions { part, icc: profile, title, author };
                let (converted, report) = pdf::to_archive(&bytes, &options)?;
                let path = out.unwrap_or_else(|| default_name(&file, "-pdfa", "pdf"));
                deliver(
                    store.as_ref(),
                    started,
                    "pdf-archive",
                    serde_json::json!({
                        "tool": "pdf-archive",
                        "part": report.part,
                        "conformant": report.conformant,
                    }),
                    Some((file.as_path(), bytes.as_slice())),
                    &path,
                    &converted,
                    format!(
                        "{}, {}",
                        report.part,
                        if report.conformant { "conformant" } else { "not conformant" }
                    ),
                )?;
                println!("{}", path.display());
                report
            };

            println!("  {}", report.part);
            for applied in &report.applied {
                println!("  applied: {applied}");
            }
            if report.conformant {
                println!("  every rule checked here passed");
            } else {
                for blocker in &report.blockers {
                    println!("  BLOCKED — {}: {}", blocker.rule, blocker.detail);
                }
            }
            println!("  {}", report.note);

            if !report.conformant && !check {
                // A file that says PDF/A and isn't should not look like
                // success to a script.
                std::process::exit(1);
            }
        }

        Command::Adjust {
            file,
            auto,
            levels,
            colour,
            exposure,
            contrast,
            saturation,
            warmth,
            sharpen,
            greyscale,
            out,
        } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let options = img::AdjustOptions {
                auto_levels: auto || levels,
                auto_colour: auto || colour,
                exposure,
                contrast,
                saturation,
                warmth,
                sharpen,
                greyscale,
            };
            if !options.changes_anything() {
                bail!("nothing to change — try --auto, or one of the sliders");
            }

            let (image, report) = img::adjust(&bytes, &options)?;
            let kind = if FileKind::sniff(&bytes) == FileKind::Png {
                FileKind::Png
            } else {
                FileKind::Jpeg
            };
            let encoded = mt_core::engine::image::encode::encode(&image, kind, 92, true)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-adjusted", kind.extension()));
            deliver(
                store.as_ref(),
                started,
                "adjust",
                serde_json::json!({ "tool": "adjust", "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &encoded,
                format!("adjusted, {}×{}", report.width, report.height),
            )?;

            println!(
                "{} × {}, {}  ({})",
                report.width,
                report.height,
                format_size(encoded.len() as u64),
                path.display()
            );
            if !report.levels.is_empty() {
                println!(
                    "  levels stretched from {}",
                    report
                        .levels
                        .iter()
                        .map(|(lo, hi)| format!("{lo}-{hi}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if let Some(gain) = report.colour_gain {
                println!("  colour corrected by {:.2}, {:.2}, {:.2}", gain[0], gain[1], gain[2]);
            }
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
        }

        Command::BlurFaces { file, models, cover, confidence, grow, strength, list, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let dir = models.unwrap_or_else(default_model_dir);
            let path = dir.join("ultraface-320.onnx");
            if !path.exists() {
                bail!(
                    "the face-detection model isn't in {}.\n\
                     Open the app once and let it fetch it, or download it there yourself:\n  \
                     https://github.com/onnx/models/raw/main/validated/vision/body_analysis/ultraface/models/version-RFB-320.onnx",
                    dir.display()
                );
            }
            let model = img::FaceModel::load(std::fs::read(&path)?)?;

            if list {
                let faces = img::find_faces(&bytes, &model, confidence)?;
                println!("{} face{}", faces.len(), if faces.len() == 1 { "" } else { "s" });
                for f in &faces {
                    println!(
                        "  {}×{} at {},{}  confidence {:.2}",
                        f.width, f.height, f.x, f.y, f.confidence
                    );
                }
                println!("  {}", img::NOT_A_GUARANTEE);
                return Ok(());
            }

            let options = img::BlurOptions {
                cover: match cover.to_ascii_lowercase().as_str() {
                    "blur" => img::Cover::Blur,
                    "pixelate" | "pixel" | "mosaic" => img::Cover::Pixelate,
                    "box" | "block" => img::Cover::Box([20, 20, 24]),
                    other => bail!("unknown cover '{other}' — try blur, pixelate or box"),
                },
                confidence,
                grow,
                strength,
            };
            let (image, report) = img::blur_faces(&bytes, &model, &options)?;
            let kind = if FileKind::sniff(&bytes) == FileKind::Png {
                FileKind::Png
            } else {
                FileKind::Jpeg
            };
            let encoded = mt_core::engine::image::encode::encode(&image, kind, 92, true)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-blurred", kind.extension()));
            deliver(
                store.as_ref(),
                started,
                "blur-faces",
                serde_json::json!({ "tool": "blur-faces", "cover": options.cover, "faces": report.covered }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &encoded,
                format!("{} faces covered", report.covered),
            )?;

            println!(
                "{} face{} covered, {}  ({})",
                report.covered,
                if report.covered == 1 { "" } else { "s" },
                format_size(encoded.len() as u64),
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);
        }

        Command::Upscale { file, scale, reconstruct, models, sharpen, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;

            let model = if reconstruct {
                let dir = models.unwrap_or_else(default_model_dir);
                let path = dir.join("realesrgan-x4.onnx");
                if !path.exists() {
                    bail!(
                        "the upscaling model isn't in {}.\n\
                         Open the app once and let it fetch it, or download it there yourself:\n  \
                         https://huggingface.co/baker76/upscale-models/resolve/main/RealESRGAN_x4.onnx\n\
                         Or drop --reconstruct to resample instead, which needs nothing.",
                        dir.display()
                    );
                }
                Some(img::UpscaleModel::load(std::fs::read(&path)?)?)
            } else {
                None
            };

            let options = img::UpscaleOptions {
                method: if reconstruct {
                    img::UpscaleMethod::Reconstruct
                } else {
                    img::UpscaleMethod::Sharpen
                },
                scale,
                sharpen,
            };
            if reconstruct {
                let image = image::load_from_memory(&bytes)?;
                eprintln!(
                    "about {:.0}s of work…",
                    img::estimate_seconds(image.width(), image.height())
                );
            }

            let (image, report) = img::upscale(&bytes, model.as_ref(), &options)?;
            let kind = if FileKind::sniff(&bytes) == FileKind::Png {
                FileKind::Png
            } else {
                FileKind::Jpeg
            };
            let encoded = mt_core::engine::image::encode::encode(&image, kind, 92, true)?;
            let path = out.unwrap_or_else(|| default_name(&file, "-bigger", kind.extension()));
            deliver(
                store.as_ref(),
                started,
                "upscale",
                serde_json::json!({ "tool": "upscale", "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &encoded,
                format!("{}×{} -> {}×{}", report.from.0, report.from.1, report.to.0, report.to.1),
            )?;

            println!(
                "{}×{} -> {}×{}, {} in {:.1}s  ({})",
                report.from.0,
                report.from.1,
                report.to.0,
                report.to.1,
                format_size(encoded.len() as u64),
                report.seconds,
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);
        }

        Command::Cutout { file, models, backdrop, softness, threshold, feather, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let dir = models.unwrap_or_else(default_model_dir);
            let model_path = dir.join("u2netp.onnx");
            if !model_path.exists() {
                bail!(
                    "the background-removal model isn't in {}.\n\
                     Open the app once and let it fetch it, or download it there yourself:\n  \
                     https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
                    dir.display()
                );
            }

            let model = img::CutoutModel::load(std::fs::read(&model_path)?)?;
            let options = img::CutoutOptions {
                backdrop: match &backdrop {
                    Some(hex) => img::Backdrop::Colour(parse_colour(hex)?),
                    None => img::Backdrop::Transparent,
                },
                softness,
                threshold,
                feather,
            };
            let (image, report) = img::remove_background(&bytes, &model, &options)?;
            let png = mt_core::engine::image::encode::encode(&image, FileKind::Png, 100, true)?;

            let path = out.unwrap_or_else(|| default_name(&file, "-cutout", "png"));
            deliver(
                store.as_ref(),
                started,
                "remove-background",
                serde_json::json!({ "tool": "remove-background", "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                &png,
                format!("background removed, {}×{}", report.width, report.height),
            )?;
            println!(
                "{} × {}, {:.1}% kept, {}  -> {}",
                report.width,
                report.height,
                report.coverage * 100.0,
                format_size(png.len() as u64),
                path.display()
            );
            for warning in &report.warnings {
                println!("  warning: {warning}");
            }
            println!("  {}", report.note);
        }

        Command::Ocr { file, models, text, all_pages, pages, max_px, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;

            let dir = models.unwrap_or_else(default_model_dir);
            let detection = dir.join("text-detection.onnx");
            let recognition = dir.join("text-recognition.onnx");
            if !detection.exists() || !recognition.exists() {
                bail!(
                    "the OCR models aren't in {}.\n\
                     Open the app once and let it fetch them, or download these two files there \
                     yourself:\n  \
                     https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx\n  \
                     https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
                    dir.display()
                );
            }

            eprintln!("loading the models…");
            let engine =
                pdf::OcrModels::load(std::fs::read(&detection)?, std::fs::read(&recognition)?)?;

            let selection = match &pages {
                Some(spec) => {
                    let total = pdf::page_count(&bytes)?;
                    Some(PageSelection::parse(spec, total)?)
                }
                None => None,
            };
            let options =
                pdf::OcrOptions { pages: selection, skip_pages_with_text: !all_pages, max_px };

            if text {
                let (found, report) = pdf::ocr_read(&bytes, &engine, &options)?;
                for page in &found {
                    println!("--- page {} ---", page.page);
                    println!("{}", page.text);
                }
                ocr_summary(&report);
            } else {
                let (searchable, report) = pdf::searchable(&bytes, &engine, &options)?;
                if report.pages_read.is_empty() {
                    ocr_summary(&report);
                    bail!("nothing was read, so there is no searchable copy to write");
                }
                let path = out.unwrap_or_else(|| default_name(&file, "-searchable", "pdf"));
                deliver(
                    store.as_ref(),
                    started,
                    "ocr",
                    serde_json::json!({ "tool": "ocr", "options": options }),
                    Some((file.as_path(), bytes.as_slice())),
                    &path,
                    &searchable,
                    format!("{} words read on {} pages", report.words, report.pages_read.len()),
                )?;
                println!("{}", path.display());
                ocr_summary(&report);
            }
        }

        Command::ToMarkdown { file, page_breaks, no_headings, no_lists, out } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let options = pdf::MarkdownOptions {
                page_breaks,
                detect_headings: !no_headings,
                detect_lists: !no_lists,
            };
            let (markdown, report) = pdf::to_markdown(&bytes, &options)?;

            println!("{}", file.display());
            println!(
                "  {} of {} pages had text · {} characters",
                report.pages_with_text, report.pages, report.characters
            );
            if let Some(note) = &report.note {
                println!("  note: {note}");
            }
            if report.pages_with_text == 0 {
                // Writing an empty file would look like success.
                bail!("nothing to write");
            }

            let path = out.unwrap_or_else(|| default_name(&file, "", "md"));
            deliver(
                store.as_ref(),
                started,
                "pdf-to-markdown",
                serde_json::json!({ "tool": "pdf-to-markdown", "options": options }),
                Some((file.as_path(), bytes.as_slice())),
                &path,
                markdown.as_bytes(),
                format!("{} pages read", report.pages_with_text),
            )?;
            println!("  -> {}", path.display());
        }

        Command::Compare { before, after, out } => {
            let a =
                std::fs::read(&before).with_context(|| format!("reading {}", before.display()))?;
            let b =
                std::fs::read(&after).with_context(|| format!("reading {}", after.display()))?;
            let report = pdf::compare(&a, &b)?;

            match out {
                Some(path) => {
                    deliver(
                        store.as_ref(),
                        started,
                        "compare",
                        serde_json::json!({ "tool": "compare" }),
                        None,
                        &path,
                        report.summary().as_bytes(),
                        format!(
                            "{} lines added, {} removed",
                            report.lines_added, report.lines_removed
                        ),
                    )?;
                    println!(
                        "{} vs {}  ({} added, {} removed)  -> {}",
                        before.display(),
                        after.display(),
                        report.lines_added,
                        report.lines_removed,
                        path.display()
                    );
                }
                None => print!("{}", report.summary()),
            }
        }

        Command::History { limit } => {
            let Some(store) = store.as_ref() else {
                bail!("history is disabled");
            };
            let rows = store.recent_outputs(limit)?;
            if rows.is_empty() {
                println!("Nothing yet.");
            }
            for r in rows {
                println!("{:<40} {:>10}  {}", r.name, format_size(r.size), &r.hash[..12]);
            }
            let usage = store.usage()?;
            println!(
                "\n{} files, {} on disk, {} jobs",
                usage.files,
                format_size(usage.bytes),
                usage.jobs
            );
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compress_one(
    path: &Path,
    target: mt_core::SizeTarget,
    bias: Bias,
    opts: &CompressOptions,
    pdf_opts: PdfCompressOptions,
    out_dir: Option<&Path>,
    store: Option<&Store>,
) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let original = bytes.len() as u64;

    let (output_kind, outcome) = if FileKind::sniff(&bytes) == FileKind::Pdf {
        let compressor = PdfCompressor::new(&bytes, pdf_opts)?;
        // The breakdown is most useful before the numbers, so print it first.
        println!("{}", path.display());
        println!("  {}", compressor.report().headline());
        let outcome = mt_core::solve(&compressor, target, bias)?;
        (FileKind::Pdf, outcome)
    } else {
        let compressor = ImageCompressor::new(&bytes, opts.clone())?;
        let kind = compressor.output_kind();
        println!("{}", path.display());
        (kind, mt_core::solve(&compressor, target, bias)?)
    };

    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let out_path = out_dir
        .map(|d| d.join(format!("{name}-kitbox.{}", output_kind.extension())))
        .unwrap_or_else(|| {
            path.with_file_name(format!("{name}-kitbox.{}", output_kind.extension()))
        });

    match &outcome {
        Outcome::Infeasible { why, suggestions, best_size, .. } => {
            println!("  cannot reach {} — {why}", target.describe());
            println!("  closest was {}", format_size(*best_size));
            for s in suggestions {
                println!("  · {s}");
            }
            bail!("target not reachable");
        }
        Outcome::Hit(s) | Outcome::Padded(s) => {
            if let Some(dir) = out_dir {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&out_path, &s.bytes)
                .with_context(|| format!("writing {}", out_path.display()))?;

            // Never round up to a flat "100% smaller" — the file still exists.
            let saved = (100.0 - (s.size as f64 / original.max(1) as f64 * 100.0)).min(99.9);
            println!(
                "  {} → {}  ({saved:.1}% smaller)  {}",
                format_size(original),
                format_size(s.size),
                s.knob
            );
            if let Some(score) = s.score {
                println!("  {}", mt_core::quality::describe(score));
            }
            println!("  {} attempts in {} ms  →  {}", s.probes, s.elapsed_ms, out_path.display());
            if let Some(note) = &s.note {
                println!("  note: {note}");
            }

            if let Some(store) = store {
                record(store, path, &bytes, &out_path, s, original, target)?;
            }
        }
    }

    Ok(())
}

fn record(
    store: &Store,
    input_path: &Path,
    input_bytes: &[u8],
    out_path: &Path,
    solved: &mt_core::Solved,
    original: u64,
    target: mt_core::SizeTarget,
) -> Result<()> {
    let input = store.put(
        input_bytes,
        FileMeta {
            name: file_name(input_path),
            kind: FileKind::sniff(input_bytes).extension().into(),
            source_path: Some(input_path.display().to_string()),
            ..Default::default()
        },
    )?;
    let output = store.put(
        &solved.bytes,
        FileMeta {
            name: file_name(out_path),
            kind: FileKind::sniff(&solved.bytes).extension().into(),
            source_path: Some(out_path.display().to_string()),
            ..Default::default()
        },
    )?;
    store.record_job(NewJob {
        tool: "compress".into(),
        recipe: serde_json::json!({ "tool": "compress", "target": target }).to_string(),
        input_hash: input.hash,
        output_hash: output.hash,
        summary: format!(
            "{} → {}, {}",
            format_size(original),
            format_size(solved.size),
            solved.knob
        ),
        duration_ms: solved.elapsed_ms as i64,
    })?;
    Ok(())
}

/// Record any one-file-in, one-file-out job.
///
/// `record` above is compress-specific and was, for a long time, the only
/// thing on the command line that wrote to history at all — so a file resized
/// or converted here never appeared in the gallery, and the chain it belonged
/// to had a hole in it. Lineage is only worth having if everything is in it.
#[allow(clippy::too_many_arguments)]
fn record_job(
    store: &Store,
    tool: &str,
    recipe: serde_json::Value,
    input: Option<(&Path, &[u8])>,
    out_path: &Path,
    out_bytes: &[u8],
    summary: String,
    duration_ms: i64,
) -> Result<()> {
    // Something built from scratch — a PDF made from photographs, a page
    // rendered from a URL — has no single file behind it. The chain starts
    // here, and an empty anchor is how the store says so.
    let input_hash = match input {
        Some((path, bytes)) => {
            store
                .put(
                    bytes,
                    FileMeta {
                        name: file_name(path),
                        kind: FileKind::sniff(bytes).extension().into(),
                        source_path: Some(path.display().to_string()),
                        ..Default::default()
                    },
                )?
                .hash
        }
        None => String::new(),
    };
    let output = store.put(
        out_bytes,
        FileMeta {
            name: file_name(out_path),
            kind: FileKind::sniff(out_bytes).extension().into(),
            source_path: Some(out_path.display().to_string()),
            ..Default::default()
        },
    )?;
    store.record_job(NewJob {
        tool: tool.into(),
        recipe: recipe.to_string(),
        input_hash,
        output_hash: output.hash,
        summary,
        duration_ms,
    })?;
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string()
}

/// Run one transform over a list of files, reporting each.
fn for_each_image<F>(
    files: &[PathBuf],
    out_dir: Option<&Path>,
    verb: &str,
    store: Option<&Store>,
    transform: F,
) -> Result<()>
where
    F: Fn(&[u8], &img::TransformOptions) -> mt_core::Result<Vec<u8>>,
{
    for_each_image_with(files, out_dir, verb, img::TransformOptions::default(), store, transform)
}

fn for_each_image_with<F>(
    files: &[PathBuf],
    out_dir: Option<&Path>,
    verb: &str,
    opts: img::TransformOptions,
    store: Option<&Store>,
    transform: F,
) -> Result<()>
where
    F: Fn(&[u8], &img::TransformOptions) -> mt_core::Result<Vec<u8>>,
{
    let mut failures = 0;
    for file in files {
        let result = (|| -> Result<()> {
            let bytes =
                std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
            let started = std::time::Instant::now();
            let output = transform(&bytes, &opts)?;
            let elapsed = started.elapsed().as_millis() as i64;
            let kind = FileKind::sniff(&output);
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
            let name = format!("{stem}-kitbox.{}", kind.extension());
            let path = match out_dir {
                Some(dir) => dir.join(name),
                None => file.with_file_name(name),
            };
            write_out(&path, &output)?;

            let dims = image::load_from_memory(&output)
                .map(|i| format!("{}×{}", i.width(), i.height()))
                .unwrap_or_default();
            println!(
                "{} -> {} {}  {}  {}",
                file.display(),
                verb,
                dims,
                format_size(output.len() as u64),
                path.display()
            );

            if let Some(store) = store {
                record_job(
                    store,
                    verb,
                    serde_json::json!({ "tool": verb }),
                    Some((file.as_path(), bytes.as_slice())),
                    &path,
                    &output,
                    format!("{} {}, {}", verb, dims, format_size(output.len() as u64)),
                    elapsed,
                )?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("{}: {e:#}", file.display());
            failures += 1;
        }
    }
    if failures > 0 {
        bail!("{failures} of {} files failed", files.len());
    }
    Ok(())
}

fn parse_rect(spec: &str) -> Result<img::Rect> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        bail!("'{spec}' should look like x,y,width,height");
    }
    let n = |i: usize| -> Result<u32> {
        parts[i].parse::<u32>().with_context(|| format!("'{}' is not a number", parts[i]))
    };
    Ok(img::Rect { x: n(0)?, y: n(1)?, width: n(2)?, height: n(3)? })
}

fn parse_aspect(spec: &str) -> Result<f32> {
    let (w, h) = spec
        .split_once([':', '/', 'x'])
        .with_context(|| format!("'{spec}' should look like 3.5:4.5"))?;
    let w: f32 = w.trim().parse()?;
    let h: f32 = h.trim().parse()?;
    if w <= 0.0 || h <= 0.0 {
        bail!("an aspect ratio needs two positive numbers");
    }
    Ok(w / h)
}

/// Read the first font the machine actually has.
///
/// `mt-core` keeps the candidate list but never opens a file, so this is where
/// the reading happens.
fn load_font() -> Result<Vec<u8>> {
    for path in mt_core::engine::image::watermark::FONT_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(bytes);
        }
    }
    bail!("no usable font found on this machine — use --logo instead of --text")
}

fn parse_placement(spec: &str) -> Result<pdf::Placement> {
    use pdf::Placement::*;
    Ok(match spec.to_ascii_lowercase().replace([' ', '_'], "-").as_str() {
        "centre" | "center" | "middle" => Centre,
        "tiled" | "tile" => Tiled,
        "top-left" => TopLeft,
        "top-centre" | "top-center" | "top" => TopCentre,
        "top-right" => TopRight,
        "middle-left" | "left" => MiddleLeft,
        "middle-right" | "right" => MiddleRight,
        "bottom-left" => BottomLeft,
        "bottom-centre" | "bottom-center" | "bottom" => BottomCentre,
        "bottom-right" => BottomRight,
        other => bail!("unknown placement '{other}' — try centre, tiled or bottom-right"),
    })
}

fn parse_number_position(spec: &str) -> Result<pdf::NumberPosition> {
    use pdf::NumberPosition::*;
    Ok(match spec.to_ascii_lowercase().replace([' ', '_'], "-").as_str() {
        "top-left" => TopLeft,
        "top-centre" | "top-center" | "top" => TopCentre,
        "top-right" => TopRight,
        "bottom-left" => BottomLeft,
        "bottom-centre" | "bottom-center" | "bottom" => BottomCentre,
        "bottom-right" => BottomRight,
        other => bail!("unknown position '{other}' — try bottom-centre or top-right"),
    })
}

/// `top,right,bottom,left`, or one value for all four, or two for
/// vertical/horizontal — the way CSS margins read.
fn parse_sides(spec: &str) -> Result<[f32; 4]> {
    let parts: Vec<f32> = spec
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|_| anyhow::anyhow!("'{p}' is not a number")))
        .collect::<Result<_>>()?;
    Ok(match parts.as_slice() {
        [all] => [*all; 4],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => bail!("'{spec}' should be one, two or four numbers"),
    })
}

fn parse_colour(spec: &str) -> Result<[u8; 3]> {
    let hex = spec.trim().trim_start_matches('#');
    if hex.len() != 6 {
        bail!("'{spec}' should look like #5a5a5a");
    }
    let part = |i: usize| -> Result<u8> {
        u8::from_str_radix(&hex[i..i + 2], 16).with_context(|| format!("'{spec}' is not a colour"))
    };
    Ok([part(0)?, part(2)?, part(4)?])
}

/// The heaviest face on this machine, for captions.
fn caption_font() -> Result<Vec<u8>> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\impact.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
        r"C:\Windows\Fonts\seguibl.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ];
    CANDIDATES
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .ok_or_else(|| anyhow::anyhow!("no font found — pass one with --font"))
}

/// The machine's own sRGB profile, for the archival output intent.
fn system_srgb_profile() -> Option<Vec<u8>> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let path = PathBuf::from(root)
        .join("System32")
        .join("spool")
        .join("drivers")
        .join("color")
        .join("sRGB Color Space Profile.icm");
    std::fs::read(path).ok()
}

/// Where the desktop app keeps the OCR models, so the two agree without being
/// told.
fn default_model_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("com.kitbox.desktop").join("models")
}

fn ocr_summary(report: &mt_core::engine::pdf::OcrReport) {
    println!(
        "  {} word{} on {} page{}",
        report.words,
        if report.words == 1 { "" } else { "s" },
        report.pages_read.len(),
        if report.pages_read.len() == 1 { "" } else { "s" }
    );
    if !report.pages_with_text.is_empty() {
        println!(
            "  already had text, left alone: {}",
            report.pages_with_text.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
        );
    }
    for warning in &report.warnings {
        println!("  note: {warning}");
    }
    println!("  {}", report.note);
}

/// `report.pdf` → `report-protected.pdf`, beside the original.
fn default_name(file: &Path, suffix: &str, ext: &str) -> PathBuf {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
    if suffix.is_empty() {
        return file.with_file_name(format!("{stem}.{ext}"));
    }
    file.with_file_name(format!("{stem}-{suffix}.{ext}"))
}

/// The repair report, laid out so what was lost is as visible as what was saved.
fn print_repair(report: &pdf::RepairReport) {
    println!("  {}", report.headline());
    for finding in &report.findings {
        println!("    found: {finding}");
    }
    for caveat in &report.caveats {
        println!("    note:  {caveat}");
    }
}

/// Write a result and tell history about it.
///
/// This is the only way a finished file should leave this program. Writing and
/// recording used to be two separate things a command had to remember to do,
/// and almost none of them did: a file made on the command line never appeared
/// in the gallery, and any lineage running through it had a hole where that
/// step should be.
///
/// `input` is what the job started from, when there is a single file to point
/// at — the first of several for a merge, and nothing at all for something
/// built from scratch.
#[allow(clippy::too_many_arguments)]
fn deliver(
    store: Option<&Store>,
    started: std::time::Instant,
    tool: &str,
    recipe: serde_json::Value,
    input: Option<(&Path, &[u8])>,
    out_path: &Path,
    out_bytes: &[u8],
    summary: String,
) -> Result<()> {
    write_out(out_path, out_bytes)?;
    if let Some(store) = store {
        record_job(
            store,
            tool,
            recipe,
            input,
            out_path,
            out_bytes,
            summary,
            started.elapsed().as_millis() as i64,
        )?;
    }
    Ok(())
}

fn write_out(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Turn `1-3,8-12` into inclusive page ranges, reusing the same selection
/// parser the rest of the tools use so the syntax can't drift.
fn parse_ranges(spec: &str, total: u32) -> Result<Vec<(u32, u32)>> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let sel = PageSelection::parse(part, total)?;
        let pages = sel.pages();
        if let (Some(first), Some(last)) = (pages.first(), pages.last()) {
            out.push((*first, *last));
        }
    }
    if out.is_empty() {
        bail!("no ranges in '{spec}'");
    }
    Ok(out)
}

fn parse_dimensions(spec: &str) -> Result<(u32, u32)> {
    let (w, h) = spec
        .split_once(['x', 'X', '*'])
        .with_context(|| format!("'{spec}' should look like 200x230"))?;
    Ok((w.trim().parse()?, h.trim().parse()?))
}

/// Per-user data folder. Keeping history beside the binary would break a
/// portable install on a read-only drive, so it lives in the user profile
/// unless overridden.
fn store_root(override_path: &Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.clone());
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .or_else(|| std::env::var_os("HOME"))
        .context("could not find a folder to keep history in")?;
    Ok(PathBuf::from(base).join("Kitbox"))
}

/// The size inspector, in text: where the bytes are and which images can
/// actually be shrunk.
fn print_pdf_breakdown(bytes: &[u8]) -> Result<()> {
    let report = pdf::inspect(bytes)?;
    println!(
        "  {} page{} · {}",
        report.pages,
        if report.pages == 1 { "" } else { "s" },
        report.kind.describe()
    );

    let rows = [
        ("images", report.image_bytes),
        ("fonts", report.font_bytes),
        ("page content", report.content_bytes),
        ("metadata", report.metadata_bytes),
        ("everything else", report.other_bytes),
    ];
    let total = report.total_bytes.max(1);
    for (label, value) in rows {
        if value == 0 {
            continue;
        }
        let pct = value as f64 / total as f64 * 100.0;
        let bar = "#".repeat(((pct / 4.0).round() as usize).min(25));
        println!("    {label:<16} {:>9}  {pct:>5.1}%  {bar}", format_size(value));
    }

    if !report.images.is_empty() {
        println!("  images:");
        let mut images = report.images.clone();
        images.sort_by_key(|i| std::cmp::Reverse(i.bytes));
        for img in images.iter().take(8) {
            let dpi =
                img.dpi.map(|d| format!("{d:.0} DPI")).unwrap_or_else(|| "DPI unknown".into());
            let note =
                img.skip_reason.as_deref().map(|r| format!("  — skipped: {r}")).unwrap_or_default();
            println!(
                "    {:>9}  {}×{}  {dpi}  {} {}{note}",
                format_size(img.bytes),
                img.width,
                img.height,
                img.color_space,
                img.filter
            );
        }
        if images.len() > 8 {
            println!("    … and {} more", images.len() - 8);
        }
    }
    Ok(())
}

/// Turn what was typed on the command line into a flat list of files.
///
/// A folder is read one level deep and nothing else: recursing into a home
/// directory because someone typed `.` is not a surprise anyone enjoys.
fn gather(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
                .with_context(|| format!("reading {}", path.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            entries.sort();
            files.extend(entries);
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            bail!("{} is not a file or a folder", path.display());
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    /// Every command that writes a file should tell history about it.
    ///
    /// This is the same guard the desktop app keeps over its own commands, and
    /// it exists because the command line failed it quietly for a long time:
    /// `compress` recorded, and the other twenty-seven commands wrote files
    /// that the gallery never heard of. A file on disk that history has no row
    /// for is a hole in the chain the gallery draws, and nobody notices until
    /// they go looking for something that should be there.
    #[test]
    fn every_command_that_writes_a_file_records_it() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("own source");

        // Protect and unlock are deliberately excluded, as they are in the
        // app: keeping a copy of a document someone just encrypted, or the
        // open version of one they unlocked, is a privacy problem rather than
        // a feature.
        let exempt = ["Protect", "Unlock"];

        for block in source.split("        Command::").skip(1) {
            let name: String = block.chars().take_while(|c| c.is_alphanumeric()).collect();
            let body = block.split("\n        Command::").next().unwrap_or("");

            let writes = body.contains("write_out(") || body.contains("std::fs::write(");
            if !writes || exempt.contains(&name.as_str()) {
                continue;
            }
            let records = body.contains("deliver(")
                || body.contains("record(")
                || body.contains("record_job(")
                || body.contains("for_each_image");
            assert!(records, "'{name}' writes a file but never records it in history");
        }
    }

    /// The writing and the recording are one call, so they cannot drift.
    #[test]
    fn delivering_is_the_way_a_file_leaves() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("own source");
        let deliver = source.split("fn deliver(").nth(1).expect("deliver still exists");
        let body = deliver.split("\nfn ").next().unwrap_or("");
        assert!(body.contains("write_out("), "deliver has to write the file");
        assert!(body.contains("record_job("), "deliver has to record it");
    }
}
