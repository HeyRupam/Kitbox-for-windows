import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { Thumbnail } from "../components/Thumbnail";
import {
  countMissing,
  entryStory,
  forgetEntry,
  formatSize,
  recentHistory,
  rerunRecipe,
  revealOutput,
  setPinned,
  storageUsage,
  tidyHistory,
} from "../lib/bridge";
import type { HistoryEntry, Link, RerunResult, StorageUsage, Story } from "../lib/types";

export function Gallery({ onBack }: { onBack: () => void }) {
  const [rows, setRows] = useState<HistoryEntry[]>([]);
  const [usage, setUsage] = useState<StorageUsage | null>(null);
  const [missing, setMissing] = useState(0);
  const [tidying, setTidying] = useState(false);
  const [swept, setSwept] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [tool, setTool] = useState("all");
  const [open, setOpen] = useState<HistoryEntry | null>(null);

  const load = () => {
    recentHistory(120).then(setRows).catch(() => setRows([]));
    storageUsage().then(setUsage).catch(() => setUsage(null));
    countMissing().then(setMissing).catch(() => setMissing(0));
  };

  useEffect(load, []);

  const tidy = async () => {
    setTidying(true);
    try {
      const result = await tidyHistory();
      const parts: string[] = [];
      if (result.forgotten > 0) {
        parts.push(
          `${result.forgotten} ${result.forgotten === 1 ? "entry" : "entries"} whose file had gone`,
        );
      }
      if (result.loose > 0) {
        parts.push(
          `${result.loose} stored ${result.loose === 1 ? "copy" : "copies"} nothing pointed at`,
        );
      }
      setSwept(
        parts.length === 0
          ? "Nothing to tidy."
          : `Forgot ${parts.join(" and ")}, freeing ${formatSize(result.freed)}.`,
      );
      load();
    } catch (e) {
      setSwept(e instanceof Error ? e.message : String(e));
    } finally {
      setTidying(false);
    }
  };

  // The tools that actually appear, rather than a list of everything Kitbox
  // can do: a filter offering twenty chips for an empty history is noise.
  const tools = [...new Set(rows.map((r) => r.tool).filter(Boolean))] as string[];
  const needle = query.trim().toLowerCase();
  const visible = rows.filter(
    (r) =>
      (!needle || r.name.toLowerCase().includes(needle)) &&
      (tool === "all" ||
        (tool === "pinned" ? r.pinned : tool === "images" ? !r.pages : r.tool === tool)),
  );

  return (
    <div className="col grow scroll" style={{ gap: 22, paddingRight: 6 }}>
      <header className="row" style={{ gap: 16, flexWrap: "wrap", justifyContent: "space-between" }}>
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
            <span className="section-heading">Local</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              History
            </h1>
          </div>
        </div>
        <label className="pressed field" style={{ flex: "0 1 320px", minHeight: 48, borderRadius: 16 }}>
          <Icon path={ICONS.search} size={18} color="var(--text-subtle)" strokeWidth={2} />
          <span className="sr-only">Search history</span>
          <input
            type="search"
            placeholder="Search by name"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
      </header>

      {usage && (
        <div
          className="raised row"
          style={{
            borderRadius: 20,
            padding: "16px 20px",
            gap: 28,
            flexWrap: "wrap",
            alignItems: "center",
          }}
        >
          <Stat label="Files kept" value={String(usage.files)} />
          <Stat label="On disk" value={formatSize(usage.bytes)} />
          <Stat label="Jobs run" value={String(usage.jobs)} />
          {missing > 0 && <Stat label="Can be tidied" value={String(missing)} />}
          <span className="grow" />
          {swept && (
            <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>{swept}</span>
          )}
          {missing > 0 && (
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 42 }}
              disabled={tidying}
              onClick={tidy}
              title="Forget entries whose file is no longer there, and stored copies nothing points at"
            >
              <Icon path={ICONS.close} size={16} strokeWidth={2} />
              {tidying ? "Tidying…" : `Tidy up ${missing}`}
            </button>
          )}
        </div>
      )}

      {rows.length > 0 && (
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          {[
            ["all", `Everything (${rows.length})`],
            ["pinned", `Pinned (${rows.filter((r) => r.pinned).length})`],
            ...tools.map((t) => [t, t.replace(/-/g, " ")] as [string, string]),
          ].map(([id, label]) => (
            <button
              key={id}
              type="button"
              aria-pressed={tool === id}
              className={tool === id ? "pressed pill" : "raised-sm pill"}
              onClick={() => setTool(id)}
              style={{ color: tool === id ? "var(--accent)" : undefined }}
            >
              {label}
            </button>
          ))}
        </div>
      )}

      {open && <StoryPanel row={open} onClose={() => setOpen(null)} />}

      {visible.length === 0 ? (
        <div className="raised col" style={{ borderRadius: "var(--r-panel)", padding: 36, gap: 10 }}>
          <h2 className="display" style={{ fontSize: 20 }}>
            Nothing here yet
          </h2>
          <p style={{ color: "var(--text-subtle)", maxWidth: 480, lineHeight: 1.6 }}>
            Every file you compress is kept here with the settings that produced it, so the
            next time a form asks for the same thing you can take the finished file rather
            than make it again — or run those same settings over a new one.
          </p>
        </div>
      ) : (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))",
            gap: 18,
          }}
        >
          {visible.map((r) => (
            <Entry
              key={r.hash}
              row={r}
              onOpen={() => setOpen(r)}
              onForgotten={() => {
                setRows((all) => all.filter((e) => e.hash !== r.hash));
                storageUsage().then(setUsage).catch(() => undefined);
                countMissing().then(setMissing).catch(() => undefined);
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * One past job: what it made, and the offer to do it again.
 *
 * The settings were recorded at the time, so "the same thing I did to last
 * month's form" is a file away rather than a hunt through settings you half
 * remember. Where that isn't possible the card says why instead of offering a
 * button that fails — several tools deliberately don't record what they did.
 */
function Entry({
  row,
  onForgotten,
  onOpen,
}: {
  row: HistoryEntry;
  onForgotten: () => void;
  onOpen: () => void;
}) {
  const [result, setResult] = useState<RerunResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [why, setWhy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [pinned, setPinnedHere] = useState(row.pinned);
  const picker = useRef<HTMLInputElement>(null);

  const forget = async () => {
    try {
      await forgetEntry(row.hash);
      onForgotten();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setConfirming(false);
    }
  };

  const run = async (file: File | null) => {
    if (!file) return;
    setRunning(true);
    setError(null);
    setResult(null);
    try {
      setResult(await rerunRecipe(row.hash, file));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  };

  const accept =
    row.replay?.takes === "pdf"
      ? "application/pdf"
      : row.replay?.takes === "image"
        ? "image/*"
        : "image/*,application/pdf";

  return (
    <div className="raised col" style={{ borderRadius: 20, padding: 14, gap: 10 }}>
      <button
        type="button"
        onClick={onOpen}
        title="Where this came from"
        style={{ padding: 0, border: "none", background: "none", cursor: "pointer" }}
      >
        <Thumbnail hash={row.thumbHash} kind={row.kind} />
      </button>
      <div className="row" style={{ gap: 10, justifyContent: "space-between" }}>
        <span className="truncate" style={{ fontSize: 13.5, fontWeight: 600 }}>
          {row.name}
        </span>
        <span className="row" style={{ gap: 8, flex: "0 0 auto" }}>
          <span style={{ fontSize: 12, color: "var(--accent)", fontWeight: 600 }}>
            {formatSize(row.size)}
          </span>
          <button
            type="button"
            aria-pressed={pinned}
            aria-label={pinned ? "Pinned — keep through a tidy-up" : "Pin this"}
            title={
              pinned
                ? "Pinned: a tidy-up leaves this alone"
                : "Pin this so a tidy-up leaves it alone"
            }
            className={pinned ? "pressed" : "raised-sm"}
            style={{
              width: 26,
              height: 26,
              borderRadius: 9,
              display: "grid",
              placeItems: "center",
              color: pinned ? "var(--accent)" : "var(--text-subtle)",
            }}
            onClick={async () => {
              const next = !pinned;
              setPinnedHere(next);
              try {
                await setPinned(row.hash, next);
              } catch {
                setPinnedHere(!next);
              }
            }}
          >
            <Icon path={ICONS.badge} size={13} strokeWidth={2} />
          </button>
        </span>
      </div>
      <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
        {row.tool && (
          <span
            className="pressed"
            style={{
              fontSize: 10.5,
              fontWeight: 600,
              letterSpacing: "0.06em",
              textTransform: "uppercase",
              padding: "3px 8px",
              borderRadius: 7,
              color: "var(--text-subtle)",
            }}
          >
            {row.tool.replace(/-/g, " ")}
          </span>
        )}
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)", alignSelf: "center" }}>
          {row.width && row.height
            ? `${row.width}×${row.height}`
            : row.pages
              ? `${row.pages} page${row.pages === 1 ? "" : "s"}`
              : ""}
        </span>
      </div>
      {row.summary && (
        <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
          {row.summary}
        </span>
      )}
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
        {new Date(row.createdAt).toLocaleString()}
      </span>

      <input
        ref={picker}
        type="file"
        hidden
        accept={accept}
        onChange={(e) => run(e.target.files?.[0] ?? null)}
      />

      {row.replay?.can ? (
        <button
          type="button"
          className="raised-sm btn"
          style={{ minHeight: 40, fontSize: 12.5 }}
          disabled={running}
          onClick={() => picker.current?.click()}
          title={row.replay.label}
        >
          <Icon path={ICONS.history} size={15} strokeWidth={2} />
          {running ? "Running…" : "Run this again"}
        </button>
      ) : row.replay?.why ? (
        <button
          type="button"
          className="row"
          aria-expanded={why}
          onClick={() => setWhy(!why)}
          style={{
            fontSize: 11.5,
            color: "var(--text-subtle)",
            gap: 6,
            textAlign: "left",
            padding: 0,
          }}
        >
          <Icon path={ICONS.warning} size={14} strokeWidth={2} />
          {why ? "Can't be run again:" : "Can't be run again"}
        </button>
      ) : null}

      {why && row.replay?.why && (
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          {row.replay.why}
        </span>
      )}

      {row.replay?.can && !result && !error && !running && (
        <span style={{ fontSize: 11, color: "var(--text-subtle)", lineHeight: 1.5 }}>
          {row.replay.label}
        </span>
      )}

      {result && (
        <div className="pressed col" style={{ borderRadius: 14, padding: 12, gap: 8 }}>
          <span style={{ fontSize: 12, fontWeight: 600 }}>
            {result.paths.length === 1
              ? "Done"
              : `Done — ${result.paths.length} files`}
          </span>
          <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
            {result.summary}
          </span>
          {result.paths[0] && (
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 36, fontSize: 12 }}
              onClick={() => revealOutput(result.paths[0])}
            >
              <Icon path={ICONS.save} size={14} strokeWidth={2} />
              Show in folder
            </button>
          )}
        </div>
      )}

      {error && (
        <span style={{ fontSize: 11.5, color: "var(--accent)", lineHeight: 1.55 }}>{error}</span>
      )}

      {confirming ? (
        <div className="pressed col" style={{ borderRadius: 14, padding: 12, gap: 8 }}>
          <span style={{ fontSize: 11.5, lineHeight: 1.6 }}>
            Forget this entry? Kitbox's copy and this row go for good. The finished file in
            your output folder is yours and stays where it is.
            {pinned ? " This one is pinned, which only protects it from a tidy-up." : ""}
          </span>
          <div className="row" style={{ gap: 8 }}>
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 36, fontSize: 12, color: "var(--accent)" }}
              onClick={forget}
            >
              Forget it
            </button>
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 36, fontSize: 12 }}
              onClick={() => setConfirming(false)}
            >
              Keep it
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          className="row"
          onClick={() => setConfirming(true)}
          style={{
            fontSize: 11.5,
            color: "var(--text-subtle)",
            gap: 6,
            textAlign: "left",
            padding: 0,
          }}
        >
          <Icon path={ICONS.close} size={13} strokeWidth={2} />
          Forget this
        </button>
      )}
    </div>
  );
}

