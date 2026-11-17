import { useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { cancelBatch, formatSize, isDesktop, revealOutput, runBatch } from "../lib/bridge";
import type { BatchOp, BatchOutcome, BatchResult, SizeTarget } from "../lib/types";

type Mode = "compress" | "resize" | "convert" | "adjust";

/**
 * The same thing, done to a pile of files.
 *
 * What this screen is really for is the moment the single-file tools can't
 * help with: forty photos that all have to come in under 100 KB. So the
 * result is a list, not a number — which files landed, which fell short of
 * the target, and which couldn't be read at all, each by name.
 *
 * A file that fails never stops the run. That is handled in the core, but it
 * is the reason this screen shows three counts rather than "done".
 */
export function Batch({
  files,
  onFiles,
  onBack,
  handed,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
  /** A run that already happened — from the Windows right-click menu. */
  handed?: { summary: string; outcomes: BatchOutcome[]; paths: string[] } | null;
}) {
  const [mode, setMode] = useState<Mode>("compress");
  const [max, setMax] = useState(100);
  const [min, setMin] = useState(0);
  const [longest, setLongest] = useState(1600);
  const [format, setFormat] = useState<"jpeg" | "png">("jpeg");

  const [running, setRunning] = useState(false);
  const [stage, setStage] = useState("");
  const [fraction, setFraction] = useState(0);
  /** Filled in as each file comes back, so you watch the pile go down. */
  const [sofar, setSofar] = useState<BatchOutcome[]>([]);
  const [result, setResult] = useState<BatchResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const op = (): BatchOp => {
    if (mode === "resize") return { kind: "resize", longest };
    if (mode === "convert") return { kind: "convert", format, stripMetadata: true };
    if (mode === "adjust")
      return {
        kind: "adjust",
        autoLevels: true,
        autoColour: true,
        exposure: 0,
        contrast: 0,
        saturation: 0,
        warmth: 0,
        sharpen: 0,
        greyscale: false,
      };
    // KB is 1000 bytes here, as everywhere else in the app: a form that says
    // "100 KB" is read the strict way, so the file passes either reading.
    const target: SizeTarget =
      min > 0
        ? { kind: "range", value: { min: min * 1000, max: max * 1000 } }
        : { kind: "at_most", value: max * 1000 };
    return {
      kind: "compress",
      target,
      bias: "balanced",
      format: "keep",
      stripMetadata: true,
      exact: null,
    };
  };

  const start = async () => {
    if (files.length === 0) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setFraction(0);
    setSofar([]);
    setStage(`${files[0].name} (1 of ${files.length})`);
    try {
      setResult(
        await runBatch(files, op(), (step, index, total) => {
          setSofar((done) => [...done, step.outcome]);
          setFraction((index + 1) / total);
          // Named before it starts rather than after it finishes: a slow file
          // should show which one is slow, not look like a hang.
          const next = files[index + 1];
          if (next) setStage(`${next.name} (${index + 2} of ${total})`);
        }),
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
      setStage("");
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
            <span className="section-heading">Everything at once</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Batch
            </h1>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept="image/*,application/pdf"
          multiple
          hidden
          onChange={(e) => {
            onFiles([...files, ...(e.target.files ?? [])]);
            setResult(null);
            setSofar([]);
          }}
        />
        <div className="row" style={{ gap: 8 }}>
          {files.length > 0 && !running && (
            <button
              type="button"
              className="raised-sm btn"
              onClick={() => {
                onFiles([]);
                setResult(null);
                setSofar([]);
              }}
            >
              Clear
            </button>
          )}
          <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
            <Icon path={ICONS.plus} size={18} strokeWidth={2} />
            Add files
          </button>
        </div>
      </header>

      <div className="row grow" style={{ gap: 22, alignItems: "stretch", minHeight: 0 }}>
        <section
          className="raised col grow"
          style={{ borderRadius: "var(--r-panel)", padding: 20, gap: 14, minWidth: 0 }}
        >
          {!isDesktop() ? (
            <Blurb
              title="This needs the desktop app"
              body="Batch work writes files to a folder, which the browser preview can't do."
            />
          ) : files.length === 0 && !result && !handed ? (
            <Blurb
              title="Drop the whole folder in"
              body="Every file gets the same treatment. One that can't be read is reported against its own name and the rest carry on — you never lose thirty-nine files to one bad one."
            />
          ) : handed ? (
            <Outcomes
              outcomes={handed.outcomes}
              heading={`From Explorer — ${handed.summary}`}
            />
          ) : result ? (
            <Outcomes outcomes={result.outcomes} heading="Every file, and what happened" />
          ) : sofar.length > 0 ? (
            <Outcomes
              outcomes={sofar}
              heading={`${sofar.length} of ${files.length} answered for`}
            />
          ) : (
            <>
              <span className="section-heading">
                {files.length} file{files.length === 1 ? "" : "s"} ready
              </span>
              <div className="pressed grow scroll" style={{ borderRadius: 18, padding: 14 }}>
                {files.map((f, i) => (
                  <div
                    key={`${f.name}-${i}`}
                    className="row"
                    style={{
                      justifyContent: "space-between",
                      padding: "7px 4px",
                      fontSize: 12.5,
                      gap: 12,
                    }}
                  >
                    <span className="truncate" style={{ minWidth: 0 }}>
                      {f.name}
                    </span>
                    <span style={{ color: "var(--text-subtle)", flex: "0 0 auto" }}>
                      {formatSize(f.size)}
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}

          {running && (
            <div className="col" style={{ gap: 8 }}>
              <div className="row" style={{ justifyContent: "space-between" }}>
                <span className="truncate" style={{ fontSize: 12.5, fontWeight: 600, minWidth: 0 }}>
                  {stage || "Starting…"}
                </span>
                <button
                  type="button"
                  className="raised-sm"
                  onClick={() => cancelBatch()}
                  style={{ borderRadius: 999, padding: "4px 13px", fontSize: 11.5, flex: "0 0 auto" }}
                >
                  Stop
                </button>
              </div>
              <div
                className="pressed"
                style={{ height: 8, borderRadius: 999, overflow: "hidden" }}
              >
                <span
                  style={{
                    display: "block",
                    height: "100%",
                    width: `${Math.round(fraction * 100)}%`,
                    background: "var(--accent)",
                    transition: "width 120ms linear",
                  }}
                />
              </div>
            </div>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          <div className="col" style={{ gap: 9 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
              Do what to all of them
            </span>
            <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
              {(
                [
                  ["compress", "Compress"],
                  ["resize", "Resize"],
                  ["convert", "Convert"],
                  ["adjust", "Auto-fix"],
                ] as [Mode, string][]
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={mode === id}
                  className={mode === id ? "pressed" : "raised-sm"}
                  onClick={() => {
                    setMode(id);
                    setResult(null);
                    setSofar([]);
                  }}
                  style={{
                    borderRadius: 12,
                    minHeight: 42,
                    fontSize: 12,
                    fontWeight: mode === id ? 700 : 500,
                    color: mode === id ? "var(--accent)" : undefined,
                  }}
                >
                  {label}
                </button>
              ))}
            </div>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.55 }}>
              {mode === "compress"
                ? "The only one that takes PDFs as well as pictures."
                : mode === "adjust"
                  ? "Levels and colour corrected automatically on every file."
                  : "Pictures only — a PDF in the pile is reported and skipped."}
            </span>
          </div>

          {mode === "compress" && (
            <div className="col" style={{ gap: 10 }}>
              <Number label="Largest, in KB" value={max} onChange={setMax} />
              <Number
                label="Smallest, in KB (0 for no minimum)"
                value={min}
                onChange={setMin}
              />
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                A minimum matters when a form insists on a range — plenty ask for 20–50 KB and
                reject anything under.
              </span>
            </div>
          )}

          {mode === "resize" && (
            <Number label="Longest edge, in pixels" value={longest} onChange={setLongest} />
          )}

          {mode === "convert" && (
            <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 8 }}>
              {(["jpeg", "png"] as const).map((f) => (
                <button
                  key={f}
                  type="button"
                  aria-pressed={format === f}
                  className={format === f ? "pressed" : "raised-sm"}
                  onClick={() => setFormat(f)}
                  style={{
                    borderRadius: 12,
                    minHeight: 42,
                    fontSize: 12,
                    fontWeight: format === f ? 700 : 500,
                    color: format === f ? "var(--accent)" : undefined,
                  }}
                >
                  {f.toUpperCase()}
                </button>
              ))}
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

          {handed && !result && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {handed.outcomes.filter((o) => o.state === "done").length} done
              </span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Run from the Explorer right-click menu, before this window opened.
              </span>
              {handed.paths[0] && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ minHeight: 40 }}
                  onClick={() => revealOutput(handed.paths[0])}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show in folder
                </button>
              )}
            </div>
          )}

          {result && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
              <span className="display" style={{ fontSize: 18 }}>
                {result.done} done
                {result.failed > 0 ? `, ${result.failed} failed` : ""}
              </span>
              {result.short > 0 && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  {result.short} couldn't reach the size and were saved at their smallest.
                </span>
              )}
              {result.cancelled && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                  Stopped part way through.
                </span>
              )}
              {result.paths[0] && (
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ minHeight: 40 }}
                  onClick={() => revealOutput(result.paths[0])}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show in folder
                </button>
              )}
            </div>
          )}

          <span className="grow" />

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={files.length === 0 || running}
            onClick={start}
          >
            {running
              ? "Working…"
              : `Do all ${files.length || ""}`.replace(/\s+/g, " ").trim()}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Outcomes({ outcomes, heading }: { outcomes: BatchOutcome[]; heading: string }) {
  return (
    <>
      <span className="section-heading">{heading}</span>
      <div className="pressed grow scroll" style={{ borderRadius: 18, padding: 14 }}>
        {outcomes.map((o, i) => (
          <Row key={i} outcome={o} />
        ))}
      </div>
    </>
  );
}

function Row({ outcome }: { outcome: BatchOutcome }) {
  if (outcome.state === "done") {
    return (
      <div className="col" style={{ padding: "8px 4px", gap: 2 }}>
        <div className="row" style={{ justifyContent: "space-between", gap: 12 }}>
          <span className="truncate" style={{ fontSize: 12.5, fontWeight: 600, minWidth: 0 }}>
            {outcome.name}
          </span>
          <span
            style={{
              fontSize: 12,
              flex: "0 0 auto",
              color: outcome.short ? "var(--accent)" : "var(--text-subtle)",
            }}
          >
            {formatSize(outcome.before)} → {formatSize(outcome.after)}
          </span>
        </div>
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
          {outcome.short ? "couldn't reach it — " : ""}
          {outcome.note}
        </span>
      </div>
    );
  }
  if (outcome.state === "failed") {
    return (
      <div className="col" style={{ padding: "8px 4px", gap: 2 }}>
        <span className="truncate" style={{ fontSize: 12.5, fontWeight: 600 }}>
          {outcome.name}
        </span>
        <span style={{ fontSize: 11.5, color: "var(--accent)", lineHeight: 1.5 }}>
          {outcome.why}
        </span>
      </div>
    );
  }
  return (
    <div className="row" style={{ padding: "8px 4px", gap: 12 }}>
      <span className="truncate" style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
        {outcome.name} — not reached
      </span>
    </div>
  );
}

function Blurb({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "36px 6px" }}>
      <h2 className="display" style={{ fontSize: 20 }}>
        {title}
      </h2>
      <p style={{ color: "var(--text-subtle)", maxWidth: 560, lineHeight: 1.7 }}>{body}</p>
    </div>
  );
}

function Number({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <div className="col" style={{ gap: 7 }}>
      <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
      <input
        type="number"
        min={0}
        value={value}
        aria-label={label}
        onChange={(e) => onChange(Math.max(0, globalThis.Number(e.target.value) || 0))}
        className="pressed"
        style={{
          borderRadius: 12,
          minHeight: 44,
          padding: "0 13px",
          fontSize: 13,
          border: "none",
          color: "var(--text)",
          width: "100%",
          fontFamily: "inherit",
        }}
      />
    </div>
  );
}
