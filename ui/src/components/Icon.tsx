/** Stroke icons in the design's language: 24×24 grid, round caps, 1.9px. */
export function Icon({
  path,
  size = 22,
  color = "currentColor",
  strokeWidth = 1.9,
}: {
  path: string;
  size?: number;
  color?: string;
  strokeWidth?: number;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={color}
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={path} />
    </svg>
  );
}

export const ICONS = {
  logo: "M3 7l9-4 9 4-9 4z M3 12l9 4 9-4 M3 17l9 4 9-4",
  search: "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14 M20 20l-4-4",
  upload: "M12 16V4 M7 9l5-5 5 5 M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3",
  history: "M12 7v5l3 2 M3 12a9 9 0 1 0 3-6.7 M3 3v4h4",
  settings: "M4 6h10 M18 6h2 M4 12h4 M12 12h8 M4 18h12 M20 18h0 M14 4v4 M8 10v4 M16 16v4",
  back: "M15 5l-7 7 7 7",
  moon: "M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z",
  plus: "M12 5v14 M5 12h14",
  close: "M6 6l12 12 M18 6L6 18",
  check: "M4 12l5 5L20 6",
  zoomIn: "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14 M20 20l-4-4 M11 8v6 M8 11h6",
  zoomOut: "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14 M20 20l-4-4 M8 11h6",
  badge: "M12 3l7 3v6c0 4.5-3 7.3-7 8-4-.7-7-3.5-7-8V6z M9 12l2 2 4-4",
  save: "M5 3h11l3 3v15H5z M8 3v6h7 M8 14h8v7H8z",
  warning: "M12 4l9 16H3z M12 10v4 M12 17h.01",
  grid: "M3 4h7v7H3z M14 4h7v7h-7z M3 15h7v5H3z M14 15h7v5h-7z",
} as const;
