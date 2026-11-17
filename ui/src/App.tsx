import { useCallback, useEffect, useState } from "react";

import { Icon, ICONS } from "./components/Icon";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Home } from "./screens/Home";
import { Compress } from "./screens/Compress";
import { Pages } from "./screens/Pages";
import { Merge } from "./screens/Merge";
import { ToPdf } from "./screens/ToPdf";
import { Transform, type TransformTool } from "./screens/Transform";
import { Security, type SecurityMode } from "./screens/Security";
import { Watermark } from "./screens/Watermark";
import { Numbers } from "./screens/Numbers";
import { CropPdf } from "./screens/CropPdf";
import { Repair } from "./screens/Repair";
import { Markdown } from "./screens/Markdown";
import { Compare } from "./screens/Compare";
import { Scan } from "./screens/Scan";
import { Sign } from "./screens/Sign";
import { Redact } from "./screens/Redact";
import { Forms } from "./screens/Forms";
import { Annotate } from "./screens/Annotate";
import { Ocr } from "./screens/Ocr";
import { Html } from "./screens/Html";
import { Cutout } from "./screens/Cutout";
import { Upscale } from "./screens/Upscale";
import { Faces } from "./screens/Faces";
import { Editor } from "./screens/Editor";
import { Archive } from "./screens/Archive";
import { Batch } from "./screens/Batch";
import { Meme } from "./screens/Meme";
import { Gallery } from "./screens/Gallery";
import { Settings } from "./screens/Settings";
import { Recipes } from "./screens/Recipes";
import { Presets } from "./screens/Presets";
import { categoriesFor, findTool, TOOLS, type Surface } from "./lib/tools";
import type { BatchOutcome, Preset, ShellRun } from "./lib/types";
import { filesFromShell, isDesktop, onShellRun, takeShellRun } from "./lib/bridge";

export type Screen =
  | { name: "home" }
  | { name: "tool"; toolId: string }
  | { name: "gallery" }
  | { name: "presets" }
  | { name: "recipes" }
  | { name: "settings" };

const ACCENTS = ["#c2410c", "#1d4ed8", "#0f766e", "#7c3aed"];

/** What to call a screen that is not a tool, when one stops. */
const SCREEN_NAMES: Record<Screen["name"], string> = {
  home: "The tool list",
  tool: "This tool",
  gallery: "History",
  presets: "Form presets",
  recipes: "Recipes",
  settings: "Settings",
};

/** Tools that share the page workspace. */
const PAGE_TOOLS = [
  "pdf-organize",
  "pdf-remove-pages",
  "pdf-extract-pages",
  "pdf-rotate",
  "pdf-split",
  "pdf-to-jpg",
];

/** Everything with a real screen behind it. */
/** Image tools that share the transform workspace. */
const TRANSFORM_TOOLS: Record<string, TransformTool> = {
  "image-resize": "resize",
  "image-crop": "crop",
  "image-rotate": "rotate",
  "image-to-jpg": "convert",
  "image-from-jpg": "convert",
};

/** The two password tools share one workspace. */
const SECURITY_TOOLS: Record<string, SecurityMode> = {
  "pdf-protect": "protect",
  "pdf-unlock": "unlock",
};

/**
 * Which tools have a screen behind them.
 *
 * Derived from the tiles' own `status` rather than listed again here. Keeping
 * two lists meant five finished tools sat on the home screen wearing a "Soon"
 * badge, unreachable, because only this one got updated.
 */
const BUILT: string[] = TOOLS.filter((t) => t.status === "ready").map((t) => t.id);

