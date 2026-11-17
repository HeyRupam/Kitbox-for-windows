import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, isDesktop, isPdf, revealOutput, transformImage } from "../lib/bridge";
import type {
  Flip,
  OutputFormat,
  ResizeMode,
  TransformOp,
  TransformResult,
} from "../lib/types";

export type TransformTool = "resize" | "crop" | "rotate" | "convert";

const TITLES: Record<TransformTool, [string, string]> = {
  resize: ["Modify", "Resize image"],
  crop: ["Modify", "Crop image"],
  rotate: ["Modify", "Rotate image"],
  convert: ["Convert", "Convert image"],
};

/** Common photo shapes, including the ones forms ask for. */
const ASPECTS: [string, number][] = [
  ["1:1", 1],
  ["3.5:4.5", 3.5 / 4.5],
  ["4:5", 0.8],
  ["3:4", 0.75],
  ["16:9", 16 / 9],
];

export function Transform({
  tool,
  files,
  onFiles,
  onBack,
}: {
  tool: TransformTool;
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const file = files.find((f) => !isPdf(f));
  const [source, setSource] = useState<{ url: string; width: number; height: number } | null>(null);
  const [result, setResult] = useState<TransformResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  // resize
  const [resizeKind, setResizeKind] = useState<"fit" | "fill" | "percent" | "longest_edge">("fit");
  const [width, setWidth] = useState(800);
  const [height, setHeight] = useState(600);
  const [percent, setPercent] = useState(50);
  const [longest, setLongest] = useState(1200);
  // crop
  const [aspect, setAspect] = useState(3.5 / 4.5);
  // rotate
  const [degrees, setDegrees] = useState(90);
  const [flip, setFlip] = useState<Flip>("none");
  // convert
  const [format, setFormat] = useState<OutputFormat>("png");
  const [quality, setQuality] = useState(88);

  useEffect(() => {
    setResult(null);
    setError(null);
    if (!file) {
      setSource(null);
      return;
    }
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => setSource({ url, width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => setSource(null);
    img.src = url;
    return () => URL.revokeObjectURL(url);
  }, [file]);

  const resizeMode = (): ResizeMode => {
    if (resizeKind === "percent") return { kind: "percent", value: percent };
    if (resizeKind === "longest_edge") return { kind: "longest_edge", value: longest };
    return { kind: resizeKind, value: { width, height } };
  };

  const op = (): TransformOp => {
    switch (tool) {
      case "resize":
        return { kind: "resize", value: resizeMode() };
      case "crop":
        return { kind: "crop_aspect", value: aspect };
      case "rotate":
        return { kind: "rotate", value: { degrees, flip } };
      case "convert":
        return { kind: "convert" };
    }
  };

  const predicted = (): string | null => {
    if (!source || tool !== "resize") return null;
    const { width: w, height: h } = source;
    const round = (n: number) => Math.max(1, Math.round(n));
    switch (resizeKind) {
      case "fill":
        return `${width} × ${height}`;
      case "fit": {
        const scale = Math.min(width / w, height / h);
        return `${round(w * scale)} × ${round(h * scale)}`;
      }
      case "percent":
        return `${round((w * percent) / 100)} × ${round((h * percent) / 100)}`;
      case "longest_edge": {
        const scale = longest / Math.max(w, h);
        return `${round(w * scale)} × ${round(h * scale)}`;
      }
    }
  };

  const run = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await transformImage(file, op(), {
          format: tool === "convert" ? format : "keep",
          quality,
          stripMetadata: true,
        }),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const [section, title] = TITLES[tool];

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
            <span className="section-heading">{section}</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              {title}
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          hidden
          onChange={(e) => onFiles([...(e.target.files ?? [])])}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          {file ? "Change image" : "Open an image"}
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 18, gap: 14, minWidth: 0 }}
        >
          <div
            className="pressed grow"
            style={{
              borderRadius: 20,
              padding: 12,
              display: "grid",
              placeItems: "center",
              minHeight: 0,
            }}
          >
            {result ? (
              <img
                src={result.previewUrl}
                alt="Result"
                style={{ maxWidth: "100%", maxHeight: 440, objectFit: "contain", borderRadius: 10 }}
              />
            ) : source ? (
              <img
                src={source.url}
                alt=""
                style={{ maxWidth: "100%", maxHeight: 440, objectFit: "contain", borderRadius: 10 }}
              />
            ) : (
              <div className="col" style={{ gap: 8, padding: 40, textAlign: "center" }}>
                <span className="display" style={{ fontSize: 18 }}>
                  {isDesktop() ? "Open an image" : "This needs the desktop app"}
                </span>
                <span style={{ fontSize: 13, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                  {isDesktop()
                    ? "Drop one anywhere in the window."
                    : "The browser preview covers compression only. Open Kitbox itself for the rest."}
                </span>
              </div>
            )}
          </div>
          {source && (
            <div
              className="row"
              style={{ justifyContent: "space-between", fontSize: 13, color: "var(--text-subtle)", padding: "0 6px" }}
            >
              <span>
                {file?.name} · {source.width} × {source.height}
              </span>
              <span>
                {result
                  ? `now ${result.width} × ${result.height} · ${formatSize(result.size)}`
                  : (predicted() ?? "")}
              </span>
            </div>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          {tool === "resize" && (
            <>
              <div className="col" style={{ gap: 10 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  How
                </span>
                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                  {(
                    [
                      ["fit", "Fit inside"],
                      ["fill", "Exact size"],
                      ["percent", "Percent"],
                      ["longest_edge", "Longest edge"],
                    ] as [typeof resizeKind, string][]
                  ).map(([id, label]) => (
                    <Pick
                      key={id}
                      active={resizeKind === id}
                      label={label}
                      onClick={() => {
                        setResizeKind(id);
                        setResult(null);
                      }}
                    />
                  ))}
                </div>
                <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {resizeKind === "fill"
                    ? "Covers the box exactly and trims the overflow — nothing gets squashed."
                    : resizeKind === "fit"
                      ? "Largest size that fits inside the box, shape unchanged."
                      : "Scaled proportionally."}
                </span>
              </div>

              {(resizeKind === "fit" || resizeKind === "fill") && (
                <div className="row" style={{ gap: 10 }}>
                  <Num label="Width" value={width} onChange={(v) => { setWidth(v); setResult(null); }} />
                  <span style={{ color: "var(--text-subtle)" }}>×</span>
                  <Num label="Height" value={height} onChange={(v) => { setHeight(v); setResult(null); }} />
                </div>
              )}
              {resizeKind === "percent" && (
                <Num label="Percent" value={percent} onChange={(v) => { setPercent(v); setResult(null); }} />
              )}
              {resizeKind === "longest_edge" && (
                <Num label="Pixels" value={longest} onChange={(v) => { setLongest(v); setResult(null); }} />
              )}
            </>
          )}

          {tool === "crop" && (
            <div className="col" style={{ gap: 10 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                Shape
              </span>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                {ASPECTS.map(([label, value]) => (
                  <Pick
                    key={label}
                    active={Math.abs(aspect - value) < 0.001}
                    label={label}
                    onClick={() => {
                      setAspect(value);
                      setResult(null);
                    }}
                  />
                ))}
              </div>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Takes the largest centred crop of that shape. Free-hand cropping with a draggable
                box isn't built yet.
              </span>
            </div>
          )}

          {tool === "rotate" && (
            <>
              <div className="col" style={{ gap: 10 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Turn
                </span>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
                  {[90, 180, 270].map((d) => (
                    <Pick
                      key={d}
                      active={degrees === d}
                      label={`${d}°`}
                      onClick={() => {
                        setDegrees(d);
                        setResult(null);
                      }}
                    />
                  ))}
                </div>
              </div>
              <div className="col" style={{ gap: 10 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Mirror
                </span>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
                  {(
                    [
                      ["none", "No"],
                      ["horizontal", "Across"],
                      ["vertical", "Down"],
                    ] as [Flip, string][]
                  ).map(([id, label]) => (
                    <Pick
                      key={id}
                      active={flip === id}
                      label={label}
                      onClick={() => {
                        setFlip(id);
                        setResult(null);
                      }}
                    />
                  ))}
                </div>
              </div>
            </>
          )}

          {tool === "convert" && (
            <>
              <div className="col" style={{ gap: 10 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Write as
                </span>
                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                  {(
                    [
                      ["jpeg", "JPG"],
                      ["png", "PNG"],
                    ] as [OutputFormat, string][]
                  ).map(([id, label]) => (
                    <Pick
                      key={id}
                      active={format === id}
                      label={label}
                      onClick={() => {
                        setFormat(id);
                        setResult(null);
                      }}
                    />
                  ))}
                </div>
                <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  JPG is smaller and has no transparency; PNG is lossless and keeps it. Converting
                  a photo to PNG usually makes it much bigger.
                </span>
              </div>
              {format === "jpeg" && (
                <div className="col" style={{ gap: 10 }}>
                  <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline" }}>
                    <label htmlFor="q" style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                      Quality
                    </label>
                    <span className="display" style={{ fontSize: 20, color: "var(--accent)" }}>
                      {quality}%
                    </span>
                  </div>
                  <input
                    id="q"
                    type="range"
                    min={10}
                    max={100}
                    step={1}
                    value={quality}
                    onChange={(e) => {
                      setQuality(Number(e.target.value));
                      setResult(null);
                    }}
                  />
                </div>
              )}
            </>
          )}

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
              <span className="display" style={{ fontSize: 19 }}>
                {result.width} × {result.height} · {formatSize(result.size)}
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
            {busy ? "Working…" : "Apply and save"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Pick({ active, label, onClick }: { active: boolean; label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      className={active ? "pressed" : "raised-sm"}
      onClick={onClick}
      style={{
        borderRadius: 12,
        minHeight: 42,
        fontSize: 12.5,
        fontWeight: active ? 700 : 500,
        color: active ? "var(--accent)" : undefined,
        padding: "0 6px",
      }}
    >
      {label}
    </button>
  );
}

function Num({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  const id = `num-${label.toLowerCase()}`;
  // minWidth: 0 is load-bearing — a flex item defaults to its content's
  // intrinsic width, which pushes the second field out of the panel.
  return (
    <span className="pressed field" style={{ flex: 1, minWidth: 0, padding: "0 8px" }}>
      <label className="sr-only" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        type="number"
        min={1}
        value={value}
        onChange={(e) => onChange(Math.max(1, Number(e.target.value)))}
      />
    </span>
  );
}
