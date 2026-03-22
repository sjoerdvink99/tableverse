import { useEffect, useRef } from "react";
import type { Table } from "apache-arrow";
import { useTableStore } from "../../stores/table";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useStatsStore } from "../../stores/stats";
import { DEFAULT_CELL_W, DEFAULT_CELL_H, tileKey, tileRowsForZoom, tileColsForZoom } from "../../lib/viewport";
import { renderCellSemantic, resolveRenderMode } from "../../lib/semantic-render";
import type { RenderMode } from "../../lib/semantic-render";
import { renderSatellitePass } from "../../lib/profile-render";
import type { SatelliteEncoding } from "../../lib/profile-render";
import { HeatmapWebGL } from "../../lib/webgl-heatmap";
import type { ColumnInfo, ColumnStats, SourceMeta } from "../../lib/types";

const EMPTY_FONT = "14px ui-sans-serif, system-ui, sans-serif";

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

type CanvasColors = {
  bg: string;
  oddRow: string;
  grid: string;
  shimmer: string;
  nullBg: string;
  selFill: string;
  selBorder: string;
  emptyText: string;
};

function readCanvasColors(): CanvasColors {
  return {
    bg: cssVar("--canvas-bg") || "#ffffff",
    oddRow: cssVar("--canvas-odd-row") || "#fafafa",
    grid: cssVar("--canvas-grid") || "#e5e7eb",
    shimmer: cssVar("--canvas-shimmer") || "#f3f4f6",
    nullBg: cssVar("--canvas-null-bg") || "#f9fafb",
    selFill: cssVar("--canvas-sel-fill") || "rgba(59,130,246,0.15)",
    selBorder: cssVar("--canvas-sel-border") || "#3b82f6",
    emptyText: cssVar("--canvas-text-2") || "#6b7280",
  };
}

type Props = {
  width: number;
  height: number;
};

function encodingToChannel(enc: SatelliteEncoding): number {
  if (enc === "null_rate") return 0;
  if (enc === "mean_normalized") return 1;
  return 2;
}

