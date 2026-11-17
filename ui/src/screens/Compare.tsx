import { useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { comparePdfs, formatSize, isDesktop, isPdf, revealOutput } from "../lib/bridge";
import type { CompareResult } from "../lib/types";

/**
 * Compare two PDFs.
 *
 * The result is a diff, shown with removals and additions coloured, because
 * "17 changes" tells you nothing you can act on. Pages that are scans carry no
 * text to compare, so they are compared as pictures and labelled as such
 * rather than counted as unchanged.
 */
export function Compare({ onBack }: { onBack: () => void }) {
  const [before, setBefore] = useState<File | null>(null);
  const [after, setAfter] = useState<File | null>(null);
  const [result, setResult] = useState<CompareResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async () => {
    if (!before || !after) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await comparePdfs(before, after, "comparison.md"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
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
          <span className="section-heading">Security</span>
          <h1 className="display" style={{ fontSize: 26 }}>
            Compare PDF
          </h1>
        </div>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          <div
            className="pressed grow scroll"
            style={{ borderRadius: 18, padding: 18, minHeight: 0 }}
          >
            {!result ? (
              <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
                <h2 className="display" style={{ fontSize: 19 }}>
                  {isDesktop() ? "Pick the two documents" : "This needs the desktop app"}
                </h2>
                <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.7, fontSize: 13.5 }}>
                  {isDesktop()
                    ? "Pages are lined up before the text is, so inserting a page doesn't report everything after it as changed."
                    : "The browser preview can compress images but not read documents. Open Kitbox itself."}
                </p>
              </div>
            ) : (
              <Diff result={result} />
            )}
          </div>
        </section>

        <aside
          className="raised col scroll"
          aria-label="Documents to compare"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 16 }}
        >
          <Slot
            label="Original"
            file={before}
            onPick={(f) => {
              setBefore(f);
              setResult(null);
            }}
          />
          <Slot
            label="Changed"
            file={after}
            onPick={(f) => {
              setAfter(f);
              setResult(null);
            }}
          />

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
                {result.identical
                  ? "No text changed"
                  : `+${result.linesAdded} / −${result.linesRemoved}`}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {result.pagesBefore} → {result.pagesAfter} pages
              </span>
              {result.outputPath && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ minHeight: 40 }}
                  onClick={() => revealOutput(result.outputPath!)}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show the report
                </button>
              )}
            </div>
          )}

          <span className="grow" />

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!before || !after || busy}
            onClick={run}
          >
            {busy ? "Comparing…" : "Compare"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Slot({
  label,
  file,
  onPick,
}: {
  label: string;
  file: File | null;
  onPick: (file: File) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  return (
    <div className="col" style={{ gap: 8 }}>
      <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
      <input
        ref={ref}
        type="file"
        accept="application/pdf"
        hidden
        onChange={(e) => {
          const picked = e.target.files?.[0];
          if (picked && isPdf(picked)) onPick(picked);
        }}
      />
      <button
        type="button"
        className={file ? "pressed" : "raised-sm"}
        onClick={() => ref.current?.click()}
        style={{
          borderRadius: 14,
          minHeight: 56,
          padding: "0 14px",
          textAlign: "left",
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
          gap: 2,
        }}
      >
        <span className="truncate" style={{ fontSize: 13, fontWeight: 600, width: "100%" }}>
          {file ? file.name : "Choose a PDF"}
        </span>
        {file && (
          <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
            {formatSize(file.size)}
          </span>
        )}
      </button>
    </div>
  );
}

function Diff({ result }: { result: CompareResult }) {
  return (
    <div className="col" style={{ gap: 16 }}>
      <span className="display" style={{ fontSize: 19 }}>
        {result.identical
          ? "The two documents have the same text on every page."
          : `${result.linesAdded} added, ${result.linesRemoved} removed`}
      </span>

      {result.note && (
        <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          {result.note}
        </span>
      )}

      {(result.pagesAdded.length > 0 || result.pagesRemoved.length > 0) && (
        <div className="row" style={{ gap: 10, flexWrap: "wrap" }}>
          {result.pagesAdded.length > 0 && (
            <span className="pressed pill">Pages added: {result.pagesAdded.join(", ")}</span>
          )}
          {result.pagesRemoved.length > 0 && (
            <span className="pressed pill">Pages removed: {result.pagesRemoved.join(", ")}</span>
          )}
        </div>
      )}

      {result.changes.length > 0 && (
        <div className="col" style={{ gap: 2 }}>
          {result.changes.map((change, index) => {
            const first = index === 0 || result.changes[index - 1].page !== change.page;
            return (
              <div key={index} className="col" style={{ gap: 2 }}>
                {first && (
                  <span
                    className="section-heading"
                    style={{ fontSize: 10.5, marginTop: index === 0 ? 0 : 12 }}
                  >
                    Page {change.page}
                  </span>
                )}
                <span
                  style={{
                    fontFamily: "var(--font-mono, ui-monospace, monospace)",
                    fontSize: 12,
                    lineHeight: 1.65,
                    padding: "3px 10px",
                    borderRadius: 7,
                    background:
                      change.kind === "added" ? "rgba(29,120,60,0.12)" : "rgba(194,65,12,0.12)",
                    color: change.kind === "added" ? "#1d6b3c" : "#9a3a10",
                    wordBreak: "break-word",
                  }}
                >
                  {change.kind === "added" ? "+ " : "− "}
                  {change.text}
                </span>
              </div>
            );
          })}
        </div>
      )}

      {result.visual.length > 0 && (
        <div className="col" style={{ gap: 8 }}>
          <span className="section-heading" style={{ fontSize: 10.5 }}>
            Scanned pages, compared as pictures
          </span>
          {result.visual.map((diff) => {
            const percent = diff.difference * 100;
            return (
              <div
                key={`${diff.pageBefore}-${diff.pageAfter}`}
                className="pressed row"
                style={{ borderRadius: 12, padding: "9px 14px", gap: 12, fontSize: 12.5 }}
              >
                <span style={{ fontWeight: 600, minWidth: 92 }}>
                  page {diff.pageBefore} vs {diff.pageAfter}
                </span>
                <span style={{ color: "var(--text-subtle)" }}>
                  {percent.toFixed(1)}% different —{" "}
                  {percent < 1 ? "looks the same" : percent < 8 ? "slightly different" : "clearly different"}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
