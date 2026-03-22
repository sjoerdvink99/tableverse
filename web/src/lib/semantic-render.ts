import { renderCell } from "./format";
import {
  viridis, pubugn, categoricalColor, djb2,
  rdbu, normalizeByQuantiles, outlierClass,
  drawNullHatch, drawOutlierBorder,
} from "./color-scales";
import type { ColumnInfo, QuickColumnStats, ColumnStats } from "./types";

export type RenderMode = "satellite" | "profile" | "heatmap" | "scan" | "read";
export type ModeResolution = { primary: RenderMode; secondary: RenderMode | null; blend: number };

const MODE_THRESHOLDS: [number, RenderMode][] = [
  [0.10, "satellite"],
  [0.28, "profile"],
  [0.55, "heatmap"],
  [0.85, "scan"],
  [3.00, "read"],
];

export function resolveRenderMode(zoom: number): ModeResolution {
  const BLEND_ZONE_FRACTION = 0.15;

  for (let i = 0; i < MODE_THRESHOLDS.length - 1; i++) {
    const [lo, loMode] = MODE_THRESHOLDS[i]!;
    const [hi, hiMode] = MODE_THRESHOLDS[i + 1]!;

    if (zoom >= lo && zoom < hi) {
      const range = hi - lo;
      const blendZone = range * BLEND_ZONE_FRACTION;

      if (zoom >= hi - blendZone) {
        const t = (zoom - (hi - blendZone)) / blendZone;
        return { primary: loMode, secondary: hiMode, blend: t };
      }
      if (zoom < lo + blendZone && i > 0) {
        const t = 1.0 - (zoom - lo) / blendZone;
        return { primary: loMode, secondary: MODE_THRESHOLDS[i - 1]![1], blend: t };
      }
      return { primary: loMode, secondary: null, blend: 0 };
    }
  }

  return { primary: "read", secondary: null, blend: 0 };
}

export function classifyDataType(dt: string): "numeric" | "boolean" | "temporal" | "string" | "unknown" {
  const d = dt.toLowerCase();
  if (d === "boolean" || d === "bool") return "boolean";
  if (d.includes("int") || d.includes("float") || d.includes("double") || d.includes("decimal") || d.includes("numeric")) return "numeric";
  if (d.includes("timestamp") || d.includes("date")) return "temporal";
  if (d.includes("utf8") || d.includes("varchar") || d.includes("char") || d === "string" || d === "text") return "string";
  return "unknown";
}

function drawClippedText(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  text: string, color: string,
  align: "left" | "right" | "center",
  alpha: number,
  fontSize: number,
): void {
  ctx.save();
  ctx.beginPath();
  ctx.rect(x, y, w, h);
  ctx.clip();
  ctx.globalAlpha = alpha;
  ctx.fillStyle = color;
  ctx.font = `${fontSize}px ui-monospace, monospace`;
  ctx.textBaseline = "middle";
  const PADDING = 6;
  if (align === "right") {
    ctx.textAlign = "right";
    ctx.fillText(text, x + w - PADDING, y + h / 2);
  } else if (align === "center") {
    ctx.textAlign = "center";
    ctx.fillText(text, x + w / 2, y + h / 2);
  } else {
    ctx.textAlign = "left";
    ctx.fillText(text, x + PADDING, y + h / 2);
  }
  ctx.restore();
  ctx.globalAlpha = 1;
}

export function renderCellSemantic(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  value: unknown,
  colInfo: ColumnInfo,
  qStats: QuickColumnStats | undefined,
  fullStats: ColumnStats | undefined,
  mode: RenderMode,
  alpha: number,
  isDark: boolean,
): void {
  if (mode === "satellite") return;

  const isNull = value === null || value === undefined;

  if (mode === "read") {
    if (isNull) {
      const nullColor = isDark ? "#4b5563" : "#9ca3af";
      drawClippedText(ctx, x, y, w, h, "—", nullColor, "left", alpha * 0.7, 13);
      return;
    }
    const info = renderCell(value, colInfo.data_type);
    const fontSize = Math.max(11, Math.min(24, Math.round(h * 0.55)));
    drawClippedText(ctx, x, y, w, h, info.text, info.color, info.align, alpha, fontSize);
    return;
  }

  if (mode === "scan") {
    renderCellScan(ctx, x, y, w, h, value, colInfo, qStats, fullStats, alpha, isDark);
    return;
  }

  if (mode === "heatmap") {
    renderCellHeatmap(ctx, x, y, w, h, value, colInfo, qStats, fullStats, alpha, isDark);
    return;
  }

  if (mode === "profile") {
    renderCellProfile(ctx, x, y, w, h, value, colInfo, qStats, fullStats, alpha, isDark);
    return;
  }
}

