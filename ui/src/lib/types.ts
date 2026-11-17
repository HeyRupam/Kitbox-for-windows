export type Bias = "balanced" | "keep_sharp" | "shrink_pixels";
export type OutputFormat = "keep" | "jpeg" | "png";
export type TargetMode = "under" | "between" | "exactly";

/**
 * Mirrors `mt_core::SizeTarget`. The shape matches its serde representation
 * (tag `kind`, content `value`) so it crosses the bridge without translation.
 */
export type SizeTarget =
  | { kind: "at_most"; value: number }
  | { kind: "range"; value: { min: number; max: number } }
  | { kind: "approx"; value: { bytes: number; tol: number } };

export function targetMax(t: SizeTarget): number {
  if (t.kind === "at_most") return t.value;
  if (t.kind === "range") return t.value.max;
  return Math.round(t.value.bytes * (1 + t.value.tol));
}

export function targetMin(t: SizeTarget): number {
  if (t.kind === "at_most") return 0;
  if (t.kind === "range") return t.value.min;
  return Math.round(t.value.bytes * (1 - t.value.tol));
}

export function targetContains(t: SizeTarget, size: number): boolean {
  return size >= targetMin(t) && size <= targetMax(t);
}

export interface CompressRequest {
  /** Absolute path, when the file came from disk via the desktop shell. */
  path?: string;
  /** Raw bytes, when the file was dropped into the window. */
  bytes?: Uint8Array;
  name: string;
  target: SizeTarget;
  bias: Bias;
  format: OutputFormat;
  stripMetadata: boolean;
  maxDimension?: number;
  exact?: { width: number; height: number };
  /** PDF only: let the solver drop colour as a last resort. */
  allowGrayscale?: boolean;
  /** PDF only: resolution floor. */
  minDpi?: number;
}

export interface PdfImageInfo {
  width: number;
  height: number;
  bytes: number;
  dpi?: number;
  colorSpace: string;
  filter: string;
  recompressible: boolean;
  skipReason?: string;
}

/** The size inspector's answer to "why is this file big?". */
export interface PdfReport {
  pages: number;
  totalBytes: number;
  imageBytes: number;
  fontBytes: number;
  contentBytes: number;
  metadataBytes: number;
  otherBytes: number;
  kind: "scan" | "text_with_images" | "text_only";
  headline: string;
  images: PdfImageInfo[];
}

export interface CompressResult {
  status: "hit" | "padded" | "infeasible";
  name: string;
  originalSize: number;
  size: number;
  /** Settings that won, in words: "quality 68, 1600×1200". */
  knob: string;
  score?: number;
  probes: number;
  elapsedMs: number;
  note?: string;
  why?: string;
  suggestions?: string[];
  /** Object URL for the after-preview. */
  previewUrl?: string;
  /** Where it was written, when the desktop shell wrote it. */
  outputPath?: string;
  /** Hash in the local history, when it was recorded. */
  hash?: string;
  /** Present for PDFs: the byte breakdown of the result. */
  report?: PdfReport;
}

export interface HistoryEntry {
  hash: string;
  name: string;
  kind: string;
  size: number;
  createdAt: number;
  summary?: string;
  pinned: boolean;
  /** Which tool made it. */
  tool?: string;
  width?: number;
  height?: number;
  pages?: number;
  /** Fetch the picture with `historyThumbnail`. */
  thumbHash?: string;
  /** Whether this job's settings can be applied to a new file. */
  replay?: Replayable;
}

/** Whether a past job can be run again, and what to call it. */
export interface Replayable {
  can: boolean;
  /** Button text: "Compress another file to 50 KB–100 KB". */
  label: string;
  /** Why not, when it can't — shown instead of the button. */
  why: string | null;
  /** What it takes: "image", "pdf" or "either". */
  takes: string;
}

export interface RerunResult {
  summary: string;
  paths: string[];
}

/** What a tidy-up removed. */
export interface Swept {
  /** Entries whose file had gone. */
  forgotten: number;
  /** Stored copies nothing pointed at any more. */
  loose: number;
  freed: number;
}

