import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  annotatePdf,
  formatSize,
  isDesktop,
  isPdf,
  pdfPages,
  pdfPreviews,
  revealOutput,
} from "../lib/bridge";
import type { AnnotateResult, Annotation, AnnotationMark, PageInfo } from "../lib/types";

type Tool = "text" | "highlight" | "whiteout" | "rectangle" | "ellipse" | "arrow" | "pen";

const TOOLS: { id: Tool; label: string; hint: string }[] = [
  { id: "highlight", label: "Highlight", hint: "Like a marker pen — the words underneath stay readable." },
  { id: "text", label: "Text", hint: "Drag a box, then type. It wraps to the width you drew." },
  { id: "pen", label: "Pen", hint: "Draw freehand. Good for a tick, a circle, a quick initial." },
  { id: "rectangle", label: "Box", hint: "An outline around something." },
  { id: "ellipse", label: "Ellipse", hint: "A ring around something." },
  { id: "arrow", label: "Arrow", hint: "Drag from the tail to the point." },
  { id: "whiteout", label: "White-out", hint: "Paints over a mistake. It hides; it does not delete." },
];

const COLOURS: [string, [number, number, number]][] = [
  ["Orange", [194, 65, 12]],
  ["Yellow", [255, 213, 74]],
  ["Blue", [29, 78, 216]],
  ["Green", [15, 118, 110]],
  ["Red", [179, 38, 30]],
  ["Black", [32, 32, 38]],
];

/**
 * Draw on a page.
 *
 * The fallback for every form that isn't fillable, which is most of them. The
 * marks are drawn into the page itself rather than added as PDF annotations —
 * they print, they show up in every reader, and they can't be peeled off,
 * which the screen says before you commit rather than after.
 */