export function GridCanvas({ width, height }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayCanvasRef = useRef<HTMLCanvasElement>(null);
  const webglCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const webglRef = useRef<HeatmapWebGL | null>(null);
  const prevZoomRef = useRef(0);
  const prevSourceRef = useRef<string | null>(null);

  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const tiles = useTableStore((s) => s.tiles);
  const loading = useTableStore((s) => s.loading);
  const provisionalTiles = useTableStore((s) => s.provisionalTiles);
  const staleTiles = useTableStore((s) => s.staleTiles);
  const selection = useTableStore((s) => s.selection);
  const viewHash = useViewStore((s) => s.viewHash);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const virtualSchema = useViewStore((s) => s.virtualSchema);
  const zoom = useUiStore((s) => s.zoom);
  const isDark = useUiStore((s) => s.isDark);
  const semanticZoomEnabled = useUiStore((s) => s.semanticZoomEnabled);
  const satelliteEncoding = useUiStore((s) => s.satelliteEncoding);
  const allStats = useStatsStore((s) => s.stats);
  const rowGroupStats = useStatsStore((s) => s.rowGroupStats);

  const cellW = DEFAULT_CELL_W * zoom;
  const cellH = DEFAULT_CELL_H * zoom;
  const effectiveRowCount = virtualRowCount ?? source?.n_rows ?? 0;

  useEffect(() => {
    const canvas = webglCanvasRef.current;
    if (!canvas) return;
    const instance = new HeatmapWebGL(canvas);
    webglRef.current = instance;
    return () => {
      instance.dispose();
      webglRef.current = null;
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !source) return;

    const dpr = window.devicePixelRatio || 1;

    prevSourceRef.current = source.id;
    prevZoomRef.current = zoom;

    canvas.width = width * dpr;
    canvas.height = height * dpr;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;

    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);

    const colors = readCanvasColors();

    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, width, height);

    if (effectiveRowCount === 0) {
      renderEmpty(ctx, width, height, colors);
      return;
    }

    const modeRes = semanticZoomEnabled
      ? resolveRenderMode(zoom)
      : { primary: "read" as RenderMode, secondary: null, blend: 0 };

    const isSatellitePrimary = modeRes.primary === "satellite";
    const isSatelliteBlending = modeRes.secondary === "satellite" || (isSatellitePrimary && modeRes.secondary !== null);

    if (isSatellitePrimary && modeRes.secondary === null) {
      const colStart = Math.floor(viewport.scrollX / cellW);
      const colEnd = Math.min(source.n_cols, Math.ceil((viewport.scrollX + width) / cellW) + 1);
      const webgl = webglRef.current;
      const glCanvas = webglCanvasRef.current;
      if (webgl?.isAvailable() && glCanvas && rowGroupStats.size > 0) {
        const visibleCols: number[] = [];
        for (let c = colStart; c < colEnd; c++) {
          if (rowGroupStats.has(`${source.id}:${c}`)) visibleCols.push(c);
        }
        if (visibleCols.length > 0) {
          const firstCol = visibleCols[0]!;
          const sampleStats = rowGroupStats.get(`${source.id}:${firstCol}`)!;
          const nRgs = sampleStats.length;
          const nCols = visibleCols.length;
          const data = new Uint8Array(nRgs * nCols * 4);
          for (let ci = 0; ci < nCols; ci++) {
            const col = visibleCols[ci]!;
            const stats = rowGroupStats.get(`${source.id}:${col}`);
            if (!stats) continue;
            for (let ri = 0; ri < nRgs; ri++) {
              const rg = stats[ri];
              if (!rg) continue;
              const nullRate = rg.row_count > 0 ? rg.null_count / rg.row_count : 0;
              const base = (ri * nCols + ci) * 4;
              data[base] = Math.round(nullRate * 255);
              data[base + 1] = rg.mean !== null ? Math.round(Math.max(0, Math.min(1, rg.mean)) * 255) : 0;
              const spread = rg.min !== null && rg.max !== null ? rg.max - rg.min : 0;
              data[base + 2] = Math.round(Math.min(255, Math.max(0, spread)));
              data[base + 3] = 255;
            }
          }
          const dpr = window.devicePixelRatio || 1;
          glCanvas.width = width * dpr;
          glCanvas.height = height * dpr;
          glCanvas.style.width = `${width}px`;
          glCanvas.style.height = `${height}px`;
          webgl.resize(width * dpr, height * dpr);
          webgl.render(data, nRgs, nCols, encodingToChannel(satelliteEncoding), isDark);
          return;
        }
      }
      renderSatellitePass(ctx, width, height, cellW, cellH, viewport.scrollX, viewport.scrollY, colStart, colEnd, source, rowGroupStats, satelliteEncoding, isDark);
      return;
    }

    const tileRows = tileRowsForZoom(zoom);
    const tileCols = tileColsForZoom(zoom);

    if (isSatelliteBlending && modeRes.secondary !== null) {
      const colStart = Math.floor(viewport.scrollX / cellW);
      const colEnd = Math.min(source.n_cols, Math.ceil((viewport.scrollX + width) / cellW) + 1);
      const satAlpha = isSatellitePrimary ? 1.0 - modeRes.blend : modeRes.blend;
      ctx.globalAlpha = satAlpha;
      renderSatellitePass(ctx, width, height, cellW, cellH, viewport.scrollX, viewport.scrollY, colStart, colEnd, source, rowGroupStats, satelliteEncoding, isDark);
      ctx.globalAlpha = 1;
      const cellMode = isSatellitePrimary ? modeRes.secondary : modeRes.primary;
      const cellAlpha = 1.0 - satAlpha;
      renderCells(ctx, width, height, cellW, cellH, viewport.scrollX, viewport.scrollY, tiles, loading, provisionalTiles, staleTiles, viewHash, effectiveRowCount, source.n_cols, virtualSchema, colors, source, allStats, { primary: cellMode, secondary: null, blend: 0 }, isDark, cellAlpha, tileRows, tileCols);
      return;
    }

    renderCells(
      ctx, width, height, cellW, cellH,
      viewport.scrollX, viewport.scrollY,
      tiles, loading, provisionalTiles, staleTiles, viewHash,
      effectiveRowCount, source.n_cols,
      virtualSchema, colors,
      source, allStats, modeRes, isDark, 1.0, tileRows, tileCols,
    );
  }, [width, height, source, viewport, tiles, loading, provisionalTiles, staleTiles, viewHash, zoom, effectiveRowCount, virtualSchema, isDark, semanticZoomEnabled, allStats, rowGroupStats, satelliteEncoding, cellW, cellH]);

  useEffect(() => {
    const overlay = overlayCanvasRef.current;
    if (!overlay) return;

    const dpr = window.devicePixelRatio || 1;
    overlay.width = width * dpr;
    overlay.height = height * dpr;
    overlay.style.width = `${width}px`;
    overlay.style.height = `${height}px`;

    const ctx = overlay.getContext("2d")!;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);

    if (!selection || !source) return;

    const colors = readCanvasColors();
    const selMinRow = Math.min(selection.anchor.row, selection.active.row);
    const selMaxRow = Math.max(selection.anchor.row, selection.active.row);
    const selMinCol = Math.min(selection.anchor.col, selection.active.col);
    const selMaxCol = Math.max(selection.anchor.col, selection.active.col);

    if (selMinRow < 0) return;

    const x = selMinCol * cellW - viewport.scrollX;
    const y = selMinRow * cellH - viewport.scrollY;
    const w = (selMaxCol - selMinCol + 1) * cellW;
    const h = (selMaxRow - selMinRow + 1) * cellH;

    ctx.fillStyle = colors.selFill;
    ctx.fillRect(x, y, w, h);

    ctx.strokeStyle = colors.selBorder;
    ctx.lineWidth = 2;
    ctx.strokeRect(x, y, w, h);
    ctx.lineWidth = 1;
  }, [selection, width, height, viewport, cellW, cellH, source]);

  return (
    <div style={{ position: "relative", width, height }}>
      <canvas ref={canvasRef} style={{ position: "absolute", top: 0, left: 0, display: "block" }} />
      <canvas
        ref={webglCanvasRef}
        style={{ position: "absolute", top: 0, left: 0, display: "block", pointerEvents: "none" }}
      />
      <canvas ref={overlayCanvasRef} style={{ position: "absolute", top: 0, left: 0, display: "block", pointerEvents: "none" }} />
    </div>
  );
}

