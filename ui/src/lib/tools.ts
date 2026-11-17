/**
 * The tool catalogue.
 *
 * Image icon paths come straight from the reference design; the PDF ones are
 * drawn in the same 24×24, 1.9px stroke language.
 *
 * `status` is deliberately honest: a tile marked "soon" is one the engine
 * doesn't implement yet, and the UI says so rather than opening a screen that
 * can't do anything.
 */

export type ToolStatus = "ready" | "soon";
export type Surface = "images" | "pdf";

export interface Tool {
  id: string;
  surface: Surface;
  category: string;
  name: string;
  desc: string;
  icon: string;
  status: ToolStatus;
}

export interface Category {
  id: string;
  name: string;
  blurb: string;
  tools: Tool[];
}

const ICON = {
  compress: "M4 14h6v6 M20 10h-6V4 M14 10l7-7 M3 21l7-7",
  upscale: "M15 3h6v6 M9 21H3v-6 M21 3l-7 7 M3 21l7-7",
  removebg: "M4 4h16v16H4z M4 12h8v8 M12 4v8h8",
  meme: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18 M8 14s1.5 2 4 2 4-2 4-2 M9 9h.01 M15 9h.01",
  editor: "M16 3l5 5L8 21H3v-5z M13 6l5 5",
  resize: "M3 3h8v8H3z M14 21h7v-7 M21 21l-7-7",
  crop: "M6 2v14a2 2 0 0 0 2 2h14 M18 22V8a2 2 0 0 0-2-2H2",
  rotate: "M21 12a9 9 0 1 1-3-6.7 M21 3v6h-6",
  tojpg: "M5 3h9l5 5v13H5z M14 3v5h5 M9 13l3 3 3-3 M12 16v-6",
  fromjpg: "M5 3h9l5 5v13H5z M14 3v5h5 M9 13l3-3 3 3 M12 10v7",
  html: "M8 8l-4 4 4 4 M16 8l4 4-4 4 M14 5l-4 14",
  watermark: "M9 11c0-2-1-3-1-5a4 4 0 0 1 8 0c0 2-1 3-1 5 M5 15h14v3H5z M5 21h14",
  blur: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18 M9 10h.01 M15 10h.01 M6 15h12 M8 18h8",

  merge: "M4 4h7v7H4z M13 13h7v7h-7z M11 7h4a2 2 0 0 1 2 2v4",
  split: "M12 3v6 M8 9h8l-4 5z M4 20h16 M4 16v4 M20 16v4",
  removePages: "M5 3h9l5 5v13H5z M14 3v5h5 M9 14h6",
  extractPages: "M5 3h9l5 5v13H5z M14 3v5h5 M12 10v7 M9 14l3 3 3-3",
  organize: "M3 4h7v7H3z M14 4h7v7h-7z M3 15h7v5H3z M14 15h7v5h-7z",
  scan: "M3 7V5a2 2 0 0 1 2-2h2 M17 3h2a2 2 0 0 1 2 2v2 M21 17v2a2 2 0 0 1-2 2h-2 M7 21H5a2 2 0 0 1-2-2v-2 M3 12h18",
  repair: "M14 7l3 3 M5 19l9-9 M14 10l4-4a4 4 0 1 0-5-5l-1 4 3 5z M7 17l-2 2",
  ocr: "M4 7V4h16v3 M9 20h6 M12 4v16",
  pdfToJpg: "M5 3h9l5 5v13H5z M14 3v5h5 M8 17l3-3 2 2 3-4v5H8z",
  markdown: "M4 7h16v10H4z M7 14v-4l2 2 2-2v4 M15 10v4 M13 12l2 2 2-2",
  pageNumbers: "M5 3h9l5 5v13H5z M14 3v5h5 M9 17h2v-4H9 M14 13h2v4h-2z",
  annotate: "M12 20h9 M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4z",
  forms: "M5 3h14v18H5z M9 8h6 M9 12h6 M9 16h3",
  unlock: "M5 11h14v10H5z M8 11V7a4 4 0 0 1 7-2.6",
  protect: "M12 3l8 3v6c0 5-3.4 8.2-8 9-4.6-.8-8-4-8-9V6z M9 12l2 2 4-4",
  sign: "M3 18c4 0 5-10 8-10s2 7 5 7 3-3 5-3 M3 21h18",
  redact: "M5 3h9l5 5v13H5z M14 3v5h5 M8 13h8v4H8z",
  compare: "M12 3v18 M8 7H4v10h4 M16 7h4v10h-4",
  pdfa: "M5 3h9l5 5v13H5z M14 3v5h5 M9 18l2-6 2 6 M9.7 16h2.6",
  batch: "M4 4h7v7H4z M13 4h7v7h-7z M4 13h7v7H4z M15 17h5 M17.5 14.5v5",
} as const;

