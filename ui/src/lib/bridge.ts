/**
 * The one place the UI talks to the outside world.
 *
 * On the desktop the work goes to the Rust core over Tauri commands. In a plain
 * browser it falls back to the canvas engine and a localStorage history, so
 * every screen can be developed and demonstrated without a build of the shell.
 * Screens never check which mode they're in — they call these functions.
 */

import { compressInBrowser } from "./browserEngine";
import type {
  CompressRequest,
  CompressResult,
  ExportedPageResult,
  HistoryEntry,
  ImagesToPdfOptions,
  PageInfo,
  PageOp,
  PageOpResult,
  PdfReport,
  Protection,
  SecurityInfo,
  SecurityResult,
  SplitMode,
  SplitPartResult,
  StorageUsage,
  TransformOp,
  TransformOptions,
  TransformResult,
  CompareResult,
  CropResult,
  ImageMark,
  ImageWatermarkOptions,
  CropSpec,
  MarkdownOptions,
  MarkdownResult,
  NumberResult,
  AnnotateResult,
  HtmlPageSetup,
  ShotResult,
  ShotSetup,
  HtmlResult,
  HtmlSource,
  AdjustOptions,
  BatchOp,
  BatchStep,
  LoadedPresets,
  Policy,
  Preset,
  PresetFixResult,
  PresetReport,
  RerunResult,
  Hotkey,
  Pressed,
  ShellMenuState,
  ShellRun,
  Swept,
  SweptByAge,
  Chain,
  Story,
  RecipeRun,
  Watch,
  BatchResult,
  ArchivePart,
  MemeOptions,
  MemeResult,
  ArchiveResult,
  AdjustResult,
  BlurOptions,
  CutoutOptions,
  FacesResult,
  UpscaleOptions,
  UpscaleResult,
  CutoutResult,
  ModelStatus,
  OcrOptions,
  OcrResult,
  Annotation,
  FillOptions,
  FillResult,
  FormField,
  RedactOptions,
  RedactResult,
  RepairResult,
  SignatureOptions,
  SignaturePreview,
  SignOptions,
  SignResult,
  ScanOptions,
  ScanPreview,
  ScanResult,
  PageNumberOptions,
  WatermarkOptions,
  WatermarkResult,
} from "./types";

export function isDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const core = await import("@tauri-apps/api/core");
  return core.invoke<T>(command, args);
}

export function isPdf(file: File): boolean {
  return file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf");
}

