import { useEffect, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import {
  clipboardKey,
  formatSize,
  isDesktop,
  outputFolder,
  revealOutput,
  setClipboardKey,
  setShellMenu,
  shellMenu,
  useClipboard,
  pickFolder,
  setWatchedFolders,
  watchedFolders,
} from "../lib/bridge";
import type { Hotkey, Pressed, ShellMenuState, Watch } from "../lib/types";

/**
 * The few things worth being able to change.
 *
 * Kept deliberately short: a settings screen is where features go to hide, and
 * everything here either changes something outside the app (the Windows menu)
 * or says where your files are.
 */
export function Settings({ onBack }: { onBack: () => void }) {
  const [menu, setMenu] = useState<ShellMenuState | null>(null);
  const [key, setKey] = useState<Hotkey | null>(null);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [pressed, setPressed] = useState<Pressed | null>(null);
  const [watches, setWatches] = useState<Watch[]>([]);
  const [watchError, setWatchError] = useState<string | null>(null);
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    shellMenu().then(setMenu).catch(() => setMenu(null));
    clipboardKey().then(setKey).catch(() => setKey(null));
    watchedFolders().then(setWatches).catch(() => setWatches([]));
    outputFolder()
      .then((path) => setFolder(path ?? ""))
      .catch(() => setFolder(""));
  }, []);

  const toggle = async () => {
    if (!menu) return;
    setBusy(true);
    setError(null);
    try {
      setMenu(await setShellMenu(!menu.installed));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // Saving takes the key first, so a combination something else already owns
  // is refused here rather than silently doing nothing later.
  const change = async (patch: Partial<Hotkey>) => {
    if (!key) return;
    setKeyError(null);
    try {
      setKey(await setClipboardKey({ ...key, ...patch }));
    } catch (e) {
      setKeyError(e instanceof Error ? e.message : String(e));
      setKey(await clipboardKey().catch(() => key));
    }
  };

  const changeWatches = async (next: Watch[]) => {
    setWatchError(null);
    try {
      setWatches(await setWatchedFolders(next));
    } catch (e) {
      setWatchError(e instanceof Error ? e.message : String(e));
      setWatches(await watchedFolders().catch(() => watches));
    }
  };

  const addFolder = async () => {
    const folder = await pickFolder().catch(() => null);
    if (!folder) return;
    if (watches.some((w) => w.path === folder)) return;
    await changeWatches([...watches, { path: folder, target: "100kb", enabled: true }]);
  };

  const tryNow = async () => {
    setPressed(null);
    try {
      setPressed(await useClipboard());
    } catch (e) {
      setPressed({
        ok: false,
        summary: e instanceof Error ? e.message : String(e),
        before: 0,
        after: 0,
        path: null,
      });
    }
  };

  return (
    <div className="col grow scroll" style={{ gap: 22, paddingRight: 6 }}>
      <header className="row" style={{ gap: 16 }}>
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
          <span className="section-heading">This machine</span>
          <h1 className="display" style={{ fontSize: 26 }}>
            Settings
          </h1>
        </div>
      </header>

      <section
        className="raised col"
        style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 14, maxWidth: 720 }}
      >
        <div className="row" style={{ gap: 16, justifyContent: "space-between", flexWrap: "wrap" }}>
          <div className="col" style={{ gap: 4, minWidth: 0 }}>
            <span style={{ fontSize: 15, fontWeight: 600 }}>Right-click menu</span>
            <span
              style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6, maxWidth: 520 }}
            >
              Compress a file straight from Explorer, without opening Kitbox. The result lands
              in your output folder and the window comes up with what it did.
            </span>
          </div>
          <button
            type="button"
            className={menu?.installed ? "pressed btn" : "raised-sm btn"}
            aria-pressed={!!menu?.installed}
            disabled={!menu || busy}
            onClick={toggle}
            style={{
              minHeight: 46,
              minWidth: 128,
              color: menu?.installed ? "var(--accent)" : undefined,
              fontWeight: menu?.installed ? 700 : 500,
            }}
          >
            {busy ? "…" : menu?.installed ? "On" : "Off"}
          </button>
        </div>

        {menu && (
          <div className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 8 }}>
            <span className="section-heading" style={{ fontSize: 11 }}>
              What it adds
            </span>
            {menu.items.map((item) => (
              <span key={item} style={{ fontSize: 12.5 }}>
                Kitbox ▸ {item}
              </span>
            ))}
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
              On {menu.types.join(", ")} files only. {menu.note}
            </span>
            <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
              Written for your account alone — no administrator, nothing outside your user
              registry, and turning it off removes exactly what it added.
            </span>
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
      </section>

      <section
        className="raised col"
        style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 14, maxWidth: 720 }}
      >
        <div className="row" style={{ gap: 16, justifyContent: "space-between", flexWrap: "wrap" }}>
          <div className="col" style={{ gap: 4, minWidth: 0 }}>
            <span style={{ fontSize: 15, fontWeight: 600 }}>Clipboard key</span>
            <span
              style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6, maxWidth: 520 }}
            >
              Copy a picture — a screenshot, something from a page — press the key, and the
              compressed <em>file</em> lands on your clipboard, ready to paste into an upload
              box. Not the picture: a clipboard picture is uncompressed again the moment
              anything reads it.
            </span>
          </div>
          <button
            type="button"
            className={key?.enabled ? "pressed btn" : "raised-sm btn"}
            aria-pressed={!!key?.enabled}
            disabled={!key}
            onClick={() => change({ enabled: !key?.enabled })}
            style={{
              minHeight: 46,
              minWidth: 128,
              color: key?.enabled ? "var(--accent)" : undefined,
              fontWeight: key?.enabled ? 700 : 500,
            }}
          >
            {key?.enabled ? "On" : "Off"}
          </button>
        </div>

        {key && (
          <>
            <Choice
              label="The key"
              value={key.combo}
              options={[
                ["CommandOrControl+Alt+K", "Ctrl + Alt + K"],
                ["CommandOrControl+Shift+K", "Ctrl + Shift + K"],
                ["CommandOrControl+Alt+C", "Ctrl + Alt + C"],
              ]}
              onPick={(combo) => change({ combo })}
            />
            <Choice
              label="Compress it to"
              value={key.target}
              options={[
                ["20-50kb", "20–50 KB"],
                ["50-100kb", "50–100 KB"],
                ["100kb", "under 100 KB"],
                ["500kb", "under 500 KB"],
              ]}
              onPick={(target) => change({ target })}
            />

            <div className="row" style={{ gap: 10, alignItems: "center", flexWrap: "wrap" }}>
              <button type="button" className="raised-sm btn" style={{ minHeight: 42 }} onClick={tryNow}>
                Try it on the clipboard now
              </button>
              {pressed && (
                <span
                  style={{
                    fontSize: 12,
                    color: pressed.ok ? "var(--text-subtle)" : "var(--accent)",
                    lineHeight: 1.5,
                  }}
                >
                  {pressed.ok
                    ? `${formatSize(pressed.before)} → ${formatSize(pressed.after)}, on your clipboard`
                    : pressed.summary}
                </span>
              )}
            </div>

            {keyError && (
              <span style={{ fontSize: 12, color: "var(--accent)", lineHeight: 1.6 }}>
                {keyError}
              </span>
            )}
          </>
        )}
      </section>

      <section
        className="raised col"
        style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 14, maxWidth: 720 }}
      >
        <div className="row" style={{ gap: 16, justifyContent: "space-between", flexWrap: "wrap" }}>
          <div className="col" style={{ gap: 4, minWidth: 0 }}>
            <span style={{ fontSize: 15, fontWeight: 600 }}>Watched folders</span>
            <span
              style={{ fontSize: 12.5, color: "var(--text-subtle)", lineHeight: 1.6, maxWidth: 520 }}
            >
              Point Kitbox at the folder your scanner or phone writes to, and anything that
              lands there is compressed to the size you set. Only new arrivals — what is
              already in the folder is left alone — and the originals are never touched.
            </span>
          </div>
          <button type="button" className="raised-sm btn" style={{ minHeight: 46 }} onClick={addFolder}>
            <Icon path={ICONS.plus} size={18} strokeWidth={2} />
            Watch a folder
          </button>
        </div>

        {watches.length > 0 && (
          <div className="col" style={{ gap: 10 }}>
            {watches.map((w) => (
              <div key={w.path} className="pressed col" style={{ borderRadius: 16, padding: 14, gap: 10 }}>
                <div className="row" style={{ gap: 10, justifyContent: "space-between" }}>
                  <span className="truncate" style={{ fontSize: 12.5, fontWeight: 600, minWidth: 0 }}>
                    {w.path}
                  </span>
                  <span className="row" style={{ gap: 8, flex: "0 0 auto" }}>
                    <button
                      type="button"
                      className="raised-sm"
                      aria-pressed={w.enabled}
                      style={{ borderRadius: 999, padding: "5px 13px", fontSize: 11.5, color: w.enabled ? "var(--accent)" : undefined }}
                      onClick={() =>
                        changeWatches(
                          watches.map((other) =>
                            other.path === w.path ? { ...other, enabled: !other.enabled } : other,
                          ),
                        )
                      }
                    >
                      {w.enabled ? "Watching" : "Paused"}
                    </button>
                    <button
                      type="button"
                      className="raised-sm"
                      aria-label={`Stop watching ${w.path}`}
                      style={{ borderRadius: 999, padding: "5px 11px", fontSize: 11.5 }}
                      onClick={() => changeWatches(watches.filter((other) => other.path !== w.path))}
                    >
                      Remove
                    </button>
                  </span>
                </div>
                <Choice
                  label="Compress what lands here to"
                  value={w.target}
                  options={[
                    ["20-50kb", "20–50 KB"],
                    ["50-100kb", "50–100 KB"],
                    ["100kb", "under 100 KB"],
                    ["500kb", "under 500 KB"],
                  ]}
                  onPick={(target) =>
                    changeWatches(
                      watches.map((other) => (other.path === w.path ? { ...other, target } : other)),
                    )
                  }
                />
              </div>
            ))}
          </div>
        )}

        {watchError && (
          <span style={{ fontSize: 12, color: "var(--accent)", lineHeight: 1.6 }}>{watchError}</span>
        )}
      </section>

      <section
        className="raised col"
        style={{ borderRadius: "var(--r-panel)", padding: 24, gap: 10, maxWidth: 720 }}
      >
        <span style={{ fontSize: 15, fontWeight: 600 }}>Where finished files go</span>
        <span className="pressed truncate" style={{ borderRadius: 12, padding: "10px 14px", fontSize: 12.5 }}>
          {folder || "…"}
        </span>
        <div className="row" style={{ gap: 10 }}>
          <button
            type="button"
            className="raised-sm btn"
            style={{ minHeight: 42 }}
            disabled={!folder}
            onClick={() => revealOutput(folder)}
          >
            <Icon path={ICONS.save} size={16} strokeWidth={2} />
            Open the folder
          </button>
        </div>
        <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
          Nothing is ever overwritten: a second run of the same file becomes
          <span style={{ fontWeight: 600 }}> photo-kitbox-2.jpg</span>.
        </span>
      </section>
    </div>
  );
}

function Choice({
  label,
  value,
  options,
  onPick,
}: {
  label: string;
  value: string;
  options: [string, string][];
  onPick: (value: string) => void;
}) {
  return (
    <div className="col" style={{ gap: 7 }}>
      <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-muted)" }}>{label}</span>
      <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
        {options.map(([id, text]) => (
          <button
            key={id}
            type="button"
            aria-pressed={value === id}
            className={value === id ? "pressed" : "raised-sm"}
            onClick={() => onPick(id)}
            style={{
              borderRadius: 12,
              minHeight: 42,
              padding: "0 14px",
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
