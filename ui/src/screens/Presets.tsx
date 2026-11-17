import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  checkPreset,
  deletePreset,
  fixForPreset,
  formatSize,
  isDesktop,
  revealOutput,
  savePreset,
} from "../lib/bridge";
import { blankPreset, byGroup, describePreset, loadPresets } from "../lib/presets";
import type { LoadedPresets, Preset, PresetEntry, PresetReport } from "../lib/types";

/**
 * Form presets: what a form demands, as something you can check a file against.
 *
 * Two things make this more than a list of numbers. Checking reports every
 * requirement it compared, passed or failed, so you can hold it against the
 * form on screen — a verdict you can't see the workings of is no use when the
 * portal just says "invalid". And every preset is editable, because the rules
 * change between cycles and the person filling the form can see the real ones.
 */
export function Presets({ onBack, onUse }: { onBack: () => void; onUse: (p: Preset) => void }) {
  const [loaded, setLoaded] = useState<LoadedPresets | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<PresetEntry | null>(null);
  const [editing, setEditing] = useState<Preset | null>(null);

  useEffect(() => {
    loadPresets()
      .then(setLoaded)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const refresh = (next: LoadedPresets, keep?: string) => {
    setLoaded(next);
    setEditing(null);
    setOpen(keep ? (next.presets.find((p) => p.id === keep) ?? null) : null);
  };

  return (
    <div className="col grow scroll" style={{ gap: 22, paddingRight: 6 }}>
      <header className="row" style={{ gap: 16, justifyContent: "space-between", flexWrap: "wrap" }}>
        <div className="row" style={{ gap: 16 }}>
          <button
            type="button"
            className="raised-sm icon-btn"
            aria-label={open || editing ? "Back to all presets" : "Back to all tools"}
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
            <span className="section-heading">Forms</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              {open ? open.name : "Form presets"}
            </h1>
          </div>
        </div>

        {!open && !editing && (
          <div className="row" style={{ gap: 8 }}>
            {loaded?.folder && (
              <button
                type="button"
                className="raised-sm btn"
                onClick={() => revealOutput(loaded.readme)}
                title={loaded.folder}
              >
                <Icon path={ICONS.save} size={18} strokeWidth={2} />
                Open the folder
              </button>
            )}
            {isDesktop() && (
              <button
                type="button"
                className="raised-sm btn"
                onClick={() => setEditing(blankPreset())}
              >
                <Icon path={ICONS.plus} size={18} strokeWidth={2} />
                New preset
              </button>
            )}
          </div>
        )}
      </header>

      {error && <Notice title="The presets couldn't be loaded" body={error} />}

      {loaded?.problems.map((p) => (
        <Notice
          key={p.file}
          title={`${p.file} couldn't be read`}
          body={`${p.why} — the rest still loaded. Fix the file and reopen this screen.`}
        />
      ))}

      {editing ? (
        <Editor
          preset={editing}
          onCancel={() => setEditing(null)}
          onSave={async (preset) => {
            const next = await savePreset(preset);
            refresh(next, preset.id);
          }}
        />
      ) : open ? (
        <Detail
          entry={open}
          onUse={() => onUse(open)}
          onEdit={() => setEditing({ ...open })}
          onForget={async () => refresh(await deletePreset(open.id))}
        />
      ) : (
        <>
          <div className="raised col" style={{ borderRadius: 20, padding: "16px 20px", gap: 6 }}>
            <span style={{ fontSize: 13, fontWeight: 600 }}>Check these against your form</span>
            <span
              style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6, maxWidth: 640 }}
            >
              These are starting points gathered from common application forms, not official
              rules. Boards change requirements between cycles and portals for the same exam
              sometimes disagree, so confirm the numbers on the page you're filling — then edit
              the preset, and it stays edited.
            </span>
          </div>

          {loaded &&
            byGroup(loaded.presets).map(({ group, items }) => (
              <section key={group} className="col" style={{ gap: 14 }}>
                <h2 className="section-heading">{group}</h2>
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))",
                    gap: 18,
                  }}
                >
                  {items.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      className="raised col tile"
                      style={{
                        borderRadius: 20,
                        padding: 18,
                        gap: 10,
                        alignItems: "stretch",
                        textAlign: "left",
                      }}
                      onClick={() => setOpen(p)}
                    >
                      <span className="row" style={{ gap: 8, justifyContent: "space-between" }}>
                        <span style={{ fontSize: 15, fontWeight: 600 }}>{p.name}</span>
                        {p.yours && (
                          <span
                            className="pressed pill"
                            style={{ color: "var(--accent)", flex: "0 0 auto" }}
                          >
                            yours
                          </span>
                        )}
                      </span>
                      <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                        {describePreset(p).map((part, i) => (
                          <span
                            key={part}
                            className="pressed pill"
                            style={{ color: i === 0 ? "var(--accent)" : undefined }}
                          >
                            {part}
                          </span>
                        ))}
                      </div>
                      {p.note && (
                        <span
                          style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.5 }}
                        >
                          {p.note}
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              </section>
            ))}
        </>
      )}
    </div>
  );
}

