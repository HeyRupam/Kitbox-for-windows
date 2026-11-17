import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  deleteRecipe,
  formatSize,
  isDesktop,
  recipes as listRecipes,
  revealOutput,
  runRecipe,
  saveRecipe,
} from "../lib/bridge";
import type { BatchOutcome, Chain, RecipeRun, Step } from "../lib/types";

/**
 * Recipes: several operations, saved as one thing.
 *
 * Every tool screen does one job. This is for the job someone actually has —
 * "crop it square, make it 200 × 230, land it between 20 and 50 KB" is one
 * task to the person and three operations to the engine. Saved once, it
 * becomes a drop.
 */
export function Recipes({ onBack }: { onBack: () => void }) {
  const [chains, setChains] = useState<Chain[]>([]);
  const [editing, setEditing] = useState<Chain | null>(null);
  const [open, setOpen] = useState<Chain | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    listRecipes().then(setChains).catch(() => setChains([]));
  }, []);

  return (
    <div className="col grow scroll" style={{ gap: 22, paddingRight: 6 }}>
      <header className="row" style={{ gap: 16, justifyContent: "space-between", flexWrap: "wrap" }}>
        <div className="row" style={{ gap: 16 }}>
          <button
            type="button"
            className="raised-sm icon-btn"
            aria-label={open || editing ? "Back to all recipes" : "Back to all tools"}
            onClick={() => {
              if (editing) return setEditing(null);
              if (open) return setOpen(null);
              onBack();
            }}
            style={{ width: 48, height: 48, borderRadius: 16 }}
          >
            <Icon path={ICONS.back} size={20} strokeWidth={2} />
          </button>
          <div className="col" style={{ gap: 2 }}>
            <span className="section-heading">Saved</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              {open ? open.name : editing ? "Edit recipe" : "Recipes"}
            </h1>
          </div>
        </div>
        {!open && !editing && isDesktop() && (
          <button type="button" className="raised-sm btn" onClick={() => setEditing(blank())}>
            <Icon path={ICONS.plus} size={18} strokeWidth={2} />
            New recipe
          </button>
        )}
      </header>

      {error && <Notice title="That didn't work" body={error} />}

      {editing ? (
        <Editor
          chain={editing}
          onCancel={() => setEditing(null)}
          onSave={async (chain) => {
            const saved = await saveRecipe(chain);
            setChains(saved);
            setEditing(null);
            setOpen(saved.find((c) => c.id === chain.id) ?? null);
          }}
        />
      ) : open ? (
        <Run
          chain={open}
          onEdit={() => setEditing({ ...open, steps: [...open.steps] })}
          onDelete={async () => {
            setChains(await deleteRecipe(open.id));
            setOpen(null);
          }}
          onError={setError}
        />
      ) : (
        <>
          <div className="raised col" style={{ borderRadius: 20, padding: "16px 20px", gap: 6 }}>
            <span style={{ fontSize: 13, fontWeight: 600 }}>One drop instead of five</span>
            <span
              style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6, maxWidth: 640 }}
            >
              A recipe runs its steps in order, every time, on as many files as you give it.
              Compressing is always the last step — anything after it would re-encode the file
              and undo the size it just solved for.
            </span>
          </div>

          {chains.length === 0 ? (
            <div className="raised col" style={{ borderRadius: "var(--r-panel)", padding: 36, gap: 10 }}>
              <h2 className="display" style={{ fontSize: 20 }}>
                Nothing saved yet
              </h2>
              <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.7 }}>
                Build one from the steps you keep repeating — the form photo you make every
                few months, the scans that always need to come in under 100 KB.
              </p>
            </div>
          ) : (
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))",
                gap: 18,
              }}
            >
              {chains.map((chain) => (
                <button
                  key={chain.id}
                  type="button"
                  className="raised col tile"
                  style={{ borderRadius: 20, padding: 18, gap: 10, alignItems: "stretch", textAlign: "left" }}
                  onClick={() => setOpen(chain)}
                >
                  <span style={{ fontSize: 15, fontWeight: 600 }}>{chain.name}</span>
                  <div className="col" style={{ gap: 4 }}>
                    {chain.steps.map((step, i) => (
                      <span key={i} style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                        {i + 1}. {describe(step)}
                      </span>
                    ))}
                  </div>
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

/** Give a recipe some files. */
function Run({
  chain,
  onEdit,
  onDelete,
  onError,
}: {
  chain: Chain;
  onEdit: () => void;
  onDelete: () => void;
  onError: (why: string) => void;
}) {
  const [files, setFiles] = useState<File[]>([]);
  const [result, setResult] = useState<RecipeRun | null>(null);
  const [running, setRunning] = useState(false);
  const picker = useRef<HTMLInputElement>(null);

  const run = async () => {
    if (files.length === 0) return;
    setRunning(true);
    setResult(null);
    try {
      setResult(await runRecipe(chain, files));
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="row" style={{ gap: 22, alignItems: "flex-start", flexWrap: "wrap" }}>
      <section
        className="raised col grow"
        style={{ borderRadius: "var(--r-panel)", padding: 22, gap: 14, minWidth: 320 }}
      >
        <span className="section-heading">What it does, in order</span>
        <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
          {chain.steps.map((step, i) => (
            <span key={i} style={{ fontSize: 12.5 }}>
              <span style={{ color: "var(--text-subtle)" }}>{i + 1}.</span> {describe(step)}
            </span>
          ))}
        </div>
        {chain.note && (
          <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6 }}>
            {chain.note}
          </span>
        )}

        <input
          ref={picker}
          type="file"
          hidden
          multiple
          accept="image/*,application/pdf"
          onChange={(e) => {
            setFiles([...(e.target.files ?? [])]);
            setResult(null);
          }}
        />

        {result ? (
          <div className="col" style={{ gap: 8 }}>
            <span className="section-heading">
              {result.done} done{result.failed > 0 ? `, ${result.failed} failed` : ""}
            </span>
            <div className="pressed col scroll" style={{ borderRadius: 16, padding: 14, gap: 8, maxHeight: 260 }}>
              {result.outcomes.map((outcome, i) => (
                <Row key={i} outcome={outcome} />
              ))}
            </div>
            {result.steps.length > 0 && (
              <div className="col" style={{ gap: 3 }}>
                {result.steps.map((line, i) => (
                  <span key={i} style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                    {line}
                  </span>
                ))}
              </div>
            )}
            {result.paths[0] && (
              <button
                type="button"
                className="raised-sm btn"
                style={{ alignSelf: "flex-start", minHeight: 40 }}
                onClick={() => revealOutput(result.paths[0])}
              >
                <Icon path={ICONS.save} size={16} strokeWidth={2} />
                Show in folder
              </button>
            )}
          </div>
        ) : (
          <div className="pressed col" style={{ borderRadius: 16, padding: 18, gap: 10 }}>
            <span style={{ fontSize: 13, fontWeight: 600 }}>
              {files.length === 0
                ? "Choose the files to run it on"
                : `${files.length} file${files.length === 1 ? "" : "s"} ready`}
            </span>
            <button
              type="button"
              className="raised-sm btn"
              style={{ alignSelf: "flex-start", minHeight: 42 }}
              onClick={() => picker.current?.click()}
            >
              {files.length === 0 ? "Choose files" : "Choose different files"}
            </button>
          </div>
        )}

        <button
          type="button"
          className="btn btn-primary"
          style={{ minHeight: 52, borderRadius: 16, fontSize: 14.5, fontWeight: 700 }}
          disabled={files.length === 0 || running}
          onClick={run}
        >
          {running ? "Running…" : `Run on ${files.length || ""} ${files.length === 1 ? "file" : "files"}`.replace(/\s+/g, " ")}
        </button>
      </section>

      <aside
        className="raised col"
        style={{ flex: "0 0 260px", borderRadius: "var(--r-panel)", padding: 22, gap: 12 }}
      >
        <button type="button" className="raised-sm btn" style={{ minHeight: 44 }} onClick={onEdit}>
          Edit the steps
        </button>
        <button type="button" className="raised-sm btn" style={{ minHeight: 44 }} onClick={onDelete}>
          Delete this recipe
        </button>
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          Saved in your presets folder as <strong>recipes.json</strong>, editable by hand like
          everything else there.
        </span>
      </aside>
    </div>
  );
}

function Row({ outcome }: { outcome: BatchOutcome }) {
  if (outcome.state === "done") {
    return (
      <div className="col" style={{ gap: 2 }}>
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
        {outcome.short && (
          <span style={{ fontSize: 11.5, color: "var(--accent)" }}>{outcome.note}</span>
        )}
      </div>
    );
  }
  if (outcome.state === "failed") {
    return (
      <div className="col" style={{ gap: 2 }}>
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
    <span style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
      {outcome.name} — not reached
    </span>
  );
}

/** Build the list of steps. */
function Editor({
  chain,
  onSave,
  onCancel,
}: {
  chain: Chain;
  onSave: (chain: Chain) => Promise<void>;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<Chain>(chain);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const setSteps = (steps: Step[]) => setDraft({ ...draft, steps });
  const move = (from: number, by: number) => {
    const to = from + by;
    if (to < 0 || to >= draft.steps.length) return;
    const steps = [...draft.steps];
    [steps[from], steps[to]] = [steps[to], steps[from]];
    setSteps(steps);
  };

  return (
    <section
      className="raised col"
      style={{ borderRadius: "var(--r-panel)", padding: 22, gap: 16, maxWidth: 680 }}
    >
      <label className="col" style={{ gap: 7 }}>
        <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-muted)" }}>Name</span>
        <input
          className="pressed"
          value={draft.name}
          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
          style={{
            borderRadius: 12,
            minHeight: 44,
            padding: "0 13px",
            fontSize: 13,
            border: "none",
            color: "var(--text)",
            fontFamily: "inherit",
          }}
        />
      </label>

      <div className="col" style={{ gap: 10 }}>
        <span className="section-heading">The steps, in order</span>
        {draft.steps.length === 0 && (
          <span style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
            Nothing yet — add a step below.
          </span>
        )}
        {draft.steps.map((step, i) => (
          <div key={i} className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 10 }}>
            <div className="row" style={{ justifyContent: "space-between", gap: 10 }}>
              <span style={{ fontSize: 12.5, fontWeight: 600 }}>
                {i + 1}. {describe(step)}
              </span>
              <span className="row" style={{ gap: 6, flex: "0 0 auto" }}>
                <Small label="↑" title="Move up" onClick={() => move(i, -1)} />
                <Small label="↓" title="Move down" onClick={() => move(i, 1)} />
                <Small
                  label="✕"
                  title="Remove this step"
                  onClick={() => setSteps(draft.steps.filter((_, at) => at !== i))}
                />
              </span>
            </div>
            <StepFields
              step={step}
              onChange={(next) => setSteps(draft.steps.map((s, at) => (at === i ? next : s)))}
            />
          </div>
        ))}
      </div>

      <div className="col" style={{ gap: 8 }}>
        <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-muted)" }}>
          Add a step
        </span>
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          {(
            [
              ["Resize", { step: "resize", longest: 1600 }],
              ["Crop and fit", { step: "fit", width: 200, height: 230 }],
              ["Turn", { step: "rotate", degrees: 90, flip: "none" }],
              ["Auto-fix", { step: "adjust", autoLevels: true, autoColour: true, exposure: 0, contrast: 0, saturation: 0, warmth: 0, sharpen: 0, greyscale: false }],
              ["Convert", { step: "convert", format: "jpeg" }],
              ["Compress", { step: "compress", target: { kind: "range", value: { min: 20000, max: 50000 } }, bias: "balanced" }],
            ] as [string, Step][]
          ).map(([label, step]) => (
            <button
              key={label}
              type="button"
              className="raised-sm"
              style={{ borderRadius: 999, padding: "8px 14px", fontSize: 12 }}
              onClick={() => setSteps([...draft.steps, step])}
            >
              + {label}
            </button>
          ))}
        </div>
      </div>

      {error && <Notice title="It wasn't saved" body={error} />}

      <div className="row" style={{ gap: 10 }}>
        <button
          type="button"
          className="btn btn-primary"
          style={{ minHeight: 48, borderRadius: 16, fontWeight: 700 }}
          disabled={saving}
          onClick={async () => {
            setSaving(true);
            setError(null);
            try {
              await onSave(draft);
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            } finally {
              setSaving(false);
            }
          }}
        >
          Save it
        </button>
        <button type="button" className="raised-sm btn" style={{ minHeight: 48 }} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </section>
  );
}

