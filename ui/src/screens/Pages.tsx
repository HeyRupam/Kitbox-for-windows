import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  applyPages,
  formatSize,
  isDesktop,
  isPdf,
  pdfPages,
  pdfPreviews,
  pdfToImages,
  revealOutput,
  splitPdf,
} from "../lib/bridge";
import type {
  ExportedPageResult,
  PageInfo,
  PageOpResult,
  SplitMode,
  SplitPartResult,
} from "../lib/types";

/**
 * One workspace for every page operation.
 *
 * Organize, remove, extract, rotate and split are the same screen with
 * different buttons pressed, because they're the same operation underneath:
 * decide which pages end up in the result, in what order, turned which way.
 * Keeping them together means you can do all of it in one pass instead of
 * running three separate tools over the same document.
 */

interface PageState {
  /** Stable across reordering, so React keeps the right DOM node. */
  key: string;
  /** 1-based page in the source document. */
  source: number;
  /** Quarter turns added on top of whatever the page already had. */
  rotate: number;
  selected: boolean;
}

export function Pages({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const file = files.find((f) => isPdf(f));
  const [info, setInfo] = useState<PageInfo[]>([]);
  const [previews, setPreviews] = useState<(string | null)[]>([]);
  const [pages, setPages] = useState<PageState[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<PageOpResult | null>(null);
  const [parts, setParts] = useState<SplitPartResult[] | null>(null);
  const [splitEvery, setSplitEvery] = useState(1);
  const [exported, setExported] = useState<ExportedPageResult[] | null>(null);
  const dragFrom = useRef<number | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setSaved(null);
    setParts(null);
    setExported(null);
    setError(null);
    if (!file) {
      setInfo([]);
      setPages([]);
      setPreviews([]);
      return;
    }
    let live = true;
    setLoading(true);
    pdfPages(file)
      .then((list) => {
        if (!live) return;
        setInfo(list);
        setPages(
          list.map((p) => ({ key: `p${p.number}`, source: p.number, rotate: 0, selected: false })),
        );
        return pdfPreviews(file, 180);
      })
      .then((thumbs) => live && setPreviews(thumbs ?? []))
      .catch((e) => live && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [file]);

  const selected = pages.filter((p) => p.selected);
  const changed = useMemo(
    () =>
      pages.length !== info.length ||
      pages.some((p, i) => p.source !== i + 1 || p.rotate !== 0),
    [pages, info.length],
  );

  const update = useCallback((fn: (list: PageState[]) => PageState[]) => {
    setPages((list) => fn(list));
    setSaved(null);
  }, []);

  const rotateSelected = (by: number) =>
    update((list) =>
      list.map((p) =>
        p.selected || list.every((q) => !q.selected) ? { ...p, rotate: p.rotate + by } : p,
      ),
    );

  const move = (from: number, to: number) =>
    update((list) => {
      if (to < 0 || to >= list.length) return list;
      const next = [...list];
      const [item] = next.splice(from, 1);
      next.splice(to, 0, item);
      return next;
    });

  const save = async () => {
    if (!file) return;
    setError(null);
    try {
      const result = await applyPages(
        file,
        pages.map((p) => ({ page: p.source, rotate: p.rotate })),
      );
      setSaved(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const runSplit = async (mode: SplitMode) => {
    if (!file) return;
    setError(null);
    try {
      setParts(await splitPdf(file, mode));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="col grow" style={{ gap: 22, minHeight: 0 }}>
      <header
        className="row"
        style={{ flexWrap: "wrap", gap: 16, justifyContent: "space-between" }}
      >
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
            <span className="section-heading">Organize</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Pages
            </h1>
          </div>
        </div>
        <div className="row" style={{ gap: 12 }}>
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
        </div>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow scroll"
          aria-label="Pages"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 16, minWidth: 0 }}
        >
          {!file ? (
            <Empty
              title={isDesktop() ? "Open a PDF to see its pages" : "Page tools need the desktop app"}
              body={
                isDesktop()
                  ? "Drop one anywhere, or use Open a PDF. You can reorder, rotate, remove and split in one pass, then save once."
                  : "The browser preview can work on images but not documents. Open Kitbox itself for the page tools."
              }
            />
          ) : loading ? (
            <Empty title="Reading the document…" body="Finding the pages and pulling thumbnails." />
          ) : (
            <>
              <div className="row" style={{ justifyContent: "space-between", gap: 12 }}>
                <span className="section-heading">
                  {pages.length} page{pages.length === 1 ? "" : "s"}
                  {pages.length !== info.length && ` · was ${info.length}`}
                </span>
                <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                  Drag to reorder
                </span>
              </div>

              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))",
                  gap: 16,
                }}
              >
                {pages.map((page, index) => (
                  <PageCard
                    key={page.key}
                    page={page}
                    index={index}
                    info={info[page.source - 1]}
                    preview={previews[page.source - 1] ?? null}
                    onToggle={() =>
                      update((list) =>
                        list.map((p) =>
                          p.key === page.key ? { ...p, selected: !p.selected } : p,
                        ),
                      )
                    }
                    onRotate={(by) =>
                      update((list) =>
                        list.map((p) =>
                          p.key === page.key ? { ...p, rotate: p.rotate + by } : p,
                        ),
                      )
                    }
                    onRemove={() =>
                      update((list) =>
                        list.length > 1 ? list.filter((p) => p.key !== page.key) : list,
                      )
                    }
                    onMove={(delta) => move(index, index + delta)}
                    onDragStart={() => (dragFrom.current = index)}
                    onDrop={() => {
                      if (dragFrom.current !== null) move(dragFrom.current, index);
                      dragFrom.current = null;
                    }}
                  />
                ))}
              </div>
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          aria-label="Page actions"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Selection
            </span>
            <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              {selected.length === 0
                ? "Nothing selected — actions apply to every page."
                : `${selected.length} of ${pages.length} selected.`}
            </span>
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              {(
                [
                  ["All", () => pages.map((p) => ({ ...p, selected: true }))],
                  ["None", () => pages.map((p) => ({ ...p, selected: false }))],
                  ["Odd", () => pages.map((p, i) => ({ ...p, selected: i % 2 === 0 }))],
                  ["Even", () => pages.map((p, i) => ({ ...p, selected: i % 2 === 1 }))],
                ] as [string, () => PageState[]][]
              ).map(([label, fn]) => (
                <button
                  key={label}
                  type="button"
                  className="raised-sm pill"
                  disabled={!file}
                  onClick={() => update(fn)}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>

          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Do this
            </span>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              <Action label="Turn left" disabled={!file} onClick={() => rotateSelected(-90)} />
              <Action label="Turn right" disabled={!file} onClick={() => rotateSelected(90)} />
              <Action
                label="Keep only these"
                disabled={!file || selected.length === 0}
                onClick={() => update((list) => list.filter((p) => p.selected))}
              />
              <Action
                label="Remove these"
                disabled={!file || selected.length === 0 || selected.length === pages.length}
                onClick={() => update((list) => list.filter((p) => !p.selected))}
              />
              <Action
                label="Duplicate"
                disabled={!file || selected.length === 0}
                onClick={() =>
                  update((list) =>
                    list.flatMap((p, i) =>
                      p.selected
                        ? [p, { ...p, key: `${p.key}-copy-${i}-${Date.now()}`, selected: false }]
                        : [p],
                    ),
                  )
                }
              />
              <Action
                label="Reverse"
                disabled={!file}
                onClick={() => update((list) => [...list].reverse())}
              />
            </div>
            {file && (
              <button
                type="button"
                disabled={!changed}
                onClick={() =>
                  setPages(
                    info.map((p) => ({
                      key: `p${p.number}`,
                      source: p.number,
                      rotate: 0,
                      selected: false,
                    })),
                  )
                }
                style={{
                  fontSize: 12,
                  color: changed ? "var(--accent)" : "var(--text-subtle)",
                  alignSelf: "flex-start",
                  fontWeight: 600,
                }}
              >
                Start over
              </button>
            )}
          </div>

          <div className="divider" />

          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Get files out
            </span>
            <div className="row" style={{ gap: 10 }}>
              <label htmlFor="every" style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
                Pages per file
              </label>
              <span className="pressed field" style={{ flex: 1, padding: "0 8px", minHeight: 42 }}>
                <input
                  id="every"
                  type="number"
                  min={1}
                  value={splitEvery}
                  onChange={(e) => setSplitEvery(Math.max(1, Number(e.target.value)))}
                />
              </span>
            </div>
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 46 }}
              disabled={!file}
              onClick={() =>
                runSplit(
                  splitEvery <= 1 ? { kind: "every_page" } : { kind: "every_n", value: splitEvery },
                )
              }
            >
              {splitEvery <= 1 ? "Split every page" : `Split every ${splitEvery} pages`}
            </button>
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 46 }}
              disabled={!file}
              onClick={async () => {
                if (!file) return;
                setError(null);
                try {
                  setExported(await pdfToImages(file, null, 88));
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
                }
              }}
            >
              Export pages as JPG
            </button>

            {exported && (
              <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  {exported.filter((p) => p.outputPath).length} of {exported.length} pages exported
                </span>
                {exported.filter((p) => p.reason).length > 0 && (
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                    {exported.find((p) => p.reason)?.reason}
                  </span>
                )}
                {exported
                  .filter((p) => p.outputPath)
                  .slice(0, 3)
                  .map((p) => (
                    <button
                      key={p.page}
                      type="button"
                      onClick={() => revealOutput(p.outputPath!)}
                      className="truncate"
                      style={{
                        fontSize: 11.5,
                        color: "var(--text-subtle)",
                        textAlign: "left",
                        width: "100%",
                      }}
                    >
                      page {p.page} · {p.width}×{p.height} · {formatSize(p.size)}
                    </button>
                  ))}
              </div>
            )}

            {parts && (
              <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  {parts.length} file{parts.length === 1 ? "" : "s"} written
                </span>
                {parts.slice(0, 4).map((p) => (
                  <button
                    key={p.outputPath}
                    type="button"
                    onClick={() => revealOutput(p.outputPath)}
                    className="truncate"
                    style={{
                      fontSize: 11.5,
                      color: "var(--text-subtle)",
                      textAlign: "left",
                      width: "100%",
                    }}
                    title={p.outputPath}
                  >
                    pages {p.pages.join(", ")} · {formatSize(p.size)}
                  </button>
                ))}
                {parts.length > 4 && (
                  <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                    and {parts.length - 4} more
                  </span>
                )}
              </div>
            )}
          </div>

          <span className="grow" />

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

          {saved && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {saved.pagesAfter} page{saved.pagesAfter === 1 ? "" : "s"} ·{" "}
                {formatSize(saved.size)}
              </span>
              {saved.note && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {saved.note}
                </span>
              )}
              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 40, marginTop: 4 }}
                onClick={() => revealOutput(saved.outputPath)}
              >
                <Icon path={ICONS.save} size={16} strokeWidth={2} />
                Show in folder
              </button>
            </div>
          )}

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!file || !changed}
            onClick={save}
          >
            Save as a new PDF
          </button>
        </aside>
      </div>
    </div>
  );
}