export interface StorageUsage {
  files: number;
  bytes: number;
  jobs: number;
}

/** A page as the engine describes it, without rendering. */
export interface PageInfo {
  number: number;
  widthPt: number;
  heightPt: number;
  rotation: number;
  label: string;
  hasPreview: boolean;
}

/** One page of the finished document: which source page, turned how far. */
export interface PageOp {
  page: number;
  rotate: number;
}

export interface PageOpResult {
  outputPath: string;
  pagesBefore: number;
  pagesAfter: number;
  size: number;
  note?: string;
}

export interface SplitPartResult {
  outputPath: string;
  pages: number[];
  size: number;
}

export type SplitMode =
  | { kind: "every_page" }
  | { kind: "every_n"; value: number }
  | { kind: "ranges"; value: [number, number][] };

/* ---- images to PDF ---- */

export type PdfPageSize = "fit_image" | "a4" | "letter" | "legal";
export type PdfOrientation = "auto" | "portrait" | "landscape";

export interface ImagesToPdfOptions {
  pageSize: PdfPageSize;
  orientation: PdfOrientation;
  marginPt: number;
  jpegQuality: number;
  maxDpi: number | null;
}

/* ---- PDF to images ---- */

export interface ExportedPageResult {
  page: number;
  outputPath?: string;
  width: number;
  height: number;
  size: number;
  /** Why this page couldn't be exported. */
  reason?: string;
}

/* ---- image transforms ---- */

export type ResizeMode =
  | { kind: "stretch"; value: { width: number; height: number } }
  | { kind: "fit"; value: { width: number; height: number } }
  | { kind: "fill"; value: { width: number; height: number } }
  | { kind: "percent"; value: number }
  | { kind: "longest_edge"; value: number };

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type Flip = "none" | "horizontal" | "vertical";

export type TransformOp =
  | { kind: "resize"; value: ResizeMode }
  | { kind: "crop"; value: Rect }
  | { kind: "crop_aspect"; value: number }
  | { kind: "rotate"; value: { degrees: number; flip: Flip } }
  | { kind: "convert" };

export interface TransformOptions {
  format: OutputFormat;
  quality: number;
  stripMetadata: boolean;
}

export interface TransformResult {
  outputPath: string;
  size: number;
  width: number;
  height: number;
  previewUrl: string;
}

/* ---- passwords and permissions ---- */

export type PdfStrength = "aes256" | "aes128";

export interface Protection {
  openPassword: string | null;
  ownerPassword: string | null;
  allowPrinting: boolean;
  allowCopying: boolean;
  allowModifying: boolean;
  allowAnnotating: boolean;
  strength: PdfStrength;
}

export interface SecurityInfo {
  encrypted: boolean;
  needsPasswordToOpen: boolean;
  algorithm: string;
  canPrint: boolean;
  canCopy: boolean;
  canModify: boolean;
  canAnnotate: boolean;
  /** The same thing in a sentence. */
  summary: string;
}

export interface SecurityResult {
  outputPath: string;
  size: number;
  info: SecurityInfo;
  /** "restrictions_only" or "password" — unlock only. */
  unlockedBy?: string;
}

/* ---- watermark ---- */

export type Placement =
  | "top_left"
  | "top_centre"
  | "top_right"
  | "middle_left"
  | "centre"
  | "middle_right"
  | "bottom_left"
  | "bottom_centre"
  | "bottom_right"
  | "tiled";

export type Mark = { kind: "text"; value: string } | { kind: "image"; value: number[] };

export interface WatermarkOptions {
  mark: Mark;
  placement: Placement;
  opacity: number;
  rotation: number;
  fontSize: number | null;
  imageScale: number;
  colour: [number, number, number];
  margin: number;
}

export interface WatermarkResult {
  outputPath: string;
  size: number;
  pagesMarked: number;
  totalPages: number;
}

/* ---- page numbers ---- */

