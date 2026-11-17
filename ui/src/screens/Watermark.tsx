import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  formatSize,
  isDesktop,
  isPdf,
  pdfPreviews,
  revealOutput,
  watermarkImage,
  watermarkPdf,
} from "../lib/bridge";
import type { Placement } from "../lib/types";

/**
 * Stamp text or a logo across a PDF.
 *
 * The preview is an approximation drawn in CSS — it uses the interface font,
 * not Helvetica, and it can only show the first page. It is here because
 * choosing opacity and placement blind is miserable, and it says what it is
 * rather than pretending to be the output.
 */

const GRID: Placement[][] = [
  ["top_left", "top_centre", "top_right"],
  ["middle_left", "centre", "middle_right"],
  ["bottom_left", "bottom_centre", "bottom_right"],
];

const COLOURS: [string, [number, number, number]][] = [
  ["Grey", [90, 90, 90]],
  ["Red", [194, 65, 12]],
  ["Blue", [29, 78, 216]],
  ["Black", [20, 20, 20]],
];

export function Watermark({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  // Either kind of file is welcome; what it is decides the rest of the screen.
  const file = files[0];
  const pdfMode = file ? isPdf(file) : false;
  const [preview, setPreview] = useState<string | null>(null);
  const [result, setResult] = useState<{ outputPath: string; summary: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const logoRef = useRef<HTMLInputElement>(null);

  const [text, setText] = useState("CONFIDENTIAL");
  const [logo, setLogo] = useState<{ name: string; bytes: number[]; url: string } | null>(null);
  const [placement, setPlacement] = useState<Placement>("centre");
  const [opacity, setOpacity] = useState(15);
  const [rotation, setRotation] = useState(45);
  const [colour, setColour] = useState<[number, number, number]>([90, 90, 90]);
  const [pages, setPages] = useState("");
  // Mark width as a percentage of the width. A PDF text stamp sizes itself
  // from the page, so this drives logos there and everything on an image.
  const [scale, setScale] = useState(45);

  useEffect(() => {
    setResult(null);
    setError(null);
    setPreview(null);
    if (!file) return;
    if (!isPdf(file)) {
      // An image is its own preview.
      const url = URL.createObjectURL(file);
      setPreview(url);
      return () => URL.revokeObjectURL(url);
    }
    let live = true;
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
      if (pdfMode) {
        const done = await watermarkPdf(file, pages.trim() || null, {
          mark: logo ? { kind: "image", value: logo.bytes } : { kind: "text", value: text },
          placement,
          opacity: opacity / 100,
          rotation,
          fontSize: null,
          imageScale: scale / 100,
          colour,
          margin: 36,
        });
        setResult({
          outputPath: done.outputPath,
          summary: `${done.pagesMarked} of ${done.totalPages} pages \u00b7 ${formatSize(done.size)}`,
        });
      } else {
        const done = await watermarkImage(
          file,
          logo ? { kind: "logo", value: logo.bytes } : { kind: "text", value: text },
          {
            placement,
            opacity: opacity / 100,
            rotation,
            scale: scale / 100,
            colour,
            margin: 0.04,
            format: "keep",
            quality: 92,
          },
        );
        setResult({
          outputPath: done.outputPath,
          summary: `${done.width}\u00d7${done.height} \u00b7 ${formatSize(done.size)}`,
        });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const spots = placement === "tiled" ? [0, 1, 2, 3, 4, 5] : [0];

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
              Add watermark
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="application/pdf,image/*"
          hidden
          onChange={(e) => onFiles([...(e.target.files ?? [])])}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          {file ? "Change file" : "Open a file"}
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
                  {isDesktop() ? "Open an image or a PDF" : "This needs the desktop app"}
                </span>
                <span style={{ fontSize: 13, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                  {isDesktop()
                    ? "Stamp text or a logo onto an image, or across a PDF's pages."
                    : "The browser preview can compress images but not stamp them. Open Kitbox itself."}
                </span>
              </div>
            ) : (
              <div
                style={{
                  position: "relative",
                  maxWidth: "100%",
                  maxHeight: 440,
                  // A page is always portrait-ish; an image takes whatever
                  // shape it is, so forcing a ratio would letterbox it.
                  aspectRatio: pdfMode ? "1 / 1.3" : undefined,
                  display: pdfMode ? undefined : "flex",
                  background: "#fff",
                  borderRadius: 10,
                  overflow: "hidden",
                  boxShadow: "0 0 0 1px rgba(0,0,0,0.06)",
                }}
              >
                {preview ? (
                  <img
                    src={preview}
                    alt=""
                    style={
                      pdfMode
                        ? { width: "100%", height: "100%", objectFit: "contain" }
                        : { maxWidth: "100%", maxHeight: 440, display: "block" }
                    }
                  />
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
                {spots.map((i) => (
                  <span
                    key={i}
                    aria-hidden="true"
                    style={{
                      position: "absolute",
                      ...overlayPosition(placement, i),
                      transform: `translate(-50%, -50%) rotate(${-rotation}deg)`,
                      opacity: opacity / 100,
                      color: `rgb(${colour[0]}, ${colour[1]}, ${colour[2]})`,
                      fontWeight: 700,
                      fontSize: placement === "tiled" ? 15 : 30,
                      whiteSpace: "nowrap",
                      pointerEvents: "none",
                    }}
                  >
                    {logo ? (
                      <img src={logo.url} alt="" style={{ width: placement === "tiled" ? 60 : 140 }} />
                    ) : (
                      text || " "
                    )}
                  </span>
                ))}
              </div>
            )}
          </div>
          <span
            style={{ fontSize: 11.5, color: "var(--text-subtle)", textAlign: "center", lineHeight: 1.5 }}
          >
            {file
              ? pdfMode
                ? "Approximate preview of page 1 — the real stamp uses Helvetica and covers the pages you pick."
                : "Approximate preview — the real stamp is drawn with a font from this machine."
              : ""}
          </span>
        </section>

        <aside
          className="raised col scroll"
          aria-label="Watermark settings"
          style={{ flex: "0 0 320px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Mark</span>
            <div className="pressed segmented" style={{ gridTemplateColumns: "1fr 1fr" }}>
              <button
                type="button"
                aria-pressed={!logo}
                className={!logo ? "raised-sm" : undefined}
                onClick={() => {
                  setLogo(null);
                  setResult(null);
                }}
              >
                Text
              </button>
              <button
                type="button"
                aria-pressed={!!logo}
                className={logo ? "raised-sm" : undefined}
                onClick={() => logoRef.current?.click()}
              >
                Logo
              </button>
            </div>
            <input
              ref={logoRef}
              type="file"
              accept="image/*"
              hidden
              onChange={async (e) => {
                const picked = e.target.files?.[0];
                if (!picked) return;
                const bytes = Array.from(new Uint8Array(await picked.arrayBuffer()));
                setLogo({ name: picked.name, bytes, url: URL.createObjectURL(picked) });
                setResult(null);
              }}
            />
            {logo ? (
              <span className="truncate" style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {logo.name}
              </span>
            ) : (
              <span className="pressed field">
                <label className="sr-only" htmlFor="wm-text">
                  Watermark text
                </label>
                <input
                  id="wm-text"
                  value={text}
                  onChange={(e) => {
                    setText(e.target.value);
                    setResult(null);
                  }}
                  style={{ fontSize: 14, fontWeight: 600 }}
                />
              </span>
            )}
          </div>

          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Where</span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 6 }}>
              {GRID.flat().map((spot) => (
                <button
                  key={spot}
                  type="button"
                  aria-label={spot.replace(/_/g, " ")}
                  aria-pressed={placement === spot}
                  className={placement === spot ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setPlacement(spot);
                    setResult(null);
                  }}
                  style={{ height: 34, borderRadius: 10, display: "grid", placeItems: "center" }}
                >
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: 2,
                      background: placement === spot ? "var(--accent)" : "var(--shadow-dark)",
                    }}
                  />
                </button>
              ))}
            </div>
            <button
              type="button"
              aria-pressed={placement === "tiled"}
              className={placement === "tiled" ? "pressed pill" : "raised-sm pill"}
              onClick={() => {
                setPlacement("tiled");
                setResult(null);
              }}
              style={{ alignSelf: "flex-start", color: placement === "tiled" ? "var(--accent)" : undefined }}
            >
              Tiled across the page
            </button>
          </div>

          <Slider label="Size" value={scale} min={5} max={100} suffix="%" onChange={(v) => { setScale(v); setResult(null); }} />
          <Slider label="Opacity" value={opacity} min={5} max={100} suffix="%" onChange={(v) => { setOpacity(v); setResult(null); }} />
          <Slider label="Angle" value={rotation} min={-90} max={90} suffix="°" onChange={(v) => { setRotation(v); setResult(null); }} />

          {!logo && (
            <div className="col" style={{ gap: 10 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Colour</span>
              <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 8 }}>
                {COLOURS.map(([name, rgb]) => (
                  <button
                    key={name}
                    type="button"
                    aria-label={name}
                    aria-pressed={colour.join() === rgb.join()}
                    className={colour.join() === rgb.join() ? "pressed" : "raised-sm"}
                    onClick={() => {
                      setColour(rgb);
                      setResult(null);
                    }}
                    style={{ height: 38, borderRadius: 11, display: "grid", placeItems: "center" }}
                  >
                    <span
                      style={{
                        width: 16,
                        height: 16,
                        borderRadius: "50%",
                        background: `rgb(${rgb.join(",")})`,
                      }}
                    />
                  </button>
                ))}
              </div>
            </div>
          )}

          {pdfMode && (
            <div className="col" style={{ gap: 8 }}>
              <label htmlFor="wm-pages" style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                Pages
              </label>
              <span className="pressed field">
                <input
                  id="wm-pages"
                  placeholder="all"
                  value={pages}
                  onChange={(e) => {
                    setPages(e.target.value);
                    setResult(null);
                  }}
                  style={{ fontSize: 14 }}
                />
              </span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                Leave empty for every page, or write 1-3,7.
              </span>
            </div>
          )}

          {error && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
                <Icon path={ICONS.warning} size={16} strokeWidth={2} />
                That didn't work
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.55 }}>
                {error}
              </span>
            </div>
          )}

          {result && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {result.summary}
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
            disabled={!file || busy || (!logo && !text.trim())}
            onClick={run}
          >
            {busy ? "Stamping…" : "Add the watermark"}
          </button>
        </aside>
      </div>
    </div>
  );
}

/** Where the approximate overlay sits, mirroring the engine's placements. */
function overlayPosition(placement: Placement, index: number): { left: string; top: string } {
  if (placement === "tiled") {
    const columns = 2;
    const left = `${((index % columns) + 0.5) * (100 / columns)}%`;
    const top = `${(Math.floor(index / columns) + 0.5) * (100 / 3)}%`;
    return { left, top };
  }
  const [row, column] = (() => {
    const flat = GRID.flat();
    const at = flat.indexOf(placement);
    return at < 0 ? [1, 1] : [Math.floor(at / 3), at % 3];
  })();
  const axis = ["12%", "50%", "88%"];
  return { left: axis[column], top: axis[row] };
}

function Slider({
  label,
  value,
  min,
  max,
  suffix,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  suffix: string;
  onChange: (value: number) => void;
}) {
  const id = `wm-${label.toLowerCase()}`;
  return (
    <div className="col" style={{ gap: 10 }}>
      <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline" }}>
        <label htmlFor={id} style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
          {label}
        </label>
        <span className="display" style={{ fontSize: 19, color: "var(--accent)" }}>
          {value}
          {suffix}
        </span>
      </div>
      <input
        id={id}
        type="range"
        min={min}
        max={max}
        step={1}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}
