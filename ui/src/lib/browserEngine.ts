/**
 * The same solver, in the browser.
 *
 * Kitbox does its real work in Rust, but the UI has to run in a plain browser
 * too — for development, and so the interface can be exercised before the
 * desktop shell is built. This mirrors `mt_core::target::solver` closely enough
 * that the screens behave the same either way: same ladder shape, same
 * interpolate-then-bisect search, same "stop once we're using most of the
 * window" rule, same honest refusal when nothing fits.
 *
 * It is not a second implementation to maintain forever — when the desktop
 * shell is present, none of this runs.
 */

import type { Bias, CompressRequest, CompressResult, OutputFormat } from "./types";
import { targetMax, targetMin } from "./types";

interface Knob {
  scale: number;
  quality: number;
}

const SCALES = [100, 90, 80, 70, 60, 50, 42, 35, 28, 22, 16, 12];

function predictedRatio(scale: number, quality: number, png: boolean): number {
  const pixels = (scale / 100) ** 2;
  if (png) return pixels;
  const q = quality / 100;
  return pixels * (0.08 + 0.42 * q * q);
}

function prefers(bias: Bias, a: Knob, b: Knob): boolean {
  if (bias === "keep_sharp") return a.scale > b.scale;
  if (bias === "shrink_pixels") return a.quality > b.quality;
  const score = (k: Knob) => k.scale * 2 + k.quality;
  return score(a) > score(b);
}

function buildLadder(bias: Bias, png: boolean, fixedScale: boolean): Knob[] {
  const scales = fixedScale ? [100] : SCALES;
  const qualities: number[] = png ? [100] : [];
  if (!png) for (let q = 10; q <= 98; q += 2) qualities.push(q);

  const buckets = new Map<number, Knob>();
  for (const scale of scales) {
    for (const quality of qualities) {
      const knob = { scale, quality };
      const bucket = Math.round(Math.log(predictedRatio(scale, quality, png)) / Math.log(1.06));
      const existing = buckets.get(bucket);
      if (!existing || prefers(bias, knob, existing)) buckets.set(bucket, knob);
    }
  }
  return [...buckets.entries()].sort((a, b) => a[0] - b[0]).map(([, k]) => k);
}

function mimeFor(format: OutputFormat, sourceType: string): string {
  if (format === "jpeg") return "image/jpeg";
  if (format === "png") return "image/png";
  return sourceType === "image/png" ? "image/png" : "image/jpeg";
}

async function draw(
  bitmap: ImageBitmap,
  width: number,
  height: number,
): Promise<OffscreenCanvas | HTMLCanvasElement> {
  if (typeof OffscreenCanvas !== "undefined") {
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext("2d")!;
    // JPEG has no alpha: paint white underneath so transparency doesn't
    // come out black, matching what the Rust engine does.
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, width, height);
    ctx.drawImage(bitmap, 0, 0, width, height);
    return canvas;
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, width, height);
  ctx.drawImage(bitmap, 0, 0, width, height);
  return canvas;
}

async function toBlob(
  canvas: OffscreenCanvas | HTMLCanvasElement,
  type: string,
  quality: number,
): Promise<Blob> {
  if ("convertToBlob" in canvas) {
    return canvas.convertToBlob({ type, quality: quality / 100 });
  }
  return new Promise((resolve) =>
    (canvas as HTMLCanvasElement).toBlob((b) => resolve(b!), type, quality / 100),
  );
}

