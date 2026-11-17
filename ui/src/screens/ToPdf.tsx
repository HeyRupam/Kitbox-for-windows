import { useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { formatSize, imagesToPdf, isDesktop, isPdf, revealOutput } from "../lib/bridge";
import type { ImagesToPdfOptions, PageOpResult, PdfOrientation, PdfPageSize } from "../lib/types";

/**
 * Photos into one PDF.
 *
 * The thing most forms actually ask for — "upload your documents as a single
 * PDF" — when what you have is four phone photos. The resolution cap is the
 * setting that matters: a 12-megapixel photo of an A4 page is four times the
 * detail any form will ever look at, and it's where all the size goes.
 */
export function ToPdf({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const images = files.filter((f) => !isPdf(f));
  const [pageSize, setPageSize] = useState<PdfPageSize>("a4");
  const [orientation, setOrientation] = useState<PdfOrientation>("auto");
  const [maxDpi, setMaxDpi] = useState<number | null>(200);
  const [result, setResult] = useState<PageOpResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const dragFrom = useRef<number | null>(null);

  const reorder = (from: number, to: number) => {
    if (to < 0 || to >= images.length) return;
    const next = [...images];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    onFiles(next);
    setResult(null);
  };

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const options: ImagesToPdfOptions = {
        pageSize,
        orientation,
        marginPt: pageSize === "fit_image" ? 0 : 18,
        jpegQuality: 82,
        maxDpi,
      };
      setResult(await imagesToPdf(images, "documents.pdf", options));
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
            <span className="section-heading">Convert to PDF</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Images to PDF
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          multiple
          hidden
          onChange={(e) => {
            onFiles([...images, ...(e.target.files ?? [])]);
            setResult(null);
          }}
        />
        <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
          <Icon path={ICONS.plus} size={18} strokeWidth={2} />
          Add images
        </button>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow scroll"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          {images.length === 0 ? (
            <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                {isDesktop() ? "Add the photos or scans" : "This needs the desktop app"}
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.6 }}>
                {isDesktop()
                  ? "One image per page, in the order shown. Drag to change it."
                  : "The browser preview can compress images but not build documents. Open Kitbox itself."}
              </p>
            </div>
          ) : (
            <>
              <span className="section-heading">
                {images.length} image{images.length === 1 ? "" : "s"}, one per page
              </span>
              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "repeat(auto-fill, minmax(140px, 1fr))",
                  gap: 14,
                }}
              >
                {images.map((f, i) => (
                  <div
                    key={`${f.name}-${f.size}-${i}`}
                    className="pressed col"
                    draggable
                    onDragStart={() => (dragFrom.current = i)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      if (dragFrom.current !== null) reorder(dragFrom.current, i);
                      dragFrom.current = null;
                    }}
                    style={{ borderRadius: 16, padding: 10, gap: 8 }}
                  >
                    <Thumb file={f} />
                    <span className="truncate" style={{ fontSize: 12, fontWeight: 600 }}>
                      {i + 1}. {f.name}
                    </span>
                    <div className="row" style={{ gap: 6, justifyContent: "space-between" }}>
                      <span style={{ fontSize: 11, color: "var(--text-subtle)" }}>
                        {formatSize(f.size)}
                      </span>
                      <span className="row" style={{ gap: 4 }}>
                        <Tiny label={`Move ${f.name} earlier`} onClick={() => reorder(i, i - 1)} path="M15 5l-7 7 7 7" />
                        <Tiny label={`Move ${f.name} later`} onClick={() => reorder(i, i + 1)} path="M9 5l7 7-7 7" />
                        <Tiny
                          label={`Remove ${f.name}`}
                          onClick={() => {
                            onFiles(images.filter((_, j) => j !== i));
                            setResult(null);
                          }}
                          path="M6 6l12 12 M18 6L6 18"
                        />
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          <Choice
            label="Page size"
            value={pageSize}
            options={[
              ["a4", "A4"],
              ["letter", "Letter"],
              ["legal", "Legal"],
              ["fit_image", "Fit image"],
            ]}
            onChange={(v) => {
              setPageSize(v as PdfPageSize);
              setResult(null);
            }}
            columns={2}
          />

          {pageSize !== "fit_image" && (
            <Choice
              label="Orientation"
              value={orientation}
              options={[
                ["auto", "Auto"],
                ["portrait", "Portrait"],
                ["landscape", "Landscape"],
              ]}
              onChange={(v) => {
                setOrientation(v as PdfOrientation);
                setResult(null);
              }}
              columns={3}
            />
          )}

          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Resolution cap
            </span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 8 }}>
              {([150, 200, 300, null] as (number | null)[]).map((dpi) => (
                <button
                  key={String(dpi)}
                  type="button"
                  aria-pressed={maxDpi === dpi}
                  className={maxDpi === dpi ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setMaxDpi(dpi);
                    setResult(null);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 42,
                    fontSize: 12,
                    fontWeight: maxDpi === dpi ? 700 : 500,
                    color: maxDpi === dpi ? "var(--accent)" : undefined,
                  }}
                >
                  {dpi ?? "Full"}
                </button>
              ))}
            </div>
            <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              A phone photo of a page is far past what any form reads. 200 DPI is plenty and this
              is where most of the size goes. Images already below the cap are embedded untouched.
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
            disabled={images.length === 0 || busy}
            onClick={run}
          >
            {busy ? "Building…" : "Make the PDF"}
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
  columns,
}: {
  label: string;
  value: string;
  options: [string, string][];
  onChange: (value: string) => void;
  columns: number;
}) {
  return (
    <div className="col" style={{ gap: 10 }}>
      <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
      <div style={{ display: "grid", gridTemplateColumns: `repeat(${columns}, 1fr)`, gap: 8 }}>
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

function Thumb({ file }: { file: File }) {
  const [url, setUrl] = useState<string>();
  if (url === undefined) {
    const next = URL.createObjectURL(file);
    setUrl(next);
  }
  return (
    <span
      style={{
        height: 88,
        borderRadius: 10,
        overflow: "hidden",
        background: "#cbd3e1",
        display: "grid",
        placeItems: "center",
      }}
    >
      {url && (
        <img src={url} alt="" style={{ width: "100%", height: "100%", objectFit: "cover" }} />
      )}
    </span>
  );
}

function Tiny({ label, onClick, path }: { label: string; onClick: () => void; path: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="raised-sm"
      style={{ width: 26, height: 26, borderRadius: 8, display: "grid", placeItems: "center" }}
    >
      <Icon path={path} size={12} strokeWidth={2} />
    </button>
  );
}