export async function compress(file: File, req: Omit<CompressRequest, "bytes" | "path">) {
  if (!isDesktop()) {
    if (isPdf(file)) {
      // The canvas fallback can decode images, not documents. Say so plainly
      // rather than failing somewhere deeper.
      throw new Error(
        "PDF compression runs in the Rust engine, which the browser preview doesn't have. Open the desktop app to compress PDFs.",
      );
    }
    const result = await compressInBrowser(file, req);
    rememberLocally(result);
    return result;
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<CompressResult>("compress_file", {
    request: { ...req, bytes: Array.from(bytes), name: file.name },
  });
}

/** Read a PDF's byte breakdown without changing it. */
export async function inspectPdf(file: File): Promise<PdfReport | null> {
  if (!isDesktop()) return null;
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<PdfReport>("inspect_pdf", { bytes: Array.from(bytes) });
}

/**
 * On the desktop the file is already written to Pictures / Kitbox by the time
 * the result comes back, so "save" means showing the user where it went.
 */
export async function revealOutput(path: string): Promise<void> {
  const opener = await import("@tauri-apps/plugin-opener");
  await opener.revealItemInDir(path);
}

export async function outputFolder(): Promise<string | null> {
  if (!isDesktop()) return null;
  return invoke<string>("output_folder");
}

export async function recentHistory(limit = 24): Promise<HistoryEntry[]> {
  if (!isDesktop()) return readLocalHistory().slice(0, limit);
  return invoke<HistoryEntry[]>("recent_history", { limit });
}

/**
 * The picture for a history entry, as a data URL.
 *
 * Fetched one at a time and cached, so opening the gallery doesn't pull every
 * thumbnail at once.
 */
const thumbCache = new Map<string, string | null>();

export async function historyThumbnail(hash: string): Promise<string | null> {
  if (!isDesktop()) return null;
  const cached = thumbCache.get(hash);
  if (cached !== undefined) return cached;
  try {
    const url = await invoke<string | null>("history_thumbnail", { hash });
    thumbCache.set(hash, url);
    return url;
  } catch {
    thumbCache.set(hash, null);
    return null;
  }
}

export async function storageUsage(): Promise<StorageUsage> {
  if (!isDesktop()) {
    const rows = readLocalHistory();
    return { files: rows.length, bytes: rows.reduce((n, r) => n + r.size, 0), jobs: rows.length };
  }
  return invoke<StorageUsage>("storage_usage");
}

/* ---- browser-mode history ---- */

const LOCAL_KEY = "kitbox.history.v1";

function readLocalHistory(): HistoryEntry[] {
  try {
    const raw = localStorage.getItem(LOCAL_KEY);
    return raw ? (JSON.parse(raw) as HistoryEntry[]) : [];
  } catch {
    return [];
  }
}

function rememberLocally(result: CompressResult) {
  if (result.status === "infeasible") return;
  try {
    const rows = readLocalHistory();
    rows.unshift({
      hash: `${Date.now()}`,
      name: result.name,
      kind: "jpeg",
      size: result.size,
      createdAt: Date.now(),
      summary: `${formatSize(result.originalSize)} → ${formatSize(result.size)}, ${result.knob}`,
      pinned: false,
    });
    localStorage.setItem(LOCAL_KEY, JSON.stringify(rows.slice(0, 200)));
  } catch {
    // A full or disabled localStorage must not break compression.
  }
}

export function formatSize(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${Math.round(bytes / 1_000)} KB`;
  return `${bytes} B`;
}

export function describeScore(score: number): string {
  const pct = Math.min(100, Math.max(0, score * 100));
  const verdict =
    score >= 0.98
      ? "near identical"
      : score >= 0.95
        ? "very close"
        : score >= 0.9
          ? "slightly softer"
          : score >= 0.8
            ? "visibly compressed"
            : "heavily compressed";
  return `${pct.toFixed(1)}% visually identical — ${verdict}`;
}

/* ---- page tools (desktop only) ---- */

function requireDesktop(what: string): never {
  throw new Error(
    `${what} runs in the Rust engine, which the browser preview doesn't have. Open the desktop app.`,
  );
}

export async function pdfPages(file: File): Promise<PageInfo[]> {
  if (!isDesktop()) requireDesktop("Page tools");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<PageInfo[]>("pdf_pages", { bytes: Array.from(bytes) });
}

/** Thumbnails as data URLs; null where the page isn't a single image. */
export async function pdfPreviews(file: File, maxPx = 180): Promise<(string | null)[]> {
  if (!isDesktop()) return [];
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<(string | null)[]>("pdf_previews", { bytes: Array.from(bytes), maxPx });
}

export async function applyPages(file: File, ops: PageOp[]): Promise<PageOpResult> {
  if (!isDesktop()) requireDesktop("Page tools");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<PageOpResult>("apply_pages", {
    request: { bytes: Array.from(bytes), name: file.name, ops },
  });
}

export async function splitPdf(file: File, mode: SplitMode): Promise<SplitPartResult[]> {
  if (!isDesktop()) requireDesktop("Split");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<SplitPartResult[]>("split_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, mode },
  });
}

export async function mergePdfs(files: File[], name: string): Promise<PageOpResult> {
  if (!isDesktop()) requireDesktop("Merge");
  const payload = await Promise.all(
    files.map(async (f) => ({ bytes: Array.from(new Uint8Array(await f.arrayBuffer())) })),
  );
  return invoke<PageOpResult>("merge_pdfs", { request: { files: payload, name } });
}

export async function imagesToPdf(
  files: File[],
  name: string,
  options: ImagesToPdfOptions,
): Promise<PageOpResult> {
  if (!isDesktop()) requireDesktop("Building a PDF");
  const images = await Promise.all(
    files.map(async (f) => ({ bytes: Array.from(new Uint8Array(await f.arrayBuffer())) })),
  );
  return invoke<PageOpResult>("images_to_pdf", { request: { images, name, options } });
}

