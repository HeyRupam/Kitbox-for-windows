import { useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, isDesktop, isPdf, mergePdfs, revealOutput } from "../lib/bridge";
import type { PageOpResult } from "../lib/types";

/** Combine PDFs end to end. The order in this list is the order in the result. */
export function Merge({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdfs = files.filter(isPdf);
  const [result, setResult] = useState<PageOpResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const dragFrom = useRef<number | null>(null);

  const reorder = (from: number, to: number) => {
    if (to < 0 || to >= pdfs.length) return;
    const next = [...pdfs];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    onFiles(next);
    setResult(null);
  };

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await mergePdfs(pdfs, "merged.pdf"));
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
            <span className="section-heading">Organize</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Merge PDF
            </h1>
          </div>
        </div>
        <div className="row" style={{ gap: 12 }}>
          <input
            ref={fileRef}
            type="file"
            accept="application/pdf"
            multiple
            hidden
            onChange={(e) => {
              onFiles([...pdfs, ...(e.target.files ?? [])]);
              setResult(null);
            }}
          />
          <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
            <Icon path={ICONS.plus} size={18} strokeWidth={2} />
            Add PDFs
          </button>
        </div>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow scroll"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 12, minWidth: 0 }}
        >
          {pdfs.length === 0 ? (
            <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                {isDesktop() ? "Add two or more PDFs" : "Merging needs the desktop app"}
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.6 }}>
                {isDesktop()
                  ? "They are joined in the order shown here. Drag to change it."
                  : "The browser preview can work on images but not documents. Open Kitbox itself to merge."}
              </p>
            </div>
          ) : (
            <>
              <span className="section-heading">
                {pdfs.length} file{pdfs.length === 1 ? "" : "s"}, joined top to bottom
              </span>
              {pdfs.map((f, i) => (
                <div
                  key={`${f.name}-${f.size}-${i}`}
                  className="pressed row"
                  draggable
                  onDragStart={() => (dragFrom.current = i)}
                  onDragOver={(e) => e.preventDefault()}
                  onDrop={() => {
                    if (dragFrom.current !== null) reorder(dragFrom.current, i);
                    dragFrom.current = null;
                  }}
                  style={{ borderRadius: 16, padding: "12px 14px", gap: 14 }}
                >
                  <span
                    className="raised-sm"
                    style={{
                      width: 34,
                      height: 34,
                      borderRadius: 10,
                      display: "grid",
                      placeItems: "center",
                      fontSize: 13,
                      fontWeight: 600,
                      flex: "0 0 auto",
                    }}
                  >
                    {i + 1}
                  </span>
                  <span className="col grow" style={{ minWidth: 0, gap: 2 }}>
                    <span className="truncate" style={{ fontSize: 14, fontWeight: 600 }}>
                      {f.name}
                    </span>
                    <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                      {formatSize(f.size)}
                    </span>
                  </span>
                  <span className="row" style={{ gap: 6 }}>
                    <Small label={`Move ${f.name} up`} onClick={() => reorder(i, i - 1)} path="M12 19V5 M5 12l7-7 7 7" />
                    <Small label={`Move ${f.name} down`} onClick={() => reorder(i, i + 1)} path="M12 5v14 M5 12l7 7 7-7" />
                    <Small
                      label={`Remove ${f.name}`}
                      onClick={() => {
                        onFiles(pdfs.filter((_, j) => j !== i));
                        setResult(null);
                      }}
                      path="M6 6l12 12 M18 6L6 18"
                    />
                  </span>
                </div>
              ))}
            </>
          )}
        </section>

        <aside
          className="raised col"
          style={{ flex: "0 0 280px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>Result</span>
          <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
            Every page is kept at its own size, so mixing A4 and Letter is fine. Bookmarks and
            tagging from the source files are dropped, because they point at pages that have moved.
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

          {result && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 20 }}>
                {result.pagesAfter} pages · {formatSize(result.size)}
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

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={pdfs.length < 2 || busy}
            onClick={run}
          >
            {busy ? "Joining…" : `Merge ${pdfs.length || ""} PDFs`.trim()}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Small({ label, onClick, path }: { label: string; onClick: () => void; path: string }) {
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