export type NumberPosition =
  | "top_left"
  | "top_centre"
  | "top_right"
  | "bottom_left"
  | "bottom_centre"
  | "bottom_right";

export interface PageNumberOptions {
  /** `{n}` is the page number, `{total}` how many are numbered. */
  template: string;
  position: NumberPosition;
  startAt: number;
  fontSize: number;
  colour: [number, number, number];
  margin: number;
}

export interface NumberResult {
  outputPath: string;
  size: number;
  pagesNumbered: number;
  totalPages: number;
}

/* ---- crop ---- */

export type CropSpec =
  | { kind: "margins"; value: { top: number; right: number; bottom: number; left: number } }
  | { kind: "percent"; value: { top: number; right: number; bottom: number; left: number } }
  | { kind: "auto"; value: { padding: number } };

export interface CropResult {
  outputPath: string;
  size: number;
  pagesCropped: number;
  totalPages: number;
  skipped: number[];
  /** Always present: cropping hides content, it does not remove it. */
  warning: string;
  note?: string;
}

/* ---- image watermark ---- */

export type ImageMark = { kind: "text"; value: string } | { kind: "logo"; value: number[] };

export interface ImageWatermarkOptions {
  placement: Placement;
  opacity: number;
  rotation: number;
  /** Width of the mark as a fraction of the image's width. */
  scale: number;
  colour: [number, number, number];
  /** Distance from the edge, as a fraction of the shorter side. */
  margin: number;
  format: OutputFormat;
  quality: number;
}

/* ---- repair ---- */

export interface RepairResult {
  severity: "healthy" | "repaired" | "salvaged";
  pages: number;
  objects: number;
  findings: string[];
  caveats: string[];
  headline: string;
  outputPath?: string;
  size?: number;
  originalSize: number;
}

/* ---- markdown ---- */

export interface MarkdownOptions {
  pageBreaks: boolean;
  detectHeadings: boolean;
  detectLists: boolean;
}

export interface MarkdownResult {
  markdown: string;
  pages: number;
  pagesWithText: number;
  characters: number;
  scannedPages: number[];
  /** Pages laid out in columns, where the reading order will be jumbled. */
  layoutPages: number[];
  note?: string;
  outputPath?: string;
}

/* ---- compare ---- */

export interface CompareChange {
  page: number;
  kind: "added" | "removed";
  text: string;
}

export interface VisualDifference {
  pageBefore: number;
  pageAfter: number;
  /** 0 identical, 1 completely different. */
  difference: number;
}

export interface CompareResult {
  pagesBefore: number;
  pagesAfter: number;
  pagesAdded: number[];
  pagesRemoved: number[];
  changes: CompareChange[];
  linesAdded: number;
  linesRemoved: number;
  visual: VisualDifference[];
  identical: boolean;
  notes: string[];
  note?: string;
  summary: string;
  outputPath?: string;
}

/* ---- Scan to PDF ---- */

export type ScanMode = "colour" | "greyscale" | "document";

export interface ScanOptions {
  mode: ScanMode;
  detectEdges: boolean;
  strength: number;
}

export interface ScanPreview {
  image: string;
  edgesFound: boolean;
  width: number;
  height: number;
}

export interface ScanResult {
  outputPath: string;
  pages: number;
  flattened: number;
  wholeFrame: number[];
  size: number;
  note?: string;
}

/* ---- Sign PDF ---- */

export type InkColour = "keep" | "black" | "blue";

export interface SignatureOptions {
  ink: InkColour;
  strength: number;
  trim: boolean;
}

export interface SignaturePreview {
  image: string;
  foundInk: boolean;
  width: number;
  height: number;
  coverage: number;
  note?: string;
}

export interface SignatureSpot {
  page: number;
  /** Centre, 0–1 across the page. */
  x: number;
  /** Centre, 0–1 down the page. */
  y: number;
  /** Width as a fraction of the page width. */
  width: number;
}

export interface SignCaption {
  text: string;
  date: string;
  size: number;
  colour: [number, number, number];
}