export async function scanPreview(
  file: File,
  options: ScanOptions,
  maxPx = 900,
): Promise<ScanPreview> {
  if (!isDesktop()) requireDesktop("Scanning a photograph");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<ScanPreview>("scan_preview", { bytes: Array.from(bytes), options, maxPx });
}

export async function scanToPdf(
  files: File[],
  name: string,
  options: ScanOptions,
  pdf: ImagesToPdfOptions,
): Promise<ScanResult> {
  if (!isDesktop()) requireDesktop("Scanning to PDF");
  const images = await Promise.all(
    files.map(async (f) => ({ bytes: Array.from(new Uint8Array(await f.arrayBuffer())) })),
  );
  return invoke<ScanResult>("scan_to_pdf", { request: { images, name, options, pdf } });
}

export async function signaturePreview(
  file: File,
  options: SignatureOptions,
): Promise<SignaturePreview> {
  if (!isDesktop()) requireDesktop("Cutting out a signature");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<SignaturePreview>("signature_preview", { bytes: Array.from(bytes), options });
}

export async function saveSignature(
  file: File,
  name: string,
  options: SignatureOptions,
): Promise<TransformResult> {
  if (!isDesktop()) requireDesktop("Saving a signature");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<TransformResult>("save_signature", {
    request: { bytes: Array.from(bytes), name, options },
  });
}

export async function signPdf(
  file: File,
  signaturePng: Uint8Array,
  options: SignOptions,
): Promise<SignResult> {
  if (!isDesktop()) requireDesktop("Signing a PDF");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<SignResult>("sign_pdf", {
    request: {
      bytes: Array.from(bytes),
      name: file.name,
      signature: Array.from(signaturePng),
      options,
    },
  });
}

/** Pull the bytes back out of a `data:` URL, for handing a preview to a tool. */
export function bytesFromDataUrl(url: string): Uint8Array {
  const binary = atob(url.slice(url.indexOf(",") + 1));
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

export async function redactPdf(file: File, options: RedactOptions): Promise<RedactResult> {
  if (!isDesktop()) requireDesktop("Redacting");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<RedactResult>("redact_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, options },
  });
}

export async function formFields(file: File): Promise<FormField[]> {
  if (!isDesktop()) requireDesktop("Reading a form");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<FormField[]>("form_fields", { bytes: Array.from(bytes) });
}

export async function fillForm(file: File, options: FillOptions): Promise<FillResult> {
  if (!isDesktop()) requireDesktop("Filling a form");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<FillResult>("fill_form", {
    request: { bytes: Array.from(bytes), name: file.name, options },
  });
}

export async function annotatePdf(
  file: File,
  marks: Annotation[],
): Promise<AnnotateResult> {
  if (!isDesktop()) requireDesktop("Drawing on a page");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<AnnotateResult>("annotate_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, options: { marks } },
  });
}

/** Which tools need a download, by the name the registry knows them by. */
export type ModelSet = "ocr" | "cutout" | "upscale" | "faces";

export async function modelsReady(set: ModelSet): Promise<ModelStatus> {
  if (!isDesktop()) requireDesktop("Checking for models");
  return invoke<ModelStatus>("models_ready", { set });
}

export async function fetchModels(set: ModelSet): Promise<ModelStatus> {
  if (!isDesktop()) requireDesktop("Downloading models");
  return invoke<ModelStatus>("fetch_models", { set });
}

export async function removeBackground(
  file: File,
  options: CutoutOptions,
  preview: boolean,
): Promise<CutoutResult> {
  if (!isDesktop()) requireDesktop("Removing a background");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<CutoutResult>("remove_background", {
    request: { bytes: Array.from(bytes), name: file.name, options, preview },
  });
}

export async function ocrPdf(
  file: File,
  options: OcrOptions,
  searchable: boolean,
): Promise<OcrResult> {
  if (!isDesktop()) requireDesktop("Reading a scan");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<OcrResult>("ocr_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, options, searchable },
  });
}

export async function htmlToPdf(
  source: HtmlSource,
  name: string,
  setup: HtmlPageSetup,
): Promise<HtmlResult> {
  if (!isDesktop()) requireDesktop("Rendering a page");
  return invoke<HtmlResult>("html_to_pdf", { request: { source, name, setup } });
}

