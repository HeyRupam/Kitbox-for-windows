import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, isDesktop, isPdf, pdfToArchive, revealOutput } from "../lib/bridge";
import type { ArchivePart, ArchiveResult } from "../lib/types";

/**
 * PDF/A — a document meant to still open in thirty years.
 *
 * The screen leads with the verdict rather than the button, because the useful
 * answer here is usually "no, and here is why". Anyone can write PDF/A in a
 * file's metadata; the question worth answering is whether the document
 * actually meets the rules, and which one it fails.
 *
 * It never says "conformant" unqualified. Kitbox checks the rules it knows and
 * fixes what it can; a certified validator is the final word, and the screen
 * says so next to the result rather than in small print.
 */
export function Archive({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdf = files.find(isPdf) ?? null;

  const [part, setPart] = useState<ArchivePart>("a2b");
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [checked, setChecked] = useState<ArchiveResult | null>(null);
  const [saved, setSaved] = useState<ArchiveResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  // Look at the document as soon as there is one: the report is the point.
  useEffect(() => {
    if (!pdf || !isDesktop()) {
      setChecked(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    setError(null);
    pdfToArchive(pdf, part, true)
      .then((r) => !cancelled && setChecked(r))
      .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => !cancelled && setBusy(false));
    return () => {
      cancelled = true;
    };
  }, [pdf, part]);

  const convert = async () => {
    if (!pdf) return;
    setBusy(true);
    setError(null);
    try {
      setSaved(await pdfToArchive(pdf, part, false, title, author));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const shown = saved ?? checked;

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
              PDF for archiving
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="application/pdf"
          hidden
          onChange={(e) => {
            const picked = e.target.files?.[0];
            if (picked) {
              onFiles([picked, ...files.filter((f) => !isPdf(f))]);
              setSaved(null);
            }
          }}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          {pdf ? "Change document" : "Choose a PDF"}
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow scroll"
          style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 18, minWidth: 0 }}
        >
          {!isDesktop() ? (
            <Blurb
              title="This needs the desktop app"
              body="The browser preview can compress images but not inspect documents."
            />
          ) : !pdf ? (
            <Blurb
              title="Choose a document"
              body="PDF/A is a PDF that carries everything it needs — its fonts, its colour, no scripts, nothing linked from outside. Kitbox will say whether yours qualifies, and fix what it can."
            />
          ) : !shown ? (
            <Blurb title={busy ? "Looking at it…" : "Ready"} body="" />
          ) : (
            <div className="col" style={{ gap: 18 }}>
              <div
                className="pressed col"
                style={{
                  borderRadius: 18,
                  padding: 18,
                  gap: 8,
                  outline: shown.conformant ? "none" : "2px solid var(--accent)",
                }}
              >
                <span className="display" style={{ fontSize: 21 }}>
                  {shown.conformant
                    ? `Meets every ${shown.part} rule Kitbox checks`
                    : `Not ${shown.part} yet`}
                </span>
                <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                  {shown.note}
                </span>
              </div>

              {shown.blockers.length > 0 && (
                <div className="col" style={{ gap: 10 }}>
                  <span className="section-heading" style={{ fontSize: 10.5 }}>
                    What stands in the way
                  </span>
                  {shown.blockers.map((b) => (
                    <div
                      key={b.rule}
                      className="pressed col"
                      style={{ borderRadius: 14, padding: 14, gap: 5 }}
                    >
                      <span style={{ fontSize: 13, fontWeight: 700 }}>{b.rule}</span>
                      <span
                        style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6 }}
                      >
                        {b.detail}
                      </span>
                    </div>
                  ))}
                </div>
              )}

              {shown.applied.length > 0 && (
                <div className="col" style={{ gap: 8 }}>
                  <span className="section-heading" style={{ fontSize: 10.5 }}>
                    What was changed
                  </span>
                  {shown.applied.map((a) => (
                    <span key={a} style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
                      · {a}
                    </span>
                  ))}
                </div>
              )}

              {!shown.profileFound && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
                  No colour profile was found on this machine, so the output intent can't be
                  written. Normally Windows supplies one.
                </span>
              )}
            </div>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Which standard
            </span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
              {(
                [
                  ["a2b", "PDF/A-2b"],
                  ["a1b", "PDF/A-1b"],
                ] as [ArchivePart, string][]
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={part === id}
                  className={part === id ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setPart(id);
                    setSaved(null);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 44,
                    fontSize: 12,
                    fontWeight: part === id ? 700 : 500,
                    color: part === id ? "var(--accent)" : undefined,
                  }}
                >
                  {label}
                </button>
              ))}
            </div>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
              {part === "a2b"
                ? "The 2011 revision, and the sensible default — it allows transparency, which nearly every modern document uses somewhere."
                : "The 2005 original. Stricter: no transparency at all, which rules out most documents with a logo or a signature in them."}
            </span>
          </div>

          <div className="col" style={{ gap: 8 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Recorded in the file
            </span>
            <input
              type="text"
              value={title}
              placeholder="Title"
              aria-label="Title"
              onChange={(e) => {
                setTitle(e.target.value);
                setSaved(null);
              }}
              className="pressed"
              style={field}
            />
            <input
              type="text"
              value={author}
              placeholder="Author"
              aria-label="Author"
              onChange={(e) => {
                setAuthor(e.target.value);
                setSaved(null);
              }}
              className="pressed"
              style={field}
            />
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              Written into the metadata, which an archive indexes by. Anything already in the
              document is replaced, so the two can't disagree.
            </span>
          </div>

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

          {saved?.outputPath && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
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
            disabled={!pdf || busy}
            onClick={convert}
          >
            {busy ? "Working…" : "Convert it"}
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
      {body && (
        <p style={{ color: "var(--text-subtle)", maxWidth: 560, lineHeight: 1.7 }}>{body}</p>
      )}
    </div>
  );
}
