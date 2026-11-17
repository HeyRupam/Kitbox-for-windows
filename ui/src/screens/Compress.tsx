import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  compress,
  describeScore,
  formatSize,
  inspectPdf,
  isDesktop,
  isPdf,
  revealOutput,
} from "../lib/bridge";
import { SizeInspector } from "../components/SizeInspector";
import { encodeAtQuality } from "../lib/browserEngine";
import { byGroup, loadPresets, type Preset } from "../lib/presets";
import type {
  Bias,
  CompressResult,
  OutputFormat,
  PdfReport,
  SizeTarget,
  TargetMode,
} from "../lib/types";
import { targetMax, targetMin } from "../lib/types";

type Mode = "target" | "quality";

export function Compress({
  files,
  onFiles,
  onBack,
  startPreset,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
  /** Set when you arrived here from Form presets, with one already chosen. */
  startPreset?: Preset | null;
}) {
  const [mode, setMode] = useState<Mode>("target");
  const [targetMode, setTargetMode] = useState<TargetMode>("between");
  const [minKb, setMinKb] = useState(50);
  const [maxKb, setMaxKb] = useState(100);
  const [quality, setQuality] = useState(78);
  const [bias, setBias] = useState<Bias>("keep_sharp");
  const [format, setFormat] = useState<OutputFormat>("keep");
  const [strip, setStrip] = useState(true);
  const [preset, setPreset] = useState<Preset | null>(startPreset ?? null);
  const [showPresets, setShowPresets] = useState(false);

  const [active, setActive] = useState(0);
  const [results, setResults] = useState<Record<string, CompressResult>>({});
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [split, setSplit] = useState(50);
  const [pdfReport, setPdfReport] = useState<PdfReport | null>(null);
  const [allowGrayscale, setAllowGrayscale] = useState(false);
  const [minDpi, setMinDpi] = useState(72);

  const fileRef = useRef<HTMLInputElement>(null);
  const file = files[active];
  const result = file ? results[keyFor(file)] : undefined;

  const target: SizeTarget = useMemo(() => {
    if (preset) return preset.target;
    if (targetMode === "under") return { kind: "at_most", value: maxKb * 1000 };
    if (targetMode === "exactly") return { kind: "approx", value: { bytes: maxKb * 1000, tol: 0.1 } };
    return { kind: "range", value: { min: minKb * 1000, max: maxKb * 1000 } };
  }, [preset, targetMode, minKb, maxKb]);

  const pdfMode = file ? isPdf(file) : false;
  const originalUrl = useObjectUrl(pdfMode ? undefined : file);

  // Read the breakdown as soon as a PDF is selected, so you can see what you
  // are up against before pressing anything.
  useEffect(() => {
    if (!file || !isPdf(file)) {
      setPdfReport(null);
      return;
    }
    let live = true;
    inspectPdf(file)
      .then((r) => live && setPdfReport(r))
      .catch(() => live && setPdfReport(null));
    return () => {
      live = false;
    };
  }, [file]);

  const run = useCallback(async () => {
    if (!files.length) return;
    setRunning(true);
    setError(null);
    try {
      const next: Record<string, CompressResult> = {};
      for (const f of files) {
        next[keyFor(f)] =
          mode === "quality"
            ? await encodeAtQuality(f, quality, format)
            : await compress(f, {
                name: f.name,
                target,
                bias,
                format: preset ? (preset.format === "png" ? "png" : "jpeg") : format,
                stripMetadata: strip,
                allowGrayscale,
                minDpi,
                exact: preset?.dimensions
                  ? { width: preset.dimensions.width, height: preset.dimensions.height }
                  : undefined,
              });
      }
      setResults(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  }, [files, mode, quality, format, target, bias, strip, preset, allowGrayscale, minDpi]);

  // Clear stale results when the file set changes, so the panel never shows a
  // number that belongs to a file you already replaced.
  useEffect(() => {
    setResults({});
    setActive(0);
  }, [files]);

  const totalBefore = files.reduce((n, f) => n + f.size, 0);
  const totalAfter = files.reduce((n, f) => n + (results[keyFor(f)]?.size ?? 0), 0);
  const allDone = files.length > 0 && files.every((f) => results[keyFor(f)]);

  return (
    <div className="col grow" style={{ gap: 22, minHeight: 0 }}>
      <header
        className="row"
        style={{ flexWrap: "wrap", gap: 16, justifyContent: "space-between" }}
      >
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
            <span className="section-heading">Optimize</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              {pdfMode ? "Compress PDF" : "Compress image"}
            </h1>
          </div>
        </div>
        <div className="row" style={{ gap: 12, flexWrap: "wrap" }}>
          <input
            ref={fileRef}
            type="file"
            multiple
            accept="image/*,application/pdf"
            hidden
            onChange={(e) => onFiles([...(e.target.files ?? [])])}
          />
          <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
            <Icon path={ICONS.plus} size={18} strokeWidth={2} />
            Add files
          </button>
        </div>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section className="col grow scroll" aria-label="Preview" style={{ gap: 20, minWidth: 0 }}>
          <div
            className="raised col"
            style={{ borderRadius: "var(--r-panel)", padding: 18, gap: 14 }}
          >
            <div className="pressed" style={{ borderRadius: 20, padding: pdfMode ? 20 : 12 }}>
              {pdfMode ? (
                pdfReport ? (
                  <SizeInspector report={pdfReport} after={result?.report} />
                ) : (
                  <div className="col" style={{ gap: 8, padding: "28px 4px" }}>
                    <span className="display" style={{ fontSize: 17 }}>
                      {isDesktop() ? "Reading the document…" : "PDFs need the desktop app"}
                    </span>
                    <span style={{ fontSize: 13, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                      {isDesktop()
                        ? "Working out where the bytes are."
                        : "The browser preview can decode images but not documents. Open Kitbox itself to compress PDFs."}
                    </span>
                  </div>
                )
              ) : (
                <BeforeAfter
                  before={originalUrl}
                  after={result?.previewUrl}
                  split={split}
                  onSplit={setSplit}
                  beforeLabel={file ? `Original · ${formatSize(file.size)}` : "No file yet"}
                  afterLabel={result ? `Compressed · ${formatSize(result.size)}` : "Not run yet"}
                />
              )}
            </div>
            <div
              className="row"
              style={{ flexWrap: "wrap", justifyContent: "space-between", gap: 12, padding: "0 6px" }}
            >
              <span style={{ fontSize: 13, color: "var(--text-subtle)" }}>
                {file ? file.name : "Drop an image or use Add images"}
              </span>
              <span style={{ fontSize: 13, color: "var(--text-subtle)" }}>
                {result ? result.knob : ""}
              </span>
            </div>
          </div>

          {files.length > 1 && (
            <div className="col" style={{ gap: 14 }}>
              <h2 className="section-heading">Queue · {files.length} images</h2>
              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))",
                  gap: 18,
                }}
              >
                {files.map((f, i) => {
                  const r = results[keyFor(f)];
                  const pct = r ? 100 : running ? 40 : 0;
                  return (
                    <button
                      key={keyFor(f)}
                      type="button"
                      onClick={() => setActive(i)}
                      aria-pressed={i === active}
                      className={i === active ? "pressed" : "raised"}
                      style={{
                        borderRadius: 20,
                        padding: 14,
                        display: "flex",
                        gap: 14,
                        alignItems: "center",
                        textAlign: "left",
                      }}
                    >
                      <span
                        className={i === active ? "raised-sm" : "pressed"}
                        style={{
                          width: 52,
                          height: 52,
                          borderRadius: 14,
                          flex: "0 0 auto",
                          display: "grid",
                          placeItems: "center",
                          fontSize: 11,
                          fontWeight: 600,
                          color: "var(--text-subtle)",
                        }}
                      >
                        {(f.name.split(".").pop() ?? "img").slice(0, 4).toUpperCase()}
                      </span>
                      <span className="col grow" style={{ gap: 6, minWidth: 0 }}>
                        <span className="truncate" style={{ fontSize: 14, fontWeight: 600 }}>
                          {f.name}
                        </span>
                        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                          {formatSize(f.size)}
                          {r ? ` → ${formatSize(r.size)}` : running ? " · working" : " · waiting"}
                        </span>
                        <span
                          className="pressed"
                          style={{ height: 8, borderRadius: 999, overflow: "hidden" }}
                        >
                          <span
                            style={{
                              display: "block",
                              height: "100%",
                              width: `${pct}%`,
                              background: "var(--accent)",
                              borderRadius: 999,
                              transition: "width .25s ease",
                            }}
                          />
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </section>

        <aside
          className="raised col scroll"
          aria-label="Compression settings"
          style={{
            flex: "0 0 320px",
            borderRadius: "var(--r-panel)",
            padding: 22,
            gap: 22,
          }}
        >
          {!pdfMode && (
            <div className="col" style={{ gap: 12 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Mode</span>
              <div className="pressed segmented" style={{ gridTemplateColumns: "1fr 1fr" }}>
                {(["target", "quality"] as Mode[]).map((m) => (
                  <button
                    key={m}
                    type="button"
                    aria-pressed={mode === m}
                    className={mode === m ? "raised-sm" : undefined}
                    onClick={() => setMode(m)}
                  >
                    {m === "target" ? "Target size" : "Quality"}
                  </button>
                ))}
              </div>
            </div>
          )}

          {mode === "target" || pdfMode ? (
            <TargetControls
              targetMode={targetMode}
              setTargetMode={(m) => {
                setPreset(null);
                setTargetMode(m);
              }}
              minKb={minKb}
              maxKb={maxKb}
              setMinKb={(v) => {
                setPreset(null);
                setMinKb(v);
              }}
              setMaxKb={(v) => {
                setPreset(null);
                setMaxKb(v);
              }}
              kind={pdfMode ? "pdf" : "image"}
              preset={preset}
              onClearPreset={() => setPreset(null)}
              showPresets={showPresets}
              onTogglePresets={() => setShowPresets((v) => !v)}
              onPickPreset={(p) => {
                setPreset(p);
                setShowPresets(false);
                setTargetMode(p.target.kind === "range" ? "between" : "under");
                setMinKb(Math.round(targetMin(p.target) / 1000));
                setMaxKb(Math.round(targetMax(p.target) / 1000));
              }}
            />
          ) : (
            <div className="col" style={{ gap: 14 }}>
              <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline" }}>
                <label htmlFor="q" style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Quality
                </label>
                <span className="display" style={{ fontSize: 22, color: "var(--accent)" }}>
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
                onChange={(e) => setQuality(Number(e.target.value))}
              />
              <div
                className="row"
                style={{ justifyContent: "space-between", fontSize: 12, color: "var(--text-subtle)" }}
              >
                <span>Smaller</span>
                <span>Sharper</span>
              </div>
            </div>
          )}

          {(mode === "target" || pdfMode) && (
            <div className="col" style={{ gap: 10 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                If something has to give
              </span>
              <div className="pressed segmented" style={{ gridTemplateColumns: "1fr 1fr" }}>
                {(
                  [
                    ["keep_sharp", "Keep sharp"],
                    ["shrink_pixels", "Shrink pixels"],
                  ] as [Bias, string][]
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    aria-pressed={bias === id}
                    className={bias === id ? "raised-sm" : undefined}
                    onClick={() => setBias(id)}
                    disabled={!!preset?.dimensions}
                  >
                    {label}
                  </button>
                ))}
              </div>
              {preset?.dimensions && (
                <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  This preset pins the pixel size, so quality is the only lever.
                </span>
              )}
            </div>
          )}

          {!pdfMode && (
            <div className="col" style={{ gap: 12 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                Output format
              </span>
              <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 10 }}>
                {(
                  [
                    ["keep", "Keep"],
                    ["jpeg", "JPG"],
                    ["png", "PNG"],
                  ] as [OutputFormat, string][]
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    aria-pressed={format === id}
                    className={format === id ? "pressed" : "raised-sm"}
                    onClick={() => setFormat(id)}
                    style={{
                      borderRadius: 12,
                      minHeight: 44,
                      fontSize: 13,
                      fontWeight: format === id ? 700 : 500,
                      color: format === id ? "var(--accent)" : undefined,
                    }}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          )}

          {pdfMode && (
            <div className="col" style={{ gap: 12 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                Don't go below
              </span>
              <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 8 }}>
                {[150, 120, 96, 72].map((dpi) => (
                  <button
                    key={dpi}
                    type="button"
                    aria-pressed={minDpi === dpi}
                    className={minDpi === dpi ? "pressed" : "raised-sm"}
                    onClick={() => setMinDpi(dpi)}
                    style={{
                      borderRadius: 12,
                      minHeight: 42,
                      fontSize: 12,
                      fontWeight: minDpi === dpi ? 700 : 500,
                      color: minDpi === dpi ? "var(--accent)" : undefined,
                    }}
                  >
                    {dpi} DPI
                  </button>
                ))}
              </div>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                150 stays crisp in print, 96 is fine on screen, 72 is the floor before text in a
                scan starts to break up.
              </span>
            </div>
          )}

          {pdfMode && (
            <Toggle
              label="Allow greyscale"
              hint="Only if colour alone can't reach the size"
              on={allowGrayscale}
              onChange={setAllowGrayscale}
            />
          )}

          <Toggle
            label="Strip metadata"
            hint="Removes location and camera data"
            on={strip}
            onChange={setStrip}
          />

          <ResultCard
            result={result}
            target={mode === "target" ? target : undefined}
            error={error}
            running={running}
          />

          {files.length > 1 && allDone && (
            <div className="pressed col" style={{ borderRadius: 18, padding: 16, gap: 4 }}>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>All {files.length} images</span>
              <span className="display" style={{ fontSize: 20 }}>
                {formatSize(totalBefore)} → {formatSize(totalAfter)}
              </span>
            </div>
          )}

          <div className="col" style={{ gap: 10, marginTop: "auto" }}>
            <button
              type="button"
              className="btn btn-primary"
              style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
              disabled={!files.length || running}
              onClick={run}
            >
              {running
                ? "Working…"
                : files.length > 1
                  ? `Compress ${files.length} images`
                  : "Compress"}
            </button>
            {result && result.status !== "infeasible" && (
              isDesktop() && result.outputPath ? (
                <button
                  type="button"
                  className="raised-sm btn"
                  onClick={() => revealOutput(result.outputPath!)}
                >
                  <Icon path={ICONS.save} size={18} strokeWidth={2} />
                  Show in folder
                </button>
              ) : (
                <a
                  className="raised-sm btn"
                  href={result.previewUrl}
                  download={downloadName(result.name)}
                  style={{ textAlign: "center" }}
                >
                  <Icon path={ICONS.save} size={18} strokeWidth={2} />
                  Download
                </a>
              )
            )}
            {result?.outputPath && (
              <span
                className="truncate"
                style={{ fontSize: 11.5, color: "var(--text-subtle)" }}
                title={result.outputPath}
              >
                Saved to {result.outputPath}
              </span>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}

function TargetControls({
  targetMode,
  setTargetMode,
  minKb,
  maxKb,
  setMinKb,
  setMaxKb,
  kind,
  preset,
  onClearPreset,
  showPresets,
  onTogglePresets,
  onPickPreset,
}: {
  kind: "image" | "pdf";
  targetMode: TargetMode;
  setTargetMode: (m: TargetMode) => void;
  minKb: number;
  maxKb: number;
  setMinKb: (v: number) => void;
  setMaxKb: (v: number) => void;
  preset: Preset | null;
  onClearPreset: () => void;
  showPresets: boolean;
  onTogglePresets: () => void;
  onPickPreset: (p: Preset) => void;
}) {
  // Loaded rather than hard-coded, so a preset you edited is the one offered
  // here too — the numbers live in your presets folder, not in this screen.
  const [presets, setPresets] = useState<Preset[]>([]);
  useEffect(() => {
    if (!showPresets) return;
    loadPresets()
      .then((loaded) => setPresets(loaded.presets))
      .catch(() => setPresets([]));
  }, [showPresets]);

  return (
    <div className="col" style={{ gap: 14 }}>
      <div className="row" style={{ gap: 14, fontSize: 13 }}>
        {(["under", "between", "exactly"] as TargetMode[]).map((m) => (
          <button
            key={m}
            type="button"
            aria-pressed={targetMode === m}
            onClick={() => setTargetMode(m)}
            style={{
              fontWeight: targetMode === m ? 700 : 600,
              color: targetMode === m ? "var(--accent)" : "var(--text-subtle)",
            }}
          >
            {m[0].toUpperCase() + m.slice(1)}
          </button>
        ))}
      </div>

      <div className="row" style={{ gap: 10 }}>
        {targetMode === "between" && (
          <>
            <span className="pressed field" style={{ flex: 1, padding: "0 8px" }}>
              <label className="sr-only" htmlFor="min">
                Minimum size in KB
              </label>
              <input
                id="min"
                type="number"
                min={1}
                value={minKb}
                onChange={(e) => setMinKb(Math.max(1, Number(e.target.value)))}
              />
            </span>
            <span style={{ color: "var(--text-subtle)" }}>to</span>
          </>
        )}
        <span className="pressed field" style={{ flex: 1, padding: "0 8px" }}>
          <label className="sr-only" htmlFor="max">
            {targetMode === "between" ? "Maximum size in KB" : "Size in KB"}
          </label>
          <input
            id="max"
            type="number"
            min={1}
            value={maxKb}
            onChange={(e) => setMaxKb(Math.max(1, Number(e.target.value)))}
          />
        </span>
        <span className="raised-sm" style={{ borderRadius: 12, height: 46, display: "grid", placeItems: "center", padding: "0 12px", fontSize: 13, fontWeight: 600 }}>
          KB
        </span>
      </div>

      <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
        {[100, 500, 1000].map((v) => (
          <button
            key={v}
            type="button"
            className="raised-sm pill"
            onClick={() => {
              onClearPreset();
              setTargetMode("under");
              setMaxKb(v);
            }}
          >
            {v >= 1000 ? `${v / 1000} MB` : `${v} KB`}
          </button>
        ))}
        <button
          type="button"
          className={showPresets || preset ? "pressed pill" : "raised-sm pill"}
          aria-pressed={showPresets || !!preset}
          aria-expanded={showPresets}
          onClick={onTogglePresets}
          style={{ color: preset ? "var(--accent)" : undefined }}
        >
          {preset ? preset.name : "Form preset"}
        </button>
      </div>

      {showPresets && (
        <div className="pressed col" style={{ borderRadius: 18, padding: 14, gap: 12, maxHeight: 280, overflowY: "auto" }}>
          <p style={{ fontSize: 11.5, lineHeight: 1.5, color: "var(--text-subtle)" }}>
            Starting points, not official rules — boards change these between cycles, so
            check against the form you're filling.
          </p>
          {byGroup(presets, kind).map(({ group, items }) => (
            <div key={group} className="col" style={{ gap: 6 }}>
              <span className="section-heading" style={{ fontSize: 11 }}>
                {group}
              </span>
              {items.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className="raised-sm"
                  onClick={() => onPickPreset(p)}
                  style={{
                    borderRadius: 12,
                    padding: "9px 12px",
                    textAlign: "left",
                    display: "flex",
                    flexDirection: "column",
                    gap: 2,
                  }}
                >
                  <span style={{ fontSize: 13, fontWeight: 600 }}>{p.name}</span>
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                    {describeTarget(p)}
                  </span>
                </button>
              ))}
            </div>
          ))}
          {preset && (
            <button
              type="button"
              className="raised-sm pill"
              onClick={onClearPreset}
              style={{ alignSelf: "flex-start" }}
            >
              Clear preset
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function ResultCard({
  result,
  target,
  error,
  running,
}: {
  result?: CompressResult;
  target?: SizeTarget;
  error: string | null;
  running: boolean;
}) {
  if (error) {
    return (
      <div className="pressed col" style={{ borderRadius: 20, padding: 16, gap: 8 }}>
        <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
          <Icon path={ICONS.warning} size={16} strokeWidth={2} />
          Something went wrong
        </span>
        <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>{error}</span>
      </div>
    );
  }

  if (running) {
    return (
      <div className="pressed col" style={{ borderRadius: 20, padding: 18, gap: 6 }}>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>Result</span>
        <span className="display" style={{ fontSize: 22 }}>
          Trying settings…
        </span>
      </div>
    );
  }

  if (!result) {
    return (
      <div className="pressed col" style={{ borderRadius: 20, padding: 18, gap: 6 }}>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>Result</span>
        <span style={{ fontSize: 13, color: "var(--text-subtle)", lineHeight: 1.5 }}>
          Kitbox encodes the file repeatedly until it lands in your range, then tells you
          exactly what it cost.
        </span>
      </div>
    );
  }

  if (result.status === "infeasible") {
    return (
      <div className="pressed col" style={{ borderRadius: 20, padding: 18, gap: 10 }}>
        <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 700 }}>
          <Icon path={ICONS.warning} size={16} strokeWidth={2} />
          Can't get there
        </span>
        <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
          {result.why}
        </span>
        <ul style={{ margin: 0, paddingLeft: 16, fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          {result.suggestions?.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ul>
      </div>
    );
  }

  const min = target ? targetMin(target) : 0;
  const max = target ? targetMax(target) : result.originalSize;
  const scale = Math.max(result.originalSize, max) || 1;
  const left = (min / scale) * 100;
  const width = Math.max(1, ((max - min) / scale) * 100);
  const marker = Math.min(100, (result.size / scale) * 100);
  const saved = Math.min(99.9, 100 - (result.size / Math.max(1, result.originalSize)) * 100);

  return (
    <div className="pressed col" style={{ borderRadius: 20, padding: 18, gap: 10 }}>
      <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>Result</span>
      <div className="row" style={{ gap: 10, alignItems: "baseline" }}>
        <span className="display" style={{ fontSize: 30, color: "var(--accent)" }}>
          {formatSize(result.size)}
        </span>
        <span style={{ fontSize: 12, fontWeight: 600, color: "var(--text-muted)" }}>
          {target ? "in range" : `${saved.toFixed(1)}% smaller`}
        </span>
      </div>

      {target && (
        <>
          <div
            style={{
              height: 10,
              borderRadius: 999,
              background: "var(--shadow-dark)",
              position: "relative",
              overflow: "hidden",
            }}
            role="img"
            aria-label={`${formatSize(result.size)} inside a window of ${formatSize(min)} to ${formatSize(max)}`}
          >
            <span
              style={{
                position: "absolute",
                left: `${left}%`,
                width: `${width}%`,
                top: 0,
                bottom: 0,
                background: "#c9d2c4",
              }}
            />
            <span
              style={{
                position: "absolute",
                left: `calc(${marker}% - 2px)`,
                width: 4,
                top: 0,
                bottom: 0,
                background: "var(--accent)",
                borderRadius: 2,
              }}
            />
          </div>
          <div
            className="row"
            style={{ justifyContent: "space-between", fontSize: 11, color: "var(--text-subtle)" }}
          >
            <span>0</span>
            <span>
              {formatSize(min)} — {formatSize(max)}
            </span>
            <span>{formatSize(result.originalSize)}</span>
          </div>
        </>
      )}

      <div style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6, marginTop: 2 }}>
        {result.score !== undefined && <div>{describeScore(result.score)}</div>}
        <div>
          {result.probes} attempt{result.probes === 1 ? "" : "s"} in {result.elapsedMs} ms ·{" "}
          {result.knob}
        </div>
        {result.note && <div style={{ marginTop: 6 }}>{result.note}</div>}
      </div>
    </div>
  );
}

function BeforeAfter({
  before,
  after,
  split,
  onSplit,
  beforeLabel,
  afterLabel,
}: {
  before?: string;
  after?: string;
  split: number;
  onSplit: (v: number) => void;
  beforeLabel: string;
  afterLabel: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const drag = (clientX: number) => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return;
    onSplit(Math.min(100, Math.max(0, ((clientX - box.left) / box.width) * 100)));
  };

  return (
    <div
      ref={ref}
      style={{
        position: "relative",
        borderRadius: 14,
        overflow: "hidden",
        height: 420,
        background: "#cbd3e1",
        display: "grid",
        placeItems: "center",
        touchAction: "none",
      }}
      onPointerDown={(e) => {
        (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
        drag(e.clientX);
      }}
      onPointerMove={(e) => {
        if (e.buttons === 1) drag(e.clientX);
      }}
    >
      {before ? (
        <>
          <img
            src={before}
            alt=""
            style={{ position: "absolute", inset: 0, width: "100%", height: "100%", objectFit: "contain" }}
          />
          {after && (
            <img
              src={after}
              alt=""
              style={{
                position: "absolute",
                inset: 0,
                width: "100%",
                height: "100%",
                objectFit: "contain",
                clipPath: `inset(0 0 0 ${split}%)`,
              }}
            />
          )}
          {after && (
            <>
              <div
                style={{
                  position: "absolute",
                  top: 0,
                  bottom: 0,
                  left: `${split}%`,
                  width: 3,
                  background: "#fff",
                  transform: "translateX(-50%)",
                }}
              />
              <div
                className="raised-sm"
                style={{
                  position: "absolute",
                  top: "50%",
                  left: `${split}%`,
                  width: 44,
                  height: 44,
                  borderRadius: "50%",
                  transform: "translate(-50%,-50%)",
                  display: "grid",
                  placeItems: "center",
                  cursor: "ew-resize",
                }}
              >
                <Icon path="M9 6l-4 6 4 6 M15 6l4 6-4 6" size={18} strokeWidth={2} />
              </div>
            </>
          )}
          <Badge style={{ top: 14, left: 14 }}>{beforeLabel}</Badge>
          <Badge style={{ top: 14, right: 14 }}>{afterLabel}</Badge>
        </>
      ) : (
        <span style={{ color: "#5b6270", fontSize: 13 }}>No image loaded</span>
      )}
    </div>
  );
}

function Badge({ children, style }: { children: React.ReactNode; style: React.CSSProperties }) {
  return (
    <span
      style={{
        position: "absolute",
        fontSize: 12,
        fontWeight: 600,
        background: "rgba(230,233,239,0.92)",
        padding: "6px 10px",
        borderRadius: 10,
        ...style,
      }}
    >
      {children}
    </span>
  );
}

function Toggle({
  label,
  hint,
  on,
  onChange,
}: {
  label: string;
  hint: string;
  on: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 12,
        minHeight: 44,
        textAlign: "left",
      }}
    >
      <span className="col" style={{ gap: 2 }}>
        <span style={{ fontSize: 14, fontWeight: 600 }}>{label}</span>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>{hint}</span>
      </span>
      <span className="pressed switch">
        <span style={{ left: on ? 30 : 4, background: on ? "var(--accent)" : "var(--surface)" }} />
      </span>
    </button>
  );
}

function describeTarget(p: Preset): string {
  const size =
    p.target.kind === "range"
      ? `${formatSize(p.target.value.min)}–${formatSize(p.target.value.max)}`
      : p.target.kind === "at_most"
        ? `under ${formatSize(p.target.value)}`
        : `about ${formatSize(p.target.value.bytes)}`;
  return p.dimensions ? `${p.dimensions.width}×${p.dimensions.height} px · ${size}` : size;
}

function downloadName(name: string): string {
  return `${name.replace(/\.[^.]+$/, "")}-kitbox.jpg`;
}

function keyFor(f: File): string {
  return `${f.name}:${f.size}:${f.lastModified}`;
}

function useObjectUrl(file?: File): string | undefined {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    if (!file) {
      setUrl(undefined);
      return;
    }
    const next = URL.createObjectURL(file);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [file]);
  return url;
}