export function App() {
  const [screen, setScreen] = useState<Screen>({ name: "home" });
  const [surface, setSurface] = useState<Surface>("images");
  const [category, setCategory] = useState<string>("all");
  const [files, setFiles] = useState<File[]>([]);
  /** A preset chosen on the presets screen, handed to Compress when it opens. */
  const [pendingPreset, setPendingPreset] = useState<Preset | null>(null);
  /** What the Windows right-click menu did, if the app was opened that way. */
  const [handed, setHanded] = useState<
    { summary: string; outcomes: BatchOutcome[]; paths: string[] } | null
  >(null);
  const [accent, setAccent] = useState(ACCENTS[0]);

  useEffect(() => {
    document.documentElement.style.setProperty("--accent", accent);
  }, [accent]);

  const openTool = useCallback((toolId: string, incoming?: File[]) => {
    if (incoming?.length) setFiles(incoming);
    setScreen({ name: "tool", toolId });
  }, []);

  // Files dropped anywhere outside a screen that handles them land on the
  // matching compress tool, which is what people want nine times out of ten.
  const acceptFiles = useCallback(
    (dropped: File[]) => {
      if (!dropped.length) return;
      const pdf = dropped[0].name.toLowerCase().endsWith(".pdf");
      setSurface(pdf ? "pdf" : "images");
      openTool(pdf ? "pdf-compress" : "image-compress", dropped);
    },
    [openTool],
  );

  // A right-click in Explorer either hands over a finished job to show, or
  // the files themselves to work on. Both can arrive before this screen has
  // mounted — the app was started by the menu — so the first look is a
  // question rather than a subscription.
  const acceptShellRun = useCallback(
    (run: ShellRun) => {
      if (run.kind === "open") {
        acceptFiles(filesFromShell(run));
        return;
      }
      setHanded({ summary: run.summary, outcomes: run.outcomes, paths: run.paths });
      setSurface("images");
      setScreen({ name: "tool", toolId: "image-batch" });
    },
    [acceptFiles],
  );

  useEffect(() => {
    let stop: (() => void) | undefined;
    takeShellRun()
      .then((run) => run && acceptShellRun(run))
      .catch(() => undefined);
    onShellRun(acceptShellRun)
      .then((unlisten) => {
        stop = unlisten;
      })
      .catch(() => undefined);
    return () => stop?.();
  }, [acceptShellRun]);

  const tool = screen.name === "tool" ? findTool(screen.toolId) : undefined;

  return (
    <div style={{ display: "flex", height: "100%", gap: 22, padding: 22 }}>
      <Rail
        screen={screen}
        surface={surface}
        category={category}
        accent={accent}
        accents={ACCENTS}
        onSurface={(s) => {
          setSurface(s);
          setCategory("all");
          setScreen({ name: "home" });
        }}
        onCategory={(c) => {
          setCategory(c);
          setScreen({ name: "home" });
        }}
        onScreen={setScreen}
        onAccent={setAccent}
      />

      <main className="grow col" style={{ minWidth: 0 }}>
        {/*
          One screen breaking should cost you that screen, not the window. The
          rail sits outside this, so even a screen that cannot draw at all
          leaves you somewhere to click.
        */}
        <ErrorBoundary
          what={tool?.name ?? SCREEN_NAMES[screen.name]}
          resetKey={screen.name === "tool" ? `tool:${screen.toolId}` : screen.name}
          onBack={() => setScreen({ name: "home" })}
        >
        {screen.name === "home" && (
          <Home
            surface={surface}
            category={category}
            onOpenTool={openTool}
            onFiles={acceptFiles}
            onOpenGallery={() => setScreen({ name: "gallery" })}
          />
        )}

        {screen.name === "tool" && tool && (tool.id === "image-compress" || tool.id === "pdf-compress") && (
          <Compress
            files={files}
            onFiles={setFiles}
            onBack={() => setScreen({ name: "home" })}
            startPreset={pendingPreset}
          />
        )}

        {screen.name === "tool" && tool && PAGE_TOOLS.includes(tool.id) && (
          <Pages files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-from-jpg" && (
          <ToPdf files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "image-meme" && (
          <Meme files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}
        {screen.name === "tool" &&
          tool &&
          (tool.id === "image-batch" || tool.id === "pdf-batch") && (
            <Batch
              files={files}
              onFiles={setFiles}
              handed={handed}
              onBack={() => {
                setHanded(null);
                setScreen({ name: "home" });
              }}
            />
          )}

        {screen.name === "tool" && tool && tool.id === "pdf-to-pdfa" && (
          <Archive files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "image-editor" && (
          <Editor files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "image-blur-face" && (
          <Faces files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "image-upscale" && (
          <Upscale files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "image-removebg" && (
          <Cutout files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-from-html" && (
          <Html as="pdf" onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "image-from-html" && (
          <Html as="image" onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-ocr" && (
          <Ocr files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-annotate" && (
          <Annotate files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-forms" && (
          <Forms files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-redact" && (
          <Redact files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-sign" && (
          <Sign files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-scan" && (
          <Scan files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-compare" && (
          <Compare onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-to-markdown" && (
          <Markdown files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-repair" && (
          <Repair files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-crop" && (
          <CropPdf files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-page-numbers" && (
          <Numbers files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && (tool.id === "pdf-watermark" || tool.id === "image-watermark") && (
          <Watermark files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && SECURITY_TOOLS[tool.id] && (
          <Security
            mode={SECURITY_TOOLS[tool.id]}
            files={files}
            onFiles={setFiles}
            onBack={() => setScreen({ name: "home" })}
          />
        )}

        {screen.name === "tool" && tool && TRANSFORM_TOOLS[tool.id] && (
          <Transform
            tool={TRANSFORM_TOOLS[tool.id]}
            files={files}
            onFiles={setFiles}
            onBack={() => setScreen({ name: "home" })}
          />
        )}

        {screen.name === "tool" && tool && tool.id === "pdf-merge" && (
          <Merge files={files} onFiles={setFiles} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "tool" && tool && !BUILT.includes(tool.id) && (
          <NotBuiltYet name={tool.name} onBack={() => setScreen({ name: "home" })} />
        )}

        {screen.name === "gallery" && <Gallery onBack={() => setScreen({ name: "home" })} />}

        {screen.name === "recipes" && <Recipes onBack={() => setScreen({ name: "home" })} />}

        {screen.name === "settings" && <Settings onBack={() => setScreen({ name: "home" })} />}

        {screen.name === "presets" && (
          <Presets
            onBack={() => setScreen({ name: "home" })}
            onUse={(preset) => {
              // Carried across rather than re-picked: you chose it here, and
              // choosing it again on the next screen is a step for nothing.
              setPendingPreset(preset);
              setSurface(preset.kind === "pdf" ? "pdf" : "images");
              openTool(preset.kind === "pdf" ? "pdf-compress" : "image-compress");
            }}
          />
        )}
        </ErrorBoundary>
      </main>
    </div>
  );
}

function Rail({
  screen,
  surface,
  category,
  accent,
  accents,
  onSurface,
  onCategory,
  onScreen,
  onAccent,
}: {
  screen: Screen;
  surface: Surface;
  category: string;
  accent: string;
  accents: string[];
  onSurface: (s: Surface) => void;
  onCategory: (c: string) => void;
  onScreen: (s: Screen) => void;
  onAccent: (c: string) => void;
}) {
  const cats = categoriesFor(surface);
  const total = cats.reduce((n, c) => n + c.tools.length, 0);
  const entries = [{ id: "all", name: "All tools", count: total }].concat(
    cats.map((c) => ({ id: c.id, name: c.name, count: c.tools.length })),
  );

  return (
    <nav
      className="raised col"
      aria-label="Tools"
      style={{
        flex: "0 0 244px",
        borderRadius: "var(--r-panel)",
        padding: "22px 16px",
        gap: 20,
      }}
    >
      <div className="row" style={{ gap: 12, padding: "0 6px" }}>
        <div
          className="raised-sm"
          style={{
            width: 44,
            height: 44,
            borderRadius: 14,
            display: "grid",
            placeItems: "center",
          }}
        >
          <Icon path={ICONS.logo} size={22} color="var(--accent)" strokeWidth={2} />
        </div>
        <div className="col">
          <span className="display" style={{ fontSize: 20 }}>
            Kitbox
          </span>
          <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>
            {isDesktop() ? "Offline toolkit" : "Preview in browser"}
          </span>
        </div>
      </div>

      <div
        className="pressed segmented"
        style={{ gridTemplateColumns: "1fr 1fr" }}
        role="group"
        aria-label="Which files"
      >
        {(["images", "pdf"] as Surface[]).map((s) => (
          <button
            key={s}
            type="button"
            aria-pressed={surface === s}
            className={surface === s ? "raised-sm" : undefined}
            onClick={() => onSurface(s)}
          >
            {s === "images" ? "Images" : "PDF"}
          </button>
        ))}
      </div>

      <div className="col grow scroll" style={{ gap: 6 }}>
        {entries.map((e) => {
          const active = screen.name === "home" && category === e.id;
          return (
            <button
              key={e.id}
              type="button"
              aria-pressed={active}
              className={active ? "pressed" : undefined}
              onClick={() => onCategory(e.id)}
              style={{
                borderRadius: 16,
                minHeight: 44,
                padding: "0 16px",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                fontWeight: active ? 600 : 500,
                fontSize: 14,
                color: active ? "var(--text)" : "var(--text-muted)",
                textAlign: "left",
              }}
            >
              <span>{e.name}</span>
              <span
                style={{
                  fontSize: 12,
                  fontWeight: 600,
                  color: active ? "var(--accent)" : "var(--text-subtle)",
                }}
              >
                {e.count}
              </span>
            </button>
          );
        })}
      </div>

      <div className="divider" />

      <div className="col" style={{ gap: 2 }}>
        <RailLink
          icon={ICONS.history}
          label="History"
          active={screen.name === "gallery"}
          onClick={() => onScreen({ name: "gallery" })}
        />
        <RailLink
          icon={ICONS.badge}
          label="Form presets"
          active={screen.name === "presets"}
          onClick={() => onScreen({ name: "presets" })}
        />
        <RailLink
          icon={ICONS.grid}
          label="Recipes"
          active={screen.name === "recipes"}
          onClick={() => onScreen({ name: "recipes" })}
        />
        <RailLink
          icon={ICONS.settings}
          label="Settings"
          active={screen.name === "settings"}
          onClick={() => onScreen({ name: "settings" })}
        />
      </div>

      <div
        className="pressed col"
        style={{ borderRadius: 20, padding: 16, gap: 8 }}
      >
        <span style={{ fontSize: 13, fontWeight: 600 }}>Runs offline</span>
        <span style={{ fontSize: 12, lineHeight: 1.5, color: "var(--text-subtle)" }}>
          Nothing is uploaded. Your files never leave this device.
        </span>
        <div className="row" style={{ gap: 8, marginTop: 4 }}>
          {accents.map((c) => (
            <button
              key={c}
              type="button"
              aria-label={`Accent ${c}`}
              aria-pressed={accent === c}
              onClick={() => onAccent(c)}
              className={accent === c ? "pressed" : "raised-sm"}
              style={{
                width: 22,
                height: 22,
                borderRadius: "50%",
                background: accent === c ? undefined : c,
                display: "grid",
                placeItems: "center",
              }}
            >
              {accent === c && (
                <span
                  style={{ width: 10, height: 10, borderRadius: "50%", background: c }}
                />
              )}
            </button>
          ))}
        </div>
      </div>
    </nav>
  );
}

function RailLink({
  icon,
  label,
  active,
  onClick,
}: {
  icon: string;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={active ? "pressed" : undefined}
      style={{
        minHeight: 44,
        padding: "0 16px",
        display: "flex",
        alignItems: "center",
        gap: 12,
        fontSize: 14,
        borderRadius: 16,
        color: active ? "var(--text)" : "var(--text-muted)",
        fontWeight: active ? 600 : 500,
      }}
    >
      <Icon path={icon} size={18} strokeWidth={2} />
      {label}
    </button>
  );
}

function NotBuiltYet({ name, onBack }: { name: string; onBack: () => void }) {
  return (
    <div className="raised col" style={{ borderRadius: "var(--r-panel)", padding: 40, gap: 16 }}>
      <h1 className="display" style={{ fontSize: 26 }}>
        {name}
      </h1>
      <p style={{ color: "var(--text-subtle)", maxWidth: 520, lineHeight: 1.6 }}>
        This tool isn't built yet. Compress image is the one that works today — it's the
        slice that proves the solver, the job queue and the history store end to end. The
        rest follow it.
      </p>
      <button type="button" className="raised-sm btn" style={{ alignSelf: "flex-start" }} onClick={onBack}>
        Back to all tools
      </button>
    </div>
  );
}