/** One preset: what it asks for, and a file measured against it. */
function Detail({
  entry,
  onUse,
  onEdit,
  onForget,
}: {
  entry: PresetEntry;
  onUse: () => void;
  onEdit: () => void;
  onForget: () => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [report, setReport] = useState<PresetReport | null>(null);
  const [fixed, setFixed] = useState<Awaited<ReturnType<typeof fixForPreset>> | null>(null);
  const [busy, setBusy] = useState<"checking" | "fixing" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const picker = useRef<HTMLInputElement>(null);

  const take = async (chosen: File | null) => {
    setFile(chosen);
    setReport(null);
    setFixed(null);
    setError(null);
    if (!chosen) return;
    setBusy("checking");
    try {
      setReport(await checkPreset(chosen, entry));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const fix = async () => {
    if (!file) return;
    setBusy("fixing");
    setError(null);
    try {
      const result = await fixForPreset(file, entry);
      setFixed(result);
      setReport(result.report);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="row" style={{ gap: 22, alignItems: "flex-start", flexWrap: "wrap" }}>
      <section
        className="raised col grow"
        style={{ borderRadius: "var(--r-panel)", padding: 22, gap: 16, minWidth: 320 }}
      >
        <span className="section-heading">What this form wants</span>
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          {describePreset(entry).map((part, i) => (
            <span
              key={part}
              className="pressed pill"
              style={{ color: i === 0 ? "var(--accent)" : undefined }}
            >
              {part}
            </span>
          ))}
        </div>
        {entry.note && (
          <span style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
            {entry.note}
          </span>
        )}

        <input
          ref={picker}
          type="file"
          hidden
          accept={entry.kind === "pdf" ? "application/pdf" : "image/*"}
          onChange={(e) => take(e.target.files?.[0] ?? null)}
        />

        {!file ? (
          <div className="pressed col" style={{ borderRadius: 18, padding: 22, gap: 12 }}>
            <span style={{ fontSize: 13.5, fontWeight: 600 }}>
              Check a {entry.kind === "pdf" ? "document" : "picture"} against it
            </span>
            <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6 }}>
              Nothing is changed by checking. You get the list of what was compared, and the
              option to fix whatever doesn't match.
            </span>
            <button
              type="button"
              className="raised-sm btn"
              style={{ alignSelf: "flex-start", minHeight: 42 }}
              onClick={() => picker.current?.click()}
            >
              Choose a file
            </button>
          </div>
        ) : (
          <div className="col" style={{ gap: 12 }}>
            <div className="row" style={{ justifyContent: "space-between", gap: 12 }}>
              <span className="truncate" style={{ fontSize: 13, fontWeight: 600, minWidth: 0 }}>
                {file.name}
              </span>
              <button
                type="button"
                className="raised-sm"
                style={{ borderRadius: 999, padding: "4px 13px", fontSize: 11.5 }}
                onClick={() => picker.current?.click()}
              >
                Another file
              </button>
            </div>

            {busy && (
              <span style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
                {busy === "checking" ? "Measuring…" : "Fixing it…"}
              </span>
            )}

            {report && <Checklist report={report} />}

            {report && !report.fits && !fixed && (
              <button
                type="button"
                className="btn btn-primary"
                style={{ minHeight: 48, borderRadius: 16, fontSize: 14.5, fontWeight: 700 }}
                disabled={!!busy}
                onClick={fix}
              >
                Fix it for this form
              </button>
            )}

            {fixed && (
              <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  {fixed.report.fits ? "Ready to upload" : "As close as it goes"} —{" "}
                  {formatSize(fixed.size)}
                </span>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                  {fixed.steps.join(", ")}.
                </span>
                {fixed.shortfall && (
                  <span style={{ fontSize: 11.5, color: "var(--accent)", lineHeight: 1.6 }}>
                    {fixed.shortfall}
                  </span>
                )}
                <button
                  type="button"
                  className="raised-sm btn"
                  style={{ alignSelf: "flex-start", minHeight: 40 }}
                  onClick={() => revealOutput(fixed.outputPath)}
                >
                  <Icon path={ICONS.save} size={16} strokeWidth={2} />
                  Show in folder
                </button>
              </div>
            )}

            {error && <Notice title="That didn't work" body={error} />}
          </div>
        )}
      </section>

      <aside
        className="raised col"
        style={{ flex: "0 0 280px", borderRadius: "var(--r-panel)", padding: 22, gap: 12 }}
      >
        {fixed?.preview && (
          <img
            src={fixed.preview}
            alt="The fixed file"
            className="pressed"
            style={{ borderRadius: 14, width: "100%", objectFit: "contain", padding: 8 }}
          />
        )}
        <button
          type="button"
          className="raised-sm btn"
          style={{ minHeight: 44 }}
          onClick={onUse}
          disabled={entry.kind === "pdf"}
          title={entry.kind === "pdf" ? "Open a PDF in Compress and pick this there" : undefined}
        >
          Use it in Compress
        </button>
        <button type="button" className="raised-sm btn" style={{ minHeight: 44 }} onClick={onEdit}>
          Edit these numbers
        </button>
        {entry.yours && (
          <button
            type="button"
            className="raised-sm btn"
            style={{ minHeight: 44 }}
            onClick={onForget}
          >
            {entry.overridesBundled ? "Restore the bundled one" : "Delete this preset"}
          </button>
        )}
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          {entry.yours
            ? entry.overridesBundled
              ? "Your version of a bundled preset. The original is still there underneath."
              : "Yours, kept in the presets folder."
            : "Bundled. Editing it writes your own copy and leaves this one alone."}
        </span>
      </aside>
    </div>
  );
}

/** Every requirement that was compared, passed or failed. */
function Checklist({ report }: { report: PresetReport }) {
  return (
    <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 10 }}>
      <span style={{ fontSize: 13, fontWeight: 600, color: report.fits ? undefined : "var(--accent)" }}>
        {report.fits ? "This file would be accepted" : "This file would be rejected"}
      </span>
      {report.checks.map((check) => (
        <div key={check.what} className="row" style={{ gap: 10, alignItems: "flex-start" }}>
          <span
            aria-hidden
            style={{
              flex: "0 0 auto",
              marginTop: 2,
              color: check.verdict === "wrong" ? "var(--accent)" : "var(--text-subtle)",
            }}
          >
            <Icon
              path={check.verdict === "ok" ? ICONS.check : check.verdict === "wrong" ? ICONS.close : ICONS.warning}
              size={15}
              strokeWidth={2}
            />
          </span>
          <span className="col" style={{ gap: 1, minWidth: 0 }}>
            <span style={{ fontSize: 12.5, fontWeight: 600 }}>
              {check.what}: {check.has}
            </span>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
              {check.verdict === "unknown"
                ? `the form wants ${check.wants}, and the file doesn't say`
                : `wants ${check.wants}`}
            </span>
          </span>
        </div>
      ))}
    </div>
  );
}

