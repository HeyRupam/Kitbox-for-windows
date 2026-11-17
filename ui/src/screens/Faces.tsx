import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  blurFaces,
  fetchModels,
  formatSize,
  isDesktop,
  isPdf,
  modelsReady,
  revealOutput,
} from "../lib/bridge";
import type { BlurOptions, Cover, FacesResult, ModelStatus } from "../lib/types";

const COVERS: [string, Cover][] = [
  ["Blur", { kind: "blur" }],
  ["Pixelate", { kind: "pixelate" }],
  ["Box", { kind: "box", value: [20, 20, 24] }],
];

/**
 * Cover the faces in a photo.
 *
 * This tool fails in the direction that matters: it misses a face, you share
 * the picture believing everything is covered, and it isn't. So the screen is
 * built to be *checked* rather than trusted — every face it found is outlined
 * over the result, the count is stated, and the warning says a count is not a
 * promise. If a face has no box around it, that is the thing to notice before
 * pressing save.
 */
export function Faces({
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
  const [cover, setCover] = useState<Cover>(COVERS[0][1]);
  const [confidence, setConfidence] = useState(0.7);
  const [grow, setGrow] = useState(0.15);
  const [strength, setStrength] = useState(0.8);
  const [outline, setOutline] = useState(true);

  const [preview, setPreview] = useState<FacesResult | null>(null);
  const [saved, setSaved] = useState<FacesResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const [shown, setShown] = useState<{ w: number; h: number } | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    modelsReady("faces").then(setModels).catch(() => undefined);
  }, []);

  const options = (): BlurOptions => ({ cover, confidence, grow, strength });

  useEffect(() => {
    if (!photo || !models?.ready || !isDesktop()) return;
    let cancelled = false;
    setBusy(true);
    const timer = setTimeout(() => {
      blurFaces(photo, { cover, confidence, grow, strength }, true)
        .then((p) => !cancelled && setPreview(p))
        .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
        .finally(() => !cancelled && setBusy(false));
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [photo, models?.ready, cover, confidence, grow, strength]);

  const download = async () => {
    setFetching(true);
    setError(null);
    try {
      setModels(await fetchModels("faces"));
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
      setSaved(await blurFaces(photo, options(), false));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = models?.ready === true;
  const found = preview?.faces ?? [];

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
            <span className="section-heading">Modify</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Blur faces
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
              body="The browser preview can compress images but not find faces in them."
            />
          ) : !ready ? (
            <div className="col" style={{ gap: 14, maxWidth: 620, padding: "12px 4px" }}>
              <h2 className="display" style={{ fontSize: 21 }}>
                Finding faces needs a small download
              </h2>
              <p style={{ color: "var(--text-subtle)", lineHeight: 1.7, fontSize: 13.5 }}>
                <strong>{models ? formatSize(models.bytes) : "about 1.2 MB"}</strong>, fetched once
                and kept — the smallest of Kitbox's models, and the fastest. After that it runs
                offline like everything else.
              </p>
              {models && (
                <div className="pressed col" style={{ borderRadius: 16, padding: 16, gap: 8 }}>
                  {models.sources.map((url) => (
                    <span
                      key={url}
                      style={{
                        fontSize: 11,
                        color: "var(--text-subtle)",
                        wordBreak: "break-all",
                        fontFamily: "var(--font-mono, ui-monospace, monospace)",
                      }}
                    >
                      {url}
                    </span>
                  ))}
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                    UltraFace, MIT licensed. It goes in {models.folder}.
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
              body="Every face found is outlined on the result, so you can see what was caught — and more to the point, what wasn't."
            />
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <span className="section-heading">
                  {busy ? "Looking…" : `${found.length} face${found.length === 1 ? "" : "s"} found`}
                </span>
                <button
                  type="button"
                  aria-pressed={outline}
                  className={outline ? "pressed" : "raised-sm"}
                  onClick={() => setOutline(!outline)}
                  style={{
                    borderRadius: 999,
                    padding: "4px 12px",
                    fontSize: 11.5,
                    fontWeight: 600,
                    color: outline ? "var(--accent)" : undefined,
                  }}
                >
                  {outline ? "Outlines on" : "Outlines off"}
                </button>
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
                <span style={{ position: "relative", display: "inline-block", maxHeight: "100%" }}>
                  <img
                    src={preview ? preview.image : URL.createObjectURL(photo)}
                    alt="The photo with its faces covered"
                    onLoad={(e) =>
                      setShown({
                        w: e.currentTarget.naturalWidth,
                        h: e.currentTarget.naturalHeight,
                      })
                    }
                    style={{ maxWidth: "100%", maxHeight: "72vh", display: "block" }}
                  />
                  {outline && shown && (
                    <svg
                      viewBox={`0 0 ${shown.w} ${shown.h}`}
                      preserveAspectRatio="none"
                      style={{ position: "absolute", inset: 0, width: "100%", height: "100%" }}
                    >
                      {found.map((f, i) => (
                        <rect
                          key={i}
                          x={f.x}
                          y={f.y}
                          width={f.width}
                          height={f.height}
                          fill="none"
                          stroke="var(--accent)"
                          strokeWidth={2}
                          vectorEffect="non-scaling-stroke"
                        />
                      ))}
                    </svg>
                  )}
                </span>
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
                  Cover with
                </span>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
                  {COVERS.map(([label, option]) => {
                    const chosen = option.kind === cover.kind;
                    return (
                      <button
                        key={label}
                        type="button"
                        aria-pressed={chosen}
                        className={chosen ? "pressed" : "raised-sm"}
                        onClick={() => {
                          setCover(option);
                          setSaved(null);
                        }}
                        style={{
                          borderRadius: 12,
                          minHeight: 42,
                          fontSize: 12,
                          fontWeight: chosen ? 700 : 500,
                          color: chosen ? "var(--accent)" : undefined,
                        }}
                      >
                        {label}
                      </button>
                    );
                  })}
                </div>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
                  {cover.kind === "box"
                    ? "A solid box destroys what was underneath. The only one of the three that certainly does."
                    : "Disguises rather than destroys — both blur and pixelation have been undone before. Use Box where it really matters."}
                </span>
              </div>

              <Slider
                label="Find faces down to"
                value={1 - confidence}
                onChange={(v) => {
                  setConfidence(Math.min(0.95, Math.max(0.05, 1 - v)));
                  setSaved(null);
                }}
                hint="Higher finds more, including things that aren't faces. Lower is stricter and misses more."
              />

              <Slider
                label="Margin round each face"
                value={grow}
                onChange={(v) => {
                  setGrow(v);
                  setSaved(null);
                }}
                hint="The box sits tight on the features; a margin takes in the ears, chin and hairline too."
              />

              {cover.kind !== "box" && (
                <Slider
                  label="Strength"
                  value={strength}
                  onChange={(v) => {
                    setStrength(v);
                    setSaved(null);
                  }}
                  hint="How hard to blur, or how few blocks to reduce a face to."
                />
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

          {preview && found.length === 0 && !busy && (
            <div className="pressed col" style={{ borderRadius: 14, padding: 13, gap: 4 }}>
              <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
                <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                Nothing found
              </span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Nothing was covered. If there is a face here, move the first slider up.
              </span>
            </div>
          )}

          {saved && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {saved.covered} covered
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

          {ready && (
            <>
              <div className="pressed col" style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}>
                <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
                  <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                  Check before you share
                </span>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  Profiles, small faces and partly hidden faces get missed routinely. Every face
                  found is outlined — look for one without a box. The count is not a promise.
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
        min={0}
        max={100}
        aria-label={label}
        value={Math.round(value * 100)}
        onChange={(e) => onChange(Number(e.target.value) / 100)}
      />
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>{hint}</span>
    </div>
  );
}
