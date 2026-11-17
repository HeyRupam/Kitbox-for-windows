//! Image tools.

pub mod adjust;
pub mod compress;
pub mod cutout;
pub mod faces;
pub mod meme;
pub mod encode;
pub mod scan;
pub mod signature;
pub mod transform;
pub mod upscale;
pub mod watermark;

pub use compress::{
    compress_to_target, CompressOptions, ImageCompressor, ImageKnob, OutputFormat,
};
pub use signature::{
    extract as extract_signature, InkColour, SignatureOptions, SignatureReport,
};
pub use adjust::{adjust, apply as adjust_image, AdjustOptions, AdjustReport};
pub use cutout::{
    cut, remove_background, Backdrop, CutoutOptions, CutoutReport, Model as CutoutModel,
};
pub use upscale::{
    enlarge, estimate_seconds, upscale, Method as UpscaleMethod, Model as UpscaleModel,
    UpscaleOptions, UpscaleReport, INVENTED, MODEL_PIXEL_LIMIT,
};
pub use faces::{
    blur_faces, find as find_faces, BlurOptions, BlurReport, Cover, Face,
    Model as FaceModel, NOT_A_GUARANTEE,
};
pub use meme::{draw as draw_meme, meme, MemeOptions, MemeReport};
pub use scan::{scan, scan_image, scan_to_pdf, ScanMode, ScanOptions, ScanReport, ScanResult};
pub use watermark::{ImageMark, ImageWatermarkOptions};
pub use transform::{
    convert, crop, crop_to_aspect, resize, resized_dimensions, rotate, Flip, Rect, ResizeMode,
    TransformOptions,
};
