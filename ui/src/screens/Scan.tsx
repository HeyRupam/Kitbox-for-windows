import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, isDesktop, isPdf, revealOutput, scanPreview, scanToPdf } from "../lib/bridge";
import type { ImagesToPdfOptions, ScanMode, ScanOptions, ScanPreview, ScanResult } from "../lib/types";

/**
 * Photographs of pages, cleaned up into a PDF that looks scanned.
 *
 * Finding the page's edges is the step that fails — on a white page lying on a
 * white table there is nothing to find. So the preview is the screen, not a
 * detail of it: you see the flattened result for the selected photograph, and a
 * badge saying whether its edges were found, before anything is written. A
 * silently mis-cropped scan is worse than no crop at all.
 */
export function Scan({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const images = files.filter((f) => !isPdf(f));
  const [mode, setMode] = useState<ScanMode>("document");
  const [detectEdges, setDetectEdges] = useState(true);
  const [strength, setStrength] = useState(0.6);
  const [selected, setSelected] = useState(0);
  const [preview, setPreview] = useState<ScanPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [result, setResult] = useState<ScanResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const current = images[Math.min(selected, images.length - 1)];
  const options: ScanOptions = { mode, detectEdges, strength };

  // Re-clean the selected photograph whenever it or a setting changes. The
  // settings that matter are cheap to get wrong, so they are shown, not
  // guessed at.
  useEffect(() => {
    if (!current || !isDesktop()) {
      setPreview(null);
      return;
    }
    let cancelled = false;
    setPreviewing(true);
    const timer = setTimeout(() => {
      scanPreview(current, { mode, detectEdges, strength })
        .then((p) => !cancelled && setPreview(p))
        .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => !cancelled && setPreviewing(false));
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [current, mode, detectEdges, strength]);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const pdf: ImagesToPdfOptions = {
        pageSize: "a4",
        orientation: "auto",
        marginPt: 0,
        jpegQuality: 86,
        maxDpi: 200,
      };
      setResult(await scanToPdf(images, "scan.pdf", options, pdf));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
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
            <span className="section-heading">Organize PDF</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Scan to PDF
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          multiple
          hidden
          onChange={(e) => {
            onFiles([...images, ...(e.target.files ?? [])]);
            setResult(null);
          }}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          Add photos
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          {!current ? (
            <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                {isDesktop() ? "Add photographs of the pages" : "This needs the desktop app"}
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 540, lineHeight: 1.65 }}>
                {isDesktop()
                  ? "Each page's edges are found and the page is flattened, so a photo taken at an angle comes out square. Lay the page on something a different colour — that contrast is what makes the edges findable."
                  : "The browser preview can compress images but not build documents. Open Kitbox itself."}
              </p>
            </div>
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center" }}>
                <span className="section-heading">
                  Page {Math.min(selected, images.length - 1) + 1} of {images.length}
                </span>
                {preview && (
                  <span
                    className="pressed pill"
                    style={{
                      fontSize: 11.5,
                      color: preview.edgesFound ? "var(--accent)" : "var(--text-subtle)",
                      fontWeight: 600,
                    }}
                  >
                    {preview.edgesFound
                      ? `Edges found · ${preview.width} × ${preview.height}`
                      : detectEdges
                        ? "No edges found — used whole"
                        : "Used whole"}
                  </span>
                )}
                {previewing && (
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>working…</span>
                )}
              </div>

              <div
                className="row grow"
                style={{ gap: 14, minHeight: 0, alignItems: "stretch" }}
              >
                <Pane label="Photograph">
                  <Original file={current} />
                </Pane>
                <Pane label="Scanned">
                  {preview ? (
                    <img
                      src={preview.image}
                      alt="The cleaned-up page"
                      style={{ maxWidth: "100%", maxHeight: "100%", objectFit: "contain" }}
                    />
                  ) : (
                    <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>…</span>
                  )}
                </Pane>
              </div>

              {images.length > 1 && (
                <div className="row scroll-x" style={{ gap: 10, paddingTop: 2 }}>
                  {images.map((f, i) => (
                    <button
                      key={`${f.name}-${f.size}-${i}`}
                      type="button"
                      aria-pressed={i === selected}
                      aria-label={`Page ${i + 1}, ${f.name}`}
                      className={i === selected ? "pressed" : "raised-sm"}
                      onClick={() => setSelected(i)}
                      style={{
                        flex: "0 0 auto",
                        width: 64,
                        height: 64,
                        borderRadius: 12,
                        padding: 4,
                        overflow: "hidden",
                      }}
                    >
                      <Original file={f} />
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Look</span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
              {(
                [
                  ["document", "Document"],
                  ["greyscale", "Grey"],
                  ["colour", "Colour"],
                ] as [ScanMode, string][]
              ).map(([id, text]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={mode === id}
                  className={mode === id ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setMode(id);
                    setResult(null);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 42,
                    fontSize: 12,
                    fontWeight: mode === id ? 700 : 500,
                    color: mode === id ? "var(--accent)" : undefined,
                  }}
                >
                  {text}
                </button>
              ))}
            </div>
            <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.55 }}>
              {mode === "document"
                ? "Evens out the lighting across the page, so the shadow down one side goes and the paper reads as white."
                : mode === "greyscale"
                  ? "Faithful, just without the colour. Smaller than colour and kinder to a yellowed page."
                  : "Flattened only, colours untouched. For anything where the colour is the point."}
            </span>
          </div>

          {mode === "document" && (
            <div className="col" style={{ gap: 8 }}>
              <div className="row" style={{ justifyContent: "space-between" }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Contrast
                </span>
                <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                  {Math.round(strength * 100)}%
                </span>
              </div>
              <input
                type="range"
                min={0}
                max={100}
                value={Math.round(strength * 100)}
                aria-label="How hard to push the contrast"
                onChange={(e) => {
                  setStrength(Number(e.target.value) / 100);
                  setResult(null);
                }}
              />
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Higher is cleaner and whiter. Too high and pencil, faint stamps and signatures
                disappear with the shadow.
              </span>
            </div>
          )}

          <label className="row" style={{ gap: 10, alignItems: "flex-start", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={detectEdges}
              onChange={(e) => {
                setDetectEdges(e.target.checked);
                setResult(null);
              }}
              style={{ marginTop: 3 }}
            />
            <span className="col" style={{ gap: 3 }}>
              <span style={{ fontSize: 13, fontWeight: 600 }}>Find the page's edges</span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Crops to the page and squares it up. Turn off if the page already fills the frame.
              </span>
            </span>
          </label>

          {error && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
                <Icon path={ICONS.warning} size={16} strokeWidth={2} />
                That didn't work
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                {error}
              </span>
            </div>
          )}

          {result && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 20 }}>
                {result.pages} page{result.pages === 1 ? "" : "s"} · {formatSize(result.size)}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                {result.flattened} of {result.pages} flattened from their edges.
                {result.note ? ` ${result.note}` : ""}
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
            disabled={images.length === 0 || busy}
            onClick={run}
          >
            {busy ? "Scanning…" : `Scan ${images.length || ""} to PDF`.trim()}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Pane({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="col grow" style={{ gap: 8, minWidth: 0, minHeight: 0 }}>
      <span className="section-heading" style={{ fontSize: 10.5 }}>
        {label}
      </span>
      <div
        className="pressed grow"
        style={{
          borderRadius: 18,
          padding: 12,
          display: "grid",
          placeItems: "center",
          minHeight: 0,
          overflow: "hidden",
        }}
      >
        {children}
      </div>
    </div>
  );
}

function Original({ file }: { file: File }) {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    const next = URL.createObjectURL(file);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [file]);
  if (!url) return null;
  return (
    <img
      src={url}
      alt=""
      style={{ maxWidth: "100%", maxHeight: "100%", objectFit: "contain" }}
    />
  );
}