export interface SignOptions {
  spots: SignatureSpot[];
  caption: SignCaption | null;
}

export interface SignResult {
  outputPath: string;
  pagesSigned: number[];
  placements: number;
  size: number;
  note: string;
  warnings: string[];
}

/* ---- Redact PDF ---- */

export type RedactTarget =
  | { kind: "text"; needle: string; matchCase: boolean }
  | { kind: "area"; page: number; x: number; y: number; width: number; height: number };

export interface RedactOptions {
  targets: RedactTarget[];
  drawBoxes: boolean;
  boxColour: [number, number, number];
  stripExtras: boolean;
}

export interface RedactResult {
  outputPath: string;
  pagesChanged: number[];
  glyphsRemoved: number;
  imagesEdited: number;
  annotationsRemoved: number;
  verified: boolean;
  leaks: string[];
  warnings: string[];
  note: string;
  size: number;
}

/* ---- PDF forms ---- */

export type FieldKind =
  | "text"
  | "multiline"
  | "password"
  | "checkbox"
  | "radio"
  | "choice"
  | "button"
  | "signature";

export interface FormField {
  name: string;
  label?: string;
  kind: FieldKind;
  value?: string;
  options: string[];
  required: boolean;
  readOnly: boolean;
  maxLength?: number;
  pages: number[];
}

export interface FillOptions {
  values: Record<string, string>;
  flatten: boolean;
}

export interface FillResult {
  outputPath: string;
  filled: string[];
  unknown: string[];
  readOnly: string[];
  stillRequired: string[];
  flattened: boolean;
  warnings: string[];
  size: number;
}

/* ---- Edit PDF ---- */

export type AnnotationMark =
  | { kind: "text"; text: string; size: number }
  | { kind: "highlight" }
  | { kind: "whiteout" }
  | { kind: "rectangle"; filled: boolean }
  | { kind: "ellipse"; filled: boolean }
  | { kind: "line"; arrow: boolean }
  | { kind: "ink"; points: [number, number][] };

export interface Annotation {
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  mark: AnnotationMark;
  colour: [number, number, number];
  lineWidth: number;
  opacity: number;
}

export interface AnnotateResult {
  outputPath: string;
  drawn: number;
  pagesChanged: number[];
  warnings: string[];
  note: string;
  size: number;
}

/* ---- OCR ---- */

export interface ModelStatus {
  ready: boolean;
  bytes: number;
  folder: string;
  sources: string[];
}

export interface OcrOptions {
  pages: null;
  skipPagesWithText: boolean;
  maxPx: number;
}

export interface OcrPageText {
  page: number;
  text: string;
  words: unknown[];
}

export interface OcrResult {
  outputPath?: string;
  pages: OcrPageText[];
  pagesRead: number[];
  pagesWithText: number[];
  pagesNotReadable: number[];
  words: number;
  note: string;
  warnings: string[];
  size: number;
}

/* ---- HTML to PDF ---- */

export type HtmlSource =
  | { kind: "html"; value: string }
  | { kind: "file"; value: string }
  | { kind: "url"; value: string };

export interface HtmlPageSetup {
  size: string;
  landscape: boolean;
  margin: number;
  backgrounds: boolean;
  scale: number;
  headerAndFooter: boolean;
  settleMs: number;
}

export interface HtmlResult {
  outputPath: string;
  pages: number;
  source: string;
  size: number;
  warnings: string[];
  note: string;
}

/* ---- Remove background ---- */

export type Backdrop =
  | { kind: "transparent" }
  | { kind: "colour"; value: [number, number, number] };

export interface CutoutOptions {
  backdrop: Backdrop;
  softness: number;
  threshold: number;
  feather: number;
}

export interface CutoutResult {
  outputPath?: string;
  image: string;
  width: number;
  height: number;
  coverage: number;
  size: number;
  note: string;
  warnings: string[];
}

/* ---- Upscale ---- */

export type UpscaleMethod = "sharpen" | "reconstruct";