/** Coarse SSIM on a downscaled grayscale pair — enough for the readout. */
async function similarity(a: ImageBitmap, b: ImageBitmap): Promise<number> {
  const w = Math.min(256, a.width);
  const h = Math.max(1, Math.round((w / a.width) * a.height));
  const grey = async (bmp: ImageBitmap) => {
    const canvas = await draw(bmp, w, h);
    const ctx = canvas.getContext("2d") as CanvasRenderingContext2D;
    const { data } = ctx.getImageData(0, 0, w, h);
    const out = new Float64Array(w * h);
    for (let i = 0; i < w * h; i++) {
      out[i] = 0.299 * data[i * 4] + 0.587 * data[i * 4 + 1] + 0.114 * data[i * 4 + 2];
    }
    return out;
  };
  const [ga, gb] = await Promise.all([grey(a), grey(b)]);

  const C1 = (0.01 * 255) ** 2;
  const C2 = (0.03 * 255) ** 2;
  const win = 8;
  let total = 0;
  let count = 0;
  for (let y = 0; y + win <= h; y += 4) {
    for (let x = 0; x + win <= w; x += 4) {
      let sa = 0, sb = 0;
      for (let j = 0; j < win; j++)
        for (let i = 0; i < win; i++) {
          sa += ga[(y + j) * w + x + i];
          sb += gb[(y + j) * w + x + i];
        }
      const n = win * win;
      const ma = sa / n;
      const mb = sb / n;
      let va = 0, vb = 0, cov = 0;
      for (let j = 0; j < win; j++)
        for (let i = 0; i < win; i++) {
          const da = ga[(y + j) * w + x + i] - ma;
          const db = gb[(y + j) * w + x + i] - mb;
          va += da * da;
          vb += db * db;
          cov += da * db;
        }
      va /= n - 1;
      vb /= n - 1;
      cov /= n - 1;
      total +=
        ((2 * ma * mb + C1) * (2 * cov + C2)) / ((ma * ma + mb * mb + C1) * (va + vb + C2));
      count++;
    }
  }
  return count ? Math.min(1, Math.max(0, total / count)) : 1;
}

function describe(knob: Knob, width: number, height: number, png: boolean): string {
  const dims = knob.scale >= 100 ? `${width}×${height}` : `${width}×${height} (${knob.scale}%)`;
  return png ? `PNG, ${dims}` : `quality ${knob.quality}, ${dims}`;
}

/** Manual mode: one encode at the quality the user picked, no searching. */
export async function encodeAtQuality(
  file: File,
  quality: number,
  format: OutputFormat,
): Promise<CompressResult> {
  const started = performance.now();
  const source = await createImageBitmap(file);
  const type = mimeFor(format, file.type);
  const canvas = await draw(source, source.width, source.height);
  const blob = await toBlob(canvas, type, quality);
  const score = await similarity(source, await createImageBitmap(blob));
  return {
    status: "hit",
    name: file.name,
    originalSize: file.size,
    size: blob.size,
    knob: describe({ scale: 100, quality }, source.width, source.height, type === "image/png"),
    score,
    probes: 1,
    elapsedMs: Math.round(performance.now() - started),
    previewUrl: URL.createObjectURL(blob),
  };
}

