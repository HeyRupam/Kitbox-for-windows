import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  bytesFromDataUrl,
  formatSize,
  isDesktop,
  isPdf,
  pdfPages,
  pdfPreviews,
  revealOutput,
  saveSignature,
  signPdf,
  signaturePreview,
} from "../lib/bridge";
import type {
  InkColour,
  PageInfo,
  SignOptions,
  SignResult,
  SignaturePreview,
  SignatureSpot,
} from "../lib/types";

/**
 * Put a signature on a PDF.
 *
 * Two halves. First the signature gets cut out of a photograph — done once,
 * kept, and never done again. Then it gets placed, by clicking the page where
 * it should go, because "bottom right, 36pt margin" is not how anyone thinks
 * about where their name goes on a form.
 *
 * The screen says plainly, every time, that this is a drawn signature and not
 * a digital certificate. People file documents believing a pasted signature is
 * legally binding and tamper-evident. It isn't, in this tool or any other, and
 * the only difference is whether the tool says so.
 */
export function Sign({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdf = files.find(isPdf) ?? null;

  const [photo, setPhoto] = useState<File | null>(null);
  const [ink, setInk] = useState<InkColour>("keep");
  const [strength, setStrength] = useState(0.5);
  const [cut, setCut] = useState<SignaturePreview | null>(null);

  const [pages, setPages] = useState<PageInfo[]>([]);
  const [previews, setPreviews] = useState<(string | null)[]>([]);
  const [page, setPage] = useState(1);
  const [spot, setSpot] = useState({ x: 0.3, y: 0.85 });
  const [width, setWidth] = useState(0.28);
  const [everyPage, setEveryPage] = useState(false);

  const [caption, setCaption] = useState(false);
  const [captionText, setCaptionText] = useState("Signed {date}");
  const [date, setDate] = useState(() => new Date().toLocaleDateString());

  const [result, setResult] = useState<SignResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  const photoRef = useRef<HTMLInputElement>(null);
  const pdfRef = useRef<HTMLInputElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);

  // Cut the signature out again whenever a setting changes. Cheap, and the
  // settings are only meaningful if you can see what they do.
  useEffect(() => {
    if (!photo || !isDesktop()) {
      setCut(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      signaturePreview(photo, { ink, strength, trim: true })
        .then((p) => !cancelled && setCut(p))
        .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)));
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [photo, ink, strength]);

  useEffect(() => {
    if (!pdf || !isDesktop()) return;
    let cancelled = false;
    pdfPages(pdf)
      .then((p) => {
        if (cancelled) return;
        setPages(p);
        setPage(p.length);
      })
      .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)));
    pdfPreviews(pdf, 900)
      .then((p) => !cancelled && setPreviews(p))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [pdf]);

  const current = pages[page - 1];
  const ready = pdf && cut?.foundInk;

  const place = (event: React.MouseEvent<HTMLDivElement>) => {
    const box = sheetRef.current?.getBoundingClientRect();
    if (!box) return;
    setSpot({
      x: Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)),
      y: Math.min(1, Math.max(0, (event.clientY - box.top) / box.height)),
    });
    setResult(null);
  };

  const run = async () => {
    if (!pdf || !cut) return;
    setBusy(true);
    setError(null);
    try {
      const targets = everyPage ? pages.map((p) => p.number) : [page];
      const spots: SignatureSpot[] = targets.map((n) => ({ page: n, x: spot.x, y: spot.y, width }));
      const options: SignOptions = {
        spots,
        caption: caption
          ? { text: captionText, date, size: 9, colour: [45, 45, 54] }
          : null,
      };
      setResult(await signPdf(pdf, bytesFromDataUrl(cut.image), options));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const keep = async () => {
    if (!photo) return;
    setError(null);
    try {
      const out = await saveSignature(photo, "signature.png", { ink, strength, trim: true });
      setSaved(out.outputPath);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  // The page as displayed, so a quarter-turned page is drawn the way it reads.
  const quarter = current ? Math.abs(current.rotation % 180) === 90 : false;
  const sheetW = current ? (quarter ? current.heightPt : current.widthPt) : 595;
  const sheetH = current ? (quarter ? current.widthPt : current.heightPt) : 842;

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
              Sign PDF
            </h1>
          </div>
        </div>
        <input
          ref={pdfRef}
          type="file"
          accept="application/pdf"
          hidden
          onChange={(e) => {
            const picked = e.target.files?.[0];
            if (picked) {
              onFiles([picked, ...files.filter((f) => !isPdf(f))]);
              setResult(null);
            }
          }}
        />
        <button type="button" className="raised-sm btn" onClick={() => pdfRef.current?.click()}>
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
                {isDesktop() ? "Choose the document to sign" : "This needs the desktop app"}
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 540, lineHeight: 1.65 }}>
                {isDesktop()
                  ? "Then photograph your signature once. It is cut out of the paper and kept, so the next form takes one click."
                  : "The browser preview can compress images but not sign documents. Open Kitbox itself."}
              </p>
            </div>
          ) : (
            <>
              <div className="row" style={{ gap: 12, alignItems: "center", flexWrap: "wrap" }}>
                <span className="section-heading">
                  {ready ? "Click the page to place it" : "Page"} {page} of {pages.length || "…"}
                </span>
                {current && (
                  <span className="pressed pill" style={{ fontSize: 11.5 }}>
                    {current.label}
                  </span>
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
                  onClick={ready ? place : undefined}
                  role={ready ? "button" : undefined}
                  aria-label={ready ? "Place the signature" : undefined}
                  style={{
                    position: "relative",
                    aspectRatio: `${sheetW} / ${sheetH}`,
                    maxHeight: "100%",
                    maxWidth: "100%",
                    height: "100%",
                    background: "#fff",
                    boxShadow: "0 2px 14px rgba(40,46,60,0.22)",
                    borderRadius: 3,
                    cursor: ready ? "crosshair" : "default",
                    overflow: "hidden",
                  }}
                >
                  {previews[page - 1] ? (
                    <img
                      src={previews[page - 1] as string}
                      alt=""
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
                      Place the signature by eye — the proportions are right.
                    </span>
                  )}

                  {cut?.foundInk && (
                    <img
                      src={cut.image}
                      alt="Where the signature will go"
                      style={{
                        position: "absolute",
                        left: `${spot.x * 100}%`,
                        top: `${spot.y * 100}%`,
                        width: `${width * 100}%`,
                        transform: "translate(-50%, -50%)",
                        pointerEvents: "none",
                      }}
                    />
                  )}
                </div>
              </div>

              {pages.length > 1 && (
                <div className="row scroll-x" style={{ gap: 8 }}>
                  {pages.map((p) => (
                    <button
                      key={p.number}
                      type="button"
                      aria-pressed={p.number === page}
                      className={p.number === page ? "pressed" : "raised-sm"}
                      onClick={() => {
                        setPage(p.number);
                        setResult(null);
                      }}
                      style={{
                        flex: "0 0 auto",
                        minWidth: 40,
                        height: 36,
                        borderRadius: 11,
                        fontSize: 12.5,
                        fontWeight: p.number === page ? 700 : 500,
                        color: p.number === page ? "var(--accent)" : undefined,
                      }}
                    >
                      {p.number}
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 320px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <div className="col" style={{ gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Your signature
            </span>
            <input
              ref={photoRef}
              type="file"
              accept="image/*"
              hidden
              onChange={(e) => {
                const picked = e.target.files?.[0];
                if (picked) {
                  setPhoto(picked);
                  setSaved(null);
                  setResult(null);
                }
              }}
            />
            {cut?.foundInk ? (
              <div
                className="pressed"
                style={{
                  borderRadius: 14,
                  padding: 10,
                  display: "grid",
                  placeItems: "center",
                  minHeight: 76,
                  // A chequerboard, so transparency is visible rather than
                  // assumed — a signature on a white tile looks identical to
                  // one that would cover the form's line.
                  backgroundImage:
                    "linear-gradient(45deg,#d6dce6 25%,transparent 25%,transparent 75%,#d6dce6 75%)," +
                    "linear-gradient(45deg,#d6dce6 25%,transparent 25%,transparent 75%,#d6dce6 75%)",
                  backgroundSize: "14px 14px",
                  backgroundPosition: "0 0, 7px 7px",
                }}
              >
                <img
                  src={cut.image}
                  alt="Your signature, cut out"
                  style={{ maxWidth: "100%", maxHeight: 90 }}
                />
              </div>
            ) : (
              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 56 }}
                onClick={() => photoRef.current?.click()}
              >
                <Icon path={ICONS.plus} size={16} strokeWidth={2} />
                Photograph of your signature
              </button>
            )}

            {cut && !cut.foundInk && (
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                {cut.note}
              </span>
            )}

            {photo && (
              <>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 8 }}>
                  {(
                    [
                      ["keep", "As shot"],
                      ["black", "Black"],
                      ["blue", "Blue"],
                    ] as [InkColour, string][]
                  ).map(([id, text]) => (
                    <button
                      key={id}
                      type="button"
                      aria-pressed={ink === id}
                      className={ink === id ? "pressed" : "raised-sm"}
                      onClick={() => setInk(id)}
                      style={{
                        borderRadius: 11,
                        minHeight: 38,
                        fontSize: 11.5,
                        fontWeight: ink === id ? 700 : 500,
                        color: ink === id ? "var(--accent)" : undefined,
                      }}
                    >
                      {text}
                    </button>
                  ))}
                </div>
                <Slider
                  label="Cut out"
                  value={strength}
                  onChange={setStrength}
                  hint="Higher removes more paper, and more of a faint stroke with it."
                />
                <div className="row" style={{ gap: 8 }}>
                  <button
                    type="button"
                    className="raised-sm btn"
                    style={{ minHeight: 38, fontSize: 12, flex: 1 }}
                    onClick={() => photoRef.current?.click()}
                  >
                    Different photo
                  </button>
                  <button
                    type="button"
                    className="raised-sm btn"
                    style={{ minHeight: 38, fontSize: 12, flex: 1 }}
                    disabled={!cut?.foundInk}
                    onClick={keep}
                  >
                    {saved ? "Kept ✓" : "Keep it"}
                  </button>
                </div>
              </>
            )}
          </div>

          {ready && (
            <>
              <Slider
                label="Size"
                value={width}
                min={0.05}
                max={0.7}
                onChange={(v) => {
                  setWidth(v);
                  setResult(null);
                }}
                hint={`About ${Math.round(width * sheetW)} points wide on this page.`}
              />

              <label className="row" style={{ gap: 10, alignItems: "center", cursor: "pointer" }}>
                <input
                  type="checkbox"
                  checked={everyPage}
                  onChange={(e) => {
                    setEveryPage(e.target.checked);
                    setResult(null);
                  }}
                />
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  Same spot on all {pages.length} pages
                </span>
              </label>

              <div className="col" style={{ gap: 8 }}>
                <label className="row" style={{ gap: 10, alignItems: "center", cursor: "pointer" }}>
                  <input
                    type="checkbox"
                    checked={caption}
                    onChange={(e) => {
                      setCaption(e.target.checked);
                      setResult(null);
                    }}
                  />
                  <span style={{ fontSize: 13, fontWeight: 600 }}>A line underneath</span>
                </label>
                {caption && (
                  <>
                    <input
                      type="text"
                      value={captionText}
                      aria-label="What the line says"
                      onChange={(e) => setCaptionText(e.target.value)}
                      className="pressed"
                      style={{
                        borderRadius: 11,
                        minHeight: 40,
                        padding: "0 12px",
                        fontSize: 12.5,
                        border: "none",
                        color: "var(--text)",
                      }}
                    />
                    <input
                      type="text"
                      value={date}
                      aria-label="The date"
                      onChange={(e) => setDate(e.target.value)}
                      className="pressed"
                      style={{
                        borderRadius: 11,
                        minHeight: 40,
                        padding: "0 12px",
                        fontSize: 12.5,
                        border: "none",
                        color: "var(--text)",
                      }}
                    />
                    <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                      {"{date}"} is replaced by whatever you put in the second box.
                    </span>
                  </>
                )}
              </div>
            </>
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
                Signed on page{result.pagesSigned.length === 1 ? "" : "s"}{" "}
                {result.pagesSigned.join(", ")}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
                {formatSize(result.size)}
              </span>
              {result.warnings.map((w) => (
                <span key={w} style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {w}
                </span>
              ))}
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

          {/* Always shown, before the button, not after the fact. */}
          <div
            className="pressed col"
            style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}
          >
            <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
              <Icon path={ICONS.warning} size={14} strokeWidth={2} />
              Not a digital certificate
            </span>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
              This draws your signature on the page. It doesn't prove who signed or make the file
              tamper-evident. Forms that take a scanned signature take this; anything asking for a
              DSC token does not.
            </span>
          </div>

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!ready || busy}
            onClick={run}
          >
            {busy ? "Signing…" : "Sign the document"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Slider({
  label,
  value,
  onChange,
  hint,
  min = 0,
  max = 1,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  hint: string;
  min?: number;
  max?: number;
}) {
  return (
    <div className="col" style={{ gap: 7 }}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
          {Math.round(((value - min) / (max - min)) * 100)}%
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        aria-label={label}
        value={Math.round(((value - min) / (max - min)) * 100)}
        onChange={(e) => onChange(min + (Number(e.target.value) / 100) * (max - min))}
      />
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>{hint}</span>
    </div>
  );
}