export async function upscaleImage(
  file: File,
  options: UpscaleOptions,
): Promise<UpscaleResult> {
  if (!isDesktop()) requireDesktop("Upscaling");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<UpscaleResult>("upscale_image", {
    request: { bytes: Array.from(bytes), name: file.name, options },
  });
}

/** Roughly how long reconstructing this many pixels will take, in seconds. */
export async function upscaleEstimate(width: number, height: number): Promise<number> {
  if (!isDesktop()) return 0;
  return invoke<number>("upscale_estimate", { width, height });
}

export async function blurFaces(
  file: File,
  options: BlurOptions,
  preview: boolean,
): Promise<FacesResult> {
  if (!isDesktop()) requireDesktop("Covering faces");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<FacesResult>("blur_faces", {
    request: { bytes: Array.from(bytes), name: file.name, options, preview },
  });
}

export async function htmlToImage(
  source: HtmlSource,
  name: string,
  setup: ShotSetup,
): Promise<ShotResult> {
  if (!isDesktop()) requireDesktop("Photographing a page");
  return invoke<ShotResult>("html_to_image", { request: { source, name, setup } });
}

export async function adjustImage(
  file: File,
  options: AdjustOptions,
  preview: boolean,
  maxPx = 1100,
): Promise<AdjustResult> {
  if (!isDesktop()) requireDesktop("Adjusting a photo");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<AdjustResult>("adjust_image", {
    request: { bytes: Array.from(bytes), name: file.name, options, preview, maxPx },
  });
}

export async function pdfToArchive(
  file: File,
  part: ArchivePart,
  checkOnly: boolean,
  title?: string,
  author?: string,
): Promise<ArchiveResult> {
  if (!isDesktop()) requireDesktop("Preparing for archiving");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<ArchiveResult>("pdf_to_archive", {
    request: {
      bytes: Array.from(bytes),
      name: file.name,
      part,
      title: title || null,
      author: author || null,
      checkOnly,
    },
  });
}

export async function makeMeme(
  file: File,
  options: MemeOptions,
  preview: boolean,
): Promise<MemeResult> {
  if (!isDesktop()) requireDesktop("Captioning a picture");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<MemeResult>("make_meme", {
    request: { bytes: Array.from(bytes), name: file.name, options, preview },
  });
}

/**
 * Run `op` over every file, one call per file.
 *
 * Sending the whole pile in one message would be the obvious shape and the
 * wrong one: bytes cross this boundary as JSON, where a 4 MB photo measures
 * about 14 MB, so forty of them would be half a gigabyte in a single string.
 * A file at a time keeps the peak at one file and lets the screen fill in as
 * it goes, which is also why `onStep` reports a finished file rather than a
 * fraction.
 */
/**
 * Do a past job again, on a file you choose now.
 *
 * Only the entry's hash goes across — the settings are read from history on
 * the other side, so what runs is what was recorded rather than something this
 * screen pieced back together.
 */
export async function rerunRecipe(hash: string, file: File): Promise<RerunResult> {
  if (!isDesktop()) requireDesktop("Running a past job again");
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
  return invoke<RerunResult>("rerun_recipe", {
    request: { hash, name: file.name, bytes },
  });
}

/**
 * Forget one entry: its row, its job and Kitbox's own copy of the bytes.
 *
 * The finished file in your output folder is yours and stays where it is.
 */
export async function forgetEntry(hash: string): Promise<StorageUsage> {
  if (!isDesktop()) requireDesktop("Forgetting an entry");
  return invoke<StorageUsage>("forget_entry", { hash });
}

/** Entries pointing at a file that is no longer there. */
export async function countMissing(): Promise<number> {
  if (!isDesktop()) return 0;
  return invoke<number>("count_missing", {});
}

/** Forget every entry whose file has gone. Pinned ones are left alone. */
export async function tidyHistory(): Promise<Swept> {
  if (!isDesktop()) requireDesktop("Tidying history");
  return invoke<Swept>("tidy_history", {});
}

/** Keep an entry whatever else is tidied away. */
export async function setPinned(hash: string, pinned: boolean): Promise<boolean> {
  if (!isDesktop()) requireDesktop("Pinning");
  return invoke<boolean>("set_pinned", { hash, pinned });
}