export const TOOLS: Tool[] = [
  // ---------- images ----------
  t("image-compress", "images", "optimize", "Compress image", "To a size, not a guess", ICON.compress, "ready"),
  t("image-batch", "images", "optimize", "Batch", "The same job on a whole folder", ICON.batch, "ready"),
  t("image-upscale", "images", "optimize", "Upscale", "Enlarge without the blur", ICON.upscale, "ready"),
  t("image-removebg", "images", "optimize", "Remove background", "One click, clean edges", ICON.removebg, "ready"),
  t("image-meme", "images", "create", "Meme generator", "Captions, top and bottom", ICON.meme, "ready"),
  t("image-editor", "images", "create", "Photo editor", "Adjust, filter, retouch", ICON.editor, "ready"),
  t("image-resize", "images", "modify", "Resize image", "Pixels, percent or presets", ICON.resize, "ready"),
  t("image-crop", "images", "modify", "Crop image", "Free or fixed ratios", ICON.crop, "ready"),
  t("image-rotate", "images", "modify", "Rotate image", "Rotate, flip, straighten", ICON.rotate, "ready"),
  t("image-to-jpg", "images", "convert", "Convert to JPG", "From PNG, WEBP, HEIC, GIF", ICON.tojpg, "ready"),
  t("image-from-jpg", "images", "convert", "Convert from JPG", "To PNG, WEBP, GIF, PDF", ICON.fromjpg, "ready"),
  t("image-from-html", "images", "convert", "HTML to image", "Snapshot a page or snippet", ICON.html, "ready"),
  t("image-watermark", "images", "security", "Watermark image", "Text or logo, in batches", ICON.watermark, "ready"),
  t("image-blur-face", "images", "security", "Blur face", "Auto-detect and hide faces", ICON.blur, "ready"),

  // ---------- pdf ----------
  t("pdf-compress", "pdf", "pdf-optimize", "Compress PDF", "To a size, not a guess", ICON.compress, "ready"),
  t("pdf-repair", "pdf", "pdf-optimize", "Repair PDF", "Recover a damaged file", ICON.repair, "ready"),
  t("pdf-ocr", "pdf", "pdf-optimize", "OCR PDF", "Make a scan searchable", ICON.ocr, "ready"),
  t("pdf-batch", "pdf", "pdf-optimize", "Batch", "Many files to one size", ICON.batch, "ready"),

  t("pdf-merge", "pdf", "pdf-organize", "Merge PDF", "Several files into one", ICON.merge, "ready"),
  t("pdf-split", "pdf", "pdf-organize", "Split PDF", "By range or every page", ICON.split, "ready"),
  t("pdf-remove-pages", "pdf", "pdf-organize", "Remove pages", "Drop what you don't need", ICON.removePages, "ready"),
  t("pdf-extract-pages", "pdf", "pdf-organize", "Extract pages", "Pull pages into a new file", ICON.extractPages, "ready"),
  t("pdf-organize", "pdf", "pdf-organize", "Organize PDF", "Reorder by dragging", ICON.organize, "ready"),
  t("pdf-scan", "pdf", "pdf-organize", "Scan to PDF", "Straighten and clean a photo", ICON.scan, "ready"),

  t("pdf-from-jpg", "pdf", "pdf-convert-to", "JPG to PDF", "Images into one document", ICON.tojpg, "ready"),
  t("pdf-from-html", "pdf", "pdf-convert-to", "HTML to PDF", "A web page, saved", ICON.html, "ready"),

  t("pdf-to-jpg", "pdf", "pdf-convert-from", "PDF to JPG", "Every page as an image", ICON.pdfToJpg, "ready"),
  t("pdf-to-markdown", "pdf", "pdf-convert-from", "PDF to Markdown", "Text and structure out", ICON.markdown, "ready"),
  t("pdf-to-pdfa", "pdf", "pdf-convert-from", "PDF to PDF/A", "For long-term archiving", ICON.pdfa, "ready"),

  t("pdf-rotate", "pdf", "pdf-edit", "Rotate PDF", "Fix sideways pages", ICON.rotate, "ready"),
  t("pdf-page-numbers", "pdf", "pdf-edit", "Add page numbers", "Position, range, format", ICON.pageNumbers, "ready"),
  t("pdf-watermark", "pdf", "pdf-edit", "Add watermark", "Text or image, any page", ICON.watermark, "ready"),
  t("pdf-crop", "pdf", "pdf-edit", "Crop PDF", "Trim margins and edges", ICON.crop, "ready"),
  t("pdf-annotate", "pdf", "pdf-edit", "Edit PDF", "Text, shapes, highlights", ICON.annotate, "ready"),
  t("pdf-forms", "pdf", "pdf-edit", "PDF forms", "Fill and flatten fields", ICON.forms, "ready"),

  t("pdf-unlock", "pdf", "pdf-security", "Unlock PDF", "Remove a password you know", ICON.unlock, "ready"),
  t("pdf-protect", "pdf", "pdf-security", "Protect PDF", "Encrypt and set permissions", ICON.protect, "ready"),
  t("pdf-sign", "pdf", "pdf-security", "Sign PDF", "Place your signature on a page", ICON.sign, "ready"),
  t("pdf-redact", "pdf", "pdf-security", "Redact PDF", "Really remove, not cover up", ICON.redact, "ready"),
  t("pdf-compare", "pdf", "pdf-security", "Compare PDF", "Text and visual differences", ICON.compare, "ready"),
];

