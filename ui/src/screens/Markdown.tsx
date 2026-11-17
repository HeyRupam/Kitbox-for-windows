import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { isDesktop, isPdf, pdfToMarkdown, revealOutput } from "../lib/bridge";
import type { MarkdownResult } from "../lib/types";

/**
 * Pull a PDF's text out as Markdown.
 *
 * The result is shown rather than just written, because whether the structure
 * came out sensibly is something you can only judge by looking. The report sits
 * above it: pages with no text layer, and pages laid out in columns where the
 * reading order will be jumbled.
 */
export function Markdown({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const file = files.find(isPdf);
  const [result, setResult] = useState<MarkdownResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const [pageBreaks, setPageBreaks] = useState(false);
  const [headings, setHeadings] = useState(true);
  const [lists, setLists] = useState(true);

  useEffect(() => {
    setResult(null);
    setError(null);
  }, [file]);

  const run = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    setCopied(false);
    try {
      setResult(
        await pdfToMarkdown(file, {
          pageBreaks,
          detectHeadings: headings,
          detectLists: lists,
        }),
      );
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
            <span className="section-heading">Convert from PDF</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              PDF to Markdown
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="application/pdf"
          hidden
          onChange={(e) => onFiles([...(e.target.files ?? [])])}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          {file ? "Change file" : "Open a PDF"}
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          {result?.note && (
            <div
              className="pressed"
              style={{
                borderRadius: 14,
                padding: "12px 16px",
                fontSize: 12.5,
                color: "var(--text-subtle)",
                lineHeight: 1.6,
              }}
            >
              {result.note}
            </div>
          )}

          <div
            className="pressed grow scroll"
            style={{ borderRadius: 18, padding: 18, minHeight: 0 }}
          >
            {!file ? (
              <Empty
                title={isDesktop() ? "Open a PDF" : "This needs the desktop app"}
                body={
                  isDesktop()
                    ? "Its text comes out as Markdown — headings from the type size, lists from the bullets, paragraphs from the spacing."
                    : "The browser preview can compress images but not read documents. Open Kitbox itself."
                }
              />
            ) : !result ? (
              <Empty
                title="Ready"
                body="Press Convert to read the document. The result appears here so you can see whether the structure came out sensibly."
              />
            ) : result.markdown.trim() ? (
              <pre
                style={{
                  margin: 0,
                  fontFamily: "var(--font-mono, ui-monospace, monospace)",
                  fontSize: 12.5,
                  lineHeight: 1.7,
                  whiteSpace: "pre-wrap",
                  wordBreak: "break-word",
                  color: "var(--text-muted)",
                }}
              >
                {result.markdown}
              </pre>
            ) : (
              <Empty
                title="Nothing to read"
                body="There is no text layer in this document, so there is nothing to convert."
              />
            )}
          </div>
        </section>

        <aside
          className="raised col scroll"
          aria-label="Conversion settings"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
            What to infer
          </span>
          <Switch
            label="Headings"
            hint="From text set larger than the body"
            on={headings}
            onChange={(v) => {
              setHeadings(v);
              setResult(null);
            }}
          />
          <Switch
            label="Lists"
            hint="From bullets and leading numbers"
            on={lists}
            onChange={(v) => {
              setLists(v);
              setResult(null);
            }}
          />
          <Switch
            label="Page markers"
            hint="A rule and a comment between pages"
            on={pageBreaks}
            onChange={(v) => {
              setPageBreaks(v);
              setResult(null);
            }}
          />

          <span
            style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}
          >
            A PDF has no headings or paragraphs, only glyphs at coordinates, so all of this is
            inferred from position and size. It reads an ordinary document well and a
            multi-column layout poorly.
          </span>

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
                {result.pagesWithText} of {result.pages} pages
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {result.characters.toLocaleString()} characters
              </span>
              {result.markdown.trim() && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ minHeight: 40 }}
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(result.markdown);
                      setCopied(true);
                    } catch {
                      setCopied(false);
                    }
                  }}
                >
                  {copied ? "Copied" : "Copy the text"}
                </button>
              )}
              {result.outputPath && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ minHeight: 40 }}
                  onClick={() => revealOutput(result.outputPath!)}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show in folder
                </button>
              )}
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
            {busy ? "Reading…" : "Convert"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Switch({
  label,
  hint,
  on,
  onChange,
}: {
  label: string;
  hint: string;
  on: boolean;
  onChange: (on: boolean) => void;
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
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>{hint}</span>
      </span>
      <span className="pressed switch">
        <span style={{ left: on ? 30 : 4, background: on ? "var(--accent)" : "var(--surface)" }} />
      </span>
    </button>
  );
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
      <h2 className="display" style={{ fontSize: 19 }}>
        {title}
      </h2>
      <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.7, fontSize: 13.5 }}>
        {body}
      </p>
    </div>
  );
}
