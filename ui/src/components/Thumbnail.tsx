import { useEffect, useState } from "react";

import { historyThumbnail } from "../lib/bridge";

/**
 * The picture for a history entry.
 *
 * Loads on mount and falls back to a labelled tile when there's nothing to
 * show — a text-only PDF has no thumbnail, and an empty grey box would just
 * look like a bug.
 */
export function Thumbnail({
  hash,
  kind,
  height = 120,
}: {
  hash?: string;
  kind: string;
  height?: number;
}) {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!hash) {
      setUrl(null);
      return;
    }
    let live = true;
    historyThumbnail(hash).then((u) => live && setUrl(u));
    return () => {
      live = false;
    };
  }, [hash]);

  return (
    <span
      className="pressed"
      style={{
        height,
        borderRadius: 12,
        overflow: "hidden",
        display: "grid",
        placeItems: "center",
      }}
    >
      {url ? (
        <img
          src={url}
          alt=""
          style={{ width: "100%", height: "100%", objectFit: "cover" }}
        />
      ) : (
        <span
          style={{
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: "0.08em",
            color: "var(--text-subtle)",
          }}
        >
          {kind.toUpperCase()}
        </span>
      )}
    </span>
  );
}