function renderEmpty(ctx: CanvasRenderingContext2D, width: number, height: number, colors: CanvasColors) {
  ctx.fillStyle = colors.emptyText;
  ctx.font = EMPTY_FONT;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("No rows match the active filters", width / 2, height / 2);
}

function renderCells(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  cellW: number,
  cellH: number,
  scrollX: number,
  scrollY: number,
  tiles: Map<string, Table>,
  loading: Set<string>,
  provisionalTiles: Set<string>,
  staleTiles: Map<string, Table>,
  viewHash: string,
  nRows: number,
  nCols: number,
  virtualSchema: ColumnInfo[] | null,
  colors: CanvasColors,
  source: SourceMeta,
  allStats: Map<string, ColumnStats>,
  modeRes: { primary: RenderMode; secondary: RenderMode | null; blend: number },
  isDark: boolean,
  baseAlpha: number,
  tileRows: number,
  tileCols: number,
) {
  const colStart = Math.floor(scrollX / cellW);
  const rowStart = Math.floor(scrollY / cellH);
  const colEnd = Math.min(nCols, Math.ceil((scrollX + width) / cellW) + 1);
  const rowEnd = Math.min(nRows, Math.ceil((scrollY + height) / cellH) + 1);

  const showOddRow = modeRes.primary !== "profile" && modeRes.primary !== "heatmap";

  for (let row = rowStart; row < rowEnd; row++) {
    const tileRow = Math.floor(row / tileRows);
    const localRow = row % tileRows;
    const y = row * cellH - scrollY;

    if (showOddRow && row % 2 === 1) {
      ctx.fillStyle = colors.oddRow;
      ctx.fillRect(0, y, width, cellH);
    }

    const firstTileKey = tileKey(tileRow, Math.floor(colStart / tileCols), viewHash);
    const rowIsProvisional = provisionalTiles.has(firstTileKey);

    for (let col = colStart; col < colEnd; col++) {
      const tileCol = Math.floor(col / tileCols);
      const x = col * cellW - scrollX;

      if (cellH >= 4) {
        ctx.strokeStyle = colors.grid;
        ctx.lineWidth = 1;
        ctx.strokeRect(x, y, cellW, cellH);
      }

      const colInfo = virtualSchema?.[col] ?? source.columns[col];
      if (!colInfo) continue;

      const qStats = source.quick_stats?.[col];
      const fullStats = allStats.get(`${source.id}:${col}`);

      const tKey = tileKey(tileRow, tileCol, viewHash);
      const tile = tiles.get(tKey);
      const staleTile = !tile ? staleTiles.get(`${tileRow}:${tileCol}`) : undefined;

      if (!tile && !staleTile) {
        if (loading.has(tKey)) {
          ctx.fillStyle = colors.shimmer;
          ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
        }
        continue;
      }

      const activeTile = tile ?? staleTile!;
      const isStale = !tile && !!staleTile;
      if (isStale) ctx.globalAlpha = baseAlpha * 0.55;

      const column = activeTile.getChildAt(col % tileCols);
      if (!column || localRow >= column.length) {
        if (isStale) ctx.globalAlpha = 1;
        continue;
      }

      const value = column.get(localRow);
      const isValueNull = value === null || value === undefined;

      const isTextLike = modeRes.primary === "scan" || modeRes.primary === "read";
      if (isValueNull && isTextLike) {
        ctx.fillStyle = colors.nullBg;
        ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
      }

      const effectiveAlpha = isStale ? baseAlpha * 0.55 : baseAlpha;
      if (modeRes.secondary === null) {
        renderCellSemantic(ctx, x, y, cellW, cellH, value, colInfo, qStats, fullStats, modeRes.primary, effectiveAlpha, isDark);
      } else {
        renderCellSemantic(ctx, x, y, cellW, cellH, value, colInfo, qStats, fullStats, modeRes.primary, effectiveAlpha * (1.0 - modeRes.blend), isDark);
        renderCellSemantic(ctx, x, y, cellW, cellH, value, colInfo, qStats, fullStats, modeRes.secondary, effectiveAlpha * modeRes.blend, isDark);
      }
      if (isStale) ctx.globalAlpha = 1;
    }

    if (rowIsProvisional && cellH >= 4) {
      ctx.strokeStyle = "#D97706";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(0, y + cellH - 1);
      ctx.lineTo(width, y + cellH - 1);
      ctx.stroke();
      ctx.lineWidth = 1;
    }
  }
}