export function Annotate({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdf = files.find(isPdf) ?? null;

  const [tool, setTool] = useState<Tool>("highlight");
  const [colour, setColour] = useState<[number, number, number]>(COLOURS[0][1]);
  const [marks, setMarks] = useState<Annotation[]>([]);
  const [pages, setPages] = useState<PageInfo[]>([]);
  const [previews, setPreviews] = useState<(string | null)[]>([]);
  const [page, setPage] = useState(1);

  const [drag, setDrag] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const [stroke, setStroke] = useState<[number, number][]>([]);
  const [typing, setTyping] = useState<{ box: Annotation; text: string } | null>(null);

  const [result, setResult] = useState<AnnotateResult | null>(null);
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
  const at = (event: React.MouseEvent): { x: number; y: number } | null => {
    const box = sheetRef.current?.getBoundingClientRect();
    if (!box) return null;
    return {
      x: Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)),
      y: Math.min(1, Math.max(0, (event.clientY - box.top) / box.height)),
    };
  };

  const add = (mark: AnnotationMark, box: { x: number; y: number; w: number; h: number }) => {
    setMarks([
      ...marks,
      {
        page,
        x: box.x,
        y: box.y,
        width: box.w,
        height: box.h,
        mark,
        colour,
        lineWidth: tool === "pen" ? 2 : 1.5,
        opacity: 1,
      },
    ]);
    setResult(null);
  };

  const finish = () => {
    if (tool === "pen") {
      if (stroke.length >= 2) {
        add({ kind: "ink", points: stroke }, { x: 0, y: 0, w: 1, h: 1 });
      }
      setStroke([]);
      start.current = null;
      return;
    }
    if (drag && drag.w > 0.004 && drag.h > 0.004) {
      const box = drag;
      if (tool === "text") {
        // The box is drawn first and typed into after, so the wrap width is
        // whatever was actually drawn.
        setTyping({
          box: {
            page,
            x: box.x,
            y: box.y,
            width: box.w,
            height: box.h,
            mark: { kind: "text", text: "", size: 11 },
            colour,
            lineWidth: 1.5,
            opacity: 1,
          },
          text: "",
        });
      } else {
        const mark: AnnotationMark =
          tool === "highlight"
            ? { kind: "highlight" }
            : tool === "whiteout"
              ? { kind: "whiteout" }
              : tool === "rectangle"
                ? { kind: "rectangle", filled: false }
                : tool === "ellipse"
                  ? { kind: "ellipse", filled: false }
                  : { kind: "line", arrow: true };
        add(mark, box);
      }
    }
    setDrag(null);
    start.current = null;
  };

  const run = async () => {
    if (!pdf || marks.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await annotatePdf(pdf, marks));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const quarter = current ? Math.abs(current.rotation % 180) === 90 : false;
  const sheetW = current ? (quarter ? current.heightPt : current.widthPt) : 595;
  const sheetH = current ? (quarter ? current.widthPt : current.heightPt) : 842;
  const onThisPage = marks.filter((m) => m.page === page);
  const rgb = (c: [number, number, number]) => `rgb(${c[0]},${c[1]},${c[2]})`;
  const hasWhiteout = marks.some((m) => m.mark.kind === "whiteout");

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
            <span className="section-heading">Edit PDF</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Draw on a page
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
              setMarks([]);
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
                  ? "Pick a tool, then drag on the page. Highlights work like a marker pen — the words underneath stay readable, even on a scan."
                  : "The browser preview can compress images but not edit documents. Open Kitbox itself."}
              </p>
            </div>
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <span className="section-heading">
                  {TOOLS.find((t) => t.id === tool)?.hint}
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
                <div
                  ref={sheetRef}
                  onMouseDown={(e) => {
                    if (typing) return;
                    const point = at(e);
                    if (!point) return;
                    start.current = point;
                    if (tool === "pen") setStroke([[point.x, point.y]]);
                    else setDrag({ x: point.x, y: point.y, w: 0, h: 0 });
                  }}
                  onMouseMove={(e) => {
                    if (!start.current) return;
                    const point = at(e);
                    if (!point) return;
                    if (tool === "pen") {
                      setStroke((s) => [...s, [point.x, point.y]]);
                    } else {
                      setDrag({
                        x: Math.min(start.current.x, point.x),
                        y: Math.min(start.current.y, point.y),
                        w: Math.abs(point.x - start.current.x),
                        h: Math.abs(point.y - start.current.y),
                      });
                    }
                  }}
                  onMouseUp={finish}
                  onMouseLeave={() => {
                    if (start.current) finish();
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
                    cursor: typing ? "default" : "crosshair",
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
                      The proportions are right, so marks can still be placed by eye.
                    </span>
                  )}

                  <svg
                    viewBox="0 0 1000 1000"
                    preserveAspectRatio="none"
                    style={{ position: "absolute", inset: 0, width: "100%", height: "100%", pointerEvents: "none" }}
                  >
                    {onThisPage.map((m, i) => (
                      <Preview key={i} mark={m} />
                    ))}
                    {stroke.length > 1 && (
                      <polyline
                        points={stroke.map(([x, y]) => `${x * 1000},${y * 1000}`).join(" ")}
                        fill="none"
                        stroke={rgb(colour)}
                        strokeWidth={3}
                        vectorEffect="non-scaling-stroke"
                      />
                    )}
                  </svg>

                  {drag && (
                    <span
                      style={{
                        position: "absolute",
                        left: `${drag.x * 100}%`,
                        top: `${drag.y * 100}%`,
                        width: `${drag.w * 100}%`,
                        height: `${drag.h * 100}%`,
                        background:
                          tool === "highlight" || tool === "whiteout"
                            ? `${rgb(colour)}55`
                            : "transparent",
                        outline: `1.5px solid ${rgb(colour)}`,
                        pointerEvents: "none",
                      }}
                    />
                  )}

                  {typing && (
                    <textarea
                      autoFocus
                      value={typing.text}
                      aria-label="What the note says"
                      onChange={(e) => setTyping({ ...typing, text: e.target.value })}
                      onBlur={() => {
                        const text = typing.text.trim();
                        if (text) {
                          setMarks([
                            ...marks,
                            { ...typing.box, mark: { kind: "text", text, size: 11 } },
                          ]);
                          setResult(null);
                        }
                        setTyping(null);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") {
                          setTyping(null);
                        }
                      }}
                      style={{
                        position: "absolute",
                        left: `${typing.box.x * 100}%`,
                        top: `${typing.box.y * 100}%`,
                        width: `${typing.box.width * 100}%`,
                        height: `${typing.box.height * 100}%`,
                        minHeight: 22,
                        border: `1.5px solid ${rgb(colour)}`,
                        background: "rgba(255,255,255,0.94)",
                        color: rgb(colour),
                        font: "600 12px/1.3 Inter, system-ui, sans-serif",
                        padding: 3,
                        resize: "none",
                        borderRadius: 2,
                      }}
                    />
                  )}
                </div>
              </div>

              {pages.length > 1 && (
                <div className="row scroll-x" style={{ gap: 8 }}>
                  {pages.map((p) => {
                    const marked = marks.some((m) => m.page === p.number);
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
                          color: marked || p.number === page ? "var(--accent)" : undefined,
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
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Tool</span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
              {TOOLS.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  aria-pressed={tool === t.id}
                  className={tool === t.id ? "pressed" : "raised-sm"}
                  onClick={() => setTool(t.id)}
                  style={{
                    borderRadius: 12,
                    minHeight: 42,
                    fontSize: 12,
                    fontWeight: tool === t.id ? 700 : 500,
                    color: tool === t.id ? "var(--accent)" : undefined,
                  }}
                >
                  {t.label}
                </button>
              ))}
            </div>
          </div>

          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Colour
            </span>
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              {COLOURS.map(([name, c]) => (
                <button
                  key={name}
                  type="button"
                  aria-label={name}
                  aria-pressed={colour.join() === c.join()}
                  className={colour.join() === c.join() ? "pressed" : "raised-sm"}
                  onClick={() => setColour(c)}
                  style={{ width: 38, height: 38, borderRadius: 11, display: "grid", placeItems: "center" }}
                >
                  <span
                    style={{
                      width: 17,
                      height: 17,
                      borderRadius: 999,
                      background: rgb(c),
                      boxShadow: "inset 0 0 0 1px rgba(0,0,0,0.12)",
                    }}
                  />
                </button>
              ))}
            </div>
          </div>

          {marks.length > 0 && (
            <div className="col" style={{ gap: 8 }}>
              <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline" }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  {marks.length} mark{marks.length === 1 ? "" : "s"}
                </span>
                <div className="row" style={{ gap: 6 }}>
                  <button
                    type="button"
                    className="raised-sm"
                    style={{ borderRadius: 999, padding: "4px 11px", fontSize: 11.5 }}
                    onClick={() => {
                      setMarks(marks.slice(0, -1));
                      setResult(null);
                    }}
                  >
                    Undo
                  </button>
                  <button
                    type="button"
                    className="raised-sm"
                    style={{ borderRadius: 999, padding: "4px 11px", fontSize: 11.5 }}
                    onClick={() => {
                      setMarks([]);
                      setResult(null);
                    }}
                  >
                    Clear
                  </button>
                </div>
              </div>
            </div>
          )}

          {hasWhiteout && (
            <div className="pressed col" style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}>
              <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
                <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                White-out only covers
              </span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                What's underneath stays in the file and can still be selected or extracted. Use
                Redact to actually delete it.
              </span>
            </div>
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
                {result.drawn} mark{result.drawn === 1 ? "" : "s"} drawn
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                Page{result.pagesChanged.length === 1 ? "" : "s"}{" "}
                {result.pagesChanged.join(", ")} · {formatSize(result.size)}
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
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              Marks become part of the page: they print and show up in every reader, and they
              can't be peeled off later. The original stays in your history.
            </span>
          </div>

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!pdf || marks.length === 0 || busy}
            onClick={run}
          >
            {busy ? "Drawing…" : "Draw them on"}
          </button>
        </aside>
      </div>
    </div>
  );
}

/** What a placed mark looks like on the preview, in a 1000×1000 viewBox. */
function Preview({ mark }: { mark: Annotation }) {
  const colour = `rgb(${mark.colour[0]},${mark.colour[1]},${mark.colour[2]})`;
  const x = mark.x * 1000;
  const y = mark.y * 1000;
  const w = mark.width * 1000;
  const h = mark.height * 1000;

  switch (mark.mark.kind) {
    case "highlight":
      return <rect x={x} y={y} width={w} height={h} fill={colour} opacity={0.42} />;
    case "whiteout":
      return <rect x={x} y={y} width={w} height={h} fill="#fff" stroke="#c9ced9" strokeWidth={1} vectorEffect="non-scaling-stroke" />;
    case "rectangle":
      return <rect x={x} y={y} width={w} height={h} fill="none" stroke={colour} strokeWidth={2} vectorEffect="non-scaling-stroke" />;
    case "ellipse":
      return <ellipse cx={x + w / 2} cy={y + h / 2} rx={w / 2} ry={h / 2} fill="none" stroke={colour} strokeWidth={2} vectorEffect="non-scaling-stroke" />;
    case "line":
      // Drawn bottom-left to top-right, matching how the core reads the box.
      return <line x1={x} y1={y + h} x2={x + w} y2={y} stroke={colour} strokeWidth={2} vectorEffect="non-scaling-stroke" />;
    case "ink":
      return (
        <polyline
          points={mark.mark.points.map(([px, py]) => `${px * 1000},${py * 1000}`).join(" ")}
          fill="none"
          stroke={colour}
          strokeWidth={3}
          vectorEffect="non-scaling-stroke"
        />
      );
    case "text":
      return (
        <text x={x + 2} y={y + 22} fill={colour} fontSize={26} fontWeight={600}>
          {mark.mark.text.slice(0, 40)}
        </text>
      );
    default:
      return null;
  }
}
