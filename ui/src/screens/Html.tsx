import { useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  formatSize,
  htmlToImage,
  htmlToPdf,
  isDesktop,
  pickFile,
  revealOutput,
} from "../lib/bridge";
import type {
  HtmlPageSetup,
  HtmlResult,
  HtmlSource,
  ShotResult,
} from "../lib/types";

type Mode = "url" | "file" | "html";

/**
 * A web page, as a PDF.
 *
 * The page is rendered by the browser engine already inside the app, in a
 * window with no access to any Kitbox command — so a site's own JavaScript
 * can't reach the tools. The screen says when a conversion will touch the
 * network, because for everything except this and the OCR models Kitbox never
 * does.
 */
export function Html({
  as,
  onBack,
}: {
  /** Which tile opened this: a document, or a picture. */
  as: "pdf" | "image";
  onBack: () => void;
}) {
  const [mode, setMode] = useState<Mode>("url");
  const [url, setUrl] = useState("");
  const [file, setFile] = useState("");
  const [markup, setMarkup] = useState("");

  const [size, setSize] = useState("a4");
  const [landscape, setLandscape] = useState(false);
  const [margin, setMargin] = useState(0.4);
  const [backgrounds, setBackgrounds] = useState(true);
  const [headerAndFooter, setHeaderAndFooter] = useState(false);

  const [result, setResult] = useState<HtmlResult | null>(null);
  const [shot, setShot] = useState<ShotResult | null>(null);
  const [shotWidth, setShotWidth] = useState(1280);
  const [fullPage, setFullPage] = useState(true);
  const [png, setPng] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const source = (): HtmlSource | null => {
    if (mode === "url")
      return url.trim() ? { kind: "url", value: url.trim() } : null;
    if (mode === "file") return file ? { kind: "file", value: file } : null;
    return markup.trim() ? { kind: "html", value: markup } : null;
  };

  const run = async () => {
    const chosen = source();
    if (!chosen) return;
    setBusy(true);
    setError(null);
    try {
      if (as === "image") {
        setShot(
          await htmlToImage(chosen, png ? "page.png" : "page.jpg", {
            width: shotWidth,
            fullPage,
            height: 900,
            png,
            settleMs: 600,
          }),
        );
      } else {
        const setup: HtmlPageSetup = {
          size,
          landscape,
          margin,
          backgrounds,
          scale: 1,
          headerAndFooter,
          settleMs: 600,
        };
        setResult(await htmlToPdf(chosen, "page.pdf", setup));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const field: React.CSSProperties = {
    borderRadius: 12,
    minHeight: 46,
    padding: "12px 14px",
    fontSize: 13,
    border: "none",
    color: "var(--text)",
    width: "100%",
    fontFamily: "inherit",
  };

  return (
    <div className="col grow" style={{ gap: 22, minHeight: 0 }}>
      <header className="row" style={{ gap: 16 }}>
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
          <span className="section-heading">
            {as === "image" ? "Convert to image" : "Convert to PDF"}
          </span>
          <h1 className="display" style={{ fontSize: 26 }}>
            {as === "image" ? "Web page to image" : "Web page to PDF"}
          </h1>
        </div>
      </header>

      <div
        className="row grow"
        style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}
      >
        <section
          className="raised col grow scroll"
          style={{
            borderRadius: "var(--r-panel)",
            padding: 24,
            gap: 18,
            minWidth: 0,
          }}
        >
          {!isDesktop() ? (
            <div className="col" style={{ gap: 10, padding: "32px 4px" }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                This needs the desktop app
              </h2>
              <p
                style={{
                  color: "var(--text-subtle)",
                  maxWidth: 540,
                  lineHeight: 1.7,
                }}
              >
                Rendering a page needs the browser engine Kitbox itself runs on.
              </p>
            </div>
          ) : (
            <>
              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "repeat(3, 1fr)",
                  gap: 8,
                  maxWidth: 420,
                }}
              >
                {(
                  [
                    ["url", "A web address"],
                    ["file", "An HTML file"],
                    ["html", "Paste HTML"],
                  ] as [Mode, string][]
                ).map(([id, label]) => (
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
                      minHeight: 44,
                      fontSize: 12.5,
                      fontWeight: mode === id ? 700 : 500,
                      color: mode === id ? "var(--accent)" : undefined,
                    }}
                  >
                    {label}
                  </button>
                ))}
              </div>

              {mode === "url" && (
                <div className="col" style={{ gap: 9, maxWidth: 620 }}>
                  <input
                    type="url"
                    value={url}
                    placeholder="https://example.com/receipt"
                    aria-label="The address to render"
                    onChange={(e) => {
                      setUrl(e.target.value);
                      setResult(null);
                    }}
                    className="pressed"
                    style={field}
                  />
                  <span
                    className="row"
                    style={{ gap: 7, alignItems: "flex-start" }}
                  >
                    <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                    <span
                      style={{
                        fontSize: 11.5,
                        color: "var(--text-subtle)",
                        lineHeight: 1.55,
                      }}
                    >
                      This one fetches from the internet — it's the only tool
                      here that does, apart from the one-off OCR download. The
                      page loads in a window with no access to Kitbox's tools.
                    </span>
                  </span>
                </div>
              )}

              {mode === "file" && (
                <div className="col" style={{ gap: 9, maxWidth: 620 }}>
                  <button
                    type="button"
                    className={file ? "pressed" : "raised-sm btn"}
                    // The system's own dialog, because this needs a location
                    // rather than bytes: the renderer opens the page itself so
                    // the stylesheets and pictures beside it come too.
                    onClick={async () => {
                      const chosen = await pickFile("Web page", ["html", "htm"]).catch(
                        () => null,
                      );
                      if (chosen) {
                        setFile(chosen);
                        setResult(null);
                        setError(null);
                      }
                    }}
                    style={{
                      ...field,
                      textAlign: "left",
                      display: "flex",
                      alignItems: "center",
                    }}
                  >
                    {file || "Choose an .html file"}
                  </button>
                  <span
                    style={{
                      fontSize: 11.5,
                      color: "var(--text-subtle)",
                      lineHeight: 1.55,
                    }}
                  >
                    Its own stylesheets and images are loaded from beside it, so
                    a saved page comes out looking like it did.
                  </span>
                </div>
              )}

              {mode === "html" && (
                <div className="col" style={{ gap: 9 }}>
                  <textarea
                    value={markup}
                    rows={14}
                    placeholder="<h1>Invoice</h1>…"
                    aria-label="The HTML to render"
                    onChange={(e) => {
                      setMarkup(e.target.value);
                      setResult(null);
                    }}
                    className="pressed"
                    style={{
                      ...field,
                      fontFamily: "var(--font-mono, ui-monospace, monospace)",
                      fontSize: 12.5,
                      lineHeight: 1.6,
                      resize: "vertical",
                    }}
                  />
                </div>
              )}
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{
            flex: "0 0 300px",
            borderRadius: "var(--r-panel)",
            padding: 22,
            gap: 18,
          }}
        >
          {as === "image" ? (
            <>
              <div className="col" style={{ gap: 9 }}>
                <span
                  style={{
                    fontSize: 13,
                    fontWeight: 600,
                    color: "var(--text-muted)",
                  }}
                >
                  Page width
                </span>
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "repeat(3, 1fr)",
                    gap: 8,
                  }}
                >
                  {[900, 1280, 1600].map((w) => (
                    <button
                      key={w}
                      type="button"
                      aria-pressed={shotWidth === w}
                      className={shotWidth === w ? "pressed" : "raised-sm"}
                      onClick={() => {
                        setShotWidth(w);
                        setShot(null);
                      }}
                      style={{
                        borderRadius: 12,
                        minHeight: 42,
                        fontSize: 12,
                        fontWeight: shotWidth === w ? 700 : 500,
                        color: shotWidth === w ? "var(--accent)" : undefined,
                      }}
                    >
                      {w}
                    </button>
                  ))}
                </div>
                <span
                  style={{
                    fontSize: 11.5,
                    color: "var(--text-subtle)",
                    lineHeight: 1.55,
                  }}
                >
                  The page is laid out as if the window were this wide. The
                  picture comes out larger if your display is scaled.
                </span>
              </div>

              <label
                className="row"
                style={{ gap: 10, alignItems: "center", cursor: "pointer" }}
              >
                <input
                  type="checkbox"
                  checked={fullPage}
                  onChange={(e) => {
                    setFullPage(e.target.checked);
                    setShot(null);
                  }}
                />
                <span style={{ fontSize: 12.5, fontWeight: 600 }}>
                  The whole page, not one screen
                </span>
              </label>

              <label
                className="row"
                style={{ gap: 10, alignItems: "center", cursor: "pointer" }}
              >
                <input
                  type="checkbox"
                  checked={png}
                  onChange={(e) => {
                    setPng(e.target.checked);
                    setShot(null);
                  }}
                />
                <span style={{ fontSize: 12.5, fontWeight: 600 }}>
                  PNG — crisp text
                </span>
              </label>
            </>
          ) : (
            <>
              <Choice
                label="Page size"
                value={size}
                options={[
                  ["a4", "A4"],
                  ["letter", "Letter"],
                  ["legal", "Legal"],
                  ["a3", "A3"],
                ]}
                onChange={(v) => {
                  setSize(v);
                  setResult(null);
                }}
              />

              <Choice
                label="Orientation"
                value={landscape ? "landscape" : "portrait"}
                options={[
                  ["portrait", "Portrait"],
                  ["landscape", "Landscape"],
                ]}
                onChange={(v) => {
                  setLandscape(v === "landscape");
                  setResult(null);
                }}
              />

              <div className="col" style={{ gap: 8 }}>
                <div
                  className="row"
                  style={{ justifyContent: "space-between" }}
                >
                  <span
                    style={{
                      fontSize: 13,
                      fontWeight: 600,
                      color: "var(--text-muted)",
                    }}
                  >
                    Margin
                  </span>
                  <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                    {margin.toFixed(2)} in
                  </span>
                </div>
                <input
                  type="range"
                  min={0}
                  max={150}
                  aria-label="Margin"
                  value={Math.round(margin * 100)}
                  onChange={(e) => {
                    setMargin(Number(e.target.value) / 100);
                    setResult(null);
                  }}
                />
              </div>

              <label
                className="row"
                style={{ gap: 10, alignItems: "center", cursor: "pointer" }}
              >
                <input
                  type="checkbox"
                  checked={backgrounds}
                  onChange={(e) => {
                    setBackgrounds(e.target.checked);
                    setResult(null);
                  }}
                />
                <span style={{ fontSize: 12.5, fontWeight: 600 }}>
                  Print backgrounds
                </span>
              </label>

              <label
                className="row"
                style={{ gap: 10, alignItems: "center", cursor: "pointer" }}
              >
                <input
                  type="checkbox"
                  checked={headerAndFooter}
                  onChange={(e) => {
                    setHeaderAndFooter(e.target.checked);
                    setResult(null);
                  }}
                />
                <span style={{ fontSize: 12.5, fontWeight: 600 }}>
                  Title and address in margins
                </span>
              </label>
            </>
          )}

          {error && (
            <div
              className="pressed col"
              style={{ borderRadius: 16, padding: 14, gap: 6 }}
            >
              <span
                className="row"
                style={{ gap: 8, fontSize: 13, fontWeight: 600 }}
              >
                <Icon path={ICONS.warning} size={16} strokeWidth={2} />
                That didn't work
              </span>
              <span
                style={{
                  fontSize: 12,
                  color: "var(--text-subtle)",
                  lineHeight: 1.5,
                }}
              >
                {error}
              </span>
            </div>
          )}

          {shot && (
            <div
              className="pressed col"
              style={{ borderRadius: 16, padding: 14, gap: 8 }}
            >
              <span className="display" style={{ fontSize: 18 }}>
                {shot.width} × {shot.height}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {formatSize(shot.size)}
              </span>
              {shot.warnings.map((w) => (
                <span
                  key={w}
                  style={{
                    fontSize: 11.5,
                    color: "var(--text-subtle)",
                    lineHeight: 1.5,
                  }}
                >
                  {w}
                </span>
              ))}
              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 40 }}
                onClick={() => revealOutput(shot.outputPath)}
              >
                <Icon path={ICONS.save} size={16} strokeWidth={2} />
                Show in folder
              </button>
            </div>
          )}

          {result && (
            <div
              className="pressed col"
              style={{ borderRadius: 16, padding: 14, gap: 8 }}
            >
              <span className="display" style={{ fontSize: 18 }}>
                {result.pages} page{result.pages === 1 ? "" : "s"}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {formatSize(result.size)}
              </span>
              {result.warnings.map((w) => (
                <span
                  key={w}
                  style={{
                    fontSize: 11.5,
                    color: "var(--text-subtle)",
                    lineHeight: 1.5,
                  }}
                >
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

          <div
            className="pressed col"
            style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}
          >
            <span
              style={{
                fontSize: 11.5,
                color: "var(--text-subtle)",
                lineHeight: 1.5,
              }}
            >
              You get the page as it looked when it loaded. Anything behind a
              login, further down a lazy-loading list, or added later by a
              script may be missing — and nothing interactive survives.
            </span>
          </div>

          <button
            type="button"
            className="btn btn-primary"
            style={{
              minHeight: 56,
              borderRadius: 18,
              fontSize: 15,
              fontWeight: 700,
            }}
            disabled={!source() || busy}
            onClick={run}
          >
            {busy
              ? "Rendering…"
              : as === "image"
                ? "Take the picture"
                : "Make the PDF"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Choice({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: [string, string][];
  onChange: (value: string) => void;
}) {
  return (
    <div className="col" style={{ gap: 9 }}>
      <span
        style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}
      >
        {label}
      </span>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: `repeat(${Math.min(options.length, 2)}, 1fr)`,
          gap: 8,
        }}
      >
        {options.map(([id, text]) => (
          <button
            key={id}
            type="button"
            aria-pressed={value === id}
            className={value === id ? "pressed" : "raised-sm"}
            onClick={() => onChange(id)}
            style={{
              borderRadius: 12,
              minHeight: 42,
              fontSize: 12.5,
              fontWeight: value === id ? 700 : 500,
              color: value === id ? "var(--accent)" : undefined,
            }}
          >
            {text}
          </button>
        ))}
      </div>
    </div>
  );
}
