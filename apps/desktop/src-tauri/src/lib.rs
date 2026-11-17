//! The desktop shell.
//!
//! Thin on purpose: it owns the window, the paths and the history database, and
//! forwards everything else to `mt-core`. Nothing here knows how compression
//! works, which is what keeps the same engine usable on Android later.

use std::path::{Path, PathBuf};
use std::time::Instant;
use std::sync::Mutex;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use mt_core::engine::image::{CompressOptions, ImageCompressor, OutputFormat};
use mt_core::engine::pdf::{PageOp, PdfCompressOptions, PdfCompressor, PdfReport, SplitMode};
use mt_core::size::format_size;
use mt_core::{solve, Bias, FileKind, Outcome, SizeTarget};
use mt_store::{FileMeta, NewJob, Store};

struct AppState {
    store: Mutex<Store>,
    /// Where finished files land, as the design says: Pictures / Kitbox.
    output_dir: PathBuf,
    /// Where the OCR models live once fetched.
    model_dir: PathBuf,
    /// Where form presets are kept, as files you can edit.
    preset_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Exact {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressRequest {
    bytes: Vec<u8>,
    name: String,
    target: SizeTarget,
    bias: Bias,
    format: OutputFormat,
    strip_metadata: bool,
    #[serde(default)]
    max_dimension: Option<u32>,
    #[serde(default)]
    exact: Option<Exact>,
    /// PDF only: let the solver drop colour as a last resort.
    #[serde(default)]
    allow_grayscale: bool,
    /// PDF only: resolution floor.
    #[serde(default)]
    min_dpi: Option<u16>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompressResponse {
    status: String,
    name: String,
    original_size: u64,
    size: u64,
    knob: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f32>,
    probes: u8,
    elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
    /// Present for PDFs: the size inspector's breakdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<ReportDto>,
}

/// The size inspector, shaped for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportDto {
    pages: usize,
    total_bytes: u64,
    image_bytes: u64,
    font_bytes: u64,
    content_bytes: u64,
    metadata_bytes: u64,
    other_bytes: u64,
    kind: String,
    headline: String,
    images: Vec<ReportImageDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportImageDto {
    width: u32,
    height: u32,
    bytes: u64,
    dpi: Option<f32>,
    color_space: String,
    filter: String,
    recompressible: bool,
    skip_reason: Option<String>,
}

impl From<&PdfReport> for ReportDto {
    fn from(r: &PdfReport) -> Self {
        let mut images: Vec<ReportImageDto> = r
            .images
            .iter()
            .map(|i| ReportImageDto {
                width: i.width,
                height: i.height,
                bytes: i.bytes,
                dpi: i.dpi,
                color_space: i.color_space.clone(),
                filter: i.filter.clone(),
                recompressible: i.recompressible,
                skip_reason: i.skip_reason.clone(),
            })
            .collect();
        images.sort_by_key(|i| std::cmp::Reverse(i.bytes));
        images.truncate(12);
        ReportDto {
            pages: r.pages,
            total_bytes: r.total_bytes,
            image_bytes: r.image_bytes,
            font_bytes: r.font_bytes,
            content_bytes: r.content_bytes,
            metadata_bytes: r.metadata_bytes,
            other_bytes: r.other_bytes,
            kind: match r.kind {
                mt_core::engine::pdf::DocKind::Scan => "scan",
                mt_core::engine::pdf::DocKind::TextWithImages => "text_with_images",
                mt_core::engine::pdf::DocKind::TextOnly => "text_only",
            }
            .into(),
            headline: r.headline(),
            images,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    hash: String,
    name: String,
    kind: String,
    size: u64,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    pinned: bool,
    /// Which tool made it.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pages: Option<u32>,
    /// Fetch the picture itself with `history_thumbnail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    thumb_hash: Option<String>,
    /// Whether this job can be run again on a new file, and what to call it.
    #[serde(skip_serializing_if = "Option::is_none")]
    replay: Option<recipe::Replayable>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageResponse {
    files: u64,
    bytes: u64,
    jobs: u64,
}

/// Read a PDF's byte breakdown without changing anything, so the inspector can
/// show what you're up against before you press the button.
#[tauri::command]
async fn inspect_pdf(bytes: Vec<u8>) -> Result<ReportDto, String> {
    let report = mt_core::engine::pdf::inspect(&bytes).map_err(|e| e.to_string())?;
    Ok(ReportDto::from(&report))
}

#[tauri::command]
async fn compress_file(
    state: State<'_, AppState>,
    request: CompressRequest,
) -> Result<CompressResponse, String> {
    let original_size = request.bytes.len() as u64;
    let is_pdf = FileKind::sniff(&request.bytes) == FileKind::Pdf;

    let (output_kind, outcome, report) = if is_pdf {
        let opts = PdfCompressOptions {
            strip_metadata: request.strip_metadata,
            allow_grayscale: request.allow_grayscale,
            min_dpi: request.min_dpi.unwrap_or(50),
        };
        let compressor = PdfCompressor::new(&request.bytes, opts).map_err(|e| e.to_string())?;
        let report = ReportDto::from(compressor.report());
        let outcome =
            solve(&compressor, request.target, request.bias).map_err(|e| e.to_string())?;
        (FileKind::Pdf, outcome, Some(report))
    } else {
        let opts = CompressOptions {
            format: request.format,
            strip_metadata: request.strip_metadata,
            max_dimension: request.max_dimension,
            exact_dimensions: request.exact.as_ref().map(|e| (e.width, e.height)),
        };
        let compressor = ImageCompressor::new(&request.bytes, opts).map_err(|e| e.to_string())?;
        let kind = compressor.output_kind();
        let outcome =
            solve(&compressor, request.target, request.bias).map_err(|e| e.to_string())?;
        (kind, outcome, None)
    };

    let (solved, status) = match outcome {
        Outcome::Infeasible { best_size, why, suggestions, probes, elapsed_ms, .. } => {
            return Ok(CompressResponse {
                status: "infeasible".into(),
                name: request.name,
                original_size,
                size: best_size,
                knob: String::new(),
                probes,
                elapsed_ms,
                why: Some(why),
                suggestions: Some(suggestions),
                report,
                ..Default::default()
            });
        }
        Outcome::Hit(solved) => (solved, "hit"),
        Outcome::Padded(solved) => (solved, "padded"),
    };

    let out_path = unique_path(&state.output_dir, &request.name, output_kind.extension())
        .map_err(|e| e.to_string())?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&out_path, &solved.bytes).map_err(|e| e.to_string())?;

    let hash = remember(
        &state,
        Recorded {
            tool: if is_pdf { "compress-pdf" } else { "compress-image" },
            recipe: serde_json::json!({
                "tool": "compress",
                "target": request.target,
                "bias": request.bias,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(out_path.clone(), solved.bytes.as_slice())],
            summary: format!(
                "{} → {}, {}",
                format_size(original_size),
                format_size(solved.size),
                solved.knob
            ),
            duration_ms: solved.elapsed_ms as i64,
        },
    )
    .ok()
    .and_then(|hashes| hashes.into_iter().next());

    Ok(CompressResponse {
        status: status.into(),
        name: request.name,
        original_size,
        size: solved.size,
        knob: solved.knob.clone(),
        score: solved.score,
        probes: solved.probes,
        elapsed_ms: solved.elapsed_ms,
        note: solved.note.clone(),
        // A base64 PDF is no use to an <img>, and the inspector is the better
        // before/after for a document anyway.
        preview_url: (!is_pdf).then(|| data_url(output_kind, &solved.bytes)),
        output_path: Some(out_path.display().to_string()),
        hash,
        report,
        why: None,
        suggestions: None,
    })
}

/* ---------- page tools ---------- */

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageOpResponse {
    output_path: String,
    pages_before: u32,
    pages_after: u32,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyPagesRequest {
    bytes: Vec<u8>,
    name: String,
    /// The finished document, page by page.
    ops: Vec<PageOp>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SplitRequest {
    bytes: Vec<u8>,
    name: String,
    mode: SplitMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeRequest {
    files: Vec<MergeInput>,
    /// Name for the combined document.
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeInput {
    bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SplitPartResponse {
    output_path: String,
    pages: Vec<u32>,
    size: u64,
}

#[tauri::command]
async fn pdf_pages(bytes: Vec<u8>) -> Result<Vec<mt_core::engine::pdf::PageInfo>, String> {
    mt_core::engine::pdf::page_summaries(&bytes).map_err(|e| e.to_string())
}

/// Thumbnails as data URLs, `null` for pages that aren't a single image.
#[tauri::command]
async fn pdf_previews(bytes: Vec<u8>, max_px: u32) -> Result<Vec<Option<String>>, String> {
    let previews =
        mt_core::engine::pdf::pages::page_previews(&bytes, max_px).map_err(|e| e.to_string())?;
    Ok(previews
        .into_iter()
        .map(|p| p.map(|jpeg| data_url(FileKind::Jpeg, &jpeg)))
        .collect())
}

#[tauri::command]
async fn apply_pages(
    state: State<'_, AppState>,
    request: ApplyPagesRequest,
) -> Result<PageOpResponse, String> {
    let started = Instant::now();
    let (bytes, report) = mt_core::engine::pdf::rebuild_ops(&request.bytes, &request.ops)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "pages",
            recipe: serde_json::json!({ "tool": "pages", "ops": request.ops }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{} → {} pages, {}",
                report.pages_before,
                report.pages_after,
                format_size(bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(PageOpResponse {
        output_path: path.display().to_string(),
        pages_before: report.pages_before,
        pages_after: report.pages_after,
        size: bytes.len() as u64,
        note: report.note(),
    })
}

#[tauri::command]
async fn split_pdf(
    state: State<'_, AppState>,
    request: SplitRequest,
) -> Result<Vec<SplitPartResponse>, String> {
    let started = Instant::now();
    let parts =
        mt_core::engine::pdf::split(&request.bytes, &request.mode).map_err(|e| e.to_string())?;
    let stem = Path::new(&request.name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string();
    let mut out = Vec::new();
    let mut written: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for part in parts {
        let name = format!("{stem}-{}.pdf", part.suffix);
        let path = write_output(&state, &name, "pdf", &part.bytes)?;
        out.push(SplitPartResponse {
            output_path: path.display().to_string(),
            pages: part.pages,
            size: part.bytes.len() as u64,
        });
        written.push((path, part.bytes));
    }

    // One job row per piece, so every file that landed on disk is findable.
    let _ = remember(
        &state,
        Recorded {
            tool: "split",
            recipe: serde_json::json!({ "tool": "split", "mode": request.mode }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: written.iter().map(|(p, b)| (p.clone(), b.as_slice())).collect(),
            summary: format!("split into {} files", written.len()),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(out)
}

#[tauri::command]
async fn merge_pdfs(
    state: State<'_, AppState>,
    request: MergeRequest,
) -> Result<PageOpResponse, String> {
    let started = Instant::now();
    let inputs: Vec<Vec<u8>> = request.files.into_iter().map(|f| f.bytes).collect();
    let (bytes, report) = mt_core::engine::pdf::merge(&inputs).map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "merge",
            recipe: serde_json::json!({ "tool": "merge", "files": inputs.len() }),
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, b)| (format!("part-{}.pdf", i + 1), b.as_slice()))
                .collect(),
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{} files → {} pages, {}",
                inputs.len(),
                report.pages_after,
                format_size(bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(PageOpResponse {
        output_path: path.display().to_string(),
        pages_before: report.pages_before,
        pages_after: report.pages_after,
        size: bytes.len() as u64,
        note: report.note(),
    })
}

/* ---------- conversion and image transforms ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToPdfRequest {
    images: Vec<MergeInput>,
    name: String,
    options: mt_core::engine::pdf::ImagesToPdfOptions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanRequest {
    images: Vec<MergeInput>,
    name: String,
    options: mt_core::engine::image::ScanOptions,
    pdf: mt_core::engine::pdf::ImagesToPdfOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanResponse {
    output_path: String,
    pages: u32,
    flattened: u32,
    whole_frame: Vec<u32>,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanPreview {
    image: String,
    edges_found: bool,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignaturePreview {
    image: String,
    found_ink: bool,
    width: u32,
    height: u32,
    coverage: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSignatureRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::image::SignatureOptions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignRequest {
    bytes: Vec<u8>,
    name: String,
    /// PNG bytes of the signature, already cut out.
    signature: Vec<u8>,
    options: mt_core::engine::pdf::SignOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignResponse {
    output_path: String,
    pages_signed: Vec<u32>,
    placements: u32,
    size: u64,
    note: String,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RedactRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::pdf::RedactOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RedactResponse {
    output_path: String,
    pages_changed: Vec<u32>,
    glyphs_removed: u32,
    images_edited: u32,
    annotations_removed: u32,
    verified: bool,
    leaks: Vec<String>,
    warnings: Vec<String>,
    note: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FillFormRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::pdf::FillOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FillFormResponse {
    output_path: String,
    filled: Vec<String>,
    unknown: Vec<String>,
    read_only: Vec<String>,
    still_required: Vec<String>,
    flattened: bool,
    warnings: Vec<String>,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnnotateRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::pdf::AnnotateOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnnotateResponse {
    output_path: String,
    drawn: u32,
    pages_changed: Vec<u32>,
    warnings: Vec<String>,
    note: String,
    size: u64,
}

/// Where the OCR models come from, and how big they are.
///
/// Stated in one place so the screen, the consent prompt and the download all
/// agree about what is being fetched.
/// One downloadable file: where it goes, where it comes from, how big it is.
type ModelFile = (&'static str, &'static str, u64);

/// A tool's models, named so one download screen can serve every tool that
/// needs one.
struct ModelSet {
    id: &'static str,
    files: &'static [ModelFile],
}

/// Everything Kitbox can fetch, in one place.
///
/// These are the only two things in the app that touch the network, so they
/// are listed together rather than buried in the tool that wants them — the
/// size and the source are what the user is asked to agree to, and both come
/// from here.
const MODEL_SETS: [ModelSet; 4] = [
    ModelSet {
        id: "ocr",
        files: &[
            (
                "text-detection.onnx",
                "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
                2_499_479,
            ),
            (
                "text-recognition.onnx",
                "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
                9_713_177,
            ),
        ],
    },
    ModelSet {
        // U^2-Net, Apache 2.0. The small variant: 4.6 MB against 176 for the
        // full one, and it holds up on a subject against a background, which
        // is what this tool is for.
        id: "cutout",
        files: &[(
            "u2netp.onnx",
            "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
            4_574_861,
        )],
    },
    ModelSet {
        // UltraFace RFB-320, MIT. Smallest of the lot, and the fastest:
        // milliseconds a picture.
        id: "faces",
        files: &[(
            "ultraface-320.onnx",
            "https://github.com/onnx/models/raw/main/validated/vision/body_analysis/ultraface/models/version-RFB-320.onnx",
            1_270_727,
        )],
    },
    ModelSet {
        // Real-ESRGAN x4. Much the largest download here, and only needed for
        // the reconstructing half of Upscale — the resampling half works with
        // nothing fetched at all.
        id: "upscale",
        files: &[(
            "realesrgan-x4.onnx",
            "https://huggingface.co/baker76/upscale-models/resolve/main/RealESRGAN_x4.onnx",
            69_879_921,
        )],
    },
];

fn model_set(id: &str) -> Result<&'static ModelSet, String> {
    MODEL_SETS
        .iter()
        .find(|set| set.id == id)
        .ok_or_else(|| format!("there is no model set called '{id}'"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelStatus {
    ready: bool,
    /// Total download in bytes, for telling the user before asking.
    bytes: u64,
    folder: String,
    sources: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcrRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::pdf::OcrOptions,
    /// Write a searchable copy rather than just handing back the text.
    searchable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    pages: Vec<mt_core::engine::pdf::PageText>,
    pages_read: Vec<u32>,
    pages_with_text: Vec<u32>,
    pages_not_readable: Vec<u32>,
    words: u32,
    note: String,
    warnings: Vec<String>,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HtmlRequest {
    source: html::Source,
    name: String,
    setup: html::PageSetup,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HtmlResponse {
    output_path: String,
    pages: u32,
    source: String,
    size: u64,
    warnings: Vec<String>,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CutoutRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::image::CutoutOptions,
    /// Hand the picture straight back instead of saving it.
    preview: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CutoutResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    image: String,
    width: u32,
    height: u32,
    coverage: f32,
    size: u64,
    note: String,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpscaleRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::image::UpscaleOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpscaleResponse {
    output_path: String,
    from: (u32, u32),
    to: (u32, u32),
    tiles: u32,
    seconds: f32,
    size: u64,
    preview_url: String,
    note: String,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FacesRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::image::BlurOptions,
    /// Hand the picture back instead of saving it.
    preview: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FacesResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    image: String,
    faces: Vec<mt_core::engine::image::Face>,
    covered: u32,
    size: u64,
    note: String,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShotRequest {
    source: html::Source,
    name: String,
    setup: html::ShotSetup,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShotResponse {
    output_path: String,
    width: u32,
    height: u32,
    source: String,
    size: u64,
    preview_url: String,
    warnings: Vec<String>,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdjustRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::image::AdjustOptions,
    /// Hand the picture back instead of saving it, for the live preview.
    preview: bool,
    /// Longest side to work at when previewing. Full size when saving.
    max_px: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdjustResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    image: String,
    width: u32,
    height: u32,
    size: u64,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveRequest {
    bytes: Vec<u8>,
    name: String,
    part: mt_core::engine::pdf::Part,
    title: Option<String>,
    author: Option<String>,
    /// Report on the document without writing anything.
    check_only: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    conformant: bool,
    part: String,
    applied: Vec<String>,
    blockers: Vec<mt_core::engine::pdf::Blocker>,
    note: String,
    size: u64,
    /// Whether a colour profile was found on this machine to embed.
    profile_found: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemeRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::image::MemeOptions,
    preview: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemeResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    image: String,
    width: u32,
    height: u32,
    top_lines: u32,
    bottom_lines: u32,
    size: u64,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckRequest {
    bytes: Vec<u8>,
    preset: mt_core::preset::Preset,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixRequest {
    bytes: Vec<u8>,
    name: String,
    preset: mt_core::preset::Preset,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FixResponse {
    size: u64,
    report: mt_core::preset::Report,
    steps: Vec<String>,
    shortfall: Option<String>,
    output_path: String,
    /// So the screen can show what it produced without reading the file back.
    preview: Option<String>,
}

/// What one file came to, sent back as soon as that file is done.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchStep {
    outcome: mt_core::batch::ItemOutcome,
    /// Where it was written, when something was written.
    path: Option<String>,
    /// How many files have been answered for so far.
    done: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchResponse {
    outcomes: Vec<mt_core::batch::ItemOutcome>,
    /// Where each produced file went, in the order they were produced.
    paths: Vec<String>,
    done: u32,
    failed: u32,
    short: u32,
    cancelled: bool,
    folder: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToImagesRequest {
    bytes: Vec<u8>,
    name: String,
    #[serde(default)]
    max_px: Option<u32>,
    quality: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedPageResponse {
    page: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    width: u32,
    height: u32,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// The image transforms, shaped so one command covers four tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum TransformOp {
    Resize(mt_core::engine::image::ResizeMode),
    Crop(mt_core::engine::image::Rect),
    CropAspect(f32),
    Rotate { degrees: i32, flip: mt_core::engine::image::Flip },
    Convert,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransformRequest {
    bytes: Vec<u8>,
    name: String,
    op: TransformOp,
    options: mt_core::engine::image::TransformOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransformResponse {
    output_path: String,
    size: u64,
    width: u32,
    height: u32,
    preview_url: String,
}

#[tauri::command]
async fn images_to_pdf(
    state: State<'_, AppState>,
    request: ToPdfRequest,
) -> Result<PageOpResponse, String> {
    let started = Instant::now();
    let inputs: Vec<Vec<u8>> = request.images.into_iter().map(|f| f.bytes).collect();
    let pages = inputs.len() as u32;
    let bytes = mt_core::engine::pdf::images_to_pdf(&inputs, &request.options)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let before: u64 = inputs.iter().map(|i| i.len() as u64).sum();
    let _ = remember(
        &state,
        Recorded {
            tool: "images-to-pdf",
            recipe: serde_json::json!({ "tool": "images-to-pdf", "options": request.options }),
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    (format!("image-{}.{}", i + 1, FileKind::sniff(b).extension()), b.as_slice())
                })
                .collect(),
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{} images → {pages} pages, {} → {}",
                inputs.len(),
                format_size(before),
                format_size(bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(PageOpResponse {
        output_path: path.display().to_string(),
        pages_before: pages,
        pages_after: pages,
        size: bytes.len() as u64,
        note: None,
    })
}

#[tauri::command]
async fn pdf_to_images(
    state: State<'_, AppState>,
    request: ToImagesRequest,
) -> Result<Vec<ExportedPageResponse>, String> {
    let started = Instant::now();
    let exported =
        mt_core::engine::pdf::export_pages(&request.bytes, request.max_px, request.quality)
            .map_err(|e| e.to_string())?;
    let stem = Path::new(&request.name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string();

    let mut out = Vec::new();
    let mut written: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for page in exported {
        match page.jpeg {
            Some(jpeg) => {
                let name = format!("{stem}-page-{}.jpg", page.page);
                let path = write_output(&state, &name, "jpg", &jpeg)?;
                out.push(ExportedPageResponse {
                    page: page.page,
                    output_path: Some(path.display().to_string()),
                    width: page.width,
                    height: page.height,
                    size: jpeg.len() as u64,
                    reason: None,
                });
                written.push((path, jpeg));
            }
            None => out.push(ExportedPageResponse {
                page: page.page,
                output_path: None,
                width: 0,
                height: 0,
                size: 0,
                reason: page.reason,
            }),
        }
    }

    if !written.is_empty() {
        let _ = remember(
            &state,
            Recorded {
                tool: "pdf-to-images",
                recipe: serde_json::json!({
                    "tool": "pdf-to-images",
                    "maxPx": request.max_px,
                    "quality": request.quality,
                }),
                inputs: vec![(request.name.clone(), request.bytes.as_slice())],
                outputs: written.iter().map(|(p, b)| (p.clone(), b.as_slice())).collect(),
                summary: format!("exported {} pages as JPG", written.len()),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
    }

    Ok(out)
}

#[tauri::command]
async fn transform_image(
    state: State<'_, AppState>,
    request: TransformRequest,
) -> Result<TransformResponse, String> {
    use mt_core::engine::image as img;

    let started = Instant::now();
    let opts = request.options;
    let output = match &request.op {
        TransformOp::Resize(mode) => img::resize(&request.bytes, *mode, &opts),
        TransformOp::Crop(rect) => img::crop(&request.bytes, *rect, &opts),
        TransformOp::CropAspect(aspect) => img::crop_to_aspect(&request.bytes, *aspect, &opts),
        TransformOp::Rotate { degrees, flip } => {
            img::rotate(&request.bytes, *degrees, *flip, &opts)
        }
        TransformOp::Convert => img::convert(&request.bytes, &opts),
    }
    .map_err(|e| e.to_string())?;

    let kind = FileKind::sniff(&output);
    let (width, height) = image::load_from_memory(&output)
        .map(|i| (i.width(), i.height()))
        .unwrap_or((0, 0));
    let path = write_output(&state, &request.name, kind.extension(), &output)?;

    let _ = remember(
        &state,
        Recorded {
            tool: transform_name(&request.op),
            recipe: serde_json::json!({ "tool": "transform", "op": request.op.clone() }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), output.as_slice())],
            summary: format!(
                "{} → {width}×{height}, {}",
                format_size(request.bytes.len() as u64),
                format_size(output.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(TransformResponse {
        output_path: path.display().to_string(),
        size: output.len() as u64,
        width,
        height,
        preview_url: data_url(kind, &output),
    })
}

/* ---------- watermark ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatermarkRequest {
    bytes: Vec<u8>,
    name: String,
    /// Page selection like `1-3,7`. Absent means every page.
    #[serde(default)]
    pages: Option<String>,
    options: mt_core::engine::pdf::WatermarkOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatermarkResponse {
    output_path: String,
    size: u64,
    pages_marked: u32,
    total_pages: u32,
}

#[tauri::command]
async fn watermark_pdf(
    state: State<'_, AppState>,
    request: WatermarkRequest,
) -> Result<WatermarkResponse, String> {
    let started = Instant::now();
    let total = mt_core::engine::pdf::page_count(&request.bytes).map_err(|e| e.to_string())?;

    let selection = match request.pages.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(spec) => Some(
            mt_core::engine::pdf::PageSelection::parse(spec, total).map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let marked = selection.as_ref().map(|s| s.pages().len() as u32).unwrap_or(total);

    let output = mt_core::engine::pdf::watermark(&request.bytes, selection.as_ref(), &request.options)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &output)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "watermark",
            recipe: serde_json::json!({
                "tool": "watermark",
                "pages": request.pages,
                "options": request.options,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), output.as_slice())],
            summary: format!("watermarked {marked} of {total} pages"),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(WatermarkResponse {
        output_path: path.display().to_string(),
        size: output.len() as u64,
        pages_marked: marked,
        total_pages: total,
    })
}

/* ---------- compare ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompareRequest {
    before: Vec<u8>,
    after: Vec<u8>,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompareResponse {
    #[serde(flatten)]
    report: mt_core::engine::pdf::CompareReport,
    /// The same thing as Markdown, for reading and saving.
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
}

#[tauri::command]
async fn compare_pdfs(
    state: State<'_, AppState>,
    request: CompareRequest,
) -> Result<CompareResponse, String> {
    let started = Instant::now();
    let report = mt_core::engine::pdf::compare(&request.before, &request.after)
        .map_err(|e| e.to_string())?;
    let summary = report.summary();

    // Two identical documents produce a one-line report; writing a file for
    // that is clutter rather than a result.
    let output_path = if report.identical {
        None
    } else {
        let path = write_output(&state, &request.name, "md", summary.as_bytes())?;
        let _ = remember(
            &state,
            Recorded {
                tool: "compare",
                recipe: serde_json::json!({ "tool": "compare" }),
                inputs: vec![
                    ("before.pdf".to_string(), request.before.as_slice()),
                    ("after.pdf".to_string(), request.after.as_slice()),
                ],
                outputs: vec![(path.clone(), summary.as_bytes())],
                summary: format!(
                    "{} lines added, {} removed",
                    report.lines_added, report.lines_removed
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
        Some(path.display().to_string())
    };

    Ok(CompareResponse { report, summary, output_path })
}

/* ---------- markdown ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkdownRequest {
    bytes: Vec<u8>,
    name: String,
    options: mt_core::engine::pdf::MarkdownOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarkdownResponse {
    markdown: String,
    #[serde(flatten)]
    report: mt_core::engine::pdf::MarkdownReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
}

/// Clean up one photograph and hand it straight back, for the live preview.
///
/// Nothing is written and nothing is recorded: this runs on every change to a
/// setting, and history is for files the user chose to keep.
#[tauri::command]
async fn scan_preview(
    bytes: Vec<u8>,
    options: mt_core::engine::image::ScanOptions,
    max_px: u32,
) -> Result<ScanPreview, String> {
    let (image, result) =
        mt_core::engine::image::scan(&bytes, &options).map_err(|e| e.to_string())?;
    // Downscale for the screen only after the real work, so what is shown is
    // what the full-size page will look like.
    let shown = if image.width().max(image.height()) > max_px && max_px > 0 {
        image.resize(max_px, max_px, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let jpeg = mt_core::engine::image::encode::encode(&shown, FileKind::Jpeg, 82, true)
        .map_err(|e| e.to_string())?;
    Ok(ScanPreview {
        image: data_url(FileKind::Jpeg, &jpeg),
        edges_found: result.edges_found,
        width: result.width,
        height: result.height,
    })
}

#[tauri::command]
async fn scan_to_pdf(
    state: State<'_, AppState>,
    request: ScanRequest,
) -> Result<ScanResponse, String> {
    let started = Instant::now();
    let inputs: Vec<Vec<u8>> = request.images.into_iter().map(|f| f.bytes).collect();
    let (bytes, report) =
        mt_core::engine::image::scan_to_pdf(&inputs, &request.options, &request.pdf)
            .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "scan-to-pdf",
            recipe: serde_json::json!({
                "tool": "scan-to-pdf",
                "options": request.options,
                "pdf": request.pdf,
            }),
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    (format!("photo-{}.{}", i + 1, FileKind::sniff(b).extension()), b.as_slice())
                })
                .collect(),
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{} photographs → {} pages, {} flattened, {}",
                report.pages,
                report.pages,
                report.flattened,
                format_size(bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(ScanResponse {
        output_path: path.display().to_string(),
        pages: report.pages,
        flattened: report.flattened,
        whole_frame: report.whole_frame,
        size: bytes.len() as u64,
        note: report.note,
    })
}

/// Cut a signature out of a photograph and hand it back for the preview.
///
/// Nothing is written: the strength and ink settings are meant to be fiddled
/// with, and history is for the one the user settles on.
#[tauri::command]
async fn signature_preview(
    bytes: Vec<u8>,
    options: mt_core::engine::image::SignatureOptions,
) -> Result<SignaturePreview, String> {
    let (image, report) = mt_core::engine::image::extract_signature(&bytes, &options)
        .map_err(|e| e.to_string())?;
    // PNG, because the whole point is the transparency.
    let png = mt_core::engine::image::encode::encode(&image, FileKind::Png, 100, true)
        .map_err(|e| e.to_string())?;
    Ok(SignaturePreview {
        image: data_url(FileKind::Png, &png),
        found_ink: report.found_ink,
        width: report.width,
        height: report.height,
        coverage: report.coverage,
        note: report.note,
    })
}

/// Keep a cut-out signature, so it never has to be cut out again.
#[tauri::command]
async fn save_signature(
    state: State<'_, AppState>,
    request: SaveSignatureRequest,
) -> Result<TransformResponse, String> {
    let started = Instant::now();
    let (image, report) = mt_core::engine::image::extract_signature(&request.bytes, &request.options)
        .map_err(|e| e.to_string())?;
    if !report.found_ink {
        // A fully transparent PNG saved to the gallery would look like success
        // and be useless later.
        return Err(report
            .note
            .unwrap_or_else(|| "no pen strokes were found in that photograph".into()));
    }
    let png = mt_core::engine::image::encode::encode(&image, FileKind::Png, 100, true)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "png", &png)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "signature",
            recipe: serde_json::json!({ "tool": "signature", "options": request.options }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), png.as_slice())],
            summary: format!(
                "signature cut out, {}×{}, {}",
                report.width,
                report.height,
                format_size(png.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(TransformResponse {
        output_path: path.display().to_string(),
        width: report.width,
        height: report.height,
        size: png.len() as u64,
        // The saved signature is shown straight back, so the screen can use it
        // without reading the file it just wrote.
        preview_url: data_url(FileKind::Png, &png),
    })
}

#[tauri::command]
async fn sign_pdf(
    state: State<'_, AppState>,
    request: SignRequest,
) -> Result<SignResponse, String> {
    let started = Instant::now();
    let (bytes, report) =
        mt_core::engine::pdf::sign(&request.bytes, &request.signature, &request.options)
            .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "sign",
            // The signature image itself stays out of the recipe: it would
            // bloat every history row, and it is a signature.
            recipe: serde_json::json!({ "tool": "sign", "spots": request.options.spots }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "signed on {} page{}, {}",
                report.pages_signed.len(),
                if report.pages_signed.len() == 1 { "" } else { "s" },
                format_size(bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(SignResponse {
        output_path: path.display().to_string(),
        pages_signed: report.pages_signed,
        placements: report.placements,
        size: bytes.len() as u64,
        note: report.note,
        warnings: report.warnings,
    })
}

/// Delete the targeted content and write the result.
///
/// The result is written whatever the verification says, because a half-done
/// redaction is still evidence of what was attempted and the user needs to see
/// it. But `verified` travels with it, and the screen leads with that.
#[tauri::command]
async fn redact_pdf(
    state: State<'_, AppState>,
    request: RedactRequest,
) -> Result<RedactResponse, String> {
    let started = Instant::now();
    let (bytes, report) = mt_core::engine::pdf::redact(&request.bytes, &request.options)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "redact",
            // The recipe records that text was redacted, never which text:
            // writing the secret into the history database would undo the
            // entire point of the tool.
            recipe: serde_json::json!({
                "tool": "redact",
                "targets": request.options.targets.len(),
                "drawBoxes": request.options.draw_boxes,
                "stripExtras": request.options.strip_extras,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "redacted on {} page{}, {}",
                report.pages_changed.len(),
                if report.pages_changed.len() == 1 { "" } else { "s" },
                if report.verified { "verified" } else { "NOT verified" }
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(RedactResponse {
        output_path: path.display().to_string(),
        pages_changed: report.pages_changed,
        glyphs_removed: report.glyphs_removed,
        images_edited: report.images_edited,
        annotations_removed: report.annotations_removed,
        verified: report.verified,
        leaks: report.leaks,
        warnings: report.warnings,
        note: report.note,
        size: bytes.len() as u64,
    })
}

/// What this form asks for, so the screen can build the right controls.
#[tauri::command]
async fn form_fields(bytes: Vec<u8>) -> Result<Vec<mt_core::engine::pdf::Field>, String> {
    mt_core::engine::pdf::fields(&bytes).map_err(|e| e.to_string())
}

#[tauri::command]
async fn fill_form(
    state: State<'_, AppState>,
    request: FillFormRequest,
) -> Result<FillFormResponse, String> {
    let started = Instant::now();
    let (bytes, report) = mt_core::engine::pdf::fill(&request.bytes, &request.options)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "fill-form",
            // Field names, never the answers: a filled form holds an address,
            // a date of birth, an account number. The history database is for
            // finding the file again, not for keeping a copy of what's in it.
            recipe: serde_json::json!({
                "tool": "fill-form",
                "fields": report.filled,
                "flatten": request.options.flatten,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{} field{} filled{}",
                report.filled.len(),
                if report.filled.len() == 1 { "" } else { "s" },
                if report.flattened { ", flattened" } else { "" }
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(FillFormResponse {
        output_path: path.display().to_string(),
        filled: report.filled,
        unknown: report.unknown,
        read_only: report.read_only,
        still_required: report.still_required,
        flattened: report.flattened,
        warnings: report.warnings,
        size: bytes.len() as u64,
    })
}

#[tauri::command]
async fn annotate_pdf(
    state: State<'_, AppState>,
    request: AnnotateRequest,
) -> Result<AnnotateResponse, String> {
    let started = Instant::now();
    let (bytes, report) = mt_core::engine::pdf::annotate(&request.bytes, &request.options)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "annotate",
            // How many marks and where, not what they say: a note on a page
            // is as personal as anything else on it.
            recipe: serde_json::json!({
                "tool": "annotate",
                "marks": request.options.marks.len(),
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{} mark{} on {} page{}",
                report.drawn,
                if report.drawn == 1 { "" } else { "s" },
                report.pages_changed.len(),
                if report.pages_changed.len() == 1 { "" } else { "s" }
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(AnnotateResponse {
        output_path: path.display().to_string(),
        drawn: report.drawn,
        pages_changed: report.pages_changed,
        warnings: report.warnings,
        note: report.note,
        size: bytes.len() as u64,
    })
}

fn model_dir(state: &AppState) -> PathBuf {
    state.model_dir.clone()
}

/// Whether the OCR models are already on this machine.
#[tauri::command]
async fn models_ready(state: State<'_, AppState>, set: String) -> Result<ModelStatus, String> {
    let set = model_set(&set)?;
    let dir = model_dir(&state);
    let ready = set.files.iter().all(|(name, _, _)| dir.join(name).exists());
    Ok(ModelStatus {
        ready,
        bytes: set.files.iter().map(|(_, _, size)| size).sum(),
        folder: dir.display().to_string(),
        sources: set.files.iter().map(|(_, url, _)| url.to_string()).collect(),
    })
}

/// Fetch the OCR models.
///
/// Only ever called after the user has seen the size and the source and said
/// yes: this is the one thing in Kitbox that touches the network, and it would
/// be a poor joke to do it quietly in an app whose main promise is that it
/// doesn't.
#[tauri::command]
async fn fetch_models(state: State<'_, AppState>, set: String) -> Result<ModelStatus, String> {
    let chosen = model_set(&set)?;
    let dir = model_dir(&state);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;

    for (name, url, expected) in chosen.files {
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        let mut response = ureq::get(*url)
            .call()
            .map_err(|e| format!("could not reach {url}: {e}"))?;
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut response.body_mut().as_reader(), &mut body)
            .map_err(|e| format!("the download from {url} broke off: {e}"))?;

        // A truncated model fails later in a confusing way, so check the size
        // the server was supposed to send before keeping it.
        if body.len() as u64 != *expected {
            return Err(format!(
                "{name} came back as {} bytes instead of {expected} — the download looks                  incomplete, so nothing was saved",
                body.len()
            ));
        }
        // Written beside the target and renamed, so an interrupted download
        // never leaves a half file that looks finished.
        let partial = path.with_extension("part");
        std::fs::write(&partial, &body).map_err(|e| format!("could not write {name}: {e}"))?;
        std::fs::rename(&partial, &path).map_err(|e| format!("could not save {name}: {e}"))?;
    }

    models_ready(state, set).await
}

#[tauri::command]
async fn ocr_pdf(state: State<'_, AppState>, request: OcrRequest) -> Result<OcrResponse, String> {
    let started = Instant::now();
    let dir = model_dir(&state);

    let mut bytes = Vec::new();
    for (name, _, _) in model_set("ocr")?.files {
        bytes.push(
            std::fs::read(dir.join(name))
                .map_err(|_| "the OCR models aren't downloaded yet".to_string())?,
        );
    }
    let mut parts = bytes.into_iter();
    let models = mt_core::engine::pdf::OcrModels::load(
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;

    if request.searchable {
        let (out, report) =
            mt_core::engine::pdf::searchable(&request.bytes, &models, &request.options)
                .map_err(|e| e.to_string())?;
        let path = write_output(&state, &request.name, "pdf", &out)?;

        let _ = remember(
            &state,
            Recorded {
                tool: "ocr",
                recipe: serde_json::json!({ "tool": "ocr", "options": request.options }),
                inputs: vec![(request.name.clone(), request.bytes.as_slice())],
                outputs: vec![(path.clone(), out.as_slice())],
                summary: format!(
                    "{} words read on {} page{}",
                    report.words,
                    report.pages_read.len(),
                    if report.pages_read.len() == 1 { "" } else { "s" }
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );

        Ok(OcrResponse {
            output_path: Some(path.display().to_string()),
            pages: Vec::new(),
            pages_read: report.pages_read,
            pages_with_text: report.pages_with_text,
            pages_not_readable: report.pages_not_readable,
            words: report.words,
            note: report.note,
            warnings: report.warnings,
            size: out.len() as u64,
        })
    } else {
        let (pages, report) =
            mt_core::engine::pdf::ocr_read(&request.bytes, &models, &request.options)
                .map_err(|e| e.to_string())?;
        Ok(OcrResponse {
            output_path: None,
            pages,
            pages_read: report.pages_read,
            pages_with_text: report.pages_with_text,
            pages_not_readable: report.pages_not_readable,
            words: report.words,
            note: report.note,
            warnings: report.warnings,
            size: 0,
        })
    }
}

/// Render a page and keep the result.
///
/// Not an `async` command: the rendering waits on the UI thread's callbacks,
/// and the waiting is done on this command's own thread. Tauri runs commands
/// off the main thread, so blocking here is safe — blocking the main thread
/// would deadlock against the very callbacks being waited for.
#[tauri::command]
async fn html_to_pdf(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: HtmlRequest,
) -> Result<HtmlResponse, String> {
    let started = Instant::now();
    let rendered = html::to_pdf(&app, &request.source, &request.setup)?;
    let path = write_output(&state, &request.name, "pdf", &rendered.bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "html-to-pdf",
            recipe: serde_json::json!({
                "tool": "html-to-pdf",
                "source": rendered.report.source,
                "setup": {
                    "size": request.setup.size,
                    "landscape": request.setup.landscape,
                },
            }),
            // The page itself is the input, and it has no bytes of its own
            // worth storing — what was rendered is named in the recipe.
            inputs: Vec::new(),
            outputs: vec![(path.clone(), rendered.bytes.as_slice())],
            summary: format!(
                "{} → {} page{}, {}",
                rendered.report.source,
                rendered.report.pages,
                if rendered.report.pages == 1 { "" } else { "s" },
                format_size(rendered.bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(HtmlResponse {
        output_path: path.display().to_string(),
        pages: rendered.report.pages,
        source: rendered.report.source,
        size: rendered.bytes.len() as u64,
        warnings: rendered.report.warnings,
        note: rendered.report.note,
    })
}

#[tauri::command]
async fn remove_background(
    state: State<'_, AppState>,
    request: CutoutRequest,
) -> Result<CutoutResponse, String> {
    let started = Instant::now();
    let dir = model_dir(&state);
    let (name, _, _) = model_set("cutout")?.files[0];
    let model = mt_core::engine::image::CutoutModel::load(
        std::fs::read(dir.join(name))
            .map_err(|_| "the background-removal model isn't downloaded yet".to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let (image, report) =
        mt_core::engine::image::remove_background(&request.bytes, &model, &request.options)
            .map_err(|e| e.to_string())?;

    // PNG whatever the source was: a cutout with no backdrop has alpha, and a
    // JPEG would throw it away without saying so.
    let png = mt_core::engine::image::encode::encode(&image, FileKind::Png, 100, true)
        .map_err(|e| e.to_string())?;

    let output_path = if request.preview {
        None
    } else {
        let path = write_output(&state, &request.name, "png", &png)?;
        let _ = remember(
            &state,
            Recorded {
                tool: "remove-background",
                recipe: serde_json::json!({
                    "tool": "remove-background",
                    "options": request.options,
                }),
                inputs: vec![(request.name.clone(), request.bytes.as_slice())],
                outputs: vec![(path.clone(), png.as_slice())],
                summary: format!(
                    "background removed, {}×{}, {}",
                    report.width,
                    report.height,
                    format_size(png.len() as u64)
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
        Some(path.display().to_string())
    };

    Ok(CutoutResponse {
        output_path,
        image: data_url(FileKind::Png, &png),
        width: report.width,
        height: report.height,
        coverage: report.coverage,
        size: png.len() as u64,
        note: report.note,
        warnings: report.warnings,
    })
}

/// Roughly how long reconstructing this picture would take, so the screen can
/// say before anyone commits to it.
#[tauri::command]
async fn upscale_estimate(width: u32, height: u32) -> Result<f32, String> {
    Ok(mt_core::engine::image::estimate_seconds(width, height))
}

#[tauri::command]
async fn upscale_image(
    state: State<'_, AppState>,
    request: UpscaleRequest,
) -> Result<UpscaleResponse, String> {
    let started = Instant::now();

    // Only loaded when it is actually needed: the resampling path works with
    // nothing downloaded, and reading 70 MB to do it would be absurd.
    let model = if request.options.method == mt_core::engine::image::UpscaleMethod::Reconstruct {
        let dir = model_dir(&state);
        let (name, _, _) = model_set("upscale")?.files[0];
        Some(
            mt_core::engine::image::UpscaleModel::load(
                std::fs::read(dir.join(name))
                    .map_err(|_| "the upscaling model isn't downloaded yet".to_string())?,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    let (image, report) =
        mt_core::engine::image::upscale(&request.bytes, model.as_ref(), &request.options)
            .map_err(|e| e.to_string())?;

    let kind = FileKind::sniff(&request.bytes);
    let kind = if kind == FileKind::Png { FileKind::Png } else { FileKind::Jpeg };
    let encoded = mt_core::engine::image::encode::encode(&image, kind, 92, true)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, kind.extension(), &encoded)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "upscale",
            recipe: serde_json::json!({ "tool": "upscale", "options": request.options }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), encoded.as_slice())],
            summary: format!(
                "{}×{} → {}×{}, {}",
                report.from.0,
                report.from.1,
                report.to.0,
                report.to.1,
                format_size(encoded.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(UpscaleResponse {
        output_path: path.display().to_string(),
        from: report.from,
        to: report.to,
        tiles: report.tiles,
        seconds: report.seconds,
        size: encoded.len() as u64,
        preview_url: data_url(kind, &encoded),
        note: report.note,
        warnings: report.warnings,
    })
}

#[tauri::command]
async fn blur_faces(
    state: State<'_, AppState>,
    request: FacesRequest,
) -> Result<FacesResponse, String> {
    let started = Instant::now();
    let dir = model_dir(&state);
    let (name, _, _) = model_set("faces")?.files[0];
    let model = mt_core::engine::image::FaceModel::load(
        std::fs::read(dir.join(name))
            .map_err(|_| "the face-detection model isn't downloaded yet".to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let (image, report) =
        mt_core::engine::image::blur_faces(&request.bytes, &model, &request.options)
            .map_err(|e| e.to_string())?;

    let kind = if FileKind::sniff(&request.bytes) == FileKind::Png {
        FileKind::Png
    } else {
        FileKind::Jpeg
    };
    let encoded = mt_core::engine::image::encode::encode(&image, kind, 92, true)
        .map_err(|e| e.to_string())?;

    let output_path = if request.preview {
        None
    } else {
        let path = write_output(&state, &request.name, kind.extension(), &encoded)?;
        let _ = remember(
            &state,
            Recorded {
                tool: "blur-faces",
                // How many and how they were covered, never where: the
                // coordinates of a face in someone's photo are the one thing
                // this tool exists to protect.
                recipe: serde_json::json!({
                    "tool": "blur-faces",
                    "cover": request.options.cover,
                    "faces": report.covered,
                }),
                inputs: vec![(request.name.clone(), request.bytes.as_slice())],
                outputs: vec![(path.clone(), encoded.as_slice())],
                summary: format!(
                    "{} face{} covered, {}",
                    report.covered,
                    if report.covered == 1 { "" } else { "s" },
                    format_size(encoded.len() as u64)
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
        Some(path.display().to_string())
    };

    Ok(FacesResponse {
        output_path,
        image: data_url(kind, &encoded),
        faces: report.faces,
        covered: report.covered,
        size: encoded.len() as u64,
        note: report.note,
        warnings: report.warnings,
    })
}

#[tauri::command]
async fn html_to_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: ShotRequest,
) -> Result<ShotResponse, String> {
    let started = Instant::now();
    let shot = html::to_image(&app, &request.source, &request.setup)?;
    let kind = if request.setup.png { FileKind::Png } else { FileKind::Jpeg };
    let path = write_output(&state, &request.name, kind.extension(), &shot.bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "html-to-image",
            recipe: serde_json::json!({
                "tool": "html-to-image",
                "source": shot.report.source,
                "width": request.setup.width,
                "fullPage": request.setup.full_page,
            }),
            inputs: Vec::new(),
            outputs: vec![(path.clone(), shot.bytes.as_slice())],
            summary: format!(
                "{} → {}×{}, {}",
                shot.report.source,
                shot.report.width,
                shot.report.height,
                format_size(shot.bytes.len() as u64)
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(ShotResponse {
        output_path: path.display().to_string(),
        width: shot.report.width,
        height: shot.report.height,
        source: shot.report.source,
        size: shot.bytes.len() as u64,
        preview_url: data_url(kind, &shot.bytes),
        warnings: shot.report.warnings,
        note: shot.report.note,
    })
}

#[tauri::command]
async fn adjust_image(
    state: State<'_, AppState>,
    request: AdjustRequest,
) -> Result<AdjustResponse, String> {
    let started = Instant::now();
    let source = image::load_from_memory(&request.bytes).map_err(|e| e.to_string())?;

    // A preview runs on a smaller copy so the sliders stay live. The settings
    // are all per-pixel and scale-free, so what is shown is what will be
    // saved — levels and colour are read from the picture's own histogram,
    // which a fair downscale preserves.
    let working = if request.preview
        && request.max_px > 0
        && source.width().max(source.height()) > request.max_px
    {
        source.resize(request.max_px, request.max_px, image::imageops::FilterType::Triangle)
    } else {
        source
    };

    let (image, report) = mt_core::engine::image::adjust_image(&working, &request.options);

    let kind = if FileKind::sniff(&request.bytes) == FileKind::Png {
        FileKind::Png
    } else {
        FileKind::Jpeg
    };
    let encoded = mt_core::engine::image::encode::encode(&image, kind, 92, true)
        .map_err(|e| e.to_string())?;

    let output_path = if request.preview {
        None
    } else {
        let path = write_output(&state, &request.name, kind.extension(), &encoded)?;
        let _ = remember(
            &state,
            Recorded {
                tool: "adjust",
                recipe: serde_json::json!({ "tool": "adjust", "options": request.options }),
                inputs: vec![(request.name.clone(), request.bytes.as_slice())],
                outputs: vec![(path.clone(), encoded.as_slice())],
                summary: format!(
                    "adjusted, {}×{}, {}",
                    report.width,
                    report.height,
                    format_size(encoded.len() as u64)
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
        Some(path.display().to_string())
    };

    Ok(AdjustResponse {
        output_path,
        image: data_url(kind, &encoded),
        width: report.width,
        height: report.height,
        size: encoded.len() as u64,
        warnings: report.warnings,
    })
}

/// The machine's own sRGB profile, for the archival output intent.
///
/// Read from the system rather than shipped: every Windows install has one,
/// it is the user's own file, and `mt-core` reads nothing from disk by rule.
/// Where there isn't one, that becomes a reported blocker instead of a guess.
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

#[tauri::command]
async fn pdf_to_archive(
    state: State<'_, AppState>,
    request: ArchiveRequest,
) -> Result<ArchiveResponse, String> {
    let started = Instant::now();

    if request.check_only {
        let report = mt_core::engine::pdf::check_archive(&request.bytes, request.part)
            .map_err(|e| e.to_string())?;
        return Ok(ArchiveResponse {
            output_path: None,
            conformant: report.conformant,
            part: report.part,
            applied: report.applied,
            blockers: report.blockers,
            note: report.note,
            size: 0,
            profile_found: system_srgb_profile().is_some(),
        });
    }

    let icc = system_srgb_profile();
    let profile_found = icc.is_some();
    let options = mt_core::engine::pdf::ArchiveOptions {
        part: request.part,
        icc,
        title: request.title.clone(),
        author: request.author.clone(),
    };

    let (bytes, report) = mt_core::engine::pdf::to_archive(&request.bytes, &options)
        .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &bytes)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "pdf-archive",
            recipe: serde_json::json!({
                "tool": "pdf-archive",
                "part": report.part,
                "conformant": report.conformant,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), bytes.as_slice())],
            summary: format!(
                "{}, {}",
                report.part,
                if report.conformant { "checks passed" } else { "not conformant" }
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(ArchiveResponse {
        output_path: Some(path.display().to_string()),
        conformant: report.conformant,
        part: report.part,
        applied: report.applied,
        blockers: report.blockers,
        note: report.note,
        size: bytes.len() as u64,
        profile_found,
    })
}

/// The heaviest face on this machine, for captions.
///
/// Impact is what this look is made of and is on every Windows install; the
/// bold fallbacks keep it working where it isn't. As everywhere else, the
/// bytes are read here and handed to `mt-core`, which opens no files.
fn caption_font() -> Result<Vec<u8>, String> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\impact.ttf",
        r"C:\Windows\Fonts\arialbd.ttf",
        r"C:\Windows\Fonts\seguibl.ttf",
        r"C:\Windows\Fonts\arial.ttf",
    ];
    CANDIDATES
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .ok_or_else(|| "no font was found on this machine to write captions with".to_string())
}

#[tauri::command]
async fn make_meme(state: State<'_, AppState>, request: MemeRequest) -> Result<MemeResponse, String> {
    let started = Instant::now();
    let font = caption_font()?;
    let (image, report) =
        mt_core::engine::image::meme(&request.bytes, &font, &request.options)
            .map_err(|e| e.to_string())?;

    // PNG: a caption has hard edges against flat colour, which is exactly what
    // JPEG smears.
    let encoded = mt_core::engine::image::encode::encode(&image, FileKind::Png, 100, true)
        .map_err(|e| e.to_string())?;

    let output_path = if request.preview {
        None
    } else {
        let path = write_output(&state, &request.name, "png", &encoded)?;
        let _ = remember(
            &state,
            Recorded {
                tool: "meme",
                recipe: serde_json::json!({ "tool": "meme", "options": request.options }),
                inputs: vec![(request.name.clone(), request.bytes.as_slice())],
                outputs: vec![(path.clone(), encoded.as_slice())],
                summary: format!(
                    "captioned, {}×{}, {}",
                    report.width,
                    report.height,
                    format_size(encoded.len() as u64)
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
        Some(path.display().to_string())
    };

    Ok(MemeResponse {
        output_path,
        image: data_url(FileKind::Png, &encoded),
        width: report.width,
        height: report.height,
        top_lines: report.top_lines,
        bottom_lines: report.bottom_lines,
        size: encoded.len() as u64,
        warnings: report.warnings,
    })
}

/// A batch in flight.
///
/// The screen sends one file per call rather than the whole pile, and this is
/// what remembers the run between those calls. Measured, not assumed: a 4 MB
/// photo crosses the IPC boundary as about 14 MB of JSON, so forty of them in
/// a single message would be half a gigabyte in one string — the size of job
/// this screen exists for would be the one that broke it. A file at a time
/// keeps the peak at one file, and makes Stop land at the next file instead
/// of at the end.
struct BatchSession {
    op: mt_core::batch::BatchOp,
    /// Every file's name, in the order they will be sent. Knowing them up
    /// front is what lets a stopped run account for the files it never
    /// reached without the screen having to ship their bytes across to be
    /// told so.
    names: Vec<String>,
    started: Instant,
    cancel: mt_core::job::Cancel,
    outcomes: Vec<mt_core::batch::ItemOutcome>,
    paths: Vec<String>,
    /// Kept for the history entry written at the end. These are outputs, the
    /// small end of every operation here, so holding them costs little.
    outputs: Vec<(PathBuf, Vec<u8>)>,
}

static BATCH: std::sync::Mutex<Option<BatchSession>> = std::sync::Mutex::new(None);

/// Begin a run. Any previous one is abandoned.
///
/// Replacing rather than refusing is deliberate: a window that closed mid-run
/// would otherwise leave a session nobody can clear, and the app would refuse
/// to batch again until it restarted.
#[tauri::command]
async fn batch_start(op: mt_core::batch::BatchOp, names: Vec<String>) -> Result<(), String> {
    let mut guard = BATCH.lock().map_err(|_| "could not start the batch")?;
    if let Some(previous) = guard.as_ref() {
        previous.cancel.cancel();
    }
    *guard = Some(BatchSession {
        op,
        names,
        started: Instant::now(),
        cancel: mt_core::job::Cancel::new(),
        outcomes: Vec::new(),
        paths: Vec::new(),
        outputs: Vec::new(),
    });
    Ok(())
}

/// Do one file, and say what happened to it.
#[tauri::command]
async fn batch_item(
    state: State<'_, AppState>,
    name: String,
    bytes: Vec<u8>,
) -> Result<BatchStep, String> {
    // The op and the flag are copied out so the lock isn't held while the file
    // is worked on — otherwise Stop would queue behind the very file it is
    // trying to interrupt.
    let (op, cancel) = {
        let guard = BATCH.lock().map_err(|_| "the batch is busy")?;
        let session = guard.as_ref().ok_or("no batch is running")?;
        (session.op.clone(), session.cancel.clone())
    };

    let (outcome, written) = if cancel.is_cancelled() {
        (mt_core::batch::ItemOutcome::Skipped { name }, None)
    } else {
        let item = mt_core::batch::Item { name, bytes };
        let mut result = mt_core::batch::run(
            std::slice::from_ref(&item),
            &op,
            &mt_core::job::Silent,
            &cancel,
        );
        let outcome = result.outcomes.pop().ok_or("the file produced no answer")?;
        match result.files.pop() {
            Some((out_name, out_bytes)) => {
                let extension =
                    out_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("bin").to_string();
                let path = write_output(&state, &out_name, &extension, &out_bytes)
                    .map_err(|e| format!("could not write {out_name}: {e}"))?;
                (outcome, Some((path, out_bytes)))
            }
            None => (outcome, None),
        }
    };

    let mut guard = BATCH.lock().map_err(|_| "the batch is busy")?;
    let session = guard.as_mut().ok_or("no batch is running")?;
    session.outcomes.push(outcome.clone());
    let shown = written.map(|(path, bytes)| {
        let shown = path.display().to_string();
        session.paths.push(shown.clone());
        session.outputs.push((path, bytes));
        shown
    });
    Ok(BatchStep { outcome, path: shown, done: session.outcomes.len() })
}

/// End the run: one history entry, and the totals for the screen.
#[tauri::command]
async fn batch_finish(state: State<'_, AppState>) -> Result<BatchResponse, String> {
    let mut session = {
        let mut guard = BATCH.lock().map_err(|_| "the batch is busy")?;
        guard.take().ok_or("no batch is running")?
    };

    // Whatever the screen stopped sending — because you pressed Stop, or
    // because something broke — is still part of the run, and says so by name.
    for name in session.names.iter().skip(session.outcomes.len()) {
        session.outcomes.push(mt_core::batch::ItemOutcome::Skipped { name: name.clone() });
    }
    let total = session.names.len();

    let done = session.outcomes.iter().filter(|o| o.is_done()).count() as u32;
    let failed = session.outcomes.iter().filter(|o| o.is_failure()).count() as u32;
    let short = session.outcomes.iter().filter(|o| o.fell_short()).count() as u32;

    // One history entry for the run, not one per file: forty rows for a single
    // action would bury everything else in the gallery.
    if !session.outputs.is_empty() {
        let recorded = remember_run(
            &state,
            Recorded {
                tool: "batch",
                recipe: serde_json::json!({
                    "tool": "batch",
                    "op": session.op,
                    "files": total,
                }),
                inputs: Vec::new(),
                outputs: session
                    .outputs
                    .iter()
                    .map(|(path, bytes)| (path.clone(), bytes.as_slice()))
                    .collect(),
                summary: format!(
                    "{} on {} file{} — {done} done, {failed} failed",
                    session.op.describe(),
                    total,
                    if total == 1 { "" } else { "s" }
                ),
                duration_ms: session.started.elapsed().as_millis() as i64,
            },
        );
        // The run itself succeeded; say so on the console rather than failing
        // it, but don't swallow the reason — a gallery quietly not recording
        // is the kind of fault nobody notices for weeks.
        if let Err(e) = recorded {
            eprintln!("batch: the run finished but history did not record it: {e}");
        }
    }

    Ok(BatchResponse {
        done,
        failed,
        short,
        cancelled: session.cancel.is_cancelled(),
        outcomes: session.outcomes,
        paths: session.paths,
        folder: state.output_dir.display().to_string(),
    })
}

/// Stop the batch that is running, if there is one.
///
/// The file already in flight finishes — interrupting an encode would leave a
/// half-written file — and everything after it is reported as not reached.
#[tauri::command]
async fn cancel_batch() -> Result<bool, String> {
    let guard = BATCH.lock().map_err(|_| "could not reach the running batch")?;
    match guard.as_ref() {
        Some(session) => {
            session.cancel.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Every preset the app knows about, and anything in the folder it couldn't
/// read.
#[tauri::command]
async fn list_presets(state: State<'_, AppState>) -> Result<presets::Loaded, String> {
    Ok(presets::load(&state.preset_dir))
}

/// Add or change one preset. Bundled ones are never touched; this writes your
/// version, which takes precedence by id.
#[tauri::command]
async fn save_preset(
    state: State<'_, AppState>,
    preset: mt_core::preset::Preset,
) -> Result<presets::Loaded, String> {
    presets::save(&state.preset_dir, preset)?;
    Ok(presets::load(&state.preset_dir))
}

/// Drop your version of a preset. If it replaced a bundled one, that comes
/// back; if it was your own, it is gone.
#[tauri::command]
async fn delete_preset(
    state: State<'_, AppState>,
    id: String,
) -> Result<presets::Loaded, String> {
    presets::forget(&state.preset_dir, &id)?;
    Ok(presets::load(&state.preset_dir))
}

/// Measure a file against a preset, and say what was compared.
#[tauri::command]
async fn check_preset(
    request: CheckRequest,
) -> Result<mt_core::preset::Report, String> {
    Ok(mt_core::preset::check(&request.bytes, &request.preset))
}

/// Make a file that conforms, then check it and report honestly.
#[tauri::command]
async fn fix_preset(
    state: State<'_, AppState>,
    request: FixRequest,
) -> Result<FixResponse, String> {
    let started = Instant::now();
    let fixed = mt_core::preset::fix(&request.bytes, &request.preset, Bias::Balanced)
        .map_err(|e| e.to_string())?;

    let path = write_output(&state, &request.name, &fixed.extension, &fixed.bytes)?;
    let _ = remember(
        &state,
        Recorded {
            tool: "preset-fix",
            recipe: serde_json::json!({ "tool": "preset-fix", "preset": request.preset }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), fixed.bytes.as_slice())],
            summary: format!(
                "{} — {}{}",
                request.preset.name,
                format_size(fixed.bytes.len() as u64),
                if fixed.report.fits { "" } else { ", still short" }
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(FixResponse {
        size: fixed.bytes.len() as u64,
        report: fixed.report,
        steps: fixed.steps,
        shortfall: fixed.shortfall,
        output_path: path.display().to_string(),
        preview: {
            // A picture can be shown straight back; a PDF cannot, and saying
            // nothing is better than a broken frame.
            let kind = FileKind::sniff(&fixed.bytes);
            kind.is_image().then(|| data_url(kind, &fixed.bytes))
        },
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RerunRequest {
    /// The history entry whose recipe to repeat.
    hash: String,
    name: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RerunResponse {
    summary: String,
    /// Where each file went, in the order they were made.
    paths: Vec<String>,
}

/// Do a past job again, on a file you choose now.
///
/// The recipe is read from history rather than passed in: what gets replayed
/// is what was recorded at the time, not something the screen reconstructed.
#[tauri::command]
async fn rerun_recipe(
    state: State<'_, AppState>,
    request: RerunRequest,
) -> Result<RerunResponse, String> {
    let started = Instant::now();

    let recorded = {
        let store = state.store.lock().map_err(|_| "history is busy")?;
        store
            .lineage(&request.hash)
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or("that entry has no recorded settings to repeat")?
    };

    let status = recipe::inspect(&recorded.recipe);
    if !status.can {
        return Err(status.why.unwrap_or_else(|| "this one can't be repeated".into()));
    }
    let parsed: recipe::Recipe =
        serde_json::from_str(&recorded.recipe).map_err(|e| e.to_string())?;

    let produced = recipe::run(&parsed, &request.name, &request.bytes)?;

    let mut paths = Vec::new();
    for (name, bytes) in &produced.files {
        let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("bin");
        paths.push(write_output(&state, name, extension, bytes)?.display().to_string());
    }

    // Recorded under the tool that originally ran, with the same recipe, so a
    // replay can itself be replayed and the gallery reads the same either way.
    let _ = remember(
        &state,
        Recorded {
            tool: &recorded.tool,
            recipe: serde_json::from_str(&recorded.recipe).unwrap_or(serde_json::Value::Null),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: produced
                .files
                .iter()
                .zip(paths.iter())
                .map(|((_, bytes), path)| (PathBuf::from(path), bytes.as_slice()))
                .collect(),
            summary: format!("{} (run again)", produced.summary),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(RerunResponse { summary: produced.summary, paths })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Swept {
    /// Entries whose file had gone, now forgotten.
    forgotten: u32,
    /// Stored copies nothing pointed at any more.
    loose: u32,
    /// Bytes the store got back.
    freed: u64,
}

/// Forget one entry: its row, its job, and Kitbox's copy of the bytes.
///
/// The finished file in your output folder is yours and is left alone — this
/// is the gallery's copy, which is the only one Kitbox put anywhere you didn't
/// ask for.
#[tauri::command]
async fn forget_entry(state: State<'_, AppState>, hash: String) -> Result<UsageResponse, String> {
    {
        let store = state.store.lock().map_err(|_| "history is busy")?;
        store.forget(&hash).map_err(|e| e.to_string())?;
    }
    storage_usage(state)
}

/// Forget every entry whose file is no longer where it was written.
///
/// What this is for: files get moved, renamed and deleted outside the app, and
/// an entry pointing at nothing is worse than no entry — it offers to show you
/// a file that isn't there. Pinned entries are left alone, because pinning is
/// how you say you want the stored copy kept whatever happens to the original.
#[tauri::command]
async fn tidy_history(state: State<'_, AppState>) -> Result<Swept, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;

    let mut forgotten = 0;
    let mut loose = 0;
    let mut freed = 0;

    for file in store.all_files().map_err(|e| e.to_string())? {
        if file.pinned {
            continue;
        }
        // No recorded path means it was an input Kitbox was handed, not a file
        // it wrote. There is nothing on disk to have gone missing.
        let Some(path) = file.source_path.as_deref() else {
            continue;
        };
        if Path::new(path).exists() {
            continue;
        }
        store.forget(&file.hash).map_err(|e| e.to_string())?;
        forgotten += 1;
        freed += file.size;
    }

    // Then the copies left holding space nothing can reach — which is mostly
    // what forgetting the entries above just created.
    for file in store.unreachable().map_err(|e| e.to_string())? {
        if file.pinned {
            continue;
        }
        store.forget(&file.hash).map_err(|e| e.to_string())?;
        loose += 1;
        freed += file.size;
    }

    Ok(Swept { forgotten, loose, freed })
}

/// How much a tidy-up would remove, so the panel can offer it rather than do
/// it and report afterwards.
#[tauri::command]
async fn count_missing(state: State<'_, AppState>) -> Result<u32, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;

    let gone = store
        .all_files()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|f| {
            !f.pinned && f.source_path.as_deref().is_some_and(|p| !Path::new(p).exists())
        })
        .count();
    let loose =
        store.unreachable().map_err(|e| e.to_string())?.iter().filter(|f| !f.pinned).count();

    Ok((gone + loose) as u32)
}

/// What a right-click produced, for the screen to show.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellRun {
    /// "compress" or "open".
    kind: String,
    /// One line for the heading: "Compressed 3 files to 50 KB–100 KB".
    summary: String,
    outcomes: Vec<mt_core::batch::ItemOutcome>,
    paths: Vec<String>,
    /// For "open": the files themselves, to put on a screen.
    files: Vec<LoadedFile>,
    folder: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedFile {
    name: String,
    bytes: Vec<u8>,
}

/// Right-clicked files waiting to be dealt with.
///
/// Explorer runs the verb once per selected file, so forty files arrive as
/// forty launches in quick succession. They are collected here and run as one
/// job: one window, one history entry, one list of results, rather than forty
/// of each.
static PENDING: Mutex<Option<(shell::Asked, Vec<PathBuf>)>> = Mutex::new(None);

/// The last result, kept for a frontend that wasn't listening yet.
///
/// A launch from Explorer starts the app and emits before the screen has
/// mounted, so the event alone would be lost exactly when the app was opened
/// by the menu — which is most of the time.
static LAST_RUN: Mutex<Option<ShellRun>> = Mutex::new(None);

/// Take whatever a right-click left for the screen.
#[tauri::command]
async fn take_shell_run() -> Result<Option<ShellRun>, String> {
    let mut guard = LAST_RUN.lock().map_err(|_| "busy")?;
    Ok(guard.take())
}

/// Note the files from one `--shell` launch and arrange for them to be done.
pub fn accept_shell_args(app: &tauri::AppHandle, argv: &[String]) {
    let Some((asked, files)) = shell::parse(argv) else {
        return;
    };

    let start = {
        let Ok(mut guard) = PENDING.lock() else {
            return;
        };
        match guard.as_mut() {
            Some((_, waiting)) => {
                waiting.extend(files);
                // A run is already scheduled; this just joins it.
                false
            }
            None => {
                *guard = Some((asked, files));
                true
            }
        }
    };

    if !start {
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        // Long enough for a multiple selection to land, short enough that one
        // file doesn't feel like a wait.
        std::thread::sleep(std::time::Duration::from_millis(700));
        let Some((asked, files)) = PENDING.lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        run_shell_job(&app, asked, files);
    });
}

fn run_shell_job(app: &tauri::AppHandle, asked: shell::Asked, files: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let started = Instant::now();

    let mut items = Vec::new();
    let mut unreadable = Vec::new();
    for path in &files {
        match std::fs::read(path) {
            Ok(bytes) => items.push(mt_core::batch::Item { name: file_name(path), bytes }),
            Err(e) => unreadable.push(mt_core::batch::ItemOutcome::Failed {
                name: file_name(path),
                why: format!("could not be read: {e}"),
            }),
        }
    }

    let run = match asked {
        shell::Asked::Open => ShellRun {
            kind: "open".into(),
            summary: format!("{} file{} from Explorer", items.len(), plural(items.len())),
            outcomes: unreadable,
            paths: Vec::new(),
            files: items
                .into_iter()
                .map(|item| LoadedFile { name: item.name, bytes: item.bytes })
                .collect(),
            folder: state.output_dir.display().to_string(),
        },
        shell::Asked::Compress(target) => {
            let op = mt_core::batch::BatchOp::Compress {
                target,
                bias: Bias::Balanced,
                format: mt_core::engine::image::OutputFormat::Keep,
                strip_metadata: true,
                exact: None,
            };
            let result = mt_core::batch::run(
                &items,
                &op,
                &mt_core::job::Silent,
                &mt_core::job::Cancel::new(),
            );

            let mut paths = Vec::new();
            let mut written = Vec::new();
            for (name, bytes) in &result.files {
                let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("bin");
                match write_output(&state, name, extension, bytes) {
                    Ok(path) => {
                        paths.push(path.display().to_string());
                        written.push((path, bytes.clone()));
                    }
                    Err(e) => eprintln!("shell: could not write {name}: {e}"),
                }
            }

            let done = result.done();
            let failed = result.failed() + unreadable.len();
            let _ = remember(
                &state,
                Recorded {
                    tool: "shell-compress",
                    recipe: serde_json::json!({ "tool": "compress", "target": target,
                                                "bias": Bias::Balanced }),
                    inputs: Vec::new(),
                    outputs: written
                        .iter()
                        .map(|(path, bytes)| (path.clone(), bytes.as_slice()))
                        .collect(),
                    summary: format!(
                        "{} file{} from Explorer to {} — {done} done, {failed} failed",
                        files.len(),
                        plural(files.len()),
                        target.describe()
                    ),
                    duration_ms: started.elapsed().as_millis() as i64,
                },
            );

            let mut outcomes = result.outcomes;
            outcomes.extend(unreadable);
            ShellRun {
                kind: "compress".into(),
                summary: format!(
                    "{} file{} to {}",
                    files.len(),
                    plural(files.len()),
                    target.describe()
                ),
                outcomes,
                paths,
                files: Vec::new(),
                folder: state.output_dir.display().to_string(),
            }
        }
    };

    if let Ok(mut guard) = LAST_RUN.lock() {
        *guard = Some(run.clone());
    }
    let _ = tauri::Emitter::emit(app, "shell-run", run);

    // Whatever happened, the window is the answer: show it.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Keep an entry whatever else is tidied away.
///
/// Pinning is the one thing that stops a sweep: it says this copy matters
/// beyond the file it came from. Forgetting a pinned entry on purpose still
/// works — that is an explicit answer to an explicit question.
#[tauri::command]
async fn set_pinned(
    state: State<'_, AppState>,
    hash: String,
    pinned: bool,
) -> Result<bool, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;
    store.set_pinned(&hash, pinned).map_err(|e| e.to_string())?;
    Ok(pinned)
}

/// Do what the clipboard key does, once.
///
/// Separate from the key itself so the work can be tested, and run from the
/// screen, without anybody pressing anything.
pub fn run_clipboard_job(app: &tauri::AppHandle) -> hotkey::Pressed {
    let state = app.state::<AppState>();
    let started = Instant::now();
    let settings = hotkey::load(&state.preset_dir);

    let attempt = || -> Result<hotkey::Pressed, String> {
        let target = settings.size_target()?;
        let source = hotkey::picture_on_clipboard()?;
        let before = source.len() as u64;

        let (bytes, note) = hotkey::squeeze(&source, target)?;
        let name = hotkey::name_for(std::time::SystemTime::now());
        let path = write_output(&state, &name, "jpg", &bytes)?;

        // The file, not the picture: a clipboard bitmap would be uncompressed
        // again the moment anything read it, which would undo the whole job.
        hotkey::put_file_on_clipboard(&path)?;

        let _ = remember(
            &state,
            Recorded {
                tool: "clipboard",
                recipe: serde_json::json!({
                    "tool": "compress",
                    "target": target,
                    "bias": Bias::Balanced,
                }),
                inputs: Vec::new(),
                outputs: vec![(path.clone(), bytes.as_slice())],
                summary: format!(
                    "from the clipboard, {} → {}, {note}",
                    format_size(before),
                    format_size(bytes.len() as u64)
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );

        Ok(hotkey::Pressed {
            ok: true,
            summary: format!(
                "{} → {}. The file is on your clipboard.",
                format_size(before),
                format_size(bytes.len() as u64)
            ),
            before,
            after: bytes.len() as u64,
            path: Some(path.display().to_string()),
        })
    };

    let pressed = match attempt() {
        Ok(pressed) => pressed,
        Err(why) => hotkey::Pressed { ok: false, summary: why, before: 0, after: 0, path: None },
    };

    // Said out loud, because the window may not even be open: a key that
    // appears to do nothing is worse than no key.
    let _ = tauri::Emitter::emit(app, "clipboard-run", pressed.clone());
    announce(app, &pressed);
    pressed
}

/// Tell the person what happened, without stealing their focus.
fn announce(app: &tauri::AppHandle, pressed: &hotkey::Pressed) {
    use tauri_plugin_notification::NotificationExt;

    let _ = app
        .notification()
        .builder()
        .title(if pressed.ok { "Kitbox" } else { "Kitbox couldn't" })
        .body(&pressed.summary)
        .show();
}

/// Start or stop listening for the clipboard key.
fn listen_for_hotkey(app: &tauri::AppHandle, settings: &hotkey::Hotkey) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    if !settings.enabled {
        return Ok(());
    }

    let combo = settings.combo.clone();
    let handle = app.clone();
    shortcuts
        .on_shortcut(combo.as_str(), move |_app, _shortcut, event| {
            // Once per press, not once per key-down repeat.
            if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    run_clipboard_job(&handle);
                });
            }
        })
        .map_err(|e| {
            format!("'{combo}' could not be taken — something else on this machine has it ({e})")
        })
}

#[tauri::command]
async fn clipboard_key(state: State<'_, AppState>) -> Result<hotkey::Hotkey, String> {
    Ok(hotkey::load(&state.preset_dir))
}

#[tauri::command]
async fn set_clipboard_key(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: hotkey::Hotkey,
) -> Result<hotkey::Hotkey, String> {
    // Taking the key first: if the combination is spoken for, nothing is saved
    // and the screen keeps showing what is actually true.
    listen_for_hotkey(&app, &settings)?;
    hotkey::save(&state.preset_dir, &settings)?;
    Ok(settings)
}

/// Run the clipboard job from the screen, for anyone who would rather click.
#[tauri::command]
async fn use_clipboard(app: tauri::AppHandle) -> Result<hotkey::Pressed, String> {
    Ok(run_clipboard_job(&app))
}

/// The folders being watched, and the watcher doing it.
///
/// One watcher for all of them: the crate can hold several paths, and one
/// thread that knows about every watch is easier to reason about than one per
/// folder racing each other.
static WATCHER: Mutex<Option<notify::RecommendedWatcher>> = Mutex::new(None);

/// What was in each watched folder when watching started, so that only
/// arrivals are taken.
static SEEN: Mutex<Option<std::collections::HashSet<PathBuf>>> = Mutex::new(None);

/// Start watching whatever the settings say, replacing any current watch.
fn start_watching(app: &tauri::AppHandle, watches: &watch::Watches) -> Result<(), String> {
    use notify::{RecursiveMode, Watcher};

    // Dropping the old watcher is what stops it; do that before building the
    // new one so a folder is never watched twice.
    if let Ok(mut guard) = WATCHER.lock() {
        *guard = None;
    }

    let live: Vec<watch::Watch> = watches.0.iter().filter(|w| w.enabled).cloned().collect();
    if live.is_empty() {
        return Ok(());
    }

    // Everything already in those folders is noted now and left alone.
    let mut seen = std::collections::HashSet::new();
    for w in &live {
        seen.extend(watch::already_there(Path::new(&w.path)));
    }
    if let Ok(mut guard) = SEEN.lock() {
        *guard = Some(seen);
    }

    let handle = app.clone();
    let folders = live.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            return;
        };
        if !matches!(
            event.kind,
            notify::EventKind::Create(_) | notify::EventKind::Modify(_)
        ) {
            return;
        }
        for path in event.paths {
            let Some(watch) = folders
                .iter()
                .find(|w| path.parent().is_some_and(|parent| parent == Path::new(&w.path)))
            else {
                continue;
            };
            if !watch::worth_taking(&path) {
                continue;
            }
            // Already handled, or there before the watch began.
            if let Ok(mut guard) = SEEN.lock() {
                if let Some(seen) = guard.as_mut() {
                    if !seen.insert(path.clone()) {
                        continue;
                    }
                }
            }

            let handle = handle.clone();
            let watch = watch.clone();
            std::thread::spawn(move || {
                handle_arrival(&handle, &watch, &path);
            });
        }
    })
    .map_err(|e| format!("the folder watcher could not start: {e}"))?;

    for w in &live {
        let folder = Path::new(&w.path);
        if !folder.is_dir() {
            return Err(format!("{} is not a folder", w.path));
        }
        watcher
            .watch(folder, RecursiveMode::NonRecursive)
            .map_err(|e| format!("{} could not be watched: {e}", w.path))?;
    }

    if let Ok(mut guard) = WATCHER.lock() {
        *guard = Some(watcher);
    }
    Ok(())
}

/// One file that has just landed in a watched folder.
fn handle_arrival(app: &tauri::AppHandle, watch: &watch::Watch, path: &Path) {
    // A file exists from its first byte. Taking it now would get a fragment.
    let Some(_size) = watch::settle(path, std::time::Duration::from_secs(20)) else {
        return;
    };

    let state = app.state::<AppState>();
    let started = Instant::now();
    let name = file_name(path);

    let attempt = || -> Result<hotkey::Pressed, String> {
        let target = watch.size_target()?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let before = bytes.len() as u64;

        let op = mt_core::batch::BatchOp::Compress {
            target,
            bias: Bias::Balanced,
            format: mt_core::engine::image::OutputFormat::Keep,
            strip_metadata: true,
            exact: None,
        };
        let item = mt_core::batch::Item { name: name.clone(), bytes };
        let mut result = mt_core::batch::run(
            std::slice::from_ref(&item),
            &op,
            &mt_core::job::Silent,
            &mt_core::job::Cancel::new(),
        );

        let (out_name, out_bytes) = result.files.pop().ok_or_else(|| {
            match result.outcomes.first() {
                Some(mt_core::batch::ItemOutcome::Failed { why, .. }) => why.clone(),
                _ => "nothing came out".to_string(),
            }
        })?;
        let extension = out_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("bin");
        let written = write_output(&state, &out_name, extension, &out_bytes)?;

        let _ = remember(
            &state,
            Recorded {
                tool: "watched-folder",
                recipe: serde_json::json!({
                    "tool": "compress",
                    "target": target,
                    "bias": Bias::Balanced,
                }),
                inputs: Vec::new(),
                outputs: vec![(written.clone(), out_bytes.as_slice())],
                summary: format!(
                    "{name} landed in {} — {} → {}",
                    watch.path,
                    format_size(before),
                    format_size(out_bytes.len() as u64)
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );

        Ok(hotkey::Pressed {
            ok: true,
            summary: format!(
                "{name}: {} → {}",
                format_size(before),
                format_size(out_bytes.len() as u64)
            ),
            before,
            after: out_bytes.len() as u64,
            path: Some(written.display().to_string()),
        })
    };

    let outcome = match attempt() {
        Ok(done) => done,
        Err(why) => hotkey::Pressed {
            ok: false,
            summary: format!("{name}: {why}"),
            before: 0,
            after: 0,
            path: None,
        },
    };

    let _ = tauri::Emitter::emit(app, "watch-run", outcome.clone());
    announce(app, &outcome);
}

#[tauri::command]
async fn watched_folders(state: State<'_, AppState>) -> Result<watch::Watches, String> {
    Ok(watch::load(&state.preset_dir))
}

/// Add, change or remove watched folders, and start watching them.
#[tauri::command]
async fn set_watched_folders(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    watches: watch::Watches,
) -> Result<watch::Watches, String> {
    for w in &watches.0 {
        w.size_target()?;
    }
    // Watching first: a folder that has gone, or a size that can't be read, is
    // refused before anything is written down.
    start_watching(&app, &watches)?;
    watch::save(&state.preset_dir, &watches)?;
    Ok(watches)
}

/// One file in a chain, enough to draw it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Link {
    hash: String,
    name: String,
    kind: String,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumb_hash: Option<String>,
    pinned: bool,
    /// False when the row is gone — the chain still shows the step, because
    /// "this came from something that has since been forgotten" is worth
    /// knowing.
    known: bool,
}

/// One step in the story of a file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Telling {
    tool: String,
    summary: String,
    created_at: i64,
    duration_ms: i64,
    /// The settings, pretty-printed, for anyone who wants to read them.
    recipe: String,
    from: Option<Link>,
    to: Option<Link>,
}

/// Everything history knows about one file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Story {
    /// The file itself.
    file: Option<Link>,
    /// What produced it, oldest first.
    chain: Vec<Telling>,
    /// What has been made from it since.
    derivatives: Vec<Telling>,
}

/// The chain that produced a file, and anything made from it.
///
/// This is what the gallery was always for: a file is not a thing on its own,
/// it is the latest step of something — the scan, then the compressed copy,
/// then the signed one. Until now that chain was recorded and never shown.
#[tauri::command]
async fn entry_story(state: State<'_, AppState>, hash: String) -> Result<Story, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;

    let link = |hash: &str| -> Option<Link> {
        if hash.is_empty() {
            return None;
        }
        Some(match store.get(hash).ok().flatten() {
            Some(record) => Link {
                hash: record.hash,
                name: record.name,
                kind: record.kind,
                size: record.size,
                thumb_hash: record.thumb_hash,
                pinned: record.pinned,
                known: true,
            },
            // Forgotten, or never stored: the step still happened.
            None => Link {
                hash: hash.to_string(),
                name: "a file no longer kept".into(),
                kind: String::new(),
                size: 0,
                thumb_hash: None,
                pinned: false,
                known: false,
            },
        })
    };

    let tell = |job: mt_store::JobRecord| Telling {
        tool: job.tool,
        summary: job.summary,
        created_at: job.created_at,
        duration_ms: job.duration_ms,
        recipe: serde_json::from_str::<serde_json::Value>(&job.recipe)
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or(job.recipe),
        from: link(&job.input_hash),
        to: link(&job.output_hash),
    };

    let chain: Vec<Telling> =
        store.lineage(&hash).map_err(|e| e.to_string())?.into_iter().map(tell).collect();
    let derivatives: Vec<Telling> =
        store.derivatives(&hash).map_err(|e| e.to_string())?.into_iter().map(tell).collect();

    Ok(Story { file: link(&hash), chain, derivatives })
}

/// Where saved recipes live: one file, beside the presets, editable by hand.
fn chains_path(dir: &Path) -> PathBuf {
    dir.join("recipes.json")
}

fn load_chains(dir: &Path) -> Vec<mt_core::chain::Chain> {
    std::fs::read_to_string(chains_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

#[tauri::command]
async fn recipes(state: State<'_, AppState>) -> Result<Vec<mt_core::chain::Chain>, String> {
    Ok(load_chains(&state.preset_dir))
}

/// Add or replace a recipe.
#[tauri::command]
async fn save_recipe(
    state: State<'_, AppState>,
    chain: mt_core::chain::Chain,
) -> Result<Vec<mt_core::chain::Chain>, String> {
    // Checked before it is written: a recipe that cannot work is better
    // refused here than at the moment someone drops forty files on it.
    chain.check().map_err(|e| e.to_string())?;

    let mut all = load_chains(&state.preset_dir);
    match all.iter().position(|c| c.id == chain.id) {
        Some(at) => all[at] = chain,
        None => all.push(chain),
    }
    write_chains(&state.preset_dir, &all)?;
    Ok(all)
}

#[tauri::command]
async fn delete_recipe(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<mt_core::chain::Chain>, String> {
    let mut all = load_chains(&state.preset_dir);
    all.retain(|c| c.id != id);
    write_chains(&state.preset_dir, &all)?;
    Ok(all)
}

fn write_chains(dir: &Path, chains: &[mt_core::chain::Chain]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(chains).map_err(|e| e.to_string())?;
    std::fs::write(chains_path(dir), json).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRecipeRequest {
    chain: mt_core::chain::Chain,
    files: Vec<BatchFileDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchFileDto {
    name: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecipeRun {
    outcomes: Vec<mt_core::batch::ItemOutcome>,
    paths: Vec<String>,
    /// The steps the last file went through, as a readable list.
    steps: Vec<String>,
    done: u32,
    failed: u32,
}

/// Run a recipe over the files handed in.
///
/// One file per call would be tidier, but a recipe is usually run on a few
/// files at once and each one is small by the time it is saved — the pile that
/// needs the one-at-a-time treatment is Batch, which has it.
#[tauri::command]
async fn run_recipe(
    state: State<'_, AppState>,
    request: RunRecipeRequest,
) -> Result<RecipeRun, String> {
    let started = Instant::now();
    request.chain.check().map_err(|e| e.to_string())?;

    // Only read when a step needs one, and said plainly when there is none.
    let font = if request.chain.needs_font() { Some(system_font()?) } else { None };

    let mut outcomes = Vec::new();
    let mut paths = Vec::new();
    let mut written = Vec::new();
    let mut steps = Vec::new();

    for file in &request.files {
        match mt_core::chain::apply(&file.bytes, &request.chain, font.as_deref()) {
            Ok(applied) => {
                let name = rename_to(&file.name, &applied.extension);
                let path = write_output(&state, &name, &applied.extension, &applied.bytes)?;
                paths.push(path.display().to_string());
                written.push((path, applied.bytes.clone()));
                steps = applied.steps.clone();
                outcomes.push(mt_core::batch::ItemOutcome::Done(mt_core::batch::Produced {
                    name,
                    before: file.bytes.len() as u64,
                    after: applied.bytes.len() as u64,
                    extension: applied.extension,
                    note: applied
                        .short
                        .clone()
                        .unwrap_or_else(|| applied.steps.len().to_string() + " steps"),
                    short: applied.short.is_some(),
                }));
            }
            Err(e) => outcomes.push(mt_core::batch::ItemOutcome::Failed {
                name: file.name.clone(),
                why: e.to_string(),
            }),
        }
    }

    let done = outcomes.iter().filter(|o| o.is_done()).count() as u32;
    let failed = outcomes.iter().filter(|o| o.is_failure()).count() as u32;

    if !written.is_empty() {
        let _ = remember_run(
            &state,
            Recorded {
                tool: "recipe",
                recipe: serde_json::json!({ "tool": "recipe", "chain": request.chain }),
                inputs: Vec::new(),
                outputs: written
                    .iter()
                    .map(|(path, bytes)| (path.clone(), bytes.as_slice()))
                    .collect(),
                summary: format!(
                    "{} on {} file{} — {done} done, {failed} failed",
                    request.chain.name,
                    request.files.len(),
                    if request.files.len() == 1 { "" } else { "s" }
                ),
                duration_ms: started.elapsed().as_millis() as i64,
            },
        );
    }

    Ok(RecipeRun { outcomes, paths, steps, done, failed })
}

/// Give a file the extension the recipe ended with.
fn rename_to(name: &str, extension: &str) -> String {
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    format!("{stem}.{extension}")
}

/// Whether the right-click menu is installed, and what it offers.
#[tauri::command]
async fn shell_menu() -> Result<shell::MenuState, String> {
    Ok(shell::state())
}

/// Add or remove the right-click menu for this account.
#[tauri::command]
async fn set_shell_menu(on: bool) -> Result<shell::MenuState, String> {
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        shell::install(&exe)?;
    } else {
        shell::remove()?;
    }
    Ok(shell::state())
}

#[tauri::command]
async fn pdf_to_markdown(
    state: State<'_, AppState>,
    request: MarkdownRequest,
) -> Result<MarkdownResponse, String> {
    let started = Instant::now();
    let (markdown, report) =
        mt_core::engine::pdf::to_markdown(&request.bytes, &request.options)
            .map_err(|e| e.to_string())?;

    // Writing an empty file would look like success; a scan gets the report
    // and nothing else.
    if report.pages_with_text == 0 {
        return Ok(MarkdownResponse { markdown, report, output_path: None });
    }

    let path = write_output(&state, &request.name, "md", markdown.as_bytes())?;
    let _ = remember(
        &state,
        Recorded {
            tool: "pdf-to-markdown",
            recipe: serde_json::json!({ "tool": "pdf-to-markdown", "options": request.options }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), markdown.as_bytes())],
            summary: format!(
                "{} of {} pages had text, {} characters",
                report.pages_with_text, report.pages, report.characters
            ),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(MarkdownResponse { markdown, report, output_path: Some(path.display().to_string()) })
}

/* ---------- repair ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepairRequest {
    bytes: Vec<u8>,
    name: String,
    /// Report what is wrong without writing anything.
    #[serde(default)]
    check: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairResponse {
    #[serde(flatten)]
    report: mt_core::engine::pdf::RepairReport,
    headline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    original_size: u64,
}

#[tauri::command]
async fn repair_pdf(
    state: State<'_, AppState>,
    request: RepairRequest,
) -> Result<RepairResponse, String> {
    let started = Instant::now();
    let original_size = request.bytes.len() as u64;

    if request.check {
        let report = mt_core::engine::pdf::diagnose(&request.bytes);
        return Ok(RepairResponse {
            headline: report.headline(),
            report,
            output_path: None,
            size: None,
            original_size,
        });
    }

    let (output, report) =
        mt_core::engine::pdf::repair(&request.bytes).map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &output)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "repair",
            recipe: serde_json::json!({ "tool": "repair" }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), output.as_slice())],
            summary: report.headline(),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(RepairResponse {
        headline: report.headline(),
        report,
        output_path: Some(path.display().to_string()),
        size: Some(output.len() as u64),
        original_size,
    })
}

/* ---------- image watermark ---------- */

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum ImageMarkDto {
    Text(String),
    Logo(Vec<u8>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageWatermarkRequest {
    bytes: Vec<u8>,
    name: String,
    mark: ImageMarkDto,
    options: mt_core::engine::image::ImageWatermarkOptions,
}

/// Read the first font this machine actually has.
///
/// `mt-core` keeps the candidate list but never opens a file, so the reading
/// happens here, where knowing what kind of machine this is belongs.
fn system_font() -> Result<Vec<u8>, String> {
    for path in mt_core::engine::image::watermark::FONT_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(bytes);
        }
    }
    Err("no usable font was found on this machine — use a logo instead of text".into())
}

#[tauri::command]
async fn watermark_image(
    state: State<'_, AppState>,
    request: ImageWatermarkRequest,
) -> Result<TransformResponse, String> {
    let started = Instant::now();
    let mark = match &request.mark {
        ImageMarkDto::Text(text) => {
            mt_core::engine::image::ImageMark::Text { text: text.clone(), font: system_font()? }
        }
        ImageMarkDto::Logo(data) => mt_core::engine::image::ImageMark::Logo(data.clone()),
    };

    let output =
        mt_core::engine::image::watermark::watermark(&request.bytes, &mark, &request.options)
            .map_err(|e| e.to_string())?;

    let kind = FileKind::sniff(&output);
    let (width, height) =
        image::load_from_memory(&output).map(|i| (i.width(), i.height())).unwrap_or((0, 0));
    let path = write_output(&state, &request.name, kind.extension(), &output)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "watermark-image",
            recipe: serde_json::json!({
                "tool": "watermark-image",
                "options": request.options,
                // The logo's bytes would bloat every history row, so the recipe
                // records only that there was one.
                "mark": match &request.mark {
                    ImageMarkDto::Text(text) => serde_json::json!({ "text": text }),
                    ImageMarkDto::Logo(_) => serde_json::json!({ "logo": true }),
                },
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), output.as_slice())],
            summary: format!("watermarked, {width}×{height}"),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(TransformResponse {
        output_path: path.display().to_string(),
        size: output.len() as u64,
        width,
        height,
        preview_url: data_url(kind, &output),
    })
}

/* ---------- crop ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CropRequest {
    bytes: Vec<u8>,
    name: String,
    #[serde(default)]
    pages: Option<String>,
    spec: mt_core::engine::pdf::CropSpec,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CropResponse {
    output_path: String,
    size: u64,
    #[serde(flatten)]
    report: mt_core::engine::pdf::CropReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[tauri::command]
async fn crop_pdf(
    state: State<'_, AppState>,
    request: CropRequest,
) -> Result<CropResponse, String> {
    let started = Instant::now();
    let total = mt_core::engine::pdf::page_count(&request.bytes).map_err(|e| e.to_string())?;

    let selection = match request.pages.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(spec) => Some(
            mt_core::engine::pdf::PageSelection::parse(spec, total).map_err(|e| e.to_string())?,
        ),
        None => None,
    };

    let (output, report) =
        mt_core::engine::pdf::crop(&request.bytes, selection.as_ref(), &request.spec)
            .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &output)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "crop-pdf",
            recipe: serde_json::json!({
                "tool": "crop-pdf",
                "pages": request.pages,
                "spec": request.spec,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), output.as_slice())],
            summary: format!("cropped {} of {total} pages", report.pages_cropped),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(CropResponse {
        output_path: path.display().to_string(),
        size: output.len() as u64,
        note: report.note(),
        report,
    })
}

/* ---------- page numbers ---------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NumberRequest {
    bytes: Vec<u8>,
    name: String,
    #[serde(default)]
    pages: Option<String>,
    options: mt_core::engine::pdf::PageNumberOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NumberResponse {
    output_path: String,
    size: u64,
    pages_numbered: u32,
    total_pages: u32,
}

#[tauri::command]
async fn number_pages(
    state: State<'_, AppState>,
    request: NumberRequest,
) -> Result<NumberResponse, String> {
    let started = Instant::now();
    let total = mt_core::engine::pdf::page_count(&request.bytes).map_err(|e| e.to_string())?;

    let selection = match request.pages.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(spec) => Some(
            mt_core::engine::pdf::PageSelection::parse(spec, total).map_err(|e| e.to_string())?,
        ),
        None => None,
    };

    let (output, numbered) =
        mt_core::engine::pdf::add_page_numbers(&request.bytes, selection.as_ref(), &request.options)
            .map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &output)?;

    let _ = remember(
        &state,
        Recorded {
            tool: "page-numbers",
            recipe: serde_json::json!({
                "tool": "page-numbers",
                "pages": request.pages,
                "options": request.options,
            }),
            inputs: vec![(request.name.clone(), request.bytes.as_slice())],
            outputs: vec![(path.clone(), output.as_slice())],
            summary: format!("numbered {numbered} of {total} pages"),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    );

    Ok(NumberResponse {
        output_path: path.display().to_string(),
        size: output.len() as u64,
        pages_numbered: numbered,
        total_pages: total,
    })
}

/* ---------- passwords and permissions ---------- */

// These two requests carry passwords, so they deliberately don't derive Debug:
// nothing should be able to print one into a log by accident.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProtectRequest {
    bytes: Vec<u8>,
    name: String,
    settings: mt_core::engine::pdf::Protection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnlockRequest {
    bytes: Vec<u8>,
    name: String,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityInfoDto {
    #[serde(flatten)]
    info: mt_core::engine::pdf::SecurityInfo,
    /// The same thing in a sentence, for the panel.
    summary: String,
}

impl From<mt_core::engine::pdf::SecurityInfo> for SecurityInfoDto {
    fn from(info: mt_core::engine::pdf::SecurityInfo) -> Self {
        let summary = info.summary();
        SecurityInfoDto { info, summary }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityResponse {
    output_path: String,
    size: u64,
    info: SecurityInfoDto,
    /// How the document was opened, for unlock only.
    #[serde(skip_serializing_if = "Option::is_none")]
    unlocked_by: Option<String>,
}

#[tauri::command]
async fn pdf_security(bytes: Vec<u8>) -> Result<SecurityInfoDto, String> {
    mt_core::engine::pdf::inspect_security(&bytes)
        .map(SecurityInfoDto::from)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn protect_pdf(
    state: State<'_, AppState>,
    request: ProtectRequest,
) -> Result<SecurityResponse, String> {
    let protected = mt_core::engine::pdf::protect(&request.bytes, &request.settings)
        .map_err(|e| e.to_string())?;
    let info = mt_core::engine::pdf::inspect_security(&protected).map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &protected)?;

    // Deliberately not recorded in history. Keeping a copy of a document
    // someone just put a password on — or the unprotected version of one they
    // unlocked — in a local store they didn't ask about is a nasty surprise.
    Ok(SecurityResponse {
        output_path: path.display().to_string(),
        size: protected.len() as u64,
        info: info.into(),
        unlocked_by: None,
    })
}

#[tauri::command]
async fn unlock_pdf(
    state: State<'_, AppState>,
    request: UnlockRequest,
) -> Result<SecurityResponse, String> {
    let (opened, kind) =
        mt_core::engine::pdf::unlock(&request.bytes, request.password.as_deref())
            .map_err(|e| e.to_string())?;
    let info = mt_core::engine::pdf::inspect_security(&opened).map_err(|e| e.to_string())?;
    let path = write_output(&state, &request.name, "pdf", &opened)?;

    Ok(SecurityResponse {
        output_path: path.display().to_string(),
        size: opened.len() as u64,
        info: info.into(),
        unlocked_by: Some(
            match kind {
                mt_core::engine::pdf::UnlockKind::RestrictionsOnly => "restrictions_only",
                mt_core::engine::pdf::UnlockKind::WithPassword => "password",
            }
            .into(),
        ),
    })
}

/// Write a result into Pictures / Kitbox under a name that won't clobber
/// anything already there.
fn write_output(
    state: &State<'_, AppState>,
    name: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let path = unique_path(&state.output_dir, name, ext).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

#[tauri::command]
fn recent_history(state: State<'_, AppState>, limit: u32) -> Result<Vec<HistoryEntry>, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;
    let rows = store.recent_outputs(limit).map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let last = store.lineage(&r.hash).ok().and_then(|chain| chain.last().cloned());
            HistoryEntry {
                hash: r.hash,
                name: r.name,
                kind: r.kind,
                size: r.size,
                created_at: r.created_at,
                summary: last.as_ref().map(|j| j.summary.clone()),
                // Worked out here rather than in the screen: whether a job can
                // be replayed depends on what was recorded, which is this
                // side's business.
                replay: last.as_ref().map(|j| recipe::inspect(&j.recipe)),
                tool: last.map(|j| j.tool),
                pinned: r.pinned,
                width: r.width,
                height: r.height,
                pages: r.pages,
                thumb_hash: r.thumb_hash,
            }
        })
        .collect())
}

/// The stored thumbnail for a history entry, as a data URL.
///
/// Kept as a separate call so the gallery listing stays small: a page of
/// entries is a few hundred bytes, and the pictures arrive as they are needed.
#[tauri::command]
async fn history_thumbnail(
    state: State<'_, AppState>,
    hash: String,
) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;
    match store.read(&hash) {
        Ok(bytes) => Ok(Some(data_url(FileKind::Jpeg, &bytes))),
        // A thumbnail that has been forgotten is not an error worth surfacing.
        Err(_) => Ok(None),
    }
}

#[tauri::command]
fn storage_usage(state: State<'_, AppState>) -> Result<UsageResponse, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;
    let u = store.usage().map_err(|e| e.to_string())?;
    Ok(UsageResponse { files: u.files, bytes: u.bytes, jobs: u.jobs })
}

#[tauri::command]
fn output_folder(state: State<'_, AppState>) -> String {
    state.output_dir.display().to_string()
}

/// One piece of finished work, as history should remember it.
///
/// Every tool records through here so the gallery sees all of them, not just
/// compression. Inputs are kept as well as outputs, which is what makes the
/// lineage view possible — and what makes "I already prepared that file last
/// month" a lookup instead of a redo.
struct Recorded<'a> {
    tool: &'a str,
    /// Enough to replay the operation later.
    recipe: serde_json::Value,
    /// Everything that went in. Lineage chains to the first.
    inputs: Vec<(String, &'a [u8])>,
    /// Everything that came out, with where it was written.
    outputs: Vec<(PathBuf, &'a [u8])>,
    summary: String,
    duration_ms: i64,
}

/// Store the inputs, the outputs and a job row per output.
///
/// A failure here must never break the work the user actually asked for, so
/// callers discard the error rather than surfacing it: a missing history entry
/// is a nuisance, a failed conversion is not.
/// Keep every output, but record the run as a single job.
///
/// `remember` writes one row per output, which is right for a tool that makes
/// one file and wrong for one that makes forty: a batch would fill the gallery
/// with identical rows and bury everything else. The files are all stored, so
/// nothing is lost — only the job row is folded into one.
fn remember_run(state: &State<'_, AppState>, work: Recorded<'_>) -> Result<String, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;

    let mut anchor: Option<String> = None;
    for (path, bytes) in &work.outputs {
        let meta =
            describe_file(&file_name(path), bytes, Some(path.display().to_string()), &store);
        let record = store.put(bytes, meta).map_err(|e| e.to_string())?;
        anchor.get_or_insert(record.hash);
    }
    let anchor = anchor.ok_or("the run produced nothing to record")?;

    store
        .record_job(NewJob {
            tool: work.tool.to_string(),
            recipe: work.recipe.to_string(),
            // A batch has no single input to chain from; the summary carries
            // how many files there were.
            input_hash: String::new(),
            output_hash: anchor.clone(),
            summary: work.summary.clone(),
            duration_ms: work.duration_ms,
        })
        .map_err(|e| e.to_string())?;

    Ok(anchor)
}

fn remember(state: &State<'_, AppState>, work: Recorded<'_>) -> Result<Vec<String>, String> {
    let store = state.store.lock().map_err(|_| "history is busy")?;

    let mut input_hashes = Vec::new();
    for (name, bytes) in &work.inputs {
        let record = store
            .put(bytes, describe_file(name, bytes, None, &store))
            .map_err(|e| e.to_string())?;
        input_hashes.push(record.hash);
    }
    // Something has to anchor the chain; with several inputs the first is it,
    // and the summary says how many there were.
    let anchor = input_hashes.first().cloned().unwrap_or_default();

    let mut output_hashes = Vec::new();
    for (path, bytes) in &work.outputs {
        let meta = describe_file(
            &file_name(path),
            bytes,
            Some(path.display().to_string()),
            &store,
        );
        let record = store.put(bytes, meta).map_err(|e| e.to_string())?;
        store
            .record_job(NewJob {
                tool: work.tool.to_string(),
                recipe: work.recipe.to_string(),
                input_hash: anchor.clone(),
                output_hash: record.hash.clone(),
                summary: work.summary.clone(),
                duration_ms: work.duration_ms,
            })
            .map_err(|e| e.to_string())?;
        output_hashes.push(record.hash);
    }

    Ok(output_hashes)
}

/// Metadata for a stored file, including a thumbnail where one can be made.
///
/// Without this the gallery is a list of filenames, which is no use for finding
/// the photo you prepared for a form three weeks ago.
fn describe_file(
    name: &str,
    bytes: &[u8],
    source_path: Option<String>,
    store: &Store,
) -> FileMeta {
    let kind = FileKind::sniff(bytes);
    let mut meta = FileMeta {
        name: name.to_string(),
        kind: kind.extension().into(),
        source_path,
        ..Default::default()
    };

    if kind == FileKind::Pdf {
        meta.pages = mt_core::engine::pdf::page_count(bytes).ok();
    } else if kind.is_image() {
        // Header only — decoding a 12-megapixel photo just for its dimensions
        // would be wasteful.
        if let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()
        {
            if let Ok((w, h)) = reader.into_dimensions() {
                meta.width = Some(w);
                meta.height = Some(h);
            }
        }
    }

    if let Some(thumb) = make_thumbnail(bytes, kind) {
        if let Ok(record) = store.put(
            &thumb,
            FileMeta {
                name: format!("{name}.thumb"),
                kind: "jpeg".into(),
                ..Default::default()
            },
        ) {
            meta.thumb_hash = Some(record.hash);
        }
    }

    meta
}

const THUMB_PX: u32 = 320;

/// A small JPEG for the gallery grid, or `None` when there's nothing to show —
/// a text-only PDF has no picture to make without a renderer.
fn make_thumbnail(bytes: &[u8], kind: FileKind) -> Option<Vec<u8>> {
    if kind == FileKind::Pdf {
        return mt_core::engine::pdf::page_preview(bytes, 1, THUMB_PX).ok().flatten();
    }
    if !kind.is_image() {
        return None;
    }
    let image = image::load_from_memory(bytes).ok()?;
    let scaled = if image.width().max(image.height()) > THUMB_PX {
        image.resize(THUMB_PX, THUMB_PX, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    mt_core::engine::pdf::images::encode_jpeg_for_pdf(&scaled, 72, false).ok()
}

fn data_url(kind: FileKind, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{};base64,{}", kind.mime(), b64)
}

fn file_name(path: &Path) -> String {
    path.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string()
}

/// `photo-kitbox.jpg`, then `photo-kitbox-2.jpg`, and so on. Overwriting a file
/// the user compressed five minutes ago would be a nasty surprise.
fn unique_path(dir: &Path, source_name: &str, ext: &str) -> std::io::Result<PathBuf> {
    let stem = Path::new(source_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    let mut candidate = dir.join(format!("{stem}-kitbox.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-kitbox-{n}.{ext}"));
        n += 1;
        if n > 9999 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "too many files with that name",
            ));
        }
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact payload `ui/src/lib/bridge.ts` sends. If either side drifts,
    /// this fails here rather than silently at runtime in the window.
    #[test]
    fn deserialises_the_request_the_ui_sends() {
        let json = r#"{
            "bytes": [255, 216, 255],
            "name": "application-photo.jpg",
            "target": { "kind": "range", "value": { "min": 50000, "max": 100000 } },
            "bias": "keep_sharp",
            "format": "keep",
            "stripMetadata": true,
            "exact": { "width": 200, "height": 230 }
        }"#;
        let req: CompressRequest = serde_json::from_str(json).expect("request shape");
        assert_eq!(req.name, "application-photo.jpg");
        assert_eq!(req.target.min_bytes(), 50_000);
        assert_eq!(req.target.max_bytes(), 100_000);
        assert_eq!(req.bias, Bias::KeepSharp);
        assert_eq!(req.format, OutputFormat::Keep);
        assert!(req.strip_metadata);
        let exact = req.exact.expect("exact dimensions");
        assert_eq!((exact.width, exact.height), (200, 230));
    }

    #[test]
    fn target_variants_all_cross_the_bridge() {
        for (json, min, max) in [
            (r#"{"kind":"at_most","value":100000}"#, 0u64, 100_000u64),
            (r#"{"kind":"range","value":{"min":20000,"max":50000}}"#, 20_000, 50_000),
            (r#"{"kind":"approx","value":{"bytes":100000,"tol":0.1}}"#, 90_000, 110_000),
        ] {
            let t: SizeTarget = serde_json::from_str(json).expect(json);
            assert_eq!(t.min_bytes(), min, "{json}");
            assert_eq!(t.max_bytes(), max, "{json}");
        }
    }

    #[test]
    fn response_uses_the_keys_the_ui_reads() {
        let res = CompressResponse {
            status: "hit".into(),
            name: "a.jpg".into(),
            original_size: 1000,
            size: 100,
            knob: "quality 68".into(),
            score: Some(0.98),
            probes: 5,
            elapsed_ms: 42,
            output_path: Some("C:/x/a-kitbox.jpg".into()),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::to_value(&res).unwrap();
        for key in ["status", "name", "originalSize", "size", "knob", "score", "probes", "elapsedMs", "outputPath"] {
            assert!(v.get(key).is_some(), "missing {key} in {v}");
        }
        // Absent fields are omitted rather than sent as null, so the optional
        // chaining on the TypeScript side behaves.
        assert!(v.get("why").is_none());
    }

    #[test]
    fn pdf_options_cross_the_bridge() {
        let json = r#"{
            "bytes": [37, 80, 68, 70],
            "name": "scan.pdf",
            "target": { "kind": "at_most", "value": 100000 },
            "bias": "keep_sharp",
            "format": "keep",
            "stripMetadata": true,
            "allowGrayscale": true,
            "minDpi": 96
        }"#;
        let req: CompressRequest = serde_json::from_str(json).expect("pdf request shape");
        assert!(req.allow_grayscale);
        assert_eq!(req.min_dpi, Some(96));
        // Image requests omit them entirely and must still parse.
        let without = r#"{
            "bytes": [], "name": "a.jpg",
            "target": { "kind": "at_most", "value": 1 },
            "bias": "balanced", "format": "jpeg", "stripMetadata": false
        }"#;
        let req: CompressRequest = serde_json::from_str(without).expect("image request shape");
        assert!(!req.allow_grayscale);
        assert_eq!(req.min_dpi, None);
    }

    #[test]
    fn the_inspector_report_reaches_the_ui_intact() {
        // A real PDF, inspected and shaped for the UI.
        let pdf = sample_pdf();
        let report = mt_core::engine::pdf::inspect(&pdf).expect("inspect");
        let dto = ReportDto::from(&report);
        let v = serde_json::to_value(&dto).unwrap();
        for key in [
            "pages",
            "totalBytes",
            "imageBytes",
            "fontBytes",
            "contentBytes",
            "metadataBytes",
            "otherBytes",
            "kind",
            "headline",
            "images",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["pages"], 1);
        assert_eq!(v["kind"], "scan");
        let image = &v["images"][0];
        assert!(image["dpi"].as_f64().unwrap() > 1.0, "DPI should be resolved");
        assert_eq!(image["recompressible"], true);
    }

    /// A one-page PDF holding a single JPEG, built without pulling in the
    /// core's test-only helpers.
    fn sample_pdf() -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};

        let mut rgb = image::RgbImage::new(600, 800);
        for (x, y, p) in rgb.enumerate_pixels_mut() {
            *p = image::Rgb([(x % 255) as u8, (y % 255) as u8, 128]);
        }
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
            .encode_image(&rgb)
            .unwrap();

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let image_id = doc.add_object(
            Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Image",
                    "Width" => 600i64, "Height" => 800i64,
                    "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8i64,
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
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1i64,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.compress();
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn page_operations_cross_the_bridge() {
        let json = r#"{
            "bytes": [37, 80, 68, 70],
            "name": "scan.pdf",
            "ops": [
                { "page": 3, "rotate": 90 },
                { "page": 1, "rotate": 0 },
                { "page": 1, "rotate": 180 }
            ]
        }"#;
        let req: ApplyPagesRequest = serde_json::from_str(json).expect("apply shape");
        assert_eq!(req.ops.len(), 3);
        assert_eq!(req.ops[0], PageOp { page: 3, rotate: 90 });
        // A page may legitimately appear twice — that duplicates it.
        assert_eq!(req.ops[2].page, 1);
    }

    #[test]
    fn split_modes_cross_the_bridge() {
        let every_page: SplitMode =
            serde_json::from_str(r#"{"kind":"every_page"}"#).expect("every_page");
        assert_eq!(every_page, SplitMode::EveryPage);

        let every_n: SplitMode =
            serde_json::from_str(r#"{"kind":"every_n","value":3}"#).expect("every_n");
        assert_eq!(every_n, SplitMode::EveryN(3));

        let ranges: SplitMode =
            serde_json::from_str(r#"{"kind":"ranges","value":[[1,3],[5,8]]}"#).expect("ranges");
        assert_eq!(ranges, SplitMode::Ranges(vec![(1, 3), (5, 8)]));
    }

    #[test]
    fn page_summaries_reach_the_ui_as_camel_case() {
        let pdf = sample_pdf();
        let pages = mt_core::engine::pdf::page_summaries(&pdf).unwrap();
        let v = serde_json::to_value(&pages).unwrap();
        let first = &v[0];
        for key in ["number", "widthPt", "heightPt", "rotation", "label", "hasPreview"] {
            assert!(first.get(key).is_some(), "missing {key} in {first}");
        }
        assert_eq!(first["number"], 1);
        assert_eq!(first["hasPreview"], true, "a full-page scan should be previewable");
    }

    /// Every `invoke("...")` in the UI must name a command this crate
    /// registers.
    ///
    /// This is the one failure the other tests can't see: the request and
    /// response shapes can be perfect and the call still goes nowhere because
    /// the name was renamed on one side only. It shows up as a dead button in
    /// the window, which is exactly the thing that is awkward to catch by hand.
    /// Both lists are parsed from source, so neither can drift.
    /// The window has to load the frontend from inside the binary.
    ///
    /// Tauri sets `cfg(dev)` to the opposite of the `custom-protocol`
    /// feature. Without that feature a built app points at the dev server and
    /// opens a connection-refused page — and nothing about the build says so:
    /// it compiles, it launches, the process sits there looking healthy.
    /// Checking that a process exists is not checking that the app works, so
    /// the build checks it instead.
    #[test]
    fn a_built_app_embeds_its_frontend_rather_than_asking_for_a_dev_server() {
        assert!(
            !cfg!(dev),
            "this build points the window at the dev server. Build with the              custom-protocol feature (it is on by default) or use `npm run app:build`."
        );
    }

    /* ---------- the screens built while the app shell was broken ----------
     *
     * Every screen below was typechecked but never once run, because a build
     * mistake meant the window loaded a dev server that wasn't there. The
     * TypeScript side proves nothing about these: `invoke` takes a plain
     * object, serde reads it with its own rules, and a name that doesn't match
     * is not an error — the field is simply dropped and takes its default.
     * A setting that silently does nothing is the result.
     *
     * So each of these pins the literal JSON the bridge builds against the
     * type that receives it, and the response keys against what the screen
     * reads back.
     */

    #[test]
    fn the_redaction_screen_can_ask_for_a_case_sensitive_match() {
        // This is the one that was broken. `rename_all = "snake_case"` on an
        // enum renames its *variants*, not the fields inside them, so the
        // `matchCase` the screen sends found no home and the `#[serde(default)]`
        // quietly supplied `false`. Ticking "Match capitals exactly" did
        // nothing at all, and nothing anywhere said so.
        let json = r#"{
            "targets": [
                { "kind": "text", "needle": "Secret", "matchCase": true },
                { "kind": "area", "page": 2, "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.05 }
            ],
            "drawBoxes": true,
            "boxColour": [0, 0, 0],
            "stripExtras": true
        }"#;
        let options: mt_core::engine::pdf::RedactOptions =
            serde_json::from_str(json).expect("redaction options");

        assert_eq!(options.targets.len(), 2);
        match &options.targets[0] {
            mt_core::engine::pdf::RedactTarget::Text { needle, match_case } => {
                assert_eq!(needle, "Secret");
                assert!(match_case, "the screen asked for a case-sensitive match");
            }
            other => panic!("expected a text target, got {other:?}"),
        }
        match &options.targets[1] {
            mt_core::engine::pdf::RedactTarget::Area { page, width, .. } => {
                assert_eq!(*page, 2);
                assert!((width - 0.3).abs() < 1e-6);
            }
            other => panic!("expected an area target, got {other:?}"),
        }
        assert!(options.draw_boxes);
        assert!(options.strip_extras);
    }

    #[test]
    fn the_scan_screen_sends_options_the_core_understands() {
        let json = r#"{ "mode": "document", "detectEdges": true, "strength": 0.6 }"#;
        let options: mt_core::engine::image::ScanOptions =
            serde_json::from_str(json).expect("scan options");
        assert_eq!(options.mode, mt_core::engine::image::ScanMode::Document);
        assert!(options.detect_edges, "the edge-finding switch has to arrive");
        assert!((options.strength - 0.6).abs() < 1e-6);

        // And the other two looks the screen offers.
        for (sent, expected) in [
            ("colour", mt_core::engine::image::ScanMode::Colour),
            ("greyscale", mt_core::engine::image::ScanMode::Greyscale),
        ] {
            let json = format!(
                r#"{{ "mode": "{sent}", "detectEdges": false, "strength": 0.5 }}"#
            );
            let options: mt_core::engine::image::ScanOptions =
                serde_json::from_str(&json).expect("scan options");
            assert_eq!(options.mode, expected);
            assert!(!options.detect_edges);
        }
    }

    #[test]
    fn the_signing_screen_sends_spots_and_a_caption() {
        let json = r#"{
            "spots": [{ "page": 1, "x": 0.3, "y": 0.85, "width": 0.28 }],
            "caption": {
                "text": "Signed {date}",
                "date": "12 November 2026",
                "size": 9,
                "colour": [45, 45, 54]
            }
        }"#;
        let options: mt_core::engine::pdf::SignOptions =
            serde_json::from_str(json).expect("sign options");
        assert_eq!(options.spots.len(), 1);
        assert_eq!(options.spots[0].page, 1);
        assert!((options.spots[0].width - 0.28).abs() < 1e-6);
        let caption = options.caption.expect("the caption should arrive");
        assert_eq!(caption.date, "12 November 2026");

        // The screen sends null when the box is unticked.
        let bare = r#"{ "spots": [{ "page": 2, "x": 0.5, "y": 0.5, "width": 0.2 }], "caption": null }"#;
        let options: mt_core::engine::pdf::SignOptions =
            serde_json::from_str(bare).expect("sign options without a caption");
        assert!(options.caption.is_none());
    }

    #[test]
    fn the_signature_cutout_options_cross_intact() {
        let json = r#"{ "ink": "blue", "strength": 0.5, "trim": true }"#;
        let options: mt_core::engine::image::SignatureOptions =
            serde_json::from_str(json).expect("signature options");
        assert_eq!(options.ink, mt_core::engine::image::InkColour::Blue);
        assert!(options.trim);
    }

    #[test]
    fn the_form_screen_sends_values_by_field_name() {
        let json = r#"{
            "values": { "applicant_name": "Rupam Biswas", "agree": "yes" },
            "flatten": true
        }"#;
        let options: mt_core::engine::pdf::FillOptions =
            serde_json::from_str(json).expect("fill options");
        assert_eq!(options.values.get("applicant_name").map(String::as_str), Some("Rupam Biswas"));
        assert!(options.flatten, "the lock-in switch has to arrive");
    }

    #[test]
    fn every_mark_the_drawing_screen_can_make_crosses_the_bridge() {
        // The screen builds these seven shapes; each has to land as itself.
        let json = r#"{
            "marks": [
                { "page": 1, "x": 0.1, "y": 0.1, "width": 0.3, "height": 0.04,
                  "mark": { "kind": "highlight" },
                  "colour": [255, 213, 74], "lineWidth": 1.5, "opacity": 1 },
                { "page": 1, "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.06,
                  "mark": { "kind": "text", "text": "Approved", "size": 11 },
                  "colour": [194, 65, 12], "lineWidth": 1.5, "opacity": 1 },
                { "page": 1, "x": 0.1, "y": 0.3, "width": 0.2, "height": 0.03,
                  "mark": { "kind": "whiteout" },
                  "colour": [32, 32, 38], "lineWidth": 1.5, "opacity": 1 },
                { "page": 1, "x": 0.1, "y": 0.4, "width": 0.2, "height": 0.1,
                  "mark": { "kind": "rectangle", "filled": false },
                  "colour": [29, 78, 216], "lineWidth": 2, "opacity": 1 },
                { "page": 1, "x": 0.1, "y": 0.5, "width": 0.2, "height": 0.1,
                  "mark": { "kind": "ellipse", "filled": false },
                  "colour": [15, 118, 110], "lineWidth": 2, "opacity": 1 },
                { "page": 1, "x": 0.1, "y": 0.6, "width": 0.2, "height": 0.1,
                  "mark": { "kind": "line", "arrow": true },
                  "colour": [179, 38, 30], "lineWidth": 2, "opacity": 1 },
                { "page": 1, "x": 0, "y": 0, "width": 1, "height": 1,
                  "mark": { "kind": "ink", "points": [[0.1, 0.1], [0.2, 0.2], [0.3, 0.15]] },
                  "colour": [32, 32, 38], "lineWidth": 2, "opacity": 1 }
            ]
        }"#;
        let options: mt_core::engine::pdf::AnnotateOptions =
            serde_json::from_str(json).expect("annotate options");
        assert_eq!(options.marks.len(), 7);

        use mt_core::engine::pdf::AnnotationMark as Mark;
        assert!(matches!(options.marks[0].mark, Mark::Highlight));
        match &options.marks[1].mark {
            Mark::Text { text, size } => {
                assert_eq!(text, "Approved");
                assert!((size - 11.0).abs() < 1e-6);
            }
            other => panic!("expected text, got {other:?}"),
        }
        assert!(matches!(options.marks[2].mark, Mark::Whiteout));
        assert!(matches!(options.marks[3].mark, Mark::Rectangle { filled: false }));
        assert!(matches!(options.marks[4].mark, Mark::Ellipse { filled: false }));
        assert!(matches!(options.marks[5].mark, Mark::Line { arrow: true }));
        match &options.marks[6].mark {
            Mark::Ink { points } => assert_eq!(points.len(), 3),
            other => panic!("expected ink, got {other:?}"),
        }

        // The stroke width is the one camelCase field here.
        assert!((options.marks[3].line_width - 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_ocr_screen_sends_its_switches() {
        let json = r#"{ "pages": null, "skipPagesWithText": false, "maxPx": 2000 }"#;
        let options: mt_core::engine::pdf::OcrOptions =
            serde_json::from_str(json).expect("ocr options");
        assert!(
            !options.skip_pages_with_text,
            "'read every page' has to actually turn the skipping off"
        );
        assert_eq!(options.max_px, 2000);
        assert!(options.pages.is_none());
    }

    #[test]
    fn the_html_screen_sends_all_three_kinds_of_source() {
        for (json, expect) in [
            (r#"{ "kind": "url", "value": "https://example.com" }"#, "https://example.com"),
            (r#"{ "kind": "file", "value": "C:/pages/invoice.html" }"#, "C:/pages/invoice.html"),
            (r#"{ "kind": "html", "value": "<h1>hi</h1>" }"#, "<h1>hi</h1>"),
        ] {
            let source: html::Source = serde_json::from_str(json).expect("html source");
            let got = match &source {
                html::Source::Url(v) | html::Source::File(v) | html::Source::Html(v) => v.clone(),
            };
            assert_eq!(got, expect);
        }
    }

    #[test]
    fn the_html_page_setup_crosses_with_its_camel_case_fields() {
        let json = r#"{
            "size": "letter",
            "landscape": true,
            "margin": 0.4,
            "backgrounds": true,
            "scale": 1,
            "headerAndFooter": true,
            "settleMs": 600
        }"#;
        let setup: html::PageSetup = serde_json::from_str(json).expect("page setup");
        assert_eq!(setup.size, "letter");
        assert!(setup.landscape);
        assert!(setup.header_and_footer, "the margins switch has to arrive");
        assert_eq!(setup.settle_ms, 600);
    }

    #[test]
    fn the_screens_read_the_keys_the_responses_actually_carry() {
        // Serialising the other way: a renamed field here shows up as
        // `undefined` in the screen, which renders as a blank or a crash
        // rather than an error anyone can act on.
        let redact = serde_json::to_value(RedactResponse {
            output_path: "out.pdf".into(),
            pages_changed: vec![1],
            glyphs_removed: 20,
            images_edited: 1,
            annotations_removed: 0,
            verified: true,
            leaks: Vec::new(),
            warnings: Vec::new(),
            note: "n".into(),
            size: 10,
        })
        .unwrap();
        for key in [
            "outputPath",
            "pagesChanged",
            "glyphsRemoved",
            "imagesEdited",
            "annotationsRemoved",
            "verified",
            "leaks",
            "warnings",
            "note",
            "size",
        ] {
            assert!(redact.get(key).is_some(), "the redaction screen reads '{key}'");
        }

        let sign = serde_json::to_value(SignResponse {
            output_path: "out.pdf".into(),
            pages_signed: vec![1],
            placements: 1,
            size: 10,
            note: "n".into(),
            warnings: Vec::new(),
        })
        .unwrap();
        for key in ["outputPath", "pagesSigned", "placements", "size", "note", "warnings"] {
            assert!(sign.get(key).is_some(), "the signing screen reads '{key}'");
        }

        let fill = serde_json::to_value(FillFormResponse {
            output_path: "out.pdf".into(),
            filled: vec!["a".into()],
            unknown: Vec::new(),
            read_only: Vec::new(),
            still_required: Vec::new(),
            flattened: true,
            warnings: Vec::new(),
            size: 10,
        })
        .unwrap();
        for key in
            ["outputPath", "filled", "unknown", "readOnly", "stillRequired", "flattened", "size"]
        {
            assert!(fill.get(key).is_some(), "the form screen reads '{key}'");
        }

        let scan = serde_json::to_value(ScanResponse {
            output_path: "out.pdf".into(),
            pages: 2,
            flattened: 1,
            whole_frame: vec![2],
            size: 10,
            note: None,
        })
        .unwrap();
        for key in ["outputPath", "pages", "flattened", "wholeFrame", "size"] {
            assert!(scan.get(key).is_some(), "the scan screen reads '{key}'");
        }

        let annotate = serde_json::to_value(AnnotateResponse {
            output_path: "out.pdf".into(),
            drawn: 3,
            pages_changed: vec![1],
            warnings: Vec::new(),
            note: "n".into(),
            size: 10,
        })
        .unwrap();
        for key in ["outputPath", "drawn", "pagesChanged", "warnings", "note", "size"] {
            assert!(annotate.get(key).is_some(), "the drawing screen reads '{key}'");
        }

        let ocr = serde_json::to_value(OcrResponse {
            output_path: Some("out.pdf".into()),
            pages: Vec::new(),
            pages_read: vec![1],
            pages_with_text: Vec::new(),
            pages_not_readable: Vec::new(),
            words: 41,
            note: "n".into(),
            warnings: Vec::new(),
            size: 10,
        })
        .unwrap();
        for key in
            ["outputPath", "pages", "pagesRead", "pagesWithText", "pagesNotReadable", "words"]
        {
            assert!(ocr.get(key).is_some(), "the OCR screen reads '{key}'");
        }

        let models = serde_json::to_value(ModelStatus {
            ready: false,
            bytes: 12_212_656,
            folder: "C:/models".into(),
            sources: vec!["https://example".into()],
        })
        .unwrap();
        for key in ["ready", "bytes", "folder", "sources"] {
            assert!(models.get(key).is_some(), "the OCR screen reads '{key}'");
        }

        let html_out = serde_json::to_value(HtmlResponse {
            output_path: "out.pdf".into(),
            pages: 1,
            source: "https://example.com".into(),
            size: 10,
            warnings: Vec::new(),
            note: "n".into(),
        })
        .unwrap();
        for key in ["outputPath", "pages", "source", "size", "warnings", "note"] {
            assert!(html_out.get(key).is_some(), "the web-page screen reads '{key}'");
        }
    }

    #[test]
    fn a_signature_preview_carries_what_the_screen_draws() {
        let preview = serde_json::to_value(SignaturePreview {
            image: "data:image/png;base64,AAA".into(),
            found_ink: true,
            width: 824,
            height: 270,
            coverage: 0.035,
            note: None,
        })
        .unwrap();
        for key in ["image", "foundInk", "width", "height", "coverage"] {
            assert!(preview.get(key).is_some(), "the signing screen reads '{key}'");
        }
        // `note` is skipped when empty, which the screen handles as optional.
        assert!(preview.get("note").is_none());

        let scan = serde_json::to_value(ScanPreview {
            image: "data:image/jpeg;base64,AAA".into(),
            edges_found: false,
            width: 100,
            height: 200,
        })
        .unwrap();
        for key in ["image", "edgesFound", "width", "height"] {
            assert!(scan.get(key).is_some(), "the scan screen reads '{key}'");
        }
    }

    #[test]
    fn a_form_field_reaches_the_screen_with_everything_it_renders() {
        let field = serde_json::to_value(mt_core::engine::pdf::Field {
            name: "applicant_name".into(),
            label: Some("Full name".into()),
            kind: mt_core::engine::pdf::FieldKind::Text,
            value: None,
            options: Vec::new(),
            required: true,
            read_only: false,
            max_length: Some(40),
            pages: vec![1],
        })
        .unwrap();
        for key in ["name", "label", "kind", "options", "required", "readOnly", "maxLength", "pages"]
        {
            assert!(field.get(key).is_some(), "the form screen reads '{key}'");
        }
        assert_eq!(field.get("kind").and_then(|v| v.as_str()), Some("text"));
    }

    #[test]
    fn every_command_the_ui_calls_is_registered() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let bridge_path = root.join("../../../ui/src/lib/bridge.ts");
        let Ok(bridge) = std::fs::read_to_string(&bridge_path) else {
            // The crate must still build and test without the UI checked out.
            eprintln!("skipping: no UI at {}", bridge_path.display());
            return;
        };
        let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("own source");

        let called = invoked_commands(&bridge);
        assert!(!called.is_empty(), "found no invoke() calls — the parser is wrong");
        let registered = registered_commands(&lib);
        assert!(!registered.is_empty(), "found no handlers — the parser is wrong");

        for command in &called {
            assert!(
                registered.contains(command),
                "the UI calls '{command}' but it is not in generate_handler!; registered: {registered:?}"
            );
        }
    }

    /// Names passed to `invoke(...)`, including the generic form
    /// `invoke<Result>("name", …)`.
    fn invoked_commands(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find("invoke") {
            rest = &rest[at + "invoke".len()..];
            // Skip the generic parameters and whitespace to the opening quote,
            // but don't run past the call itself.
            let Some(open) = rest.find('"') else { break };
            if rest[..open].contains(';') || rest[..open].contains('\n') && !rest[..open].contains('(')
            {
                continue;
            }
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            let name = &after[..close];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                out.push(name.to_string());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Names inside the `generate_handler!` list.
    fn registered_commands(source: &str) -> Vec<String> {
        // Assembled at runtime so this parser doesn't find its own source text
        // before the real macro call further down the file.
        let needle = concat!("generate", "_handler![");
        let Some(start) = source.find(needle) else { return Vec::new() };
        let body = &source[start + needle.len()..];
        let Some(end) = body.find(']') else { return Vec::new() };
        body[..end]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    fn sample_jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut rgb = image::RgbImage::new(w, h);
        for (x, y, p) in rgb.enumerate_pixels_mut() {
            *p = image::Rgb([(x % 255) as u8, (y % 255) as u8, 90]);
        }
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&rgb)
            .unwrap();
        out
    }

    #[test]
    fn stored_images_get_dimensions_and_a_thumbnail() {
        let (_dir, store) = temp_store();
        let bytes = sample_jpeg(1200, 800);
        let meta = describe_file("holiday.jpg", &bytes, None, &store);

        assert_eq!(meta.kind, "jpg");
        assert_eq!((meta.width, meta.height), (Some(1200), Some(800)));

        let hash = meta.thumb_hash.expect("an image should get a thumbnail");
        let thumb = image::load_from_memory(&store.read(&hash).unwrap()).unwrap();
        assert!(
            thumb.width().max(thumb.height()) <= THUMB_PX,
            "thumbnail should be small, got {}×{}",
            thumb.width(),
            thumb.height()
        );
    }

    #[test]
    fn stored_pdfs_get_a_page_count() {
        let (_dir, store) = temp_store();
        let meta = describe_file("scan.pdf", &sample_pdf(), None, &store);
        assert_eq!(meta.kind, "pdf");
        assert_eq!(meta.pages, Some(1));
        // The sample is a full-page scan, so it previews.
        assert!(meta.thumb_hash.is_some());
    }

    #[test]
    fn a_file_with_no_picture_simply_has_no_thumbnail() {
        // Not an image and not a PDF: nothing to show, and that must not be an
        // error — the entry still has to be stored.
        let (_dir, store) = temp_store();
        let meta = describe_file("notes.txt", b"just some text", None, &store);
        assert!(meta.thumb_hash.is_none());
        assert!(meta.width.is_none() && meta.pages.is_none());
        assert_eq!(meta.name, "notes.txt");
    }

    /// Every tool that writes a file should also record it. This walks the
    /// source looking for `write_output` calls that aren't accompanied by a
    /// `remember`, which is the way this regresses: a new tool gets added, the
    /// output lands on disk, and the gallery never hears about it.
    #[test]
    fn every_tool_that_writes_a_file_records_it() {
        let source = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        )
        .expect("own source");

        // Protect and unlock are deliberately excluded: keeping a copy of a
        // document someone just encrypted, or the open version of one they
        // unlocked, is a privacy problem rather than a feature.
        //
        // `batch_item` writes a file per call and is recorded by its partner
        // `batch_finish`, which is the point: a run of forty files is one
        // entry in the gallery, not forty. The assertion below still holds it
        // to that promise.
        let exempt = ["protect_pdf", "unlock_pdf", "batch_item"];

        for block in source.split("#[tauri::command]").skip(1) {
            let name = block
                .split("fn ")
                .nth(1)
                .and_then(|rest| rest.split('(').next())
                .unwrap_or("")
                .trim()
                .to_string();
            if !block.contains("write_output(") || exempt.contains(&name.as_str()) {
                continue;
            }
            // Either recorder counts: `remember` writes a row per output,
            // `remember_run` folds a whole run into one. What must never
            // happen is neither.
            assert!(
                block.contains("remember(") || block.contains("remember_run("),
                "'{name}' writes a file but never records it in history"
            );
        }

        // The one exemption that is a deferral rather than a refusal: if
        // nothing records the batch, the exemption above has quietly become a
        // hole.
        let finish = source
            .split("async fn batch_finish(")
            .nth(1)
            .expect("batch_finish still exists");
        assert!(
            finish.contains("remember_run("),
            "batch_item's outputs are recorded by batch_finish, and it no longer does"
        );
    }

    #[test]
    fn output_names_never_overwrite() {
        let dir = std::env::temp_dir().join("kitbox-unique-test");
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "photo.jpg", "jpg").unwrap();
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "photo.jpg", "jpg").unwrap();
        assert_ne!(first, second);
        assert!(second.to_string_lossy().contains("photo-kitbox-2"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

pub mod html;
pub mod presets;
pub mod recipe;
pub mod shell;
pub mod hotkey;
pub mod watch;


/// Render one page and quit, for the `--render` flag.
fn render_once(target: String, out: String) {
    let source = if target.starts_with("http://") || target.starts_with("https://") {
        html::Source::Url(target)
    } else {
        html::Source::File(target)
    };
    // The output's extension says which job this is: .pdf prints the page,
    // .png or .jpg photographs it. One flag, no mode to remember.
    let lower = out.to_lowercase();
    let as_image = lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg");
    let png = lower.ends_with(".png");

    let result = tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();
            // The work waits on callbacks the event loop delivers, so it
            // cannot run on the thread about to become that loop.
            std::thread::spawn(move || {
                let outcome: Result<(Vec<u8>, String, Vec<String>, String), String> = if as_image
                {
                    let setup = html::ShotSetup { png, ..Default::default() };
                    html::to_image(&handle, &source, &setup).map(|shot| {
                        (
                            shot.bytes,
                            format!(
                                "{} -> {} ({}×{})",
                                shot.report.source, out, shot.report.width, shot.report.height
                            ),
                            shot.report.warnings,
                            shot.report.note,
                        )
                    })
                } else {
                    html::to_pdf(&handle, &source, &html::PageSetup::default()).map(|rendered| {
                        (
                            rendered.bytes,
                            format!(
                                "{} -> {} ({} page{})",
                                rendered.report.source,
                                out,
                                rendered.report.pages,
                                if rendered.report.pages == 1 { "" } else { "s" }
                            ),
                            rendered.report.warnings,
                            rendered.report.note,
                        )
                    })
                };

                let code = match outcome {
                    Ok((bytes, summary, warnings, note)) => match std::fs::write(&out, &bytes) {
                        Ok(()) => {
                            println!("{summary}, {}", format_size(bytes.len() as u64));
                            for warning in &warnings {
                                println!("  warning: {warning}");
                            }
                            println!("  {note}");
                            0
                        }
                        Err(e) => {
                            eprintln!("could not write {out}: {e}");
                            1
                        }
                    },
                    Err(e) => {
                        eprintln!("{e}");
                        1
                    }
                };
                // Nothing else is running; leaving the loop spinning would
                // hang a script forever.
                std::process::exit(code);
            });
            Ok(())
        })
        .run(tauri::generate_context!());

    if let Err(e) = result {
        eprintln!("could not start the renderer: {e}");
        std::process::exit(1);
    }
}

pub fn run() {
    // `kitbox-desktop --render <url-or-file> <out.pdf>` renders one page and
    // exits.
    //
    // Rendering is the one tool the `kitbox` CLI cannot carry: it needs a real
    // browser engine and an event loop, and the CLI has neither. Rather than
    // leave it clickable-only — unscriptable, and untestable without a person
    // in front of it — the app itself takes the job from the command line.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--render") {
        let Some(target) = args.get(at + 1) else {
            eprintln!("usage: kitbox-desktop --render <url-or-file> <out.pdf>");
            std::process::exit(2);
        };
        let Some(out) = args.get(at + 2) else {
            eprintln!("usage: kitbox-desktop --render <url-or-file> <out.pdf>");
            std::process::exit(2);
        };
        render_once(target.clone(), out.clone());
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Explorer runs the verb once per selected file. Without this, forty
        // right-clicked photos would be forty copies of the app.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            accept_shell_args(app, &argv);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = Store::open(&data_dir)?;

            let output_dir = app
                .path()
                .picture_dir()
                .or_else(|_| app.path().document_dir())
                .unwrap_or_else(|_| data_dir.clone())
                .join("Kitbox");
            std::fs::create_dir_all(&output_dir)?;

            let model_dir = data_dir.join("models");

            let preset_dir = data_dir.join("presets");
            let preset_dir_for_hotkey = preset_dir.clone();
            // Writing the reference copy and the README every start keeps the
            // folder true to the build, and costs two small files.
            if let Err(e) = presets::prepare(&preset_dir) {
                eprintln!("presets: the folder could not be prepared: {e}");
            }

            app.manage(AppState {
                store: Mutex::new(store),
                output_dir,
                model_dir,
                preset_dir,
            });

            // An installed menu names an exe; if this is a different copy of
            // Kitbox, the menu should run this one rather than whatever path
            // it was set up with.
            if let Ok(exe) = std::env::current_exe() {
                if let Err(e) = shell::refresh(&exe) {
                    eprintln!("shell: the right-click menu could not be updated: {e}");
                }
            }

            // The clipboard key, if it was asked for. A combination that is
            // no longer free is worth saying out loud rather than leaving the
            // screen claiming a key that does nothing.
            let settings = hotkey::load(&preset_dir_for_hotkey);
            if let Err(e) = listen_for_hotkey(app.handle(), &settings) {
                eprintln!("hotkey: {e}");
            }

            // And the folders it was asked to keep an eye on.
            let watches = watch::load(&preset_dir_for_hotkey);
            if let Err(e) = start_watching(app.handle(), &watches) {
                eprintln!("watch: {e}");
            }

            // This instance may itself have been started by the menu.
            let argv: Vec<String> = std::env::args().collect();
            accept_shell_args(app.handle(), &argv);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            compress_file,
            inspect_pdf,
            pdf_pages,
            pdf_previews,
            apply_pages,
            split_pdf,
            merge_pdfs,
            images_to_pdf,
            pdf_to_images,
            transform_image,
            watermark_pdf,
            number_pages,
            crop_pdf,
            watermark_image,
            repair_pdf,
            pdf_to_markdown,
            scan_preview,
            scan_to_pdf,
            signature_preview,
            save_signature,
            sign_pdf,
            redact_pdf,
            form_fields,
            fill_form,
            annotate_pdf,
            models_ready,
            fetch_models,
            ocr_pdf,
            html_to_pdf,
            html_to_image,
            remove_background,
            upscale_image,
            blur_faces,
            adjust_image,
            pdf_to_archive,
            make_meme,
            rerun_recipe,
            forget_entry,
            set_pinned,
            entry_story,
            recipes,
            save_recipe,
            delete_recipe,
            run_recipe,
            watched_folders,
            set_watched_folders,
            clipboard_key,
            set_clipboard_key,
            use_clipboard,
            take_shell_run,
            shell_menu,
            set_shell_menu,
            tidy_history,
            count_missing,
            list_presets,
            save_preset,
            delete_preset,
            check_preset,
            fix_preset,
            batch_start,
            batch_item,
            batch_finish,
            cancel_batch,
            upscale_estimate,
            compare_pdfs,
            pdf_security,
            protect_pdf,
            unlock_pdf,
            recent_history,
            history_thumbnail,
            storage_usage,
            output_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kitbox");
}

/// Which tool a transform came from, so history distinguishes a resize from a
/// crop instead of lumping them together.
fn transform_name(op: &TransformOp) -> &'static str {
    match op {
        TransformOp::Resize(_) => "resize",
        TransformOp::Crop(_) | TransformOp::CropAspect(_) => "crop",
        TransformOp::Rotate { .. } => "rotate",
        TransformOp::Convert => "convert",
    }
}