function Action({
  label,
  disabled,
  onClick,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="raised-sm"
      disabled={disabled}
      onClick={onClick}
      style={{ borderRadius: 12, minHeight: 44, fontSize: 12.5, fontWeight: 600, padding: "0 8px" }}
    >
      {label}
    </button>
  );
}

function PageCard({
  page,
  index,
  info,
  preview,
  onToggle,
  onRotate,
  onRemove,
  onMove,
  onDragStart,
  onDrop,
}: {
  page: PageState;
  index: number;
  info?: PageInfo;
  preview: string | null;
  onToggle: () => void;
  onRotate: (by: number) => void;
  onRemove: () => void;
  onMove: (delta: number) => void;
  onDragStart: () => void;
  onDrop: () => void;
}) {
  const total = ((info?.rotation ?? 0) + page.rotate) % 360;
  const quarter = ((total % 360) + 360) % 360;
  const portrait = (info?.heightPt ?? 1) >= (info?.widthPt ?? 1);
  const swapped = quarter === 90 || quarter === 270;
  const tall = swapped ? !portrait : portrait;

  return (
    <div
      className={page.selected ? "pressed" : "raised"}
      draggable
      onDragStart={onDragStart}
      onDragOver={(e) => e.preventDefault()}
      onDrop={onDrop}
      style={{ borderRadius: 18, padding: 12, display: "flex", flexDirection: "column", gap: 8 }}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-pressed={page.selected}
        aria-label={`Page ${page.source}${page.selected ? ", selected" : ""}`}
        style={{
          borderRadius: 10,
          overflow: "hidden",
          height: 128,
          display: "grid",
          placeItems: "center",
          background: "#cbd3e1",
          outline: page.selected ? "2px solid var(--accent)" : "none",
          outlineOffset: 2,
          position: "relative",
        }}
      >
        {preview ? (
          <img
            src={preview}
            alt=""
            style={{
              width: "100%",
              height: "100%",
              objectFit: "contain",
              transform: `rotate(${quarter}deg)`,
              transition: "transform .18s ease",
            }}
          />
        ) : (
          <span
            style={{
              width: tall ? 64 : 92,
              height: tall ? 92 : 64,
              background: "#e9edf4",
              border: "1px solid #b9c2d1",
              borderRadius: 4,
              display: "grid",
              placeItems: "center",
              fontSize: 11,
              color: "#5b6270",
              transition: "all .18s ease",
            }}
          >
            {info?.label.split(" ")[0] ?? "Page"}
          </span>
        )}
        <span
          style={{
            position: "absolute",
            bottom: 6,
            left: 6,
            fontSize: 11,
            fontWeight: 600,
            background: "rgba(230,233,239,0.92)",
            padding: "2px 7px",
            borderRadius: 7,
          }}
        >
          {page.source}
        </span>
        {quarter !== 0 && (
          <span
            style={{
              position: "absolute",
              bottom: 6,
              right: 6,
              fontSize: 11,
              fontWeight: 600,
              background: "rgba(230,233,239,0.92)",
              padding: "2px 7px",
              borderRadius: 7,
              color: "var(--accent)",
            }}
          >
            {quarter}°
          </span>
        )}
      </button>

      <div className="row" style={{ gap: 4, justifyContent: "space-between" }}>
        <IconAction label={`Move page ${page.source} earlier`} onClick={() => onMove(-1)} path="M15 5l-7 7 7 7" />
        <IconAction label={`Turn page ${page.source} left`} onClick={() => onRotate(-90)} path="M3 12a9 9 0 1 0 3-6.7 M3 3v4h4" />
        <IconAction label={`Turn page ${page.source} right`} onClick={() => onRotate(90)} path="M21 12a9 9 0 1 1-3-6.7 M21 3v6h-6" />
        <IconAction label={`Remove page ${page.source}`} onClick={onRemove} path="M6 6l12 12 M18 6L6 18" />
        <IconAction label={`Move page ${page.source} later`} onClick={() => onMove(1)} path="M9 5l7 7-7 7" />
      </div>
      <span className="sr-only">Position {index + 1}</span>
    </div>
  );
}

function IconAction({
  label,
  onClick,
  path,
}: {
  label: string;
  onClick: () => void;
  path: string;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="raised-sm"
      style={{ width: 30, height: 30, borderRadius: 9, display: "grid", placeItems: "center" }}
    >
      <Icon path={path} size={14} strokeWidth={2} />
    </button>
  );
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
      <h2 className="display" style={{ fontSize: 20 }}>
        {title}
      </h2>
      <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.6 }}>{body}</p>
    </div>
  );
}
