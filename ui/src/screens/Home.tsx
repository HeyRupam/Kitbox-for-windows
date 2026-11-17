import { useEffect, useMemo, useRef, useState } from "react";

import { Icon, ICONS } from "../components/Icon";
import { Thumbnail } from "../components/Thumbnail";
import { categoriesFor, searchTools, type Surface, type Tool } from "../lib/tools";
import { formatSize, recentHistory } from "../lib/bridge";
import type { HistoryEntry } from "../lib/types";

export function Home({
  surface,
  category,
  onOpenTool,
  onFiles,
  onOpenGallery,
}: {
  surface: Surface;
  category: string;
  onOpenTool: (toolId: string, files?: File[]) => void;
  onFiles: (files: File[]) => void;
  onOpenGallery: () => void;
}) {
  const [query, setQuery] = useState("");
  const [dragging, setDragging] = useState(false);
  const [recent, setRecent] = useState<HistoryEntry[]>([]);
  const searchRef = useRef<HTMLInputElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    recentHistory(8).then(setRecent).catch(() => setRecent([]));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const categories = useMemo(() => {
    const all = categoriesFor(surface);
    return category === "all" ? all : all.filter((c) => c.id === category);
  }, [surface, category]);

  // Search reaches across both surfaces, so the Images/PDF switch never hides
  // something you're actively looking for.
  const hits = searchTools(query);

  return (
    <div className="col grow scroll" style={{ gap: 26, paddingRight: 6 }}>
      <header
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: 20,
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <div className="col" style={{ gap: 4 }}>
          <h1 className="display" style={{ fontSize: 32 }}>
            What are we fixing today?
          </h1>
          <p style={{ fontSize: 15, color: "var(--text-subtle)" }}>
            Pick a tool, or drop files and choose after.
          </p>
        </div>

        <label className="pressed field" style={{ flex: "0 1 380px", minHeight: 50, borderRadius: 18 }}>
          <Icon path={ICONS.search} size={18} color="var(--text-subtle)" strokeWidth={2} />
          <span className="sr-only">Search tools</span>
          <input
            ref={searchRef}
            type="search"
            placeholder="Search tools"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <span
            className="raised-sm"
            style={{ fontSize: 11, fontWeight: 600, padding: "4px 8px", borderRadius: 8 }}
          >
            Ctrl K
          </span>
        </label>
      </header>

      <section
        aria-label="Add files"
        className="raised"
        style={{ borderRadius: "var(--r-panel)", padding: 18 }}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          onFiles([...e.dataTransfer.files]);
        }}
      >
        <div
          className="pressed"
          style={{
            borderRadius: 22,
            padding: 28,
            display: "flex",
            flexWrap: "wrap",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 20,
            outline: dragging ? "2px dashed var(--accent)" : "none",
            outlineOffset: -10,
          }}
        >
          <div className="row" style={{ gap: 18 }}>
            <div
              className="raised-sm"
              style={{ width: 60, height: 60, borderRadius: "50%", display: "grid", placeItems: "center" }}
            >
              <Icon path={ICONS.upload} size={26} color="var(--accent)" strokeWidth={2} />
            </div>
            <div className="col" style={{ gap: 4 }}>
              <span className="display" style={{ fontSize: 19, fontWeight: 600 }}>
                {dragging ? "Drop them here" : "Drop images, PDFs or folders"}
              </span>
              <span style={{ fontSize: 13, color: "var(--text-subtle)" }}>
                JPG · PNG · WEBP · HEIC · GIF · BMP · TIFF · PDF
              </span>
            </div>
          </div>

          <div className="row" style={{ gap: 12, flexWrap: "wrap" }}>
            <input
              ref={fileRef}
              type="file"
              multiple
              hidden
              onChange={(e) => onFiles([...(e.target.files ?? [])])}
            />
            <button type="button" className="raised-sm btn" onClick={() => fileRef.current?.click()}>
              Browse files
            </button>
          </div>
        </div>
      </section>

      {recent.length > 0 && (
        <section className="col" style={{ gap: 14 }}>
          <div className="row" style={{ gap: 12, justifyContent: "space-between" }}>
            <h2 className="section-heading">Recent</h2>
            <button
              type="button"
              onClick={onOpenGallery}
              style={{ fontSize: 13, fontWeight: 600, color: "var(--accent)" }}
            >
              Open history
            </button>
          </div>
          <div className="row noscroll" style={{ gap: 14, overflowX: "auto", paddingBottom: 4 }}>
            {recent.map((r) => (
              <div
                key={r.hash}
                className="raised col"
                style={{
                  flex: "0 0 auto",
                  borderRadius: 18,
                  padding: 12,
                  width: 160,
                  gap: 8,
                }}
              >
                <Thumbnail hash={r.thumbHash} kind={r.kind} height={92} />
                <div className="truncate" style={{ fontSize: 12.5, fontWeight: 600 }}>
                  {r.name}
                </div>
                <div style={{ fontSize: 11.5, color: "var(--text-subtle)" }}>
                  {formatSize(r.size)}
                </div>
              </div>
            ))}
          </div>
        </section>
      )}

      {query.trim() ? (
        <ToolGrid
          heading={`${hits.length} result${hits.length === 1 ? "" : "s"}`}
          blurb={`for "${query.trim()}"`}
          tools={hits}
          onOpen={onOpenTool}
        />
      ) : (
        categories.map((c) => (
          <ToolGrid
            key={c.id}
            heading={c.name}
            blurb={c.blurb}
            tools={c.tools}
            onOpen={onOpenTool}
          />
        ))
      )}
    </div>
  );
}

function ToolGrid({
  heading,
  blurb,
  tools,
  onOpen,
}: {
  heading: string;
  blurb: string;
  tools: Tool[];
  onOpen: (id: string) => void;
}) {
  return (
    <section className="col" style={{ gap: 16 }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 12 }}>
        <h2 className="section-heading">{heading}</h2>
        <span style={{ fontSize: 12, color: "var(--text-subtle)" }}>{blurb}</span>
      </div>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))",
          gap: 20,
        }}
      >
        {tools.map((t) => (
          <button
            key={t.id}
            type="button"
            className="raised tile"
            onClick={() => onOpen(t.id)}
            style={{
              borderRadius: "var(--r-tile)",
              padding: 18,
              display: "flex",
              alignItems: "center",
              gap: 16,
              minHeight: 86,
              textAlign: "left",
            }}
          >
            <span
              className="pressed"
              style={{
                flex: "0 0 auto",
                width: 52,
                height: 52,
                borderRadius: 16,
                display: "grid",
                placeItems: "center",
              }}
            >
              <Icon path={t.icon} size={24} color="var(--accent)" />
            </span>
            <span className="col" style={{ gap: 3, minWidth: 0 }}>
              <span className="row" style={{ gap: 8 }}>
                <span style={{ fontWeight: 600, fontSize: 15 }}>{t.name}</span>
                {t.status === "soon" && (
                  <span
                    className="pressed"
                    style={{
                      fontSize: 10,
                      fontWeight: 600,
                      letterSpacing: "0.06em",
                      textTransform: "uppercase",
                      padding: "3px 7px",
                      borderRadius: 7,
                      color: "var(--text-subtle)",
                    }}
                  >
                    Soon
                  </span>
                )}
              </span>
              <span style={{ fontSize: 12.5, lineHeight: 1.4, color: "var(--text-subtle)" }}>
                {t.desc}
              </span>
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
