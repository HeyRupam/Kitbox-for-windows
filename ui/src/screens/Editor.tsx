import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { adjustImage, formatSize, isDesktop, isPdf, revealOutput } from "../lib/bridge";
import type { AdjustOptions, AdjustResult } from "../lib/types";

const NOTHING: AdjustOptions = {
  autoLevels: false,
  autoColour: false,
  exposure: 0,
  contrast: 0,
  saturation: 0,
  warmth: 0,
  sharpen: 0,
  greyscale: false,
};

/**
 * Tone and colour.
 *
 * The geometry tools crop and rotate; this is the other half, for the picture
 * everyone actually has — a document shot indoors that came out dim and
 * yellow.
 *
 * Preview runs on a downscaled copy so the sliders stay live. That is honest
 * here in a way it wouldn't be for a geometry tool: every adjustment is
 * per-pixel and scale-free, and the two automatic ones read the picture's own
 * histogram, which a fair downscale preserves. What you see is what gets
 * saved, at full size.
 */
export function Editor({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const photo = files.find((f) => !isPdf(f)) ?? null;

  const [options, setOptions] = useState<AdjustOptions>(NOTHING);
  const [preview, setPreview] = useState<AdjustResult | null>(null);
  const [saved, setSaved] = useState<AdjustResult | null>(null);
  const [before, setBefore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const [original, setOriginal] = useState<string | null>(null);

  useEffect(() => {
    if (!photo) {
      setOriginal(null);
      return;
    }
    const url = URL.createObjectURL(photo);
    setOriginal(url);
    return () => URL.revokeObjectURL(url);
  }, [photo]);

  const changed =
    options.autoLevels ||
    options.autoColour ||
    options.greyscale ||
    options.exposure !== 0 ||
    options.contrast !== 0 ||
    options.saturation !== 0 ||
    options.warmth !== 0 ||
    options.sharpen !== 0;

  useEffect(() => {
    if (!photo || !isDesktop() || !changed) {
      setPreview(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    const timer = setTimeout(() => {
      adjustImage(photo, options, true)
        .then((p) => !cancelled && setPreview(p))
        .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => !cancelled && setBusy(false));
    }, 110);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [photo, options, changed]);

  const set = (patch: Partial<AdjustOptions>) => {
    setOptions({ ...options, ...patch });
    setSaved(null);
  };

  const save = async () => {
    if (!photo) return;
    setBusy(true);
    setError(null);
    try {
      setSaved(await adjustImage(photo, options, false));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const shown = before || !preview ? original : preview.image;

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
            <span className="section-heading">Create</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Photo editor
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
              setPreview(null);
              setSaved(null);
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
              body="The browser preview can compress images but not adjust them."
            />
          ) : !photo ? (
            <Blurb
              title="Choose a photo"
              body="Best on the picture everyone has: a document or a portrait shot indoors that came out dim, flat and yellow. Auto fixes most of that in one press."
            />
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <span className="section-heading">
                  {busy ? "Working…" : changed ? "Adjusted" : "Untouched"}
                </span>
                {changed && (
                  <button
                    type="button"
                    className={before ? "pressed" : "raised-sm"}
                    aria-pressed={before}
                    onMouseDown={() => setBefore(true)}
                    onMouseUp={() => setBefore(false)}
                    onMouseLeave={() => setBefore(false)}
                    style={{
                      borderRadius: 999,
                      padding: "4px 13px",
                      fontSize: 11.5,
                      fontWeight: 600,
                      color: before ? "var(--accent)" : undefined,
                    }}
                  >
                    Hold to see before
                  </button>
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
                  overflow: "hidden",
                }}
              >
                {shown && (
                  <img
                    src={shown}
                    alt="The photo"
                    style={{ maxWidth: "100%", maxHeight: "100%", objectFit: "contain" }}
                  />
                )}
              </div>
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 16 }}
        >
          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Fix it automatically
            </span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
              <Toggle
                label="Levels"
                on={options.autoLevels}
                onClick={() => set({ autoLevels: !options.autoLevels })}
              />
              <Toggle
                label="Colour"
                on={options.autoColour}
                onClick={() => set({ autoColour: !options.autoColour })}
              />
            </div>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
              Levels stretches a flat picture to use the full range. Colour reads the light from
              the brightest part — the paper, usually — and neutralises it.
            </span>
          </div>

          <Slider
            label="Exposure"
            value={options.exposure}
            min={-2}
            max={2}
            suffix=" stops"
            onChange={(v) => set({ exposure: v })}
          />
          <Slider
            label="Contrast"
            value={options.contrast}
            min={-1}
            max={1}
            onChange={(v) => set({ contrast: v })}
          />
          <Slider
            label="Saturation"
            value={options.saturation}
            min={-1}
            max={1}
            onChange={(v) => set({ saturation: v })}
          />
          <Slider
            label="Warmth"
            value={options.warmth}
            min={-1}
            max={1}
            onChange={(v) => set({ warmth: v })}
          />
          <Slider
            label="Sharpen"
            value={options.sharpen}
            min={0}
            max={1}
            onChange={(v) => set({ sharpen: v })}
          />

          <Toggle
            label="Black and white"
            on={options.greyscale}
            onClick={() => set({ greyscale: !options.greyscale })}
          />

          {changed && (
            <button
              type="button"
              className="raised-sm"
              onClick={() => {
                setOptions(NOTHING);
                setSaved(null);
              }}
              style={{ borderRadius: 999, padding: "5px 12px", fontSize: 11.5, alignSelf: "flex-start" }}
            >
              Start again
            </button>
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

          {preview?.warnings.map((w) => (
            <span key={w} style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              {w}
            </span>
          ))}

          {saved && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {saved.width} × {saved.height}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {formatSize(saved.size)}
              </span>
              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 40 }}
                onClick={() => revealOutput(saved.outputPath as string)}
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
            disabled={!photo || busy || !changed}
            onClick={save}
          >
            {busy ? "Working…" : "Save it"}
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

function Toggle({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      className={on ? "pressed" : "raised-sm"}
      onClick={onClick}
      style={{
        borderRadius: 12,
        minHeight: 42,
        fontSize: 12,
        fontWeight: on ? 700 : 500,
        color: on ? "var(--accent)" : undefined,
      }}
    >
      {label}
    </button>
  );
}

function Slider({
  label,
  value,
  min,
  max,
  suffix = "",
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  suffix?: string;
  onChange: (value: number) => void;
}) {
  // 100 steps across whatever range this slider has, so every control feels
  // the same however it is scaled underneath.
  const position = ((value - min) / (max - min)) * 100;
  return (
    <div className="col" style={{ gap: 6 }}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
        <span
          style={{
            fontSize: 12,
            color: value === 0 ? "var(--text-subtle)" : "var(--accent)",
            fontWeight: value === 0 ? 400 : 700,
          }}
        >
          {value > 0 ? "+" : ""}
          {value.toFixed(2)}
          {suffix}
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        aria-label={label}
        value={Math.round(position)}
        onChange={(e) => {
          const next = min + (Number(e.target.value) / 100) * (max - min);
          // Snap to nothing when near it, so a slider can be put back.
          onChange(Math.abs(next) < (max - min) / 60 ? 0 : Number(next.toFixed(3)));
        }}
      />
    </div>
  );
}
