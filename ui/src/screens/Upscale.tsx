import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  fetchModels,
  formatSize,
  isDesktop,
  isPdf,
  modelsReady,
  revealOutput,
  upscaleEstimate,
  upscaleImage,
} from "../lib/bridge";
import type { ModelStatus, UpscaleMethod, UpscaleResult } from "../lib/types";

/** Past this many input pixels the model refuses; the core says the same. */
const MODEL_PIXEL_LIMIT = 1_100_000;

/**
 * Make a picture bigger, two ways.
 *
 * Sharpen is instant, works at any size, needs no download and invents
 * nothing. Reconstruct runs a model that puts plausible detail back, costs
 * about two minutes a megapixel on a CPU, and is capped because a
 * twenty-five-minute wait is not something to start by accident.
 *
 * The screen's job is to make that trade visible *before* the press, not
 * after: the wait is shown as a number, and the difference between recovered
 * detail and invented detail is stated rather than implied.
 */
export function Upscale({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const photo = files.find((f) => !isPdf(f)) ?? null;

  const [method, setMethod] = useState<UpscaleMethod>("sharpen");
  const [scale, setScale] = useState(2);
  const [sharpen, setSharpen] = useState(0.4);
  const [models, setModels] = useState<ModelStatus | null>(null);
  const [fetching, setFetching] = useState(false);

  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  const [seconds, setSeconds] = useState<number | null>(null);
  const [result, setResult] = useState<UpscaleResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    modelsReady("upscale").then(setModels).catch(() => undefined);
  }, []);

  // The picture's own size, so the wait can be quoted before anything runs.
  useEffect(() => {
    if (!photo) {
      setSize(null);
      return;
    }
    const url = URL.createObjectURL(photo);
    const img = new Image();
    img.onload = () => {
      setSize({ w: img.naturalWidth, h: img.naturalHeight });
      URL.revokeObjectURL(url);
    };
    img.onerror = () => URL.revokeObjectURL(url);
    img.src = url;
  }, [photo]);

  useEffect(() => {
    if (!size || !isDesktop()) return;
    upscaleEstimate(size.w, size.h)
      .then(setSeconds)
      .catch(() => setSeconds(null));
  }, [size]);

  const pixels = size ? size.w * size.h : 0;
  const tooBig = pixels > MODEL_PIXEL_LIMIT;
  const modelReady = models?.ready === true;

  const download = async () => {
    setFetching(true);
    setError(null);
    try {
      setModels(await fetchModels("upscale"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setFetching(false);
    }
  };

  const run = async () => {
    if (!photo) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await upscaleImage(photo, { method, scale, sharpen }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const wait = seconds === null ? null : seconds < 90
    ? `about ${Math.max(1, Math.round(seconds))} seconds`
    : `about ${Math.round(seconds / 60)} minutes`;

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
            <span className="section-heading">Optimize</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Upscale
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          hidden
          onChange={(e) => {
            const picked = e.target.files?.[0];
            if (picked) {
              onFiles([picked, ...files.filter(isPdf)]);
              setResult(null);
            }
          }}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          {photo ? "Change photo" : "Choose a photo"}
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          {!isDesktop() ? (
            <Blurb
              title="This needs the desktop app"
              body="The browser preview can compress images but not enlarge them."
            />
          ) : !photo ? (
            <Blurb
              title="Choose a photo"
              body="Sharpen resamples what is there — instant, any size. Reconstruct runs a model that puts plausible detail back, which is slower and limited in size."
            />
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <span className="section-heading">
                  {size ? `${size.w} × ${size.h}` : "…"}
                  {size ? ` → ${size.w * scale} × ${size.h * scale}` : ""}
                </span>
                {result && (
                  <span className="pressed pill" style={{ fontSize: 11.5 }}>
                    done in {result.seconds.toFixed(1)}s
                    {result.tiles > 0 ? `, ${result.tiles} tiles` : ""}
                  </span>
                )}
              </div>
              <div
                className="pressed grow"
                style={{
                  borderRadius: 18,
                  padding: 16,
                  display: "grid",
                  placeItems: "center",
                  minHeight: 0,
                  overflow: "auto",
                }}
              >
                <img
                  src={result ? result.previewUrl : URL.createObjectURL(photo)}
                  alt={result ? "The enlarged picture" : "The picture as it is"}
                  style={{ maxWidth: "100%", imageRendering: result ? "auto" : "pixelated" }}
                />
              </div>
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>How</span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
              {(
                [
                  ["sharpen", "Sharpen"],
                  ["reconstruct", "Reconstruct"],
                ] as [UpscaleMethod, string][]
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={method === id}
                  className={method === id ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setMethod(id);
                    setResult(null);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 44,
                    fontSize: 12.5,
                    fontWeight: method === id ? 700 : 500,
                    color: method === id ? "var(--accent)" : undefined,
                  }}
                >
                  {label}
                </button>
              ))}
            </div>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
              {method === "sharpen"
                ? "Resamples what is already there and sharpens the result. Instant, any size, nothing to download."
                : "Runs a model that adds detail the picture never had. Slower, and limited in size."}
            </span>
          </div>

          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              How much bigger
            </span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
              {[2, 3, 4].map((n) => (
                <button
                  key={n}
                  type="button"
                  aria-pressed={scale === n}
                  className={scale === n ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setScale(n);
                    setResult(null);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 42,
                    fontSize: 12.5,
                    fontWeight: scale === n ? 700 : 500,
                    color: scale === n ? "var(--accent)" : undefined,
                  }}
                >
                  {n}×
                </button>
              ))}
            </div>
          </div>

          {method === "sharpen" && (
            <div className="col" style={{ gap: 7 }}>
              <div className="row" style={{ justifyContent: "space-between" }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Sharpening
                </span>
                <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                  {Math.round(sharpen * 100)}%
                </span>
              </div>
              <input
                type="range"
                min={0}
                max={100}
                aria-label="Sharpening"
                value={Math.round(sharpen * 100)}
                onChange={(e) => {
                  setSharpen(Number(e.target.value) / 100);
                  setResult(null);
                }}
              />
            </div>
          )}

          {method === "reconstruct" && (
            <>
              {tooBig ? (
                <div className="pressed col" style={{ borderRadius: 14, padding: 13, gap: 4 }}>
                  <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
                    <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                    Too big to reconstruct
                  </span>
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                    This is {(pixels / 1_000_000).toFixed(1)} megapixels, which would take{" "}
                    {wait ?? "a very long time"}. Crop it first, or use Sharpen — it has no limit.
                  </span>
                </div>
              ) : !modelReady ? (
                <div className="pressed col" style={{ borderRadius: 14, padding: 13, gap: 8 }}>
                  <span style={{ fontSize: 12, fontWeight: 700 }}>Needs a one-off download</span>
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                    {models ? formatSize(models.bytes) : "about 70 MB"}, fetched once and kept.
                    It's much the largest download in Kitbox — Sharpen needs nothing at all.
                  </span>
                  <button
                    type="button"
                    className="raised-sm btn"
                    style={{ minHeight: 38, fontSize: 12 }}
                    disabled={fetching}
                    onClick={download}
                  >
                    {fetching ? "Downloading…" : "Download it"}
                  </button>
                </div>
              ) : (
                wait && (
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                    This picture will take {wait}. It runs on your processor, on this machine.
                  </span>
                )
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
              <span className="display" style={{ fontSize: 18 }}>
                {result.to[0]} × {result.to[1]}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {formatSize(result.size)}
              </span>
              {result.warnings.map((w) => (
                <span key={w} style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {w}
                </span>
              ))}
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

          <div className="pressed col" style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              Reconstruct adds detail that was never in the picture — a plausible guess at what
              was probably there. Fine for a portrait or a logo, misleading for anything being
              read as evidence. Sharpen only resamples what is already there.
            </span>
          </div>

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={
              !photo || busy || (method === "reconstruct" && (tooBig || !modelReady))
            }
            onClick={run}
          >
            {busy ? "Working…" : "Make it bigger"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Blurb({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "36px 6px" }}>
      <h2 className="display" style={{ fontSize: 20 }}>
        {title}
      </h2>
      <p style={{ color: "var(--text-subtle)", maxWidth: 560, lineHeight: 1.7 }}>{body}</p>
    </div>
  );
}