/** The numbers a step needs, if any. */
function StepFields({ step, onChange }: { step: Step; onChange: (step: Step) => void }) {
  const field: React.CSSProperties = {
    borderRadius: 10,
    minHeight: 38,
    padding: "0 11px",
    fontSize: 12.5,
    border: "none",
    color: "var(--text)",
    width: 96,
    fontFamily: "inherit",
  };

  if (step.step === "resize") {
    return (
      <Labelled label="Longest edge, in pixels">
        <input
          type="number"
          className="pressed"
          style={field}
          value={step.longest}
          onChange={(e) => onChange({ ...step, longest: Math.max(1, Number(e.target.value) || 1) })}
        />
      </Labelled>
    );
  }
  if (step.step === "fit") {
    return (
      <Labelled label="Exactly">
        <div className="row" style={{ gap: 8, alignItems: "center" }}>
          <input
            type="number"
            aria-label="Width"
            className="pressed"
            style={field}
            value={step.width}
            onChange={(e) => onChange({ ...step, width: Math.max(1, Number(e.target.value) || 1) })}
          />
          <span style={{ color: "var(--text-subtle)" }}>×</span>
          <input
            type="number"
            aria-label="Height"
            className="pressed"
            style={field}
            value={step.height}
            onChange={(e) => onChange({ ...step, height: Math.max(1, Number(e.target.value) || 1) })}
          />
        </div>
      </Labelled>
    );
  }
  if (step.step === "rotate") {
    return (
      <Labelled label="Quarter turns">
        <div className="row" style={{ gap: 8 }}>
          {[90, 180, 270].map((degrees) => (
            <button
              key={degrees}
              type="button"
              aria-pressed={step.degrees === degrees}
              className={step.degrees === degrees ? "pressed" : "raised-sm"}
              style={{
                borderRadius: 10,
                minHeight: 38,
                padding: "0 12px",
                fontSize: 12,
                color: step.degrees === degrees ? "var(--accent)" : undefined,
              }}
              onClick={() => onChange({ ...step, degrees })}
            >
              {degrees}°
            </button>
          ))}
        </div>
      </Labelled>
    );
  }
  if (step.step === "convert") {
    return (
      <Labelled label="Save as">
        <div className="row" style={{ gap: 8 }}>
          {(["jpeg", "png"] as const).map((format) => (
            <button
              key={format}
              type="button"
              aria-pressed={step.format === format}
              className={step.format === format ? "pressed" : "raised-sm"}
              style={{
                borderRadius: 10,
                minHeight: 38,
                padding: "0 12px",
                fontSize: 12,
                color: step.format === format ? "var(--accent)" : undefined,
              }}
              onClick={() => onChange({ ...step, format })}
            >
              {format.toUpperCase()}
            </button>
          ))}
        </div>
      </Labelled>
    );
  }
  if (step.step === "compress") {
    const min = step.target.kind === "range" ? step.target.value.min : 0;
    const max =
      step.target.kind === "range"
        ? step.target.value.max
        : step.target.kind === "at_most"
          ? step.target.value
          : step.target.value.bytes;
    const set = (nextMin: number, nextMax: number) =>
      onChange({
        ...step,
        target:
          nextMin > 0
            ? { kind: "range", value: { min: nextMin, max: nextMax } }
            : { kind: "at_most", value: nextMax },
      });
    return (
      <Labelled label="Between, in KB (0 for no minimum)">
        <div className="row" style={{ gap: 8, alignItems: "center" }}>
          <input
            type="number"
            aria-label="Smallest in KB"
            className="pressed"
            style={field}
            value={Math.round(min / 1000)}
            onChange={(e) => set(Math.max(0, Number(e.target.value) || 0) * 1000, max)}
          />
          <span style={{ color: "var(--text-subtle)" }}>–</span>
          <input
            type="number"
            aria-label="Largest in KB"
            className="pressed"
            style={field}
            value={Math.round(max / 1000)}
            onChange={(e) => set(min, Math.max(1, Number(e.target.value) || 1) * 1000)}
          />
        </div>
      </Labelled>
    );
  }
  return null;
}