function renderCellProfile(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  value: unknown,
  colInfo: ColumnInfo,
  qStats: QuickColumnStats | undefined,
  _fullStats: ColumnStats | undefined,
  alpha: number,
  isDark: boolean,
): void {
  const isNull = value === null || value === undefined;
  const dt = classifyDataType(colInfo.data_type);

  if (isNull) {
    drawNullHatch(ctx, x, y, w, h, isDark);
    return;
  }

  if (dt === "boolean") {
    const boolVal = typeof value === "boolean" ? value : String(value).toLowerCase() === "true";
    ctx.globalAlpha = alpha * 0.75;
    ctx.fillStyle = boolVal ? "#22c55e" : "#ef4444";
    ctx.fillRect(x, y, w, h);
    ctx.globalAlpha = 1;
    return;
  }

  if (dt === "numeric") {
    const num = Number(value);
    const minVal = qStats?.min !== undefined ? Number(qStats.min) : null;
    const maxVal = qStats?.max !== undefined ? Number(qStats.max) : null;
    const range = minVal !== null && maxVal !== null ? maxVal - minVal : 0;
    const t = range > 0 ? Math.max(0, Math.min(1, (num - minVal!) / range)) : 0.5;
    const barW = Math.max(1, t * (w - 2));
    const [r, g, b] = viridis(t);
    ctx.globalAlpha = alpha * 0.85;
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(x + 1, y + 1, barW, h - 2);
    ctx.globalAlpha = 1;
    return;
  }

  if (dt === "temporal") {
    const num = Number(value);
    const minVal = qStats?.min !== undefined ? Number(qStats.min) : null;
    const maxVal = qStats?.max !== undefined ? Number(qStats.max) : null;
    const range = minVal !== null && maxVal !== null ? maxVal - minVal : 0;
    const t = range > 0 ? Math.max(0, Math.min(1, (num - minVal!) / range)) : 0.5;
    const [r, g, b] = pubugn(t);
    ctx.globalAlpha = alpha * 0.8;
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(x, y, w, h);
    ctx.globalAlpha = 1;
    return;
  }

  const hash = djb2(String(value));
  const isHighCard = _fullStats?.cardinality_category === "high_cardinality" || _fullStats?.cardinality_category === "unique";
  const [r, g, b] = isHighCard ? [153, 153, 153] as [number, number, number] : categoricalColor(hash);
  ctx.globalAlpha = alpha * 0.65;
  ctx.fillStyle = `rgb(${r},${g},${b})`;
  ctx.fillRect(x, y, w, h);
  ctx.globalAlpha = 1;
}

function renderCellHeatmap(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  value: unknown,
  colInfo: ColumnInfo,
  qStats: QuickColumnStats | undefined,
  fullStats: ColumnStats | undefined,
  alpha: number,
  isDark: boolean,
): void {
  const isNull = value === null || value === undefined;
  const dt = classifyDataType(colInfo.data_type);

  if (isNull) {
    drawNullHatch(ctx, x, y, w, h, isDark);
    return;
  }

  if (dt === "boolean") {
    const boolVal = typeof value === "boolean" ? value : String(value).toLowerCase() === "true";
    ctx.globalAlpha = alpha * 0.6;
    ctx.fillStyle = boolVal ? "#22c55e" : "#ef4444";
    ctx.fillRect(x, y, w, h);
    ctx.globalAlpha = 1;
    return;
  }

  if (dt === "numeric") {
    const num = Number(value);

    if (fullStats?.quantiles) {
      const t = normalizeByQuantiles(num, fullStats.quantiles);
      const [r, g, b] = rdbu(t);
      ctx.globalAlpha = alpha;
      ctx.fillStyle = `rgb(${r},${g},${b})`;
      ctx.fillRect(x, y, w, h);
      ctx.globalAlpha = 1;

      const oc = outlierClass(num, fullStats.quantiles);
      if (oc !== "none") {
        drawOutlierBorder(ctx, x, y, w, h, oc, alpha * 0.8);
      }
    } else {
      const minVal = qStats?.min !== undefined ? Number(qStats.min) : null;
      const maxVal = qStats?.max !== undefined ? Number(qStats.max) : null;
      const range = minVal !== null && maxVal !== null ? maxVal - minVal : 0;
      const t = range > 0 ? Math.max(0, Math.min(1, (num - minVal!) / range)) : 0.5;
      const [r, g, b] = viridis(t);
      ctx.globalAlpha = alpha;
      ctx.fillStyle = `rgb(${r},${g},${b})`;
      ctx.fillRect(x, y, w, h);
      ctx.globalAlpha = 1;
    }
    return;
  }

  if (dt === "temporal") {
    const num = Number(value);
    const minVal = qStats?.min !== undefined ? Number(qStats.min) : null;
    const maxVal = qStats?.max !== undefined ? Number(qStats.max) : null;
    const range = minVal !== null && maxVal !== null ? maxVal - minVal : 0;
    const t = range > 0 ? Math.max(0, Math.min(1, (num - minVal!) / range)) : 0.5;
    const [r, g, b] = pubugn(t);
    ctx.globalAlpha = alpha;
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(x, y, w, h);
    ctx.globalAlpha = 1;
    return;
  }

  const hash = djb2(String(value));
  const isHighCard = fullStats?.cardinality_category === "high_cardinality" || fullStats?.cardinality_category === "unique";
  const [r, g, b] = isHighCard ? [153, 153, 153] as [number, number, number] : categoricalColor(hash);
  ctx.globalAlpha = alpha * 0.5;
  ctx.fillStyle = `rgb(${r},${g},${b})`;
  ctx.fillRect(x, y, w, h);
  ctx.globalAlpha = 1;
}