/** The chain that produced a file, and anything made from it since. */
export async function entryStory(hash: string): Promise<Story> {
  if (!isDesktop()) requireDesktop("Looking at a file's history");
  return invoke<Story>("entry_story", { hash });
}

/* ---------- recipes ---------- */

export async function recipes(): Promise<Chain[]> {
  if (!isDesktop()) requireDesktop("Recipes");
  return invoke<Chain[]>("recipes", {});
}

export async function saveRecipe(chain: Chain): Promise<Chain[]> {
  if (!isDesktop()) requireDesktop("Recipes");
  return invoke<Chain[]>("save_recipe", { chain });
}

export async function deleteRecipe(id: string): Promise<Chain[]> {
  if (!isDesktop()) requireDesktop("Recipes");
  return invoke<Chain[]>("delete_recipe", { id });
}

/** Run every step of a recipe over the files, in order. */
export async function runRecipe(chain: Chain, files: File[]): Promise<RecipeRun> {
  if (!isDesktop()) requireDesktop("Recipes");
  const payload = await Promise.all(
    files.map(async (f) => ({
      name: f.name,
      bytes: Array.from(new Uint8Array(await f.arrayBuffer())),
    })),
  );
  return invoke<RecipeRun>("run_recipe", { request: { chain, files: payload } });
}

/* ---------- letting history forget by itself ---------- */

export async function retention(): Promise<Policy> {
  if (!isDesktop()) requireDesktop("Retention");
  return invoke<Policy>("retention", {});
}

export async function setRetention(policy: Policy): Promise<Policy> {
  if (!isDesktop()) requireDesktop("Retention");
  return invoke<Policy>("set_retention", { policy });
}

/** What the rule would take right now — answered even when it is off. */
export async function retentionPreview(policy: Policy): Promise<SweptByAge> {
  if (!isDesktop()) requireDesktop("Retention");
  return invoke<SweptByAge>("retention_preview", { policy });
}

/** Apply it now rather than at the next start. */
export async function runRetention(policy: Policy): Promise<SweptByAge> {
  if (!isDesktop()) requireDesktop("Retention");
  return invoke<SweptByAge>("run_retention", { policy });
}

/* ---------- watched folders ---------- */

export async function watchedFolders(): Promise<Watch[]> {
  if (!isDesktop()) requireDesktop("Watching a folder");
  return invoke<Watch[]>("watched_folders", {});
}

/**
 * Save the list and start watching it.
 *
 * Watching happens first on the other side: a folder that has gone, or a size
 * that can't be read, is refused before anything is written down.
 */
export async function setWatchedFolders(watches: Watch[]): Promise<Watch[]> {
  if (!isDesktop()) requireDesktop("Watching a folder");
  return invoke<Watch[]>("set_watched_folders", { watches });
}

/**
 * Ask for a file on disk, by path.
 *
 * The browser's file input hands over bytes and a name, never a location —
 * which is no use to the renderer, because it has to load the page itself to
 * follow the stylesheets and pictures sitting beside it.
 */
export async function pickFile(
  what: string,
  extensions: string[],
): Promise<string | null> {
  if (!isDesktop()) requireDesktop("Choosing a file");
  const { open } = await import("@tauri-apps/plugin-dialog");
  const chosen = await open({
    multiple: false,
    filters: [{ name: what, extensions }],
  });
  return typeof chosen === "string" ? chosen : null;
}

/** Ask for a folder. Returns null if the dialog was dismissed. */
export async function pickFolder(): Promise<string | null> {
  if (!isDesktop()) requireDesktop("Choosing a folder");
  const { open } = await import("@tauri-apps/plugin-dialog");
  const chosen = await open({ directory: true, multiple: false });
  return typeof chosen === "string" ? chosen : null;
}

/* ---------- the clipboard key ---------- */

export async function clipboardKey(): Promise<Hotkey> {
  if (!isDesktop()) requireDesktop("The clipboard key");
  return invoke<Hotkey>("clipboard_key", {});
}