export async function compressInBrowser(
  file: File,
  req: Omit<CompressRequest, "bytes" | "path">,
): Promise<CompressResult> {
  const started = performance.now();
  const source = await createImageBitmap(file);

  const type = mimeFor(req.format, file.type);
  const png = type === "image/png";

  // Hard dimension constraints come first, exactly as in the Rust engine:
  // an exact size from a form preset takes pixels off the table.
  let baseW = source.width;
  let baseH = source.height;
  let working = source;
  if (req.exact) {
    const { width, height } = req.exact;
    const scale = Math.max(width / source.width, height / source.height);
    const sw = Math.max(width, Math.ceil(source.width * scale));
    const sh = Math.max(height, Math.ceil(source.height * scale));
    const covered = await draw(source, sw, sh);
    working = await createImageBitmap(covered, (sw - width) / 2, (sh - height) / 2, width, height);
    baseW = width;
    baseH = height;
  } else if (req.maxDimension && Math.max(source.width, source.height) > req.maxDimension) {
    const f = req.maxDimension / Math.max(source.width, source.height);
    baseW = Math.round(source.width * f);
    baseH = Math.round(source.height * f);
    working = await createImageBitmap((await draw(source, baseW, baseH)));
  }

  const ladder = buildLadder(req.bias, png, !!req.exact);
  const cache = new Map<number, Blob>();
  let probes = 0;

  const encodeAt = async (index: number): Promise<Blob> => {
    const hit = cache.get(index);
    if (hit) return hit;
    const knob = ladder[index];
    const w = Math.max(1, Math.round((baseW * knob.scale) / 100));
    const h = Math.max(1, Math.round((baseH * knob.scale) / 100));
    const canvas = await draw(working, w, h);
    const blob = await toBlob(canvas, type, knob.quality);
    cache.set(index, blob);
    probes++;
    return blob;
  };
  const sizeAt = async (i: number) => (await encodeAt(i)).size;

  const max = targetMax(req.target);
  const min = targetMin(req.target);
  const goodEnough = Math.max(min, Math.floor(max * 0.95));

  const finish = async (index: number): Promise<CompressResult> => {
    const blob = await encodeAt(index);
    const knob = ladder[index];
    const w = Math.max(1, Math.round((baseW * knob.scale) / 100));
    const h = Math.max(1, Math.round((baseH * knob.scale) / 100));
    const score = await similarity(working, await createImageBitmap(blob));
    const under = blob.size < min;
    return {
      status: under ? "padded" : "hit",
      name: file.name,
      originalSize: file.size,
      size: blob.size,
      knob: describe(knob, w, h, png),
      score,
      probes,
      elapsedMs: Math.round(performance.now() - started),
      previewUrl: URL.createObjectURL(blob),
      note: under
        ? `Best quality came to ${blob.size} bytes, under the minimum. The desktop app pads this losslessly; the browser preview does not.`
        : undefined,
    };
  };

  const top = ladder.length - 1;
  const sizeTop = await sizeAt(top);
  if (sizeTop <= max) return finish(top);

  const sizeBottom = await sizeAt(0);
  if (sizeBottom > max) {
    return {
      status: "infeasible",
      name: file.name,
      originalSize: file.size,
      size: sizeBottom,
      knob: describe(ladder[0], baseW, baseH, png),
      probes,
      elapsedMs: Math.round(performance.now() - started),
      why: `The smallest setting still produces ${Math.round(sizeBottom / 1000)} KB, over the limit.`,
      suggestions: req.exact
        ? ["The dimensions are pinned by the preset, so quality is the only lever left."]
        : [
            "Allow smaller dimensions, or crop to just the part you need.",
            "Convert to grayscale if colour isn't required.",
          ],
    };
  }
  if (sizeBottom >= goodEnough) return finish(0);

  let lo = 0;
  let hi = top;
  let sizeLo = sizeBottom;
  let sizeHi = sizeTop;

  const interpolate = (): number | null => {
    if (sizeLo <= 0 || sizeHi <= sizeLo || max < sizeLo) return null;
    const frac = (Math.log(max) - Math.log(sizeLo)) / (Math.log(sizeHi) - Math.log(sizeLo));
    return lo + Math.round(Math.min(1, Math.max(0, frac)) * (hi - lo));
  };

  for (let round = 0; round < 2 && hi - lo > 3; round++) {
    const raw = interpolate();
    if (raw === null) break;
    const guess = Math.min(hi - 1, Math.max(lo + 1, raw));
    if (guess === lo || guess === hi) break;
    const s = await sizeAt(guess);
    if (s <= max) {
      if (s >= goodEnough) return finish(guess);
      lo = guess;
      sizeLo = s;
    } else {
      hi = guess;
      sizeHi = s;
    }
  }

  while (hi - lo > 1 && probes < 24) {
    const mid = lo + Math.floor((hi - lo) / 2);
    const s = await sizeAt(mid);
    if (s <= max) {
      if (s >= goodEnough) return finish(mid);
      lo = mid;
    } else {
      hi = mid;
    }
  }

  return finish(lo);
}
