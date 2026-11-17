import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  fetchModels,
  formatSize,
  isDesktop,
  isPdf,
  modelsReady,
  ocrPdf,
  revealOutput,
} from "../lib/bridge";
import type { ModelStatus, OcrResult } from "../lib/types";

/**
 * Read the text in a scan.
 *
 * Two things this screen is careful about.
 *
 * The models are a download, and Kitbox's whole promise is that it doesn't
 * need the internet. So the download is never silent: the size, the source and
 * the destination are on screen before anything is fetched, and it happens
 * once.
 *
 * And the output is a guess. The page itself is never altered — the words go
 * into a hidden layer behind the scan — which is said plainly, because the
 * difference between "we rewrote your document" and "we added an index you
 * can search" is the difference between a dangerous tool and a useful one.
 */
export function Ocr({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdf = files.find(isPdf) ?? null;

  const [models, setModels] = useState<ModelStatus | null>(null);
  const [fetching, setFetching] = useState(false);
  const [allPages, setAllPages] = useState(false);
  const [result, setResult] = useState<OcrResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    modelsReady("ocr")
      .then(setModels)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const download = async () => {
    setFetching(true);
    setError(null);
    try {
      setModels(await fetchModels("ocr"));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setFetching(false);
    }
  };

  const run = async (searchable: boolean) => {
    if (!pdf) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await ocrPdf(pdf, { pages: null, skipPagesWithText: !allPages, maxPx: 2000 }, searchable),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = models?.ready === true;

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
              Read a scan
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
          className="raised col grow scroll"
          style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 16, minWidth: 0 }}
        >
          {!isDesktop() ? (
            <Blurb
              title="This needs the desktop app"
              body="The browser preview can compress images but not read documents. Open Kitbox itself."
            />
          ) : !ready ? (
            <div className="col" style={{ gap: 14, maxWidth: 620 }}>
              <h2 className="display" style={{ fontSize: 21 }}>
                Reading text needs a one-off download
              </h2>
              <p style={{ color: "var(--text-subtle)", lineHeight: 1.7, fontSize: 13.5 }}>
                Everything else in Kitbox works with no internet at all. Recognising text is the
                one exception: it needs two model files, about{" "}
                <strong>{models ? formatSize(models.bytes) : "12 MB"}</strong> together. They're
                fetched once, kept on this machine, and never needed again — after that OCR works
                offline like everything else.
              </p>
              {models && (
                <div className="pressed col" style={{ borderRadius: 16, padding: 16, gap: 10 }}>
                  <span className="section-heading" style={{ fontSize: 10.5 }}>
                    What gets downloaded
                  </span>
                  {models.sources.map((url) => (
                    <span
                      key={url}
                      style={{
                        fontSize: 11.5,
                        color: "var(--text-subtle)",
                        wordBreak: "break-all",
                        fontFamily: "var(--font-mono, ui-monospace, monospace)",
                      }}
                    >
                      {url}
                    </span>
                  ))}
                  <span className="section-heading" style={{ fontSize: 10.5, marginTop: 4 }}>
                    Where it goes
                  </span>
                  <span
                    style={{
                      fontSize: 11.5,
                      color: "var(--text-subtle)",
                      wordBreak: "break-all",
                      fontFamily: "var(--font-mono, ui-monospace, monospace)",
                    }}
                  >
                    {models.folder}
                  </span>
                </div>
              )}
              <button
                type="button"
                className="btn btn-primary"
                style={{ minHeight: 52, borderRadius: 18, fontSize: 14.5, fontWeight: 700, alignSelf: "flex-start", padding: "0 26px" }}
                disabled={fetching}
                onClick={download}
              >
                {fetching ? "Downloading…" : "Download them"}
              </button>
            </div>
          ) : !pdf ? (
            <Blurb
              title="Choose a scan"
              body="A scanned page is a photograph, so searching, comparing and redacting all come up empty on it. Reading it puts the words back — invisibly, behind the picture, so the page still looks exactly the same."
            />
          ) : result && result.pages.length > 0 ? (
            <div className="col" style={{ gap: 16 }}>
              <span className="section-heading">What it read</span>
              {result.pages.map((page) => (
                <div key={page.page} className="col" style={{ gap: 6 }}>
                  <span className="section-heading" style={{ fontSize: 10.5 }}>
                    Page {page.page}
                  </span>
                  <p
                    className="pressed"
                    style={{
                      borderRadius: 14,
                      padding: 14,
                      fontSize: 12.5,
                      lineHeight: 1.65,
                      whiteSpace: "pre-wrap",
                      color: "var(--text-muted)",
                    }}
                  >
                    {page.text}
                  </p>
                </div>
              ))}
            </div>
          ) : (
            <Blurb
              title={result ? "Nothing was read" : "Ready"}
              body={
                result
                  ? "No page here was a scan with readable words on it."
                  : "Make a searchable copy, or just look at the text first to see how well it reads."
              }
            />
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          {ready && (
            <label className="row" style={{ gap: 10, alignItems: "flex-start", cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={allPages}
                onChange={(e) => {
                  setAllPages(e.target.checked);
                  setResult(null);
                }}
                style={{ marginTop: 3 }}
              />
              <span className="col" style={{ gap: 3 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Read every page</span>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  Normally pages that already have text are left alone — they don't need reading
                  and a second layer would only confuse a search.
                </span>
              </span>
            </label>
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
                {result.words} word{result.words === 1 ? "" : "s"}
              </span>
              <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                {result.pagesRead.length > 0
                  ? `Read on page${result.pagesRead.length === 1 ? "" : "s"} ${result.pagesRead.join(", ")}.`
                  : "No page could be read."}
                {result.pagesWithText.length > 0
                  ? ` Page${result.pagesWithText.length === 1 ? "" : "s"} ${result.pagesWithText.join(", ")} already had text.`
                  : ""}
              </span>
              {result.warnings.map((w) => (
                <span key={w} style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {w}
                </span>
              ))}
              {result.outputPath && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ minHeight: 40 }}
                  onClick={() => revealOutput(result.outputPath as string)}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show in folder
                </button>
              )}
            </div>
          )}

          <span className="grow" />

          {ready && (
            <>
              <div className="pressed col" style={{ borderRadius: 14, padding: "12px 14px", gap: 4 }}>
                <span className="row" style={{ gap: 7, fontSize: 12, fontWeight: 700 }}>
                  <Icon path={ICONS.warning} size={14} strokeWidth={2} />
                  It's a guess
                </span>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  Expect mistakes in numbers, names and anything handwritten. The scan itself is
                  never changed — the words go in a hidden layer behind it, so a misread word
                  costs you a failed search, never a wrong document.
                </span>
              </div>

              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 44 }}
                disabled={!pdf || busy}
                onClick={() => run(false)}
              >
                {busy ? "Reading…" : "Just show me the text"}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
                disabled={!pdf || busy}
                onClick={() => run(true)}
              >
                {busy ? "Reading…" : "Make it searchable"}
              </button>
            </>
          )}
        </aside>
      </div>
    </div>
  );
}

function Blurb({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "32px 4px" }}>
      <h2 className="display" style={{ fontSize: 20 }}>
        {title}
      </h2>
      <p style={{ color: "var(--text-subtle)", maxWidth: 560, lineHeight: 1.7 }}>{body}</p>
    </div>
  );
}
