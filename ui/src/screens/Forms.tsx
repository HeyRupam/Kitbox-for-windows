import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { fillForm, formFields, isDesktop, isPdf, revealOutput } from "../lib/bridge";
import type { FillResult, FormField } from "../lib/types";

/**
 * Fill in a PDF form.
 *
 * The form tells us what it wants — names, types, which answers a checkbox
 * will accept, what's required — so the screen builds the right control for
 * each field instead of showing a wall of text boxes. A dropdown that only
 * takes three values gets three buttons, not a free-text field that silently
 * refuses everything else.
 *
 * Flattening is offered rather than assumed, and described as what it is:
 * one-way.
 */
export function Forms({
  files,
  onFiles,
  onBack,
}: {
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const pdf = files.find(isPdf) ?? null;

  const [fields, setFields] = useState<FormField[] | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [flatten, setFlatten] = useState(false);
  const [result, setResult] = useState<FillResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!pdf || !isDesktop()) return;
    let cancelled = false;
    setError(null);
    formFields(pdf)
      .then((found) => {
        if (cancelled) return;
        setFields(found);
        // Start from what the form already says, so an existing answer isn't
        // wiped by saving without touching it.
        setValues(
          Object.fromEntries(
            found.filter((f) => f.value).map((f) => [f.name, f.value as string]),
          ),
        );
      })
      .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      cancelled = true;
    };
  }, [pdf]);

  const editable = (fields ?? []).filter(
    (f) => f.kind !== "button" && f.kind !== "signature" && !f.readOnly,
  );
  const missing = editable.filter(
    (f) => f.required && !(values[f.name] ?? "").trim(),
  );

  const set = (name: string, value: string) => {
    setValues({ ...values, [name]: value });
    setResult(null);
  };

  const run = async () => {
    if (!pdf) return;
    setBusy(true);
    setError(null);
    try {
      // Only what can actually be written: sending a read-only field back
      // would just collect a refusal in the report.
      const writable = Object.fromEntries(
        Object.entries(values).filter(([name]) => editable.some((f) => f.name === name)),
      );
      setResult(await fillForm(pdf, { values: writable, flatten }));
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
            <span className="section-heading">Edit PDF</span>
            <h1 className="display" style={{ fontSize: 26 }}>
              Fill a form
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
              setFields(null);
              setValues({});
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
          style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 18, minWidth: 0 }}
        >
          {!pdf ? (
            <Empty
              title={isDesktop() ? "Choose the form" : "This needs the desktop app"}
              body={
                isDesktop()
                  ? "A fillable PDF has real fields in it. Kitbox reads what each one wants and builds the right box for it."
                  : "The browser preview can compress images but not edit documents. Open Kitbox itself."
              }
            />
          ) : fields === null ? (
            <Empty title="Reading the form…" body="" />
          ) : fields.length === 0 ? (
            <Empty
              title="There's no form in this PDF"
              body="A form that was scanned or printed to PDF is a picture of a form — there are no fields to type into. Use Sign PDF to place text on it, or fill it by hand."
            />
          ) : (
            <>
              <span className="section-heading">
                {fields.length} field{fields.length === 1 ? "" : "s"}
              </span>
              {fields.map((field) => (
                <Control
                  key={field.name}
                  field={field}
                  value={values[field.name] ?? ""}
                  onChange={(v) => set(field.name, v)}
                />
              ))}
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          style={{ flex: "0 0 300px", borderRadius: "var(--r-panel)", padding: 22, gap: 18 }}
        >
          {missing.length > 0 && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span style={{ fontSize: 13, fontWeight: 600 }}>
                {missing.length} required field{missing.length === 1 ? "" : "s"} still empty
              </span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                {missing.map((f) => f.label ?? f.name).join(", ")}
              </span>
            </div>
          )}

          <label className="row" style={{ gap: 10, alignItems: "flex-start", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={flatten}
              onChange={(e) => {
                setFlatten(e.target.checked);
                setResult(null);
              }}
              style={{ marginTop: 3 }}
            />
            <span className="col" style={{ gap: 3 }}>
              <span style={{ fontSize: 13, fontWeight: 600 }}>Lock the answers in</span>
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                Draws the values into the pages and removes the form, so nothing can be edited or
                cleared by accident. It can't be undone — the unflattened copy stays in your
                history.
              </span>
            </span>
          </label>

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
                {result.filled.length} filled{result.flattened ? ", locked" : ""}
              </span>
              {result.readOnly.length > 0 && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  The form won't let these be changed: {result.readOnly.join(", ")}
                </span>
              )}
              {result.stillRequired.length > 0 && (
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  Still required: {result.stillRequired.join(", ")}
                </span>
              )}
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

          <button
            type="button"
            className="btn btn-primary"
            style={{ minHeight: 56, borderRadius: 18, fontSize: 15, fontWeight: 700 }}
            disabled={!fields || fields.length === 0 || busy}
            onClick={run}
          >
            {busy ? "Filling…" : flatten ? "Fill and lock" : "Fill the form"}
          </button>
        </aside>
      </div>
    </div>
  );
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
      <h2 className="display" style={{ fontSize: 20 }}>
        {title}
      </h2>
      {body && (
        <p style={{ color: "var(--text-subtle)", maxWidth: 560, lineHeight: 1.65 }}>{body}</p>
      )}
    </div>
  );
}