export interface UpscaleOptions {
  method: UpscaleMethod;
  scale: number;
  sharpen: number;
}

export interface UpscaleResult {
  outputPath: string;
  from: [number, number];
  to: [number, number];
  tiles: number;
  seconds: number;
  size: number;
  previewUrl: string;
  note: string;
  warnings: string[];
}

/* ---- Blur faces ---- */

export type Cover =
  | { kind: "blur" }
  | { kind: "pixelate" }
  | { kind: "box"; value: [number, number, number] };

export interface BlurOptions {
  cover: Cover;
  confidence: number;
  grow: number;
  strength: number;
}

export interface DetectedFace {
  x: number;
  y: number;
  width: number;
  height: number;
  confidence: number;
}

export interface FacesResult {
  outputPath?: string;
  image: string;
  faces: DetectedFace[];
  covered: number;
  size: number;
  note: string;
  warnings: string[];
}

/* ---- HTML to image ---- */

export interface ShotSetup {
  width: number;
  fullPage: boolean;
  height: number;
  png: boolean;
  settleMs: number;
}

export interface ShotResult {
  outputPath: string;
  width: number;
  height: number;
  source: string;
  size: number;
  previewUrl: string;
  warnings: string[];
  note: string;
}

/* ---- Photo editor ---- */

export interface AdjustOptions {
  autoLevels: boolean;
  autoColour: boolean;
  exposure: number;
  contrast: number;
  saturation: number;
  warmth: number;
  sharpen: number;
  greyscale: boolean;
}

export interface AdjustResult {
  outputPath?: string;
  image: string;
  width: number;
  height: number;
  size: number;
  warnings: string[];
}

/* ---- PDF/A ---- */

export type ArchivePart = "a1b" | "a2b";

export interface ArchiveBlocker {
  rule: string;
  detail: string;
  fixableElsewhere: boolean;
}

export interface ArchiveResult {
  outputPath?: string;
  conformant: boolean;
  part: string;
  applied: string[];
  blockers: ArchiveBlocker[];
  note: string;
  size: number;
  profileFound: boolean;
}

/* ---- Meme ---- */

export interface MemeOptions {
  top: string;
  bottom: string;
  uppercase: boolean;
  size: number;
  outline: number;
  colour: [number, number, number];
  outlineColour: [number, number, number];
  band: boolean;
}

export interface MemeResult {
  outputPath?: string;
  image: string;
  width: number;
  height: number;
  topLines: number;
  bottomLines: number;
  size: number;
  warnings: string[];
}

/* ---- Batch ---- */

export type BatchOp =
  | {
      kind: "compress";
      target: SizeTarget;
      bias: Bias;
      format: OutputFormat;
      stripMetadata: boolean;
      exact: [number, number] | null;
    }
  | { kind: "resize"; longest: number }
  | { kind: "convert"; format: OutputFormat; stripMetadata: boolean }
  | { kind: "adjust"; autoLevels: boolean; autoColour: boolean; exposure: number;
      contrast: number; saturation: number; warmth: number; sharpen: number; greyscale: boolean };

/** One file's answer, sent back the moment that file is done. */
export interface BatchStep {
  outcome: BatchOutcome;
  path: string | null;
  done: number;
}

/** What became of one file: a union tagged on `state`, as Rust sends it. */
export type BatchOutcome =
  | {
      state: "done";
      name: string;
      before: number;
      after: number;
      extension: string;
      note: string;
      /** Produced, but bigger than the size that was asked for. */
      short: boolean;
    }
  | { state: "failed"; name: string; why: string }
  | { state: "skipped"; name: string };

export interface BatchResult {
  outcomes: BatchOutcome[];
  paths: string[];
  done: number;
  failed: number;
  short: number;
  cancelled: boolean;
  folder: string;
}

/* ---------- form presets ---------- */

export type PresetKind = "image" | "pdf";
export type PresetFormat = "jpeg" | "png" | "pdf" | "any";
/** How the stated pixel size is meant to be read. */
export type PresetFit = "exact" | "at_least" | "at_most";

