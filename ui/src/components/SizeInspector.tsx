import { formatSize } from "../lib/bridge";
import type { PdfReport } from "../lib/types";

/**
 * Where a PDF's bytes went.
 *
 * This is the answer to the question every other compressor leaves hanging:
 * you press the button, it barely shrinks, and you never learn that the file is
 * 90% scanned images at 288 DPI. Shown before compressing, and again after, so
 * the change is visible rather than asserted.
 */

const SEGMENTS = [
  { key: "imageBytes", label: "Images", color: "var(--accent)" },
  { key: "fontBytes", label: "Fonts", color: "#0f766e" },
  { key: "contentBytes", label: "Page content", color: "#1d4ed8" },
  { key: "metadataBytes", label: "Metadata", color: "#7c3aed" },
  { key: "otherBytes", label: "Structure", color: "#8e9bb3" },
] as const;

export function SizeInspector({ report, after }: { report: PdfReport; after?: PdfReport }) {
  return (
    <div className="col" style={{ gap: 16 }}>
      <div className="col" style={{ gap: 4 }}>
        <span className="display" style={{ fontSize: 17 }}>
          {report.headline}
        </span>
        <span style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
          {report.pages} page{report.pages === 1 ? "" : "s"} · {describeKind(report.kind)}
        </span>
      </div>

      <Bar report={report} caption="Before" />
      {after && <Bar report={after} caption="After" />}

      <div className="row" style={{ gap: 14, flexWrap: "wrap" }}>
        {SEGMENTS.filter((s) => report[s.key] > 0).map((s) => (
          <span
            key={s.key}
            className="row"
            style={{ gap: 6, fontSize: 12, color: "var(--text-subtle)" }}
          >
            <span
              style={{ width: 9, height: 9, borderRadius: 3, background: s.color, flex: "0 0 auto" }}
            />
            {s.label} {formatSize(report[s.key])}
          </span>
        ))}
      </div>

      {report.images.length > 0 && (
        <div className="col" style={{ gap: 8 }}>
          <span className="section-heading" style={{ fontSize: 11 }}>
            Largest images
          </span>
          {report.images.slice(0, 5).map((img, i) => (
            <div
              key={i}
              className="pressed row"
              style={{
                borderRadius: 12,
                padding: "8px 12px",
                gap: 12,
                justifyContent: "space-between",
                fontSize: 12,
              }}
            >
              <span style={{ fontWeight: 600, minWidth: 62 }}>{formatSize(img.bytes)}</span>
              <span className="truncate" style={{ color: "var(--text-subtle)", flex: 1 }}>
                {img.width}×{img.height}
                {img.dpi ? ` · ${Math.round(img.dpi)} DPI` : ""} · {img.colorSpace}
              </span>
              {img.skipReason ? (
                <span style={{ color: "var(--text-subtle)", fontStyle: "italic" }} title={img.skipReason}>
                  left alone
                </span>
              ) : (
                <span style={{ color: "var(--accent)", fontWeight: 600 }}>can shrink</span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Bar({ report, caption }: { report: PdfReport; caption: string }) {
  const total = Math.max(1, report.totalBytes);
  return (
    <div className="col" style={{ gap: 6 }}>
      <div
        className="row"
        style={{ justifyContent: "space-between", fontSize: 12, color: "var(--text-subtle)" }}
      >
        <span>{caption}</span>
        <span style={{ fontWeight: 600, color: "var(--text-muted)" }}>
          {formatSize(report.totalBytes)}
        </span>
      </div>
      <div
        className="row"
        style={{ height: 16, borderRadius: 8, overflow: "hidden", background: "var(--shadow-dark)" }}
        role="img"
        aria-label={SEGMENTS.filter((s) => report[s.key] > 0)
          .map((s) => `${s.label} ${formatSize(report[s.key])}`)
          .join(", ")}
      >
        {SEGMENTS.map((s) => {
          const pct = (report[s.key] / total) * 100;
          if (pct <= 0) return null;
          return <span key={s.key} style={{ width: `${pct}%`, background: s.color }} />;
        })}
      </div>
    </div>
  );
}

function describeKind(kind: PdfReport["kind"]): string {
  if (kind === "scan") return "a scanned document";
  if (kind === "text_only") return "text only — nothing to downsample";
  return "text with images";
}
