import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  formatSize,
  isDesktop,
  isPdf,
  numberPages,
  pdfPages,
  pdfPreviews,
  revealOutput,
} from "../lib/bridge";
import type { NumberPosition, NumberResult } from "../lib/types";

/**
 * Page numbers.
 *
 * The preview is a CSS approximation of page 1 — the interface font, not
 * Helvetica — but it is honest about position and size, which is what you are
 * actually choosing here.
 */

const FORMATS: [string, string][] = [
  ["{n}", "1"],
  ["Page {n}", "Page 1"],
  ["{n} of {total}", "1 of 9"],
  ["Page {n} of {total}", "Page 1 of 9"],
  ["- {n} -", "- 1 -"],
];

const POSITIONS: NumberPosition[][] = [
  ["top_left", "top_centre", "top_right"],
  ["bottom_left", "bottom_centre", "bottom_right"],
];

export function Numbers({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const file = files.find(isPdf);
  const [preview, setPreview] = useState<string | null>(null);
  const [pageCount, setPageCount] = useState<number | null>(null);
  const [result, setResult] = useState<NumberResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const [template, setTemplate] = useState("{n}");
  const [position, setPosition] = useState<NumberPosition>("bottom_centre");
  const [startAt, setStartAt] = useState(1);
  const [fontSize, setFontSize] = useState(11);
  const [pages, setPages] = useState("");

  useEffect(() => {
    setResult(null);
    setError(null);
    setPreview(null);
    setPageCount(null);
    if (!file) return;
    let live = true;
    pdfPages(file)
      .then((list) => live && setPageCount(list.length))
      .catch(() => {});
    pdfPreviews(file, 420)
      .then((thumbs) => live && setPreview(thumbs?.[0] ?? null))
      .catch(() => live && setPreview(null));
    return () => {
      live = false;
    };
  }, [file]);

  const run = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await numberPages(file, pages.trim() || null, {
          template,
          position,
          startAt,
          fontSize,
          colour: [40, 40, 40],
          margin: 28,
        }),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // What page 1 of the numbered range will say.
  const sample = template
    .replace("{n}", String(startAt))
    .replace("{total}", String(pageCount ?? 9));

  return (
    <div className="col grow" style={{ gap: 22, minHeight: 0 }}>
      <header className="row" style={{ gap: 16, justifyContent: "space-between", flexWrap: "wrap" }}>
        <div className="row" style={{ gap: 16 }}>
          <button
            type="button"
            className="raised-sm icon-btn"
            aria-label="Back to all tools"
            onClick={onBack}
            style={{ width: 48, height: 48, borderRadius: 16 }}
          >
            <Icon path={ICONS.back} size={20} strokeWidth={2} />
          </button>
          <div className="col" style={{ gap: 2 }}>
            <span className="section-heading">Edit</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Add page numbers
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="application/pdf"
          hidden
          onChange={(e) => onFiles([...(e.target.files ?? [])])}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          {file ? "Change file" : "Open a PDF"}
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 18, gap: 12, minWidth: 0 }}
        >
          <div
            className="pressed grow"
            style={{ borderRadius: 20, padding: 14, display: "grid", placeItems: "center", minHeight: 0 }}
          >
            {!file ? (
              <div className="col" style={{ gap: 8, padding: 40, textAlign: "center" }}>
                <span className="display" style={{ fontSize: 18 }}>
                  {isDesktop() ? "Open a PDF" : "This needs the desktop app"}
                </span>
                <span style={{ fontSize: 13, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                  {isDesktop()
                    ? "Number every page, or start after the cover."
                    : "The browser preview can compress images but not edit documents. Open Kitbox itself."}
                </span>
              </div>
            ) : (
              <div
                style={{
                  position: "relative",
                  maxWidth: "100%",
                  maxHeight: 440,
                  aspectRatio: "1 / 1.3",
                  background: "#fff",
                  borderRadius: 10,
                  overflow: "hidden",
                  boxShadow: "0 0 0 1px rgba(0,0,0,0.06)",
                }}
              >
                {preview ? (
                  <img src={preview} alt="" style={{ width: "100%", height: "100%", objectFit: "contain" }} />
                ) : (
                  <span
                    style={{
                      position: "absolute",
                      inset: 0,
                      display: "grid",
                      placeItems: "center",
                      fontSize: 12,
                      color: "#5b6270",
                    }}
                  >
                    page 1
                  </span>
                )}
                <span
                  aria-hidden="true"
                  style={{
                    position: "absolute",
                    ...overlayPosition(position),
                    color: "#282828",
                    fontSize: Math.max(9, fontSize),
                    fontWeight: 500,
                    whiteSpace: "nowrap",
                  }}
                >
                  {sample}
                </span>
              </div>
            )}
          </div>
          <span style={{ fontSize: 11.5, color: "var(--text-subtle)", textAlign: "center", lineHeight: 1.5 }}>
            {file
              ? "Approximate preview of page 1. Pages stored sideways are handled — the number comes out upright."
              : ""}
          </span>
        </section>

        <aside
          className="raised col scroll"
          aria-label="Page number settings"
          style={{ flex: "0 0 320px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Format</span>
            <div className="col" style={{ gap: 6 }}>
              {FORMATS.map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={template === value}
                  className={template === value ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setTemplate(value);
                    setResult(null);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 40,
                    fontSize: 13,
                    fontWeight: template === value ? 700 : 500,
                    color: template === value ? "var(--accent)" : undefined,
                  }}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>

          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Where</span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 6 }}>
              {POSITIONS.flat().map((spot) => (
                <button
                  key={spot}
                  type="button"
                  aria-label={spot.replace(/_/g, " ")}
                  aria-pressed={position === spot}
                  className={position === spot ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setPosition(spot);
                    setResult(null);
                  }}
                  style={{ height: 36, borderRadius: 10, display: "grid", placeItems: "center" }}
                >
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: 2,
                      background: position === spot ? "var(--accent)" : "var(--shadow-dark)",
                    }}
                  />
                </button>
              ))}
            </div>
          </div>

          <div className="row" style={{ gap: 12 }}>
            <Field
              label="Start at"
              value={startAt}
              onChange={(v) => {
                setStartAt(v);
                setResult(null);
              }}
            />
            <Field
              label="Size"
              value={fontSize}
              onChange={(v) => {
                setFontSize(v);
                setResult(null);
              }}
            />
          </div>

          <div className="col" style={{ gap: 8 }}>
            <label htmlFor="num-pages" style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Pages
            </label>
            <span className="pressed field">
              <input
                id="num-pages"
                placeholder="all"
                value={pages}
                onChange={(e) => {
                  setPages(e.target.value);
                  setResult(null);
                }}
                style={{ fontSize: 14 }}
              />
            </span>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              Write 2- to skip the cover. "{"{total}"}" counts the pages that get a number, so
              skipping one keeps the count honest.
            </span>
          </div>

          {error && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
                <Icon path={ICONS.warning} size={16} strokeWidth={2} />
                That didn't work
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.55 }}>{error}</span>
            </div>
          )}

          {result && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {result.pagesNumbered} of {result.totalPages} pages · {formatSize(result.size)}
              </span>
              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 40 }}
                onClick={() => revealOutput(result.outputPath)}
              >
                <Icon path={ICONS.save} size={16} strokeWidth={2} />
                Show in folder
              </button>
            </div>
          )}

          <span className="grow" />

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!file || busy}
            onClick={run}
          >
            {busy ? "Numbering…" : "Add the numbers"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function overlayPosition(position: NumberPosition): {
  left?: string;
  right?: string;
  top?: string;
  bottom?: string;
  transform?: string;
} {
  const edge = "5%";
  const vertical = position.startsWith("top") ? { top: edge } : { bottom: edge };
  if (position.endsWith("left")) return { ...vertical, left: edge };
  if (position.endsWith("right")) return { ...vertical, right: edge };
  return { ...vertical, left: "50%", transform: "translateX(-50%)" };
}

function Field({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  const id = `num-${label.replace(/\s+/g, "-").toLowerCase()}`;
  return (
    <div className="col grow" style={{ gap: 8, minWidth: 0 }}>
      <label htmlFor={id} style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
        {label}
      </label>
      <span className="pressed field" style={{ minWidth: 0, padding: "0 8px" }}>
        <input
          id={id}
          type="number"
          min={1}
          value={value}
          onChange={(e) => onChange(Math.max(1, Number(e.target.value)))}
        />
      </span>
    </div>
  );
}
