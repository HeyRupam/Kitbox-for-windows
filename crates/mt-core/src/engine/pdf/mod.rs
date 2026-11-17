//! PDF tools.
//!
//! Compression here is staged rather than one-shot, because "make this PDF
//! smaller" means completely different work depending on what the PDF is:
//!
//! 0. **Inspect** — attribute every byte, and work out each image's real DPI.
//! 1. **Lossless** — repack streams, drop metadata and orphans, dedupe
//!    identical images. Nothing visible changes. Often enough on its own.
//! 2. **Downsample** — re-encode embedded images at a lower resolution and
//!    quality. Text stays text and stays selectable.
//!
//! A text-only document that is still too big is reported honestly instead of
//! being flattened into pictures of words behind the user's back.

pub mod annotate;
pub mod archive;
pub mod build;
pub mod compare;
pub mod compress;
pub mod crop;
pub mod forms;
pub mod images;
pub mod inspect;
pub mod lossless;
pub mod markdown;
pub mod ocr;
pub mod overlay;
pub mod pagenumbers;
pub mod pages;
pub mod redact;
pub mod repair;
pub mod security;
pub mod sign;
pub mod watermark;

pub use build::{images_to_pdf, ImagesToPdfOptions, Orientation, PageSize};
pub use compress::{compress_to_target, PdfCompressOptions, PdfCompressor, PdfKnob};
pub use inspect::{inspect, DocKind, ImageInfo, PdfReport};
pub use lossless::{lossless_pass, LosslessStats};
pub use markdown::{to_markdown, MarkdownOptions, MarkdownReport};
pub use compare::{compare, Change, ChangeKind, CompareReport};
pub use crop::{crop, CropReport, CropSpec};
pub use pagenumbers::{add_page_numbers, NumberPosition, PageNumberOptions};
pub use watermark::{
    watermark, Mark, Placement, WatermarkOptions,
};
pub use sign::{
    sign, Caption, SignOptions, SignReport, SignatureSpot, NOT_CRYPTOGRAPHIC,
};
pub use annotate::{
    annotate, AnnotateOptions, AnnotateReport, Annotation, Mark as AnnotationMark,
    PART_OF_THE_PAGE, WHITEOUT_COVERS,
};
pub use archive::{
    check as check_archive, to_archive, ArchiveOptions, ArchiveReport, Blocker, Part,
    NOT_A_CERTIFICATE,
};
pub use forms::{
    fields, fill, flatten, Field, FieldKind, FillOptions, FillReport, FlattenReport, NOT_A_FORM,
};
pub use ocr::{
    read as ocr_read, searchable, Models as OcrModels, OcrOptions, OcrReport, PageText, A_GUESS,
};
pub use redact::{
    redact, RedactOptions, RedactReport, Target as RedactTarget, VECTOR_CAVEAT,
};
pub use repair::{diagnose, repair, RepairReport, Severity};
pub use security::{
    inspect_security, protect, unlock, Protection, SecurityInfo, Strength, UnlockKind,
};
pub use pages::{
    export_pages, extract, merge, page_count, page_preview, page_previews, page_summaries, rebuild, rebuild_ops, remove,
    reorder, rotate, split, PageOp, ExportedPage, PageInfo, PageOpReport, PageSelection, SplitMode, SplitPart,
};
