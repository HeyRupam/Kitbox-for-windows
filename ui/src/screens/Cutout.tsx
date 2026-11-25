import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  fetchModels,
  formatSize,
  isDesktop,
  isPdf,
  modelsReady,
  removeBackground,
  revealOutput,
} from "../lib/bridge";
import type { Backdrop, CutoutOptions, CutoutResult, ModelStatus } from "../lib/types";

const BACKDROPS: [string, Backdrop][] = [
  ["None", { kind: "transparent" }],
  ["White", { kind: "colour", value: [255, 255, 255] }],
  ["Form blue", { kind: "colour", value: [214, 229, 247] }],
  ["Grey", { kind: "colour", value: [236, 238, 242] }],
];

/**
 * Take the background out of a photo.
 *
 * Built around the case that prompted it: a form that wants a passport photo
 * on plain white or blue, and what you have is a photo against a wall.
 *
 * The result is shown before it is saved, because the model is guessing and
 * the edges are where the guess shows. Saving is a separate press.
 */
export function Cutout({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const photo = files.find((f) => !isPdf(f)) ?? null;

  const [models, setModels] = useState<ModelStatus | null>(null);
  const [fetching, setFetching] = useState(false);
  const [backdrop, setBackdrop] = useState<Backdrop>(BACKDROPS[1][1]);
  const [softness, setSoftness] = useState(0.35);
  const [threshold, setThreshold] = useState(0.5);

  const [preview, setPreview] = useState<CutoutResult | null>(null);
  const [saved, setSaved] = useState<CutoutResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    modelsReady("cutout")
      .then(setModels)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const options = (): CutoutOptions => ({ backdrop, softness, threshold, feather: 1 });

  // Re-cut whenever a setting changes, so the sliders mean something.
  useEffect(() => {
    if (!photo || !models?.ready || !isDesktop()) return;
    let cancelled = false;
    setBusy(true);
    const timer = setTimeout(() => {
      removeBackground(photo, { backdrop, softness, threshold, feather: 1 }, true)
        .then((p) => !cancelled && setPreview(p))
        .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => !cancelled && setBusy(false));
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [photo, models?.ready, backdrop, softness, threshold]);

  const download = async () => {
    setFetching(true);
    setError(null);
    try {
      setModels(await fetchModels("cutout"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setFetching(false);
    }
  };

  const save = async () => {
    if (!photo) return;
    setBusy(true);
    setError(null);
    try {
      setSaved(await removeBackground(photo, options(), false));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = models?.ready === true;

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
              Remove background
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
              body="The browser preview can compress images but not run the model this uses."
            />
          ) : !ready ? (
            <div className="col" style={{ gap: 14, maxWidth: 620, padding: "12px 4px" }}>
              <h2 className="display" style={{ fontSize: 21 }}>
                This one needs a one-off download
              </h2>
              <p style={{ color: "var(--text-subtle)", lineHeight: 1.7, fontSize: 13.5 }}>
                Working out what the subject is takes a model —{" "}
                <strong>{models ? formatSize(models.bytes) : "about 4.6 MB"}</strong>, fetched
                once and kept on this machine. After that it runs offline like everything else,
                in about a fifth of a second a photo.
              </p>
              {models && (
                <div className="pressed col" style={{ borderRadius: 16, padding: 16, gap: 10 }}>
                  <span className="section-heading" style={{ fontSize: 10.5 }}>
                    What gets downloaded
                  </span>
                  {models.sources.map((url) => (
                    <span
                      key={url}
                      style={{
                        fontSize: 11.5,
                        color: "var(--text-subtle)",
                        wordBreak: "break-all",
                        fontFamily: "var(--font-mono, ui-monospace, monospace)",
                      }}
                    >
                      {url}
                    </span>
                  ))}
                  {/* The URL above breaks anywhere; this line is prose with a
                      Windows path in the middle of it, which has no spaces to
                      break at and ran past the box on a narrow window. */}
                  <span
                    style={{
                      fontSize: 11.5,
                      color: "var(--text-subtle)",
                      lineHeight: 1.5,
                      overflowWrap: "anywhere",
                    }}
                  >
                    U²-Net, Apache 2.0 licensed. It goes in {models.folder}.
                  </span>
                </div>
              )}
              <button
                type="button"
                className="btn btn-primary"
                style={{
                  minHeight: 52,
                  borderRadius: 18,
                  fontSize: 14.5,
                  fontWeight: 700,
                  alignSelf: "flex-start",
                  padding: "0 26px",
                }}
                disabled={fetching}
                onClick={download}
              >
                {fetching ? "Downloading…" : "Download it"}
              </button>
            </div>
          ) : !photo ? (
            <Blurb
              title="Choose a photo"
              body="A person or an object against a background is what this is good at. A landscape or a flat wall has no subject to find, and it will say so rather than pretend."
            />
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center" }}>
                <span className="section-heading">
                  {busy ? "Working…" : preview ? `${Math.round(preview.coverage * 100)}% kept` : "Ready"}
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
                  // A chequerboard, so "transparent" looks transparent rather
                  // than white-on-white.
                  backgroundImage:
                    "linear-gradient(45deg,#d6dce6 25%,transparent 25%,transparent 75%,#d6dce6 75%)," +
                    "linear-gradient(45deg,#d6dce6 25%,transparent 25%,transparent 75%,#d6dce6 75%)",
                  backgroundSize: "18px 18px",
                  backgroundPosition: "0 0, 9px 9px",
                }}
              >
                {preview ? (
                  <img
                    src={preview.image}
                    alt="The photo with its background removed"
                    style={{ maxWidth: "100%", maxHeight: "100%", objectFit: "contain" }}
                  />
                ) : (
                  <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>…</span>
                )}
              </div>
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          {ready && (
            <>
              <div className="col" style={{ gap: 9 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Behind the subject
                </span>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
                  {BACKDROPS.map(([label, option]) => {
                    const chosen = JSON.stringify(option) === JSON.stringify(backdrop);
                    return (
                      <button
                        key={label}
                        type="button"
                        aria-pressed={chosen}
                        className={chosen ? "pressed" : "raised-sm"}
                        onClick={() => {
                          setBackdrop(option);
                          setSaved(null);
                        }}
                        style={{
                          borderRadius: 12,
                          minHeight: 44,
                          fontSize: 12,
                          fontWeight: chosen ? 700 : 500,
                          color: chosen ? "var(--accent)" : undefined,
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                          gap: 7,
                        }}
                      >
                        <span
                          style={{
                            width: 13,
                            height: 13,
                            borderRadius: 999,
                            background:
                              option.kind === "colour"
                                ? `rgb(${option.value.join(",")})`
                                : "transparent",
                            boxShadow: "inset 0 0 0 1px rgba(0,0,0,0.18)",
                          }}
                        />
                        {label}
                      </button>
                    );
                  })}
                </div>
              </div>

              <Slider
                label="Edge softness"
                value={softness}
                onChange={(v) => {
                  setSoftness(v);
                  setSaved(null);
                }}
                hint="Higher keeps more of the doubtful edge — kinder to hair, likelier to drag a halo of background along."
              />

              <Slider
                label="How much to keep"
                value={1 - threshold}
                onChange={(v) => {
                  setThreshold(1 - v);
                  setSaved(null);
                }}
                hint="Push this up if parts of the subject are being cut away, down if background is clinging on."
              />
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

          {preview?.warnings.map((w) => (
            <div key={w} className="pressed col" style={{ borderRadius: 14, padding: 13, gap: 4 }}>
              <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
                <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                Look at this one
              </span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                {w}
              </span>
            </div>
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

          {ready && (
            <>
              <div className="pressed col" style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  The model works out what the subject is; nobody tells it. Check the edges —
                  hair, glasses and thin straps are where it shows. Saved as PNG, so a clear
                  background stays clear.
                </span>
              </div>
              <button
                type="button"
                className="btn btn-primary"
                style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
                disabled={!photo || busy}
                onClick={save}
              >
                {busy ? "Working…" : "Save it"}
              </button>
            </>
          )}
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

function Slider({
  label,
  value,
  onChange,
  hint,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  hint: string;
}) {
  return (
    <div className="col" style={{ gap: 7 }}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
          {Math.round(value * 100)}%
        </span>
      </div>
      <input
        type="range"
        min={5}
        max={95}
        aria-label={label}
        value={Math.round(value * 100)}
        onChange={(e) => onChange(Number(e.target.value) / 100)}
      />
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>{hint}</span>
    </div>
  );
}