function renderCellScan(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
  value: unknown,
  colInfo: ColumnInfo,
  qStats: QuickColumnStats | undefined,
  fullStats: ColumnStats | undefined,
  alpha: number,
  isDark: boolean,
): void {
  const isNull = value === null || value === undefined;
  const dt = colInfo.data_type.toLowerCase();

  if (isNull) {
    const nullColor = isDark ? "#4b5563" : "#94a3b8";
    const fontSize = Math.max(7, Math.min(12, h * 0.5));
    drawClippedText(ctx, x, y, w, h, "—", nullColor, "left", alpha * 0.8, fontSize);
    return;
  }

  let text: string;
  let align: "left" | "right" | "center" = "left";
  let color: string;

  if (dt === "boolean" || typeof value === "boolean") {
    const boolVal = typeof value === "boolean" ? value : String(value).toLowerCase() === "true";
    text = boolVal ? "true" : "false";
    align = "center";
    color = boolVal ? "#16a34a" : "#dc2626";
  } else if (
    dt.includes("int") || dt.includes("float") || dt.includes("double") ||
    dt.includes("decimal") || typeof value === "number" || typeof value === "bigint"
  ) {
    text = toCompact(Number(value));
    align = "right";
    color = isDark ? "#93c5fd" : "#1d4ed8";
  } else if (dt.includes("timestamp") || dt.includes("date")) {
    text = abbreviateDate(value);
    align = "left";
    color = isDark ? "#e5e7eb" : "#374151";
  } else {
    const s = String(value);
    text = s.length > 5 ? s.slice(0, 5) + "…" : s;
    align = "left";
    color = isDark ? "#e5e7eb" : "#374151";
  }

  const fontSize = Math.max(7, Math.min(12, h * 0.5));
  drawClippedText(ctx, x, y, w, h, text, color, align, alpha, fontSize);

  if (fullStats?.quantiles && (dt.includes("int") || dt.includes("float") || dt.includes("double") || dt.includes("decimal") || typeof value === "number")) {
    const oc = outlierClass(Number(value), fullStats.quantiles);
    if (oc !== "none") {
      const dotColor = oc === "high" ? "#f97316" : "#818cf8";
      ctx.save();
      ctx.globalAlpha = alpha * 0.9;
      ctx.fillStyle = dotColor;
      ctx.beginPath();
      ctx.arc(x + w - 4, y + 4, 2.5, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      ctx.globalAlpha = 1;
    }
  }

  const nullRate = qStats?.null_rate ?? 0;
  if (nullRate > 0.02) {
    const [r, g, b] = [nullRate < 0.05 ? 34 : nullRate < 0.2 ? 245 : 239, nullRate < 0.05 ? 197 : nullRate < 0.2 ? 158 : 68, nullRate < 0.05 ? 94 : nullRate < 0.2 ? 11 : 68];
    ctx.save();
    ctx.globalAlpha = alpha * 0.6;
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(x, y + h - 1, w, 1);
    ctx.restore();
    ctx.globalAlpha = 1;
  }
}

function toCompact(n: number): string {
  if (!isFinite(n)) return String(n);
  const abs = Math.abs(n);
  const sign = n < 0 ? "-" : "";
  if (abs >= 1_000_000_000) return `${sign}${(abs / 1_000_000_000).toFixed(1)}B`;
  if (abs >= 1_000_000) return `${sign}${(abs / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${sign}${(abs / 1_000).toFixed(1)}K`;
  if (Number.isInteger(n)) return String(n);
  return n.toPrecision(3).replace(/\.?0+$/, "");
}

function abbreviateDate(value: unknown): string {
  const s = String(value);
  const d = s.replace("T", " ").slice(0, 10);
  const parts = d.split("-");
  if (parts.length >= 3) {
    const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const monthIdx = parseInt(parts[1]!, 10) - 1;
    const day = parseInt(parts[2]!, 10);
    return `${day} ${months[monthIdx] ?? "?"}`;
  }
  return s.slice(0, 6);
}