/**
 * Where a file came from, and what came of it.
 *
 * Every job records its input, its output and the settings between them, so
 * this chain has existed since the first version — it had just never been put
 * on screen. "The scan → the compressed copy → the signed one" is the thing
 * that makes a gallery worth keeping.
 */
function StoryPanel({ row, onClose }: { row: HistoryEntry; onClose: () => void }) {
  const [story, setStory] = useState<Story | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setStory(null);
    entryStory(row.hash)
      .then(setStory)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [row.hash]);

  return (
    <section className="raised col" style={{ borderRadius: "var(--r-panel)", padding: 22, gap: 16 }}>
      <div className="row" style={{ justifyContent: "space-between", gap: 12 }}>
        <div className="col" style={{ gap: 2, minWidth: 0 }}>
          <span className="section-heading">Where this came from</span>
          <span className="truncate" style={{ fontSize: 15, fontWeight: 600 }}>
            {row.name}
          </span>
        </div>
        <button
          type="button"
          className="raised-sm icon-btn"
          aria-label="Close"
          onClick={onClose}
          style={{ width: 38, height: 38, borderRadius: 12, flex: "0 0 auto" }}
        >
          <Icon path={ICONS.close} size={16} strokeWidth={2} />
        </button>
      </div>

      {error && <span style={{ fontSize: 12, color: "var(--accent)" }}>{error}</span>}

      {story && story.chain.length === 0 && (
        <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          Nothing recorded before this one — it is where the chain starts.
        </span>
      )}

      {story?.chain.map((step, i) => (
        <div key={i} className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
          <div className="row" style={{ gap: 10, justifyContent: "space-between", flexWrap: "wrap" }}>
            <span style={{ fontSize: 12.5, fontWeight: 600 }}>{step.tool.replace(/-/g, " ")}</span>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
              {new Date(step.createdAt).toLocaleString()} · {step.durationMs} ms
            </span>
          </div>
          <div className="row" style={{ gap: 10, alignItems: "center", flexWrap: "wrap" }}>
            <Side link={step.from} fallback="handed in" />
            <span style={{ color: "var(--text-subtle)" }}>→</span>
            <Side link={step.to} fallback="produced" />
          </div>
          <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}>
            {step.summary}
          </span>
          <details>
            <summary style={{ fontSize: 11.5, color: "var(--text-subtle)", cursor: "pointer" }}>
              The settings it used
            </summary>
            <pre
              className="scroll"
              style={{
                fontSize: 11,
                lineHeight: 1.5,
                margin: "8px 0 0",
                maxHeight: 180,
                color: "var(--text-muted)",
                whiteSpace: "pre-wrap",
              }}
            >
              {step.recipe}
            </pre>
          </details>
        </div>
      ))}

      {story && story.derivatives.length > 0 && (
        <div className="col" style={{ gap: 8 }}>
          <span className="section-heading">And what came of it</span>
          {story.derivatives.map((step, i) => (
            <span key={i} style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
              {step.tool.replace(/-/g, " ")} — {step.summary}
            </span>
          ))}
        </div>
      )}
    </section>
  );
}

function Side({ link, fallback }: { link: Link | null; fallback: string }) {
  if (!link) {
    return (
      <span style={{ fontSize: 12, color: "var(--text-subtle)", fontStyle: "italic" }}>
        {fallback}
      </span>
    );
  }
  return (
    <span className="col" style={{ gap: 1, minWidth: 0 }}>
      <span
        className="truncate"
        style={{
          fontSize: 12.5,
          fontWeight: 600,
          color: link.known ? undefined : "var(--text-subtle)",
          fontStyle: link.known ? undefined : "italic",
        }}
      >
        {link.name}
      </span>
      {link.known && (
        <span style={{ fontSize: 11, color: "var(--text-subtle)" }}>{formatSize(link.size)}</span>
      )}
    </span>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="col" style={{ gap: 2 }}>
      <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>{label}</span>
      <span className="display" style={{ fontSize: 20 }}>
        {value}
      </span>
    </div>
  );
}