/**
 * Save the key and start listening.
 *
 * The key is taken first: if the combination is already spoken for, nothing is
 * saved and this throws with what has it, so the screen never claims a key
 * that does nothing.
 */
export async function setClipboardKey(settings: Hotkey): Promise<Hotkey> {
  if (!isDesktop()) requireDesktop("The clipboard key");
  return invoke<Hotkey>("set_clipboard_key", { settings });
}

/** Do what the key does, for anyone who would rather click. */
export async function useClipboard(): Promise<Pressed> {
  if (!isDesktop()) requireDesktop("The clipboard");
  return invoke<Pressed>("use_clipboard", {});
}

/* ---------- the Windows right-click menu ---------- */

export async function shellMenu(): Promise<ShellMenuState> {
  if (!isDesktop()) requireDesktop("The right-click menu");
  return invoke<ShellMenuState>("shell_menu", {});
}

export async function setShellMenu(on: boolean): Promise<ShellMenuState> {
  if (!isDesktop()) requireDesktop("The right-click menu");
  return invoke<ShellMenuState>("set_shell_menu", { on });
}

/**
 * Whatever a right-click left for the screen, if anything.
 *
 * A launch from Explorer starts the app and finishes the work before the
 * screen has mounted, so the event alone would be missed exactly when the app
 * was opened by the menu. This picks up what was left.
 */
export async function takeShellRun(): Promise<ShellRun | null> {
  if (!isDesktop()) return null;
  return invoke<ShellRun | null>("take_shell_run", {});
}

/** Called again while the app is already open and a right-click arrives. */
export async function onShellRun(handle: (run: ShellRun) => void): Promise<() => void> {
  if (!isDesktop()) return () => undefined;
  const { listen } = await import("@tauri-apps/api/event");
  return listen<ShellRun>("shell-run", (event) => handle(event.payload));
}

/** Rebuild the files a shell "Open in Kitbox" handed over. */
export function filesFromShell(run: ShellRun): File[] {
  return run.files.map(
    (f) => new File([new Uint8Array(f.bytes)], f.name, { type: guessType(f.name) }),
  );
}

function guessType(name: string): string {
  const extension = name.toLowerCase().split(".").pop() ?? "";
  if (extension === "pdf") return "application/pdf";
  if (extension === "png") return "image/png";
  if (extension === "webp") return "image/webp";
  return "image/jpeg";
}

/* ---------- form presets ---------- */

export async function listPresets(): Promise<LoadedPresets> {
  if (!isDesktop()) requireDesktop("Reading the presets folder");
  return invoke<LoadedPresets>("list_presets", {});
}

export async function savePreset(preset: Preset): Promise<LoadedPresets> {
  if (!isDesktop()) requireDesktop("Saving a preset");
  return invoke<LoadedPresets>("save_preset", { preset });
}

/** Drop your version. A bundled preset it replaced comes back. */
export async function deletePreset(id: string): Promise<LoadedPresets> {
  if (!isDesktop()) requireDesktop("Deleting a preset");
  return invoke<LoadedPresets>("delete_preset", { id });
}

export async function checkPreset(file: File, preset: Preset): Promise<PresetReport> {
  if (!isDesktop()) requireDesktop("Checking a file against a preset");
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
  return invoke<PresetReport>("check_preset", { request: { bytes, preset } });
}

export async function fixForPreset(file: File, preset: Preset): Promise<PresetFixResult> {
  if (!isDesktop()) requireDesktop("Fixing a file for a form");
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
  return invoke<PresetFixResult>("fix_preset", {
    request: { bytes, name: file.name, preset },
  });
}

export async function runBatch(
  files: File[],
  op: BatchOp,
  onStep: (step: BatchStep, index: number, total: number) => void,
): Promise<BatchResult> {
  if (!isDesktop()) requireDesktop("Batch processing");
  await invoke("batch_start", { op, names: files.map((f) => f.name) });
  try {
    for (const [index, file] of files.entries()) {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const step = await invoke<BatchStep>("batch_item", { name: file.name, bytes });
      onStep(step, index, files.length);
      // Stopped. The rest would only be sent across to be handed straight
      // back, and on a folder of photos that is a lot of bytes for nothing;
      // the run accounts for them by name at the end instead.
      if (step.outcome.state === "skipped") break;
    }
  } catch (e) {
    // Close the run even when it breaks, so the next one starts clean and
    // whatever did get written is still recorded.
    await invoke<BatchResult>("batch_finish").catch(() => undefined);
    throw e;
  }
  return invoke<BatchResult>("batch_finish");
}

