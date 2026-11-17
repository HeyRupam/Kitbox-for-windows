import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, isDesktop, isPdf, repairPdf, revealOutput } from "../lib/bridge";
import type { RepairResult } from "../lib/types";

/**
 * Recover a damaged PDF.
 *
 * It checks the file as soon as you open it, before you press anything, so the
 * diagnosis comes first. Findings and caveats get equal weight: a repair tool
 * that reports success on a file it quietly emptied is worse than one that
 * refuses, because the result gets sent on before anyone notices.
 */
export function Repair({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const file = files.find(isPdf);
  const [check, setCheck] = useState<RepairResult | null>(null);
  const [result, setResult] = useState<RepairResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setCheck(null);
    setResult(null);
    setError(null);
    if (!file) return;
    let live = true;
    repairPdf(file, true)
      .then((r) => live && setCheck(r))
      .catch((e) => live && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [file]);

  const run = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await repairPdf(file, false));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const shown = result ?? check;

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
              Repair PDF
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
          className="raised col grow scroll"
          style={{ borderRadius: "var(--r-panel)", padding: 26, gap: 20, minWidth: 0 }}
        >
          {!file ? (
            <div className="col" style={{ gap: 10, padding: "30px 0" }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                {isDesktop() ? "Open a PDF that won't open properly" : "This needs the desktop app"}
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 540, lineHeight: 1.7 }}>
                {isDesktop()
                  ? "Kitbox reads whatever survived — a broken index, a file cut off mid-download, pages that lost the list holding them together — and writes a clean copy. It tells you what it could not get back."
                  : "The browser preview can compress images but not rebuild documents. Open Kitbox itself."}
              </p>
            </div>
          ) : !shown ? (
            <span className="display" style={{ fontSize: 19 }}>
              Checking the file…
            </span>
          ) : (
            <>
              <div className="col" style={{ gap: 8 }}>
                <span className="section-heading">
                  {result ? "Repaired" : "What's wrong"}
                </span>
                <span className="display" style={{ fontSize: 21, lineHeight: 1.35 }}>
                  {shown.headline}
                </span>
                <span style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
                  {file.name} · {formatSize(shown.originalSize)}
                  {result?.size ? ` → ${formatSize(result.size)}` : ""}
                </span>
              </div>

              <Severity severity={shown.severity} />

              {shown.findings.length > 0 && (
                <List title="Found" items={shown.findings} />
              )}
              {shown.caveats.length > 0 && (
                <List title="Can't be recovered" items={shown.caveats} accent />
              )}

              {result?.outputPath && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ alignSelf: "flex-start", minHeight: 42 }}
                  onClick={() => revealOutput(result.outputPath!)}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show in folder
                </button>
              )}
            </>
          )}
        </section>

        <aside
          className="raised col"
          style={{ flex: "0 0 280px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
            What repairing does
          </span>
          <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.65 }}>
            It rebuilds the index a reader uses to find things, puts loose pages back into a
            document, drops objects that lead nowhere, and writes the result out cleanly.
          </span>
          <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.65 }}>
            It cannot invent content that is genuinely gone. If a file was cut off, whatever came
            after the cut stays missing — and the report says so rather than quietly leaving you
            with blank pages.
          </span>

          {error && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
                <Icon path={ICONS.warning} size={16} strokeWidth={2} />
                Couldn't be opened
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.55 }}>
                {error}
              </span>
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
            {busy ? "Repairing…" : result ? "Repair again" : "Repair and save"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Severity({ severity }: { severity: RepairResult["severity"] }) {
  const [label, detail] =
    severity === "healthy"
      ? ["Healthy", "Nothing was broken; the file was simply repacked."]
      : severity === "repaired"
        ? ["Repaired", "Something had to be worked around, but the document was readable."]
        : ["Salvaged", "It only opened by scanning the file for whatever objects remained."];
  return (
    <div className="pressed row" style={{ borderRadius: 16, padding: "12px 16px", gap: 14 }}>
      <span
        style={{
          fontSize: 12,
          fontWeight: 700,
          letterSpacing: "0.06em",
          textTransform: "uppercase",
          color: severity === "healthy" ? "var(--text-muted)" : "var(--accent)",
          flex: "0 0 auto",
        }}
      >
        {label}
      </span>
      <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>{detail}</span>
    </div>
  );
}

function List({ title, items, accent }: { title: string; items: string[]; accent?: boolean }) {
  return (
    <div className="col" style={{ gap: 8 }}>
      <span className="section-heading" style={{ fontSize: 11, color: accent ? "var(--accent)" : undefined }}>
        {title}
      </span>
      {items.map((item) => (
        <div
          key={item}
          className="pressed"
          style={{
            borderRadius: 12,
            padding: "10px 14px",
            fontSize: 12.5,
            color: "var(--text-subtle)",
            lineHeight: 1.6,
          }}
        >
          {item}
        </div>
      ))}
    </div>
  );
}
