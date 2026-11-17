import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, isDesktop, isPdf, makeMeme, revealOutput } from "../lib/bridge";
import type { MemeOptions, MemeResult } from "../lib/types";

const BLANK: MemeOptions = {
  top: "",
  bottom: "",
  uppercase: true,
  size: 0.11,
  outline: 0.06,
  colour: [255, 255, 255],
  outlineColour: [0, 0, 0],
  band: false,
};

/**
 * Captions on a picture.
 *
 * Typing is the whole interaction, so the preview redraws as you type and the
 * two boxes sit at the top of the panel where they belong. Everything else is
 * a detail you touch once.
 */
export function Meme({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const photo = files.find((f) => !isPdf(f)) ?? null;

  const [options, setOptions] = useState<MemeOptions>(BLANK);
  const [preview, setPreview] = useState<MemeResult | null>(null);
  const [saved, setSaved] = useState<MemeResult | null>(null);
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

  const written = options.top.trim() !== "" || options.bottom.trim() !== "";

  useEffect(() => {
    if (!photo || !isDesktop() || !written) {
      setPreview(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    const timer = setTimeout(() => {
      makeMeme(photo, options, true)
        .then((p) => !cancelled && setPreview(p))
        .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => !cancelled && setBusy(false));
    }, 160);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [photo, options, written]);

  const set = (patch: Partial<MemeOptions>) => {
    setOptions({ ...options, ...patch });
    setSaved(null);
  };

  const save = async () => {
    if (!photo) return;
    setBusy(true);
    setError(null);
    try {
      setSaved(await makeMeme(photo, options, false));
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
            <span className="section-heading">Create</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Caption a picture
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
          {photo ? "Change picture" : "Choose a picture"}
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
              body="Captions are drawn with a font from your machine, which the browser preview can't reach."
            />
          ) : !photo ? (
            <Blurb
              title="Choose a picture"
              body="Type into the two boxes and it appears as you go — heavy white capitals with a black outline, the way these have always looked."
            />
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center" }}>
                <span className="section-heading">
                  {busy
                    ? "Drawing…"
                    : preview
                      ? `${preview.width} × ${preview.height}`
                      : "Type a caption"}
                </span>
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
                {(preview?.image ?? original) && (
                  <img
                    src={preview?.image ?? (original as string)}
                    alt="The captioned picture"
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
          <div className="col" style={{ gap: 8 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Top line
            </span>
            <input
              type="text"
              value={options.top}
              aria-label="Top caption"
              onChange={(e) => set({ top: e.target.value })}
              className="pressed"
              style={field}
            />
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Bottom line
            </span>
            <input
              type="text"
              value={options.bottom}
              aria-label="Bottom caption"
              onChange={(e) => set({ bottom: e.target.value })}
              className="pressed"
              style={field}
            />
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              Long lines wrap on their own and never reach the edge.
            </span>
          </div>

          <Slider
            label="Text size"
            value={options.size}
            min={0.03}
            max={0.25}
            onChange={(v) => set({ size: v })}
          />
          <Slider
            label="Outline"
            value={options.outline}
            min={0}
            max={0.2}
            onChange={(v) => set({ outline: v })}
          />

          <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
            <Toggle
              label="CAPITALS"
              on={options.uppercase}
              onClick={() => set({ uppercase: !options.uppercase })}
            />
            <Toggle
              label="Band on top"
              on={options.band}
              onClick={() => set({ band: !options.band })}
            />
          </div>
          <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
            A band puts the top line on white above the picture instead of over it, so nothing is
            covered.
          </span>

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
                {formatSize(saved.size)} · PNG
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
            disabled={!photo || busy || !written}
            onClick={save}
          >
            {busy ? "Drawing…" : "Save it"}
          </button>
        </aside>
      </div>
    </div>
  );
}

const field: React.CSSProperties = {
  borderRadius: 12,
  minHeight: 44,
  padding: "0 13px",
  fontSize: 13,
  border: "none",
  color: "var(--text)",
  width: "100%",
  fontFamily: "inherit",
};

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
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}) {
  return (
    <div className="col" style={{ gap: 6 }}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
          {Math.round(((value - min) / (max - min)) * 100)}%
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        aria-label={label}
        value={Math.round(((value - min) / (max - min)) * 100)}
        onChange={(e) =>
          onChange(Number((min + (Number(e.target.value) / 100) * (max - min)).toFixed(4)))
        }
      />
    </div>
  );
}