export interface PresetDimensions {
  width: number;
  height: number;
  fit: PresetFit;
}

/** A form's constraints, as written on the form. */
export interface Preset {
  id: string;
  name: string;
  group: string;
  kind: PresetKind;
  target: SizeTarget;
  format: PresetFormat;
  dimensions?: PresetDimensions;
  /** Minimum resolution, when the form asks for one. */
  dpi?: number;
  note?: string;
}

export interface PresetEntry extends Preset {
  /** Came from your presets folder rather than the bundled set. */
  yours: boolean;
  /** Replaces a bundled preset, so the original can be put back. */
  overridesBundled: boolean;
}

/** `unknown` is for what the file doesn't say, which is not a failure. */
export type Verdict = "ok" | "wrong" | "unknown";

export interface PresetCheck {
  what: string;
  verdict: Verdict;
  wants: string;
  has: string;
}

export interface PresetReport {
  checks: PresetCheck[];
  fits: boolean;
}

export interface LoadedPresets {
  presets: PresetEntry[];
  folder: string;
  /** The README in that folder; revealing it is how the folder is opened. */
  readme: string;
  /** Files in the folder that couldn't be read, said out loud. */
  problems: { file: string; why: string }[];
}

export interface PresetFixResult {
  size: number;
  report: PresetReport;
  steps: string[];
  shortfall: string | null;
  outputPath: string;
  preview: string | null;
}

/* ---------- the Windows right-click menu ---------- */

export interface ShellMenuState {
  installed: boolean;
  /** The lines it adds, for the switch to describe itself. */
  items: string[];
  /** The file types it appears on. */
  types: string[];
  note: string;
}

/** What a right-click produced. */
export interface ShellRun {
  kind: "compress" | "open";
  summary: string;
  outcomes: BatchOutcome[];
  paths: string[];
  /** For "open": the files themselves. */
  files: { name: string; bytes: number[] }[];
  folder: string;
}

/* ---------- the clipboard key ---------- */

export interface Hotkey {
  enabled: boolean;
  /** An accelerator, e.g. "CommandOrControl+Alt+K". */
  combo: string;
  /** A size as the parser reads it: "100kb", "20-50kb". */
  target: string;
}

/** What one press did. */
export interface Pressed {
  ok: boolean;
  summary: string;
  before: number;
  after: number;
  path: string | null;
}

/* ---------- watched folders ---------- */

export interface Watch {
  path: string;
  /** A size as the parser reads it: "100kb", "20-50kb". */
  target: string;
  enabled: boolean;
}

/* ---------- recipes: several steps, saved as one ---------- */

export type Step =
  | { step: "resize"; longest: number }
  | { step: "fit"; width: number; height: number }
  | { step: "aspect"; ratio: number }
  | { step: "rotate"; degrees: number; flip?: string }
  | ({ step: "adjust" } & AdjustOptions)
  | { step: "watermark"; text: string; options?: unknown }
  | { step: "convert"; format: OutputFormat }
  | { step: "compress"; target: SizeTarget; bias: Bias };

export interface Chain {
  id: string;
  name: string;
  steps: Step[];
  note?: string;
}

export interface RecipeRun {
  outcomes: BatchOutcome[];
  paths: string[];
  /** What the last file went through, step by step. */
  steps: string[];
  done: number;
  failed: number;
}

/* ---------- what history knows about one file ---------- */

export interface Link {
  hash: string;
  name: string;
  kind: string;
  size: number;
  thumbHash?: string;
  pinned: boolean;
  /** False when the file has since been forgotten; the step still happened. */
  known: boolean;
}

export interface Telling {
  tool: string;
  summary: string;
  createdAt: number;
  durationMs: number;
  /** The settings that were used, pretty-printed. */
  recipe: string;
  from: Link | null;
  to: Link | null;
}

export interface Story {
  file: Link | null;
  /** What produced it, oldest first. */
  chain: Telling[];
  /** What has been made from it since. */
  derivatives: Telling[];
}
