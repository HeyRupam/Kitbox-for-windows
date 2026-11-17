import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { cropPdf, formatSize, isDesktop, isPdf, pdfPreviews, revealOutput } from "../lib/bridge";
import type { CropResult, CropSpec } from "../lib/types";

type Mode = "margins" | "percent" | "auto";

/**
 * Crop a PDF's pages.
 *
 * The preview draws the crop over page 1 so you can see what survives. The
 * warning about content remaining in the file is on screen before you press
 * anything, not buried in the result — someone cropping to hide an account
 * number needs to know first.
 */
export function CropPdf({
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
  const [result, setResult] = useState<CropResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const [mode, setMode] = useState<Mode>("margins");
  const [sides, setSides] = useState({ top: 20, right: 20, bottom: 20, left: 20 });
  const [linked, setLinked] = useState(true);
  const [padding, setPadding] = useState(6);
  const [pages, setPages] = useState("");

  useEffect(() => {
    setResult(null);
    setError(null);
    setPreview(null);
    if (!file) return;
    let live = true;
    pdfPreviews(file, 420)
      .then((thumbs) => live && setPreview(thumbs?.[0] ?? null))
      .catch(() => live && setPreview(null));
    return () => {
      live = false;
    };
  }, [file]);

  const setSide = (key: keyof typeof sides, value: number) => {
    setResult(null);
    setSides((current) => (linked ? { top: value, right: value, bottom: value, left: value } : { ...current, [key]: value }));
  };

  const spec = (): CropSpec => {
    if (mode === "auto") return { kind: "auto", value: { padding } };
    if (mode === "percent") {
      return {
        kind: "percent",
        value: {
          top: sides.top / 100,
          right: sides.right / 100,
          bottom: sides.bottom / 100,
          left: sides.left / 100,
        },
      };
    }
    return { kind: "margins", value: sides };
  };

  const run = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await cropPdf(file, pages.trim() || null, spec()));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // Rough overlay: percentages of the previewed page.
  const inset =
    mode === "auto"
      ? null
      : mode === "percent"
        ? sides
        : {
            // A4-ish page is ~595 × 842pt; good enough to show the idea.
            top: (sides.top / 842) * 100,
            bottom: (sides.bottom / 842) * 100,
            left: (sides.left / 595) * 100,
            right: (sides.right / 595) * 100,
          };

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
              Crop PDF
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
                    ? "Trim margins by hand, or let it shrink each page to the ink on it."
                    : "The browser preview can compress images but not edit documents. Open Kitbox itself."}
                </span>
              </div>
            ) : (
              <div
                style={{
                  position: "relative",
                  maxWidth: "100%",
                  maxHeight: 420,
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
                {inset ? (
                  <span
                    aria-hidden="true"
                    style={{
                      position: "absolute",
                      top: `${inset.top}%`,
                      right: `${inset.right}%`,
                      bottom: `${inset.bottom}%`,
                      left: `${inset.left}%`,
                      outline: "2px solid var(--accent)",
                      boxShadow: "0 0 0 9999px rgba(20,20,20,0.34)",
                    }}
                  />
                ) : (
                  <span
                    aria-hidden="true"
                    style={{
                      position: "absolute",
                      inset: 0,
                      display: "grid",
                      placeItems: "end center",
                      padding: 12,
                      fontSize: 11,
                      fontWeight: 600,
                      color: "var(--accent)",
                    }}
                  >
                    measured per page
                  </span>
                )}
              </div>
            )}
          </div>
          <span style={{ fontSize: 11.5, color: "var(--text-subtle)", textAlign: "center", lineHeight: 1.5 }}>
            {file ? "Approximate preview of page 1. Pages stored sideways are handled." : ""}
          </span>
        </section>

        <aside
          className="raised col scroll"
          aria-label="Crop settings"
          style={{ flex: "0 0 320px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>How</span>
            <div className="pressed segmented" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
              {(
                [
                  ["margins", "Points"],
                  ["percent", "Percent"],
                  ["auto", "Automatic"],
                ] as [Mode, string][]
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={mode === id}
                  className={mode === id ? "raised-sm" : undefined}
                  onClick={() => {
                    setMode(id);
                    setResult(null);
                  }}
                >
                  {label}
                </button>
              ))}
            </div>
            {mode === "auto" && (
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Each page shrinks to the ink on it. This measures the scanned image, so pages made
                of text and vector graphics are left alone and reported.
              </span>
            )}
          </div>

          {mode === "auto" ? (
            <Field label="Breathing room (pt)" value={padding} onChange={(v) => { setPadding(v); setResult(null); }} />
          ) : (
            <div className="col" style={{ gap: 12 }}>
              <div className="row" style={{ justifyContent: "space-between" }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Trim {mode === "percent" ? "(%)" : "(pt)"}
                </span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={linked}
                  onClick={() => setLinked(!linked)}
                  style={{ fontSize: 12, fontWeight: 600, color: linked ? "var(--accent)" : "var(--text-subtle)" }}
                >
                  {linked ? "All sides" : "Each side"}
                </button>
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
                {(["top", "right", "bottom", "left"] as const).map((side) => (
                  <Field
                    key={side}
                    label={side[0].toUpperCase() + side.slice(1)}
                    value={sides[side]}
                    onChange={(v) => setSide(side, v)}
                  />
                ))}
              </div>
            </div>
          )}

          <div className="col" style={{ gap: 8 }}>
            <label htmlFor="crop-pages" style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Pages
            </label>
            <span className="pressed field">
              <input
                id="crop-pages"
                placeholder="all"
                value={pages}
                onChange={(e) => {
                  setPages(e.target.value);
                  setResult(null);
                }}
                style={{ fontSize: 14 }}
              />
            </span>
          </div>

          <div
            className="pressed"
            style={{ borderRadius: 16, padding: 14, fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6 }}
          >
            Cropping sets the area readers display. Anything outside it is still in the file and
            can be recovered, so this is not a way to hide information.
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
                {result.pagesCropped} of {result.totalPages} pages · {formatSize(result.size)}
              </span>
              {result.note && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {result.note}
                </span>
              )}
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
            {busy ? "Cropping…" : "Crop the pages"}
          </button>
        </aside>
      </div>
    </div>
  );
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
  const id = `crop-${label.replace(/[^a-z]/gi, "").toLowerCase()}`;
  return (
    <div className="col" style={{ gap: 6, minWidth: 0 }}>
      <label htmlFor={id} style={{ fontSize: 12, color: "var(--text-subtle)" }}>
        {label}
      </label>
      <span className="pressed field" style={{ minWidth: 0, padding: "0 8px", minHeight: 42 }}>
        <input
          id={id}
          type="number"
          min={0}
          value={value}
          onChange={(e) => onChange(Math.max(0, Number(e.target.value)))}
        />
      </span>
    </div>
  );
}