export async function cancelBatch(): Promise<boolean> {
  if (!isDesktop()) return false;
  return invoke<boolean>("cancel_batch", {});
}

export async function pdfToImages(
  file: File,
  maxPx: number | null,
  quality = 88,
): Promise<ExportedPageResult[]> {
  if (!isDesktop()) requireDesktop("Exporting pages");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<ExportedPageResult[]>("pdf_to_images", {
    request: { bytes: Array.from(bytes), name: file.name, maxPx, quality },
  });
}

export async function transformImage(
  file: File,
  op: TransformOp,
  options: TransformOptions,
): Promise<TransformResult> {
  if (!isDesktop()) requireDesktop("Image tools");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<TransformResult>("transform_image", {
    request: { bytes: Array.from(bytes), name: file.name, op, options },
  });
}

/** Read a PDF's protection without changing it. */
export async function pdfSecurity(file: File): Promise<SecurityInfo | null> {
  if (!isDesktop()) return null;
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<SecurityInfo>("pdf_security", { bytes: Array.from(bytes) });
}

export async function protectPdf(file: File, settings: Protection): Promise<SecurityResult> {
  if (!isDesktop()) requireDesktop("Protecting a PDF");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<SecurityResult>("protect_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, settings },
  });
}

export async function unlockPdf(file: File, password: string | null): Promise<SecurityResult> {
  if (!isDesktop()) requireDesktop("Unlocking a PDF");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<SecurityResult>("unlock_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, password },
  });
}

export async function watermarkPdf(
  file: File,
  pages: string | null,
  options: WatermarkOptions,
): Promise<WatermarkResult> {
  if (!isDesktop()) requireDesktop("Watermarking");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<WatermarkResult>("watermark_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, pages, options },
  });
}

export async function numberPages(
  file: File,
  pages: string | null,
  options: PageNumberOptions,
): Promise<NumberResult> {
  if (!isDesktop()) requireDesktop("Page numbers");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<NumberResult>("number_pages", {
    request: { bytes: Array.from(bytes), name: file.name, pages, options },
  });
}

export async function cropPdf(
  file: File,
  pages: string | null,
  spec: CropSpec,
): Promise<CropResult> {
  if (!isDesktop()) requireDesktop("Cropping a PDF");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<CropResult>("crop_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, pages, spec },
  });
}

export async function watermarkImage(
  file: File,
  mark: ImageMark,
  options: ImageWatermarkOptions,
): Promise<TransformResult> {
  if (!isDesktop()) requireDesktop("Watermarking");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<TransformResult>("watermark_image", {
    request: { bytes: Array.from(bytes), name: file.name, mark, options },
  });
}

export async function repairPdf(file: File, check: boolean): Promise<RepairResult> {
  if (!isDesktop()) requireDesktop("Repairing a PDF");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<RepairResult>("repair_pdf", {
    request: { bytes: Array.from(bytes), name: file.name, check },
  });
}

export async function pdfToMarkdown(
  file: File,
  options: MarkdownOptions,
): Promise<MarkdownResult> {
  if (!isDesktop()) requireDesktop("Reading a PDF");
  const bytes = new Uint8Array(await file.arrayBuffer());
  return invoke<MarkdownResult>("pdf_to_markdown", {
    request: { bytes: Array.from(bytes), name: file.name, options },
  });
}

export async function comparePdfs(
  before: File,
  after: File,
  name: string,
): Promise<CompareResult> {
  if (!isDesktop()) requireDesktop("Comparing PDFs");
  const [a, b] = await Promise.all([before.arrayBuffer(), after.arrayBuffer()]);
  const result = await invoke<CompareResult>("compare_pdfs", {
    request: {
      before: Array.from(new Uint8Array(a)),
      after: Array.from(new Uint8Array(b)),
      name,
    },
  });
  // The engine returns a list; the screen shows one line of context.
  return { ...result, note: result.notes.join(" ") || undefined };
}
