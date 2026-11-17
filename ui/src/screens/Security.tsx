import { useEffect, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  formatSize,
  isDesktop,
  isPdf,
  pdfSecurity,
  protectPdf,
  revealOutput,
  unlockPdf,
} from "../lib/bridge";
import type { PdfStrength, SecurityInfo, SecurityResult } from "../lib/types";

export type SecurityMode = "protect" | "unlock";

/**
 * Passwords and permissions.
 *
 * Protect and unlock share a screen because they share the thing you need to
 * see first: what protection the document has right now. "Opens freely but
 * won't print" and "needs a password to open at all" are completely different
 * situations, and every other tool makes you guess which one you're in.
 */
export function Security({
  mode,
  files,
  onFiles,
  onBack,
}: {
  mode: SecurityMode;
  files: File[];
  onFiles: (files: File[]) => void;
  onBack: () => void;
}) {
  const file = files.find(isPdf);
  const [info, setInfo] = useState<SecurityInfo | null>(null);
  const [result, setResult] = useState<SecurityResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const [openPassword, setOpenPassword] = useState("");
  const [ownerPassword, setOwnerPassword] = useState("");
  const [unlockPassword, setUnlockPassword] = useState("");
  const [reveal, setReveal] = useState(false);
  const [allowPrinting, setAllowPrinting] = useState(true);
  const [allowCopying, setAllowCopying] = useState(true);
  const [allowModifying, setAllowModifying] = useState(false);
  const [allowAnnotating, setAllowAnnotating] = useState(false);
  const [strength, setStrength] = useState<PdfStrength>("aes256");

  useEffect(() => {
    setResult(null);
    setError(null);
    setInfo(null);
    if (!file) return;
    let live = true;
    pdfSecurity(file)
      .then((i) => live && setInfo(i))
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
      if (mode === "protect") {
        setResult(
          await protectPdf(file, {
            openPassword: openPassword || null,
            ownerPassword: ownerPassword || null,
            allowPrinting,
            allowCopying,
            allowModifying,
            allowAnnotating,
            strength,
          }),
        );
      } else {
        setResult(await unlockPdf(file, unlockPassword || null));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const title = mode === "protect" ? "Protect PDF" : "Unlock PDF";
  const canRun =
    !!file &&
    !busy &&
    (mode === "unlock" || openPassword.length > 0 || ownerPassword.length > 0);

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
              {title}
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
          style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 20, minWidth: 0 }}
        >
          {!file ? (
            <Empty
              title={isDesktop() ? "Open a PDF" : "This needs the desktop app"}
              body={
                isDesktop()
                  ? mode === "protect"
                    ? "Add a password, and choose whether printing, copying and editing stay allowed."
                    : "Remove a password you know, or lift the restrictions from a document that already opens freely."
                  : "The browser preview can compress images but not handle document encryption. Open Kitbox itself."
              }
            />
          ) : (
            <>
              <div className="col" style={{ gap: 6 }}>
                <span className="section-heading">Right now</span>
                <span className="display" style={{ fontSize: 19, lineHeight: 1.4 }}>
                  {info ? info.summary : "Reading the document…"}
                </span>
                <span style={{ fontSize: 12.5, color: "var(--text-subtle)" }}>
                  {file.name} · {formatSize(file.size)}
                </span>
              </div>

              {info && (
                <div className="row" style={{ gap: 10, flexWrap: "wrap" }}>
                  <Chip label="Printing" allowed={info.canPrint} />
                  <Chip label="Copying text" allowed={info.canCopy} />
                  <Chip label="Editing" allowed={info.canModify} />
                  <Chip label="Annotating" allowed={info.canAnnotate} />
                </div>
              )}

              {info && mode === "protect" && info.encrypted && (
                <Notice>
                  This document is already protected. Unlock it first, then protect it again with
                  the settings you want.
                </Notice>
              )}

              {info && mode === "unlock" && !info.encrypted && (
                <Notice>
                  This document has no protection on it, so there is nothing to remove.
                </Notice>
              )}

              {info && mode === "unlock" && info.encrypted && !info.needsPasswordToOpen && (
                <Notice>
                  This one opens without a password — only printing and copying are restricted.
                  Leave the password box empty and these restrictions come off.
                </Notice>
              )}

              {mode === "unlock" && (
                <Notice>
                  Kitbox removes protection from documents you can open. It does not guess or
                  attack passwords, so a file you can't open stays closed.
                </Notice>
              )}

              {result && (
                <div className="pressed col" style={{ borderRadius: 20, padding: 18, gap: 10 }}>
                  <span className="display" style={{ fontSize: 20 }}>
                    {mode === "protect" ? "Protected" : "Unlocked"} · {formatSize(result.size)}
                  </span>
                  <span style={{ fontSize: 13, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                    {result.info.summary}
                  </span>
                  <span style={{ fontSize: 12, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                    Not added to history — Kitbox doesn't keep its own copy of a document you just
                    put a password on, or the open version of one you unlocked.
                  </span>
                  <button
                    type="button"
                    className="raised-sm btn"
                    style={{ minHeight: 40, alignSelf: "flex-start" }}
                    onClick={() => revealOutput(result.outputPath)}
                  >
                    <Icon path={ICONS.save} size={16} strokeWidth={2} />
                    Show in folder
                  </button>
                </div>
              )}
            </>
          )}
        </section>

        <aside
          className="raised col scroll"
          aria-label={`${title} settings`}
          style={{ flex: "0 0 320px", borderRadius: "var(--r-panel)", padding: 22, gap: 20 }}
        >
          {mode === "protect" ? (
            <>
              <Password
                label="Password to open"
                hint="Leave empty to restrict printing and copying without locking the document"
                value={openPassword}
                onChange={setOpenPassword}
                reveal={reveal}
                onReveal={setReveal}
              />
              <Password
                label="Owner password"
                hint="Controls the permissions below. Defaults to the password above."
                value={ownerPassword}
                onChange={setOwnerPassword}
                reveal={reveal}
                onReveal={setReveal}
              />

              <div className="col" style={{ gap: 12 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Still allowed
                </span>
                <Switch label="Printing" on={allowPrinting} onChange={setAllowPrinting} />
                <Switch label="Copying text" on={allowCopying} onChange={setAllowCopying} />
                <Switch label="Editing" on={allowModifying} onChange={setAllowModifying} />
                <Switch label="Annotating" on={allowAnnotating} onChange={setAllowAnnotating} />
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  Screen readers can always extract text, whatever these say. Refusing that locks
                  out people, not copying.
                </span>
              </div>

              <div className="col" style={{ gap: 10 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
                  Encryption
                </span>
                <div className="pressed segmented" style={{ gridTemplateColumns: "1fr 1fr" }}>
                  {(
                    [
                      ["aes256", "AES-256"],
                      ["aes128", "AES-128"],
                    ] as [PdfStrength, string][]
                  ).map(([id, label]) => (
                    <button
                      key={id}
                      type="button"
                      aria-pressed={strength === id}
                      className={strength === id ? "raised-sm" : undefined}
                      onClick={() => setStrength(id)}
                    >
                      {label}
                    </button>
                  ))}
                </div>
                <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>
                  AES-256 unless you need a reader older than about 2010.
                </span>
              </div>
            </>
          ) : (
            <Password
              label="Password"
              hint={
                info && info.encrypted && !info.needsPasswordToOpen
                  ? "Not needed for this document — leave it empty"
                  : "The password that opens this document"
              }
              value={unlockPassword}
              onChange={setUnlockPassword}
              reveal={reveal}
              onReveal={setReveal}
            />
          )}

          {error && (
            <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
              <span className="row" style={{ gap: 8, fontSize: 13, fontWeight: 600 }}>
                <Icon path={ICONS.warning} size={16} strokeWidth={2} />
                That didn't work
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
            disabled={!canRun}
            onClick={run}
          >
            {busy
              ? mode === "protect"
                ? "Protecting…"
                : "Unlocking…"
              : mode === "protect"
                ? "Protect"
                : "Remove protection"}
          </button>
          {mode === "protect" && !openPassword && !ownerPassword && file && (
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", textAlign: "center" }}>
              Set at least one password
            </span>
          )}
        </aside>
      </div>
    </div>
  );
}

function Password({
  label,
  hint,
  value,
  onChange,
  reveal,
  onReveal,
}: {
  label: string;
  hint: string;
  value: string;
  onChange: (value: string) => void;
  reveal: boolean;
  onReveal: (reveal: boolean) => void;
}) {
  const id = `pw-${label.replace(/\s+/g, "-").toLowerCase()}`;
  return (
    <div className="col" style={{ gap: 8 }}>
      <label htmlFor={id} style={{ fontSize: 13, fontWeight: 600, color: "var(--text-muted)" }}>
        {label}
      </label>
      <div className="pressed field" style={{ padding: "0 8px 0 14px" }}>
        <input
          id={id}
          type={reveal ? "text" : "password"}
          autoComplete="new-password"
          spellCheck={false}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          style={{ fontSize: 14 }}
        />
        <button
          type="button"
          aria-label={reveal ? "Hide passwords" : "Show passwords"}
          aria-pressed={reveal}
          onClick={() => onReveal(!reveal)}
          className="raised-sm"
          style={{ width: 32, height: 32, borderRadius: 9, display: "grid", placeItems: "center" }}
        >
          <Icon
            path={
              reveal
                ? "M3 3l18 18 M10.6 10.6a2 2 0 0 0 2.8 2.8 M9.4 5.2A9.5 9.5 0 0 1 12 5c5 0 9 4.5 9 7a12 12 0 0 1-2.4 3.2 M6.3 6.4A12.4 12.4 0 0 0 3 12c0 2.5 4 7 9 7a9.6 9.6 0 0 0 3.4-.6"
                : "M3 12s4-7 9-7 9 7 9 7-4 7-9 7-9-7-9-7z M12 10a2 2 0 1 0 0 4a2 2 0 1 0 0-4"
            }
            size={15}
            strokeWidth={2}
          />
        </button>
      </div>
      <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.5 }}>{hint}</span>
    </div>
  );
}

function Switch({
  label,
  on,
  onChange,
}: {
  label: string;
  on: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 12,
        minHeight: 40,
        textAlign: "left",
        fontSize: 14,
        fontWeight: 600,
      }}
    >
      {label}
      <span className="pressed switch">
        <span style={{ left: on ? 30 : 4, background: on ? "var(--accent)" : "var(--surface)" }} />
      </span>
    </button>
  );
}

function Chip({ label, allowed }: { label: string; allowed: boolean }) {
  return (
    <span
      className="pressed"
      style={{
        borderRadius: 999,
        padding: "7px 14px",
        fontSize: 12,
        fontWeight: 600,
        color: allowed ? "var(--text-muted)" : "var(--accent)",
      }}
    >
      {allowed ? label : `No ${label.toLowerCase()}`}
    </span>
  );
}

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="pressed"
      style={{
        borderRadius: 16,
        padding: 14,
        fontSize: 12.5,
        color: "var(--text-subtle)",
        lineHeight: 1.6,
      }}
    >
      {children}
    </div>
  );
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="col" style={{ gap: 10, padding: "40px 8px" }}>
      <h2 className="display" style={{ fontSize: 20 }}>
        {title}
      </h2>
      <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.6 }}>{body}</p>
    </div>
  );
}
