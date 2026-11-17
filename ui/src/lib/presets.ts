/**
 * Form presets — the constraints a form states, as data.
 *
 * There is one source for these: `crates/mt-core/src/bundled_presets.json`,
 * compiled into the engine and imported here. A second list in TypeScript
 * would drift the first time someone fixed a number in one of them, and the
 * number being wrong is the whole problem presets exist to solve.
 *
 * On the desktop the engine merges that set with whatever is in your presets
 * folder, so what you see is what you edited. In a plain browser there is no
 * folder to read, and the bundled set is shown as-is.
 */

import bundled from "../../../crates/mt-core/src/bundled_presets.json";
import { isDesktop, listPresets } from "./bridge";
import type { LoadedPresets, Preset, PresetEntry } from "./types";

/** The bundled set, with the defaults the engine would apply. */
export const BUNDLED: Preset[] = (bundled as unknown as Partial<Preset>[]).map((p) => ({
  group: "Other",
  format: "any",
  ...p,
}) as Preset);

export async function loadPresets(): Promise<LoadedPresets> {
  if (!isDesktop()) {
    return {
      presets: BUNDLED.map((p) => ({ ...p, yours: false, overridesBundled: false })),
      folder: "",
      readme: "",
      problems: [],
    };
  }
  return listPresets();
}

export function byGroup<T extends Preset>(
  presets: T[],
  kind?: "image" | "pdf",
): { group: string; items: T[] }[] {
  const groups: string[] = [];
  for (const p of presets) {
    if (kind && p.kind !== kind) continue;
    if (!groups.includes(p.group)) groups.push(p.group);
  }
  return groups.map((group) => ({
    group,
    items: presets.filter((p) => p.group === group && (!kind || p.kind === kind)),
  }));
}

/** What this preset demands, in one line. */
export function describePreset(p: Preset): string[] {
  const parts: string[] = [];
  if (p.target.kind === "range") {
    parts.push(`${kb(p.target.value.min)}–${kb(p.target.value.max)}`);
  } else if (p.target.kind === "at_most") {
    parts.push(`under ${kb(p.target.value)}`);
  } else {
    parts.push(`about ${kb(p.target.value.bytes)}`);
  }
  if (p.dimensions) {
    const { width, height, fit } = p.dimensions;
    parts.push(
      fit === "exact"
        ? `${width}×${height} px`
        : fit === "at_least"
          ? `at least ${width}×${height}`
          : `at most ${width}×${height}`,
    );
  }
  if (p.format !== "any") parts.push(p.format === "jpeg" ? "JPG" : p.format.toUpperCase());
  if (p.dpi) parts.push(`${p.dpi} DPI`);
  return parts;
}

function kb(bytes: number): string {
  if (bytes >= 1_000_000) return `${+(bytes / 1_000_000).toFixed(1)} MB`;
  return `${Math.round(bytes / 1000)} KB`;
}

/** A new preset, with fields that are safe to save as they stand. */
export function blankPreset(): Preset {
  return {
    id: `mine-${Date.now().toString(36)}`,
    name: "My form",
    group: "Mine",
    kind: "image",
    target: { kind: "range", value: { min: 20000, max: 50000 } },
    format: "jpeg",
  };
}

export type { Preset, PresetEntry };
