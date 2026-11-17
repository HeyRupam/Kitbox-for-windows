import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  formatSize,
  isDesktop,
  isPdf,
  pdfPages,
  pdfPreviews,
  redactPdf,
  revealOutput,
} from "../lib/bridge";
import type { PageInfo, RedactOptions, RedactResult, RedactTarget } from "../lib/types";

/**
 * Take content out of a PDF.
 *
 * The screen leads with the check, not the claim. After a run it says whether
 * the finished file was re-read and found clean, because "redacted" is a thing
 * a tool asserts and "I looked for it again and it wasn't there" is a thing it
 * can show. If anything leaked, that is the loudest element on the screen and
 * the file is marked as not safe to share.
 */
export function Redact({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdf = files.find(isPdf) ?? null;

  const [terms, setTerms] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [boxes, setBoxes] = useState<Exclude<RedactTarget, { kind: "text" }>[]>([]);
  const [drawBoxes, setDrawBoxes] = useState(true);
  const [stripExtras, setStripExtras] = useState(true);

  const [pages, setPages] = useState<PageInfo[]>([]);
  const [previews, setPreviews] = useState<(string | null)[]>([]);
  const [page, setPage] = useState(1);

  const [dragging, setDragging] = useState<{ x: number; y: number; w: number; h: number } | null>(
    null,
  );
  const [result, setResult] = useState<RedactResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const fileRef = useRef<HTMLInputElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const start = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    if (!pdf || !isDesktop()) return;
    let cancelled = false;
    pdfPages(pdf)
      .then((p) => !cancelled && setPages(p))
      .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)));
    pdfPreviews(pdf, 900)
      .then((p) => !cancelled && setPreviews(p))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [pdf]);

  const current = pages[page - 1];
  const targets: RedactTarget[] = [
    ...terms.map((needle) => ({ kind: "text" as const, needle, matchCase })),
    ...boxes,
  ];

  const at = (event: React.MouseEvent) => {
    const box = sheetRef.current?.getBoundingClientRect();
    if (!box) return null;
    return {
      x: Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)),
      y: Math.min(1, Math.max(0, (event.clientY - box.top) / box.height)),
    };
  };

  const addTerm = () => {
    const value = draft.trim();
    if (!value || terms.includes(value)) return;
    setTerms([...terms, value]);
    setDraft("");
    setResult(null);
  };

  const run = async () => {
    if (!pdf || targets.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      const options: RedactOptions = {
        targets,
        drawBoxes,
        boxColour: [0, 0, 0],
        stripExtras,
      };
      setResult(await redactPdf(pdf, options));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const quarter = current ? Math.abs(current.rotation % 180) === 90 : false;
  const sheetW = current ? (quarter ? current.heightPt : current.widthPt) : 595;
  const sheetH = current ? (quarter ? current.widthPt : current.heightPt) : 842;
  const onThisPage = boxes.filter((b) => b.page === page);

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
            <span className="section-heading">Security</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Redact PDF
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
              setBoxes([]);
              setResult(null);
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
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          {!pdf ? (
            <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                {isDesktop() ? "Choose the document" : "This needs the desktop app"}
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 540, lineHeight: 1.65 }}>
                {isDesktop()
                  ? "Type the words to remove, or drag a box over anything on the page. The content is deleted from the file, not covered up — and the result is re-read afterwards to check."
                  : "The browser preview can compress images but not edit documents. Open Kitbox itself."}
              </p>
            </div>
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <span className="section-heading">
                  Drag a box over anything to remove it · page {page} of {pages.length || "…"}
                </span>
                {onThisPage.length > 0 && (
                  <button
                    type="button"
                    className="raised-sm"
                    style={{ borderRadius: 999, padding: "4px 12px", fontSize: 11.5 }}
                    onClick={() => {
                      setBoxes(boxes.filter((b) => b.page !== page));
                      setResult(null);
                    }}
                  >
                    Clear {onThisPage.length} on this page
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
                <div
                  ref={sheetRef}
                  onMouseDown={(e) => {
                    const point = at(e);
                    if (!point) return;
                    start.current = point;
                    setDragging({ x: point.x, y: point.y, w: 0, h: 0 });
                  }}
                  onMouseMove={(e) => {
                    if (!start.current) return;
                    const point = at(e);
                    if (!point) return;
                    setDragging({
                      x: Math.min(start.current.x, point.x),
                      y: Math.min(start.current.y, point.y),
                      w: Math.abs(point.x - start.current.x),
                      h: Math.abs(point.y - start.current.y),
                    });
                  }}
                  onMouseUp={() => {
                    if (dragging && dragging.w > 0.004 && dragging.h > 0.004) {
                      setBoxes([
                        ...boxes,
                        {
                          kind: "area",
                          page,
                          x: dragging.x,
                          y: dragging.y,
                          width: dragging.w,
                          height: dragging.h,
                        },
                      ]);
                      setResult(null);
                    }
                    start.current = null;
                    setDragging(null);
                  }}
                  onMouseLeave={() => {
                    start.current = null;
                    setDragging(null);
                  }}
                  style={{
                    position: "relative",
                    aspectRatio: `${sheetW} / ${sheetH}`,
                    height: "100%",
                    maxHeight: "100%",
                    maxWidth: "100%",
                    background: "#fff",
                    boxShadow: "0 2px 14px rgba(40,46,60,0.22)",
                    borderRadius: 3,
                    cursor: "crosshair",
                    overflow: "hidden",
                    userSelect: "none",
                  }}
                >
                  {previews[page - 1] ? (
                    <img
                      src={previews[page - 1] as string}
                      alt=""
                      draggable={false}
                      style={{ width: "100%", height: "100%", objectFit: "fill" }}
                    />
                  ) : (
                    <span
                      style={{
                        position: "absolute",
                        inset: 0,
                        display: "grid",
                        placeItems: "center",
                        fontSize: 12,
                        color: "#9aa2b1",
                        textAlign: "center",
                        padding: 20,
                        lineHeight: 1.6,
                      }}
                    >
                      This page is text, so there's no picture of it to show.
                      <br />
                      Removing by word works regardless; a box is placed by eye.
                    </span>
                  )}

                  {[...onThisPage.map((b) => ({ ...b, live: false })), ...(dragging
                    ? [{ x: dragging.x, y: dragging.y, width: dragging.w, height: dragging.h, live: true }]
                    : [])].map((b, i) => (
                    <span
                      key={i}
                      style={{
                        position: "absolute",
                        left: `${b.x * 100}%`,
                        top: `${b.y * 100}%`,
                        width: `${b.width * 100}%`,
                        height: `${b.height * 100}%`,
                        background: b.live ? "rgba(194,65,12,0.35)" : "rgba(20,20,24,0.88)",
                        outline: b.live ? "1.5px solid var(--accent)" : "none",
                        pointerEvents: "none",
                      }}
                    />
                  ))}
                </div>
              </div>

              {pages.length > 1 && (
                <div className="row scroll-x" style={{ gap: 8 }}>
                  {pages.map((p) => {
                    const marked = boxes.some((b) => b.page === p.number);
                    return (
                      <button
                        key={p.number}
                        type="button"
                        aria-pressed={p.number === page}
                        className={p.number === page ? "pressed" : "raised-sm"}
                        onClick={() => setPage(p.number)}
                        style={{
                          flex: "0 0 auto",
                          minWidth: 40,
                          height: 36,
                          borderRadius: 11,
                          fontSize: 12.5,
                          fontWeight: p.number === page ? 700 : 500,
                          color: marked
                            ? "var(--accent)"
                            : p.number === page
                              ? "var(--accent)"
                              : undefined,
                        }}
                      >
                        {p.number}
                        {marked ? " •" : ""}
                      </button>
                    );
                  })}
                </div>
              )}
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 320px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Words to remove
            </span>
            <div className="row" style={{ gap: 8 }}>
              <input
                type="text"
                value={draft}
                placeholder="a name, a number…"
                aria-label="Text to remove"
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addTerm();
                  }
                }}
                className="pressed"
                style={{
                  flex: 1,
                  minWidth: 0,
                  borderRadius: 11,
                  minHeight: 42,
                  padding: "0 12px",
                  fontSize: 12.5,
                  border: "none",
                  color: "var(--text)",
                }}
              />
              <button
                type="button"
                className="raised-sm"
                aria-label="Add this word"
                onClick={addTerm}
                style={{ width: 42, height: 42, borderRadius: 11 }}
              >
                <Icon path={ICONS.plus} size={16} strokeWidth={2} />
              </button>
            </div>
            {terms.length > 0 && (
              <div className="row" style={{ gap: 7, flexWrap: "wrap" }}>
                {terms.map((term) => (
                  <button
                    key={term}
                    type="button"
                    className="pressed"
                    aria-label={`Stop removing ${term}`}
                    onClick={() => {
                      setTerms(terms.filter((t) => t !== term));
                      setResult(null);
                    }}
                    style={{
                      borderRadius: 999,
                      padding: "5px 11px",
                      fontSize: 11.5,
                      fontWeight: 600,
                      display: "flex",
                      gap: 6,
                      alignItems: "center",
                    }}
                  >
                    {term}
                    <Icon path="M6 6l12 12 M18 6L6 18" size={10} strokeWidth={2.5} />
                  </button>
                ))}
              </div>
            )}
            <label className="row" style={{ gap: 9, alignItems: "center", cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={matchCase}
                onChange={(e) => {
                  setMatchCase(e.target.checked);
                  setResult(null);
                }}
              />
              <span style={{ fontSize: 12.5 }}>Match capitals exactly</span>
            </label>
          </div>

          <div className="col" style={{ gap: 9 }}>
            <label className="row" style={{ gap: 9, alignItems: "center", cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={drawBoxes}
                onChange={(e) => {
                  setDrawBoxes(e.target.checked);
                  setResult(null);
                }}
              />
              <span style={{ fontSize: 12.5, fontWeight: 600 }}>Draw black bars</span>
            </label>
            <label className="row" style={{ gap: 9, alignItems: "flex-start", cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={stripExtras}
                onChange={(e) => {
                  setStripExtras(e.target.checked);
                  setResult(null);
                }}
                style={{ marginTop: 3 }}
              />
              <span className="col" style={{ gap: 3 }}>
                <span style={{ fontSize: 12.5, fontWeight: 600 }}>Strip the hiding places</span>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  Metadata, bookmarks, embedded files and form data. A title or a bookmark often
                  repeats the thing you're removing.
                </span>
              </span>
            </label>
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

          {result && (
            <div
              className="pressed col"
              style={{
                borderRadius: 16,
                padding: 14,
                gap: 8,
                outline: result.verified ? "none" : "2px solid #b3261e",
              }}
            >
              <span
                className="display"
                style={{ fontSize: 17, color: result.verified ? undefined : "#b3261e" }}
              >
                {result.verified ? "Checked — it's gone" : "Not verified"}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.55 }}>
                {result.verified
                  ? `The finished file was read back and none of it could be found. ${result.glyphsRemoved} letters and ${result.imagesEdited} image${result.imagesEdited === 1 ? "" : "s"} changed on page${result.pagesChanged.length === 1 ? "" : "s"} ${result.pagesChanged.join(", ")}.`
                  : "Some of it survived. Don't share this file."}
              </span>
              {result.leaks.map((leak) => (
                <span key={leak} style={{ fontSize: 11.5, color: "#b3261e", lineHeight: 1.5 }}>
                  {leak}
                </span>
              ))}
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {formatSize(result.size)}
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

          <div className="pressed col" style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}>
            <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
              <Icon path={ICONS.warning} size={14} strokeWidth={2} />
              What a bar can't cover
            </span>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              Text and image content under each bar is deleted. Vector drawings — ruled lines,
              shapes, a signature stored as paths — are covered but their geometry stays in the
              file.
            </span>
          </div>

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!pdf || targets.length === 0 || busy}
            onClick={run}
          >
            {busy
              ? "Removing…"
              : `Remove ${targets.length || ""} ${targets.length === 1 ? "thing" : "things"}`.replace(
                  /\s+/g,
                  " ",
                )}
          </button>
        </aside>
      </div>
    </div>
  );
}