/** The numbers, as a form you can change. */
function Editor({
  preset,
  onSave,
  onCancel,
}: {
  preset: Preset;
  onSave: (p: Preset) => Promise<void>;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<Preset>(preset);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = (patch: Partial<Preset>) => setDraft({ ...draft, ...patch });
  const min = draft.target.kind === "range" ? draft.target.value.min : 0;
  const max =
    draft.target.kind === "range"
      ? draft.target.value.max
      : draft.target.kind === "at_most"
        ? draft.target.value
        : draft.target.value.bytes;

  const setSize = (nextMin: number, nextMax: number) =>
    set({
      target:
        nextMin > 0
          ? { kind: "range", value: { min: nextMin, max: nextMax } }
          : { kind: "at_most", value: nextMax },
    });

  return (
    <section
      className="raised col"
      style={{ borderRadius: "var(--r-panel)", padding: 22, gap: 16, maxWidth: 620 }}
    >
      <span className="section-heading">The numbers this form states</span>

      <Field label="Name">
        <input
          className="pressed"
          style={input}
          value={draft.name}
          onChange={(e) => set({ name: e.target.value })}
        />
      </Field>

      <div className="row" style={{ gap: 12 }}>
        <Field label="Group">
          <input
            className="pressed"
            style={input}
            value={draft.group}
            onChange={(e) => set({ group: e.target.value })}
          />
        </Field>
        <Field label="This is a">
          <Segmented
            options={[
              ["image", "Picture"],
              ["pdf", "PDF"],
            ]}
            value={draft.kind}
            onChange={(v) =>
              set({
                kind: v as Preset["kind"],
                format: v === "pdf" ? "pdf" : "jpeg",
                dimensions: v === "pdf" ? undefined : draft.dimensions,
              })
            }
          />
        </Field>
      </div>

      <div className="row" style={{ gap: 12 }}>
        <Field label="Smallest, in KB (0 for no minimum)">
          <input
            type="number"
            min={0}
            className="pressed"
            style={input}
            value={Math.round(min / 1000)}
            onChange={(e) => setSize(Math.max(0, Number(e.target.value) || 0) * 1000, max)}
          />
        </Field>
        <Field label="Largest, in KB">
          <input
            type="number"
            min={1}
            className="pressed"
            style={input}
            value={Math.round(max / 1000)}
            onChange={(e) => setSize(min, Math.max(1, Number(e.target.value) || 1) * 1000)}
          />
        </Field>
      </div>

      {draft.kind === "image" && (
        <>
          <Field label="Pixel size">
            <div className="row" style={{ gap: 10, alignItems: "center" }}>
              <input
                type="number"
                min={0}
                aria-label="Width in pixels"
                className="pressed"
                style={{ ...input, width: 110 }}
                value={draft.dimensions?.width ?? 0}
                onChange={(e) =>
                  set({
                    dimensions: {
                      fit: draft.dimensions?.fit ?? "exact",
                      height: draft.dimensions?.height ?? 0,
                      width: Math.max(0, Number(e.target.value) || 0),
                    },
                  })
                }
              />
              <span style={{ color: "var(--text-subtle)" }}>×</span>
              <input
                type="number"
                min={0}
                aria-label="Height in pixels"
                className="pressed"
                style={{ ...input, width: 110 }}
                value={draft.dimensions?.height ?? 0}
                onChange={(e) =>
                  set({
                    dimensions: {
                      fit: draft.dimensions?.fit ?? "exact",
                      width: draft.dimensions?.width ?? 0,
                      height: Math.max(0, Number(e.target.value) || 0),
                    },
                  })
                }
              />
              <button
                type="button"
                className="raised-sm"
                style={{ borderRadius: 999, padding: "6px 14px", fontSize: 11.5 }}
                onClick={() => set({ dimensions: undefined })}
              >
                No pixel rule
              </button>
            </div>
          </Field>

          {draft.dimensions && (
            <Field label="Read that as">
              <Segmented
                options={[
                  ["exact", "Exactly"],
                  ["at_least", "At least"],
                  ["at_most", "At most"],
                ]}
                value={draft.dimensions.fit}
                onChange={(v) =>
                  set({ dimensions: { ...draft.dimensions!, fit: v as "exact" } })
                }
              />
            </Field>
          )}

          <div className="row" style={{ gap: 12 }}>
            <Field label="Format">
              <Segmented
                options={[
                  ["jpeg", "JPG"],
                  ["png", "PNG"],
                  ["any", "Any"],
                ]}
                value={draft.format}
                onChange={(v) => set({ format: v as Preset["format"] })}
              />
            </Field>
            <Field label="Resolution it asks for (0 if none)">
              <input
                type="number"
                min={0}
                className="pressed"
                style={input}
                value={draft.dpi ?? 0}
                onChange={(e) => set({ dpi: Math.max(0, Number(e.target.value) || 0) || undefined })}
              />
            </Field>
          </div>
        </>
      )}

      <Field label="Note — where these numbers came from">
        <input
          className="pressed"
          style={input}
          value={draft.note ?? ""}
          placeholder="What the form actually said"
          onChange={(e) => set({ note: e.target.value || undefined })}
        />
      </Field>

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
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
        Saved into your presets folder as a file you can also edit by hand.
      </span>
    </section>
  );
}

const input: React.CSSProperties = {
  borderRadius: 12,
  minHeight: 44,
  padding: "0 13px",
  fontSize: 13,
  border: "none",
  color: "var(--text)",
  width: "100%",
  fontFamily: "inherit",
};

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="col grow" style={{ gap: 7, minWidth: 0 }}>
      <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
      {children}
    </label>
  );
}

function Segmented({
  options,
  value,
  onChange,
}: {
  options: [string, string][];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="row" style={{ gap: 8 }}>
      {options.map(([id, label]) => (
        <button
          key={id}
          type="button"
          aria-pressed={value === id}
          className={value === id ? "pressed" : "raised-sm"}
          onClick={() => onChange(id)}
          style={{
            borderRadius: 12,
            minHeight: 44,
            padding: "0 16px",
            fontSize: 12.5,
            fontWeight: value === id ? 700 : 500,
            color: value === id ? "var(--accent)" : undefined,
          }}
        >
          {label}
        </button>
      ))}
    </div>
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
