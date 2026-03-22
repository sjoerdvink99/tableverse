import type { Quantiles } from "./types";

const YLGNBU_STOPS: [number, number, number][] = [
  [255, 255, 217],
  [237, 248, 177],
  [199, 233, 180],
  [127, 205, 187],
  [ 44, 127, 184],
  [ 37,  52, 148],
];

const PUBUGN_STOPS: [number, number, number][] = [
  [246, 239, 247],
  [208, 209, 230],
  [166, 189, 219],
  [103, 169, 207],
  [ 28, 144, 153],
  [  1, 100,  80],
];

const RDBU_STOPS: [number, number, number][] = [
  [178,  24,  43],
  [214,  96,  77],
  [244, 165, 130],
  [253, 219, 199],
  [247, 247, 247],
  [209, 229, 240],
  [146, 197, 222],
  [ 67, 147, 195],
  [ 33, 102, 172],
];

const VIRIDIS_STOPS: [number, number, number][] = [
  [ 68,   1,  84],
  [ 59,  82, 139],
  [ 33, 145, 140],
  [ 94, 201, 100],
  [253, 231,  37],
];

const NULL_RATE_STOPS: [number, number, number][] = [
  [240, 253, 244],
  [254, 249, 195],
  [254, 215, 170],
  [254, 202, 202],
];

const OKABE_ITO: [number, number, number][] = [
  [230, 159,   0],
  [ 86, 180, 233],
  [  0, 158, 115],
  [240, 228,  66],
  [  0, 114, 178],
  [213,  94,   0],
  [204, 121, 167],
  [  0,   0,   0],
  [153, 153, 153],
  [230,  25,  75],
  [ 60, 180,  75],
  [ 67,  99, 216],
];

export function interpolate(stops: [number, number, number][], t: number): [number, number, number] {
  const clamped = Math.max(0, Math.min(1, t));
  const idx = clamped * (stops.length - 1);
  const lo = Math.floor(idx);
  const hi = Math.min(stops.length - 1, lo + 1);
  const frac = idx - lo;
  return [
    Math.round(stops[lo]![0] + frac * (stops[hi]![0] - stops[lo]![0])),
    Math.round(stops[lo]![1] + frac * (stops[hi]![1] - stops[lo]![1])),
    Math.round(stops[lo]![2] + frac * (stops[hi]![2] - stops[lo]![2])),
  ];
}

export function ylgnbu(t: number): [number, number, number] {
  return interpolate(YLGNBU_STOPS, t);
}

export function pubugn(t: number): [number, number, number] {
  return interpolate(PUBUGN_STOPS, t);
}

export function rdbu(t: number): [number, number, number] {
  return interpolate(RDBU_STOPS, t);
}

export function viridis(t: number): [number, number, number] {
  return interpolate(VIRIDIS_STOPS, t);
}

export function nullRateColor(rate: number): [number, number, number] {
  return interpolate(NULL_RATE_STOPS, Math.max(0, Math.min(1, rate)));
}

export function categoricalColor(hash: number): [number, number, number] {
  return OKABE_ITO[Math.abs(hash) % 12]!;
}

export function djb2(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = (((h << 5) + h) ^ s.charCodeAt(i)) & 0x7fffffff;
  }
  return h;
}

export function packRGBA(r: number, g: number, b: number, a: number): number {
  return ((r & 0xff) | ((g & 0xff) << 8) | ((b & 0xff) << 16) | ((a & 0xff) << 24)) >>> 0;
}

export function normalizeByQuantiles(value: number, quantiles: Quantiles): number {
  const range = quantiles.p95 - quantiles.p5;
  if (range <= 0) return 0.5;
  return Math.max(0, Math.min(1, (value - quantiles.p5) / range));
}

export function outlierClass(value: number, quantiles: Quantiles): "high" | "low" | "none" {
  if (value > quantiles.p95) return "high";
  if (value < quantiles.p5) return "low";
  return "none";
}

export function drawNullHatch(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  isDark: boolean,
): void {
  ctx.save();
  ctx.beginPath();
  ctx.rect(x, y, w, h);
  ctx.clip();
  ctx.strokeStyle = isDark ? "rgba(200,200,200,0.12)" : "rgba(100,100,100,0.18)";
  ctx.lineWidth = 0.5;
  const step = 4;
  for (let d = -h; d < w + h; d += step) {
    ctx.beginPath();
    ctx.moveTo(x + d, y);
    ctx.lineTo(x + d + h, y + h);
    ctx.stroke();
  }
  ctx.restore();
}

export function drawOutlierBorder(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  severity: "high" | "low",
  alpha: number,
): void {
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.strokeStyle = severity === "high" ? "#f97316" : "#818cf8";
  ctx.lineWidth = 1.5;
  ctx.strokeRect(x + 0.75, y + 0.75, w - 1.5, h - 1.5);
  ctx.restore();
  ctx.globalAlpha = 1;
}