function Control({
  field,
  value,
  onChange,
}: {
  field: FormField;
  value: string;
  onChange: (value: string) => void;
}) {
  const locked = field.readOnly || field.kind === "button" || field.kind === "signature";
  const label = field.label ?? field.name;

  const box: React.CSSProperties = {
    borderRadius: 12,
    minHeight: 44,
    padding: "10px 13px",
    fontSize: 13,
    border: "none",
    color: "var(--text)",
    width: "100%",
    opacity: locked ? 0.55 : 1,
    fontFamily: "inherit",
  };

  return (
    <div className="col" style={{ gap: 7, maxWidth: 620 }}>
      <span className="row" style={{ gap: 7, alignItems: "baseline", flexWrap: "wrap" }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
        {field.required && (
          <span style={{ fontSize: 11, color: "var(--accent)", fontWeight: 700 }}>required</span>
        )}
        {locked && (
          <span style={{ fontSize: 11, color: "var(--text-subtle)" }}>
            {field.kind === "signature" ? "signature field" : "not editable"}
          </span>
        )}
        {field.label && field.label !== field.name && (
          <span style={{ fontSize: 11, color: "var(--text-subtle)" }}>{field.name}</span>
        )}
      </span>

      {field.kind === "checkbox" ? (
        <button
          type="button"
          role="switch"
          aria-checked={value !== "" && value.toLowerCase() !== "off"}
          disabled={locked}
          className={value !== "" && value.toLowerCase() !== "off" ? "pressed" : "raised-sm"}
          onClick={() => onChange(value && value.toLowerCase() !== "off" ? "off" : "yes")}
          style={{
            borderRadius: 12,
            minHeight: 44,
            padding: "0 16px",
            alignSelf: "flex-start",
            fontSize: 13,
            fontWeight: 600,
            color: value && value.toLowerCase() !== "off" ? "var(--accent)" : undefined,
          }}
        >
          {value && value.toLowerCase() !== "off" ? "Ticked" : "Not ticked"}
        </button>
      ) : field.kind === "radio" || (field.kind === "choice" && field.options.length > 0) ? (
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          {field.options.map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={value === option}
              disabled={locked}
              className={value === option ? "pressed" : "raised-sm"}
              onClick={() => onChange(value === option ? "" : option)}
              style={{
                borderRadius: 12,
                minHeight: 42,
                padding: "0 15px",
                fontSize: 12.5,
                fontWeight: value === option ? 700 : 500,
                color: value === option ? "var(--accent)" : undefined,
              }}
            >
              {option}
            </button>
          ))}
        </div>
      ) : field.kind === "multiline" ? (
        <textarea
          value={value}
          disabled={locked}
          aria-label={label}
          rows={3}
          maxLength={field.maxLength}
          onChange={(e) => onChange(e.target.value)}
          className="pressed"
          style={{ ...box, resize: "vertical", lineHeight: 1.5 }}
        />
      ) : (
        <input
          type={field.kind === "password" ? "password" : "text"}
          value={value}
          disabled={locked}
          aria-label={label}
          maxLength={field.maxLength}
          onChange={(e) => onChange(e.target.value)}
          className="pressed"
          style={box}
        />
      )}

      {field.maxLength && !locked && (
        <span style={{ fontSize: 11, color: "var(--text-subtle)" }}>
          {value.length} of {field.maxLength} characters
        </span>
      )}
    </div>
  );
}
