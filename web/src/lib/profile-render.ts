import { viridis, nullRateColor, interpolate } from "./color-scales";
import type { RowGroupStat, SourceMeta } from "./types";

export type SatelliteEncoding = "null_rate" | "mean_normalized" | "spread";

export function renderSatellitePass(
  ctx: CanvasRenderingContext2D,
  canvasW: number,
  canvasH: number,
  cellW: number,
  cellH: number,
  scrollX: number,
  scrollY: number,
  colStart: number,
  colEnd: number,
  source: SourceMeta,
  rowGroupStatsByCol: Map<string, RowGroupStat[]>,
  encoding: SatelliteEncoding,
  isDark: boolean,
): void {
  const shimmer = isDark ? "#1f2937" : "#f3f4f6";
  const colSep = isDark ? "rgba(255,255,255,0.07)" : "rgba(0,0,0,0.06)";
  const rgSep = isDark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.04)";

  for (let col = colStart; col < colEnd; col++) {
    const x = col * cellW - scrollX;
    const key = `${source.id}:${col}`;
    const rgStats = rowGroupStatsByCol.get(key);

    if (!rgStats || rgStats.length === 0) {
      ctx.fillStyle = shimmer;
      ctx.fillRect(x, 0, cellW, canvasH);
      continue;
    }

    const colorFn = buildColorFn(rgStats, encoding, isDark);

    for (const rg of rgStats) {
      const rgY = rg.row_offset * cellH - scrollY;
      const rgH = Math.max(1, rg.row_count * cellH);
      if (rgY + rgH < 0 || rgY > canvasH) continue;

      const clipY = Math.max(0, rgY);
      const clipH = Math.min(rgH, canvasH - clipY) + Math.min(0, rgY);
      if (clipH <= 0) continue;

      ctx.fillStyle = colorFn(rg);
      ctx.fillRect(x, clipY, cellW, clipH);
    }

    ctx.strokeStyle = rgSep;
    ctx.lineWidth = 0.5;
    for (const rg of rgStats) {
      const rgY = rg.row_offset * cellH - scrollY;
      if (rgY > 1 && rgY < canvasH - 1) {
        ctx.beginPath();
        ctx.moveTo(x, rgY);
        ctx.lineTo(x + cellW, rgY);
        ctx.stroke();
      }
    }
  }

  ctx.strokeStyle = colSep;
  ctx.lineWidth = 0.5;
  for (let col = colStart; col <= colEnd; col++) {
    const x = col * cellW - scrollX;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, canvasH);
    ctx.stroke();
  }
}

function buildColorFn(
  rgStats: RowGroupStat[],
  encoding: SatelliteEncoding,
  isDark: boolean,
): (rg: RowGroupStat) => string {
  const noData = isDark ? "#1f2937" : "#f3f4f6";

  if (encoding === "null_rate") {
    return (rg) => {
      const rate = rg.row_count > 0 ? rg.null_count / rg.row_count : 0;
      const [r, g, b] = nullRateColor(rate);
      return `rgb(${r},${g},${b})`;
    };
  }

  if (encoding === "mean_normalized") {
    let globalMin = Infinity;
    let globalMax = -Infinity;
    for (const rg of rgStats) {
      if (rg.mean !== null) {
        if (rg.mean < globalMin) globalMin = rg.mean;
        if (rg.mean > globalMax) globalMax = rg.mean;
      }
    }
    const range = globalMax - globalMin;

    return (rg) => {
      if (rg.mean === null) return noData;
      const t = range <= 0 ? 0.5 : (rg.mean - globalMin) / range;
      const [r, g, b] = viridis(Math.max(0, Math.min(1, t)));
      return `rgb(${r},${g},${b})`;
    };
  }

  let globalSpread = 0;
  for (const rg of rgStats) {
    if (rg.min !== null && rg.max !== null) {
      const s = rg.max - rg.min;
      if (s > globalSpread) globalSpread = s;
    }
  }

  return (rg) => {
    if (rg.min === null || rg.max === null) return noData;
    const spread = rg.max - rg.min;
    const t = globalSpread <= 0 ? 0.5 : Math.min(1, spread / globalSpread);
    const [r, g, b] = interpolate(
      [[240, 249, 255], [56, 189, 248], [2, 132, 199]],
      t,
    );
    return `rgb(${r},${g},${b})`;
  };
}

export function buildDriftSparklinePoints(
  rgStats: RowGroupStat[],
  x: number,
  y: number,
  w: number,
  h: number,
): Array<{ px: number; py: number }> | null {
  const means = rgStats.map((rg) => rg.mean ?? ((rg.min !== null && rg.max !== null) ? (rg.min + rg.max) / 2 : null));
  const valid = means.filter((m): m is number => m !== null);
  if (valid.length < 2) return null;

  const minV = Math.min(...valid);
  const maxV = Math.max(...valid);
  const range = maxV - minV;

  return means.map((m, i) => {
    const px = x + (i / (means.length - 1)) * w;
    const py = range <= 0
      ? y + h / 2
      : y + h - ((m ?? minV) - minV) / range * h;
    return { px, py };
  });
}