function t(
  id: string,
  surface: Surface,
  category: string,
  name: string,
  desc: string,
  icon: string,
  status: ToolStatus = "soon",
): Tool {
  return { id, surface, category, name, desc, icon, status };
}

const CATEGORY_META: Record<string, { name: string; blurb: string }> = {
  optimize: { name: "Optimize", blurb: "Lighter, sharper, cleaner" },
  create: { name: "Create", blurb: "Make something new" },
  modify: { name: "Modify", blurb: "Shape and orient" },
  convert: { name: "Convert", blurb: "Any format in, any out" },
  security: { name: "Security", blurb: "Protect and anonymize" },
  "pdf-optimize": { name: "Optimize", blurb: "Smaller, cleaner, readable" },
  "pdf-organize": { name: "Organize", blurb: "Pages where you want them" },
  "pdf-convert-to": { name: "Convert to PDF", blurb: "Anything in" },
  "pdf-convert-from": { name: "Convert from PDF", blurb: "Anything out" },
  "pdf-edit": { name: "Edit", blurb: "Change what's on the page" },
  "pdf-security": { name: "Security", blurb: "Protect, sign, verify" },
};

export function categoriesFor(surface: Surface): Category[] {
  const order: string[] = [];
  for (const tool of TOOLS) {
    if (tool.surface !== surface) continue;
    if (!order.includes(tool.category)) order.push(tool.category);
  }
  return order.map((id) => ({
    id,
    name: CATEGORY_META[id]?.name ?? id,
    blurb: CATEGORY_META[id]?.blurb ?? "",
    tools: TOOLS.filter((t) => t.surface === surface && t.category === id),
  }));
}

export function toolsFor(surface: Surface): Tool[] {
  return TOOLS.filter((t) => t.surface === surface);
}

export function findTool(id: string): Tool | undefined {
  return TOOLS.find((t) => t.id === id);
}

export function searchTools(query: string): Tool[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  return TOOLS.filter(
    (t) => t.name.toLowerCase().includes(q) || t.desc.toLowerCase().includes(q),
  );
}

/** Which surface a dropped file belongs to, so the right tools come up. */
export function surfaceForFile(name: string): Surface {
  return name.toLowerCase().endsWith(".pdf") ? "pdf" : "images";
}