function Labelled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="col" style={{ gap: 5 }}>
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>{label}</span>
      {children}
    </div>
  );
}

function Small({ label, title, onClick }: { label: string; title: string; onClick: () => void }) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      className="raised-sm"
      onClick={onClick}
      style={{ width: 26, height: 26, borderRadius: 9, fontSize: 12 }}
    >
      {label}
    </button>
  );
}

function Notice({ title, body }: { title: string; body: string }) {
  return (
    <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
      <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
        <Icon path={ICONS.warning} size={16} strokeWidth={2} />
        {title}
      </span>
      <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.55 }}>{body}</span>
    </div>
  );
}

/** The same words the engine uses, so a recipe reads the same everywhere. */
function describe(step: Step): string {
  switch (step.step) {
    case "resize":
      return `Resize to ${step.longest} px on the longest edge`;
    case "fit":
      return `Crop and fit to ${step.width}×${step.height}`;
    case "aspect":
      return `Crop to a ${step.ratio.toFixed(2)} shape`;
    case "rotate":
      return `Turn ${step.degrees}°`;
    case "adjust":
      return "Correct levels and colour";
    case "watermark":
      return `Stamp "${step.text}" on it`;
    case "convert":
      return `Save as ${step.format === "jpeg" ? "JPG" : step.format === "png" ? "PNG" : "it is"}`;
    case "compress":
      return `Compress to ${
        step.target.kind === "range"
          ? `${Math.round(step.target.value.min / 1000)}–${Math.round(step.target.value.max / 1000)} KB`
          : step.target.kind === "at_most"
            ? `under ${Math.round(step.target.value / 1000)} KB`
            : `about ${Math.round(step.target.value.bytes / 1000)} KB`
      }`;
  }
}

function blank(): Chain {
  return {
    id: `recipe-${Date.now().toString(36)}`,
    name: "My recipe",
    steps: [
      { step: "fit", width: 200, height: 230 },
      {
        step: "compress",
        target: { kind: "range", value: { min: 20000, max: 50000 } },
        bias: "balanced",
      },
    ],
  };
}
