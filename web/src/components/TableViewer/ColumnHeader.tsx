import { useEffect, useRef, useState } from "react";
import { useTableStore } from "../../stores/table";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useStatsStore, getStatsKey } from "../../stores/stats";
import { DEFAULT_CELL_W, HEADER_HEIGHT } from "../../lib/viewport";
import { activePredicates } from "../../lib/addPredicate";
import { categoricalColor, nullRateColor, rdbu } from "../../lib/color-scales";
import { speculativeSort } from "../../lib/api";
import { buildDriftSparklinePoints } from "../../lib/profile-render";

type Props = {
  width: number;
  leftOffset: number;
};

const FONT_BOLD = "bold 12px ui-sans-serif, sans-serif";
const FONT_TYPE = "10px ui-monospace, monospace";
const FONT_SMALL = "bold 9px ui-sans-serif, sans-serif";
const SPARKLINE_HEIGHT = 22;
const NULL_BAR_H = 3;

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function headerColors() {
  return {
    bg: cssVar("--canvas-header-bg") || "#f9fafb",
    border: cssVar("--canvas-grid") || "#e5e7eb",
    text: cssVar("--canvas-text") || "#111827",
    textType: cssVar("--canvas-text-2") || "#6b7280",
    sort: cssVar("--canvas-sort-color") || "#3b82f6",
    filterDot: cssVar("--canvas-filter-dot") || "#f59e0b",
    filterBar: "#fbbf24",
    sparkline: cssVar("--canvas-sparkline") || "#93c5fd",
    drift: "#94a3b8",
  };
}

function dataTypeInfo(dt: string): { code: string; swatchIdx: number } {
  const d = dt.toLowerCase();
  if (d.includes("int") || d.includes("float") || d.includes("double") || d.includes("decimal")) {
    return { code: "N", swatchIdx: 4 };
  }
  if (d === "boolean" || d === "bool") return { code: "B", swatchIdx: 2 };
  if (d.includes("timestamp") || d.includes("date")) return { code: "D", swatchIdx: 0 };
  if (d.includes("utf8") || d.includes("varchar") || d === "string") return { code: "S", swatchIdx: 1 };
  return { code: "?", swatchIdx: 5 };
}

function drawNullRateBar(
  ctx: CanvasRenderingContext2D,
  x: number,
  cellW: number,
  nullRate: number,
  hasFilter: boolean,
): void {
  if (nullRate <= 0.01 || hasFilter) return;
  const [r, g, b] = nullRateColor(nullRate);
  const fillW = Math.max(2, Math.round(cellW * nullRate));
  ctx.globalAlpha = 0.85;
  ctx.fillStyle = `rgb(${r},${g},${b})`;
  ctx.fillRect(x, HEADER_HEIGHT - NULL_BAR_H, fillW, NULL_BAR_H);
  ctx.globalAlpha = 0.2;
  ctx.fillRect(x + fillW, HEADER_HEIGHT - NULL_BAR_H, cellW - fillW, NULL_BAR_H);
  ctx.globalAlpha = 1;
}

export function ColumnHeader({ width, leftOffset }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const ops = useViewStore((s) => s.ops);
  const setSort = useViewStore((s) => s.setSort);
  const zoom = useUiStore((s) => s.zoom);
  const isDark = useUiStore((s) => s.isDark);
  const setContextMenu = useUiStore((s) => s.setContextMenu);
  const setHoveredColumnIndex = useUiStore((s) => s.setHoveredColumnIndex);
  const incrementSortAccess = useUiStore((s) => s.incrementSortAccess);
  const pinnedCorrelationCol = useUiStore((s) => s.pinnedCorrelationCol);
  const setPinnedCorrelationCol = useUiStore((s) => s.setPinnedCorrelationCol);
  const sourceId = useViewStore((s) => s.sourceId);
  const allStats = useStatsStore((s) => s.stats);
  const fetchStats = useStatsStore((s) => s.fetchStats);
  const rowGroupStats = useStatsStore((s) => s.rowGroupStats);
  const correlations = useStatsStore((s) => s.correlations);

  const cellW = DEFAULT_CELL_W * zoom;

  const activeJobId = useUiStore((s) => s.activeJobId);
  const [spinnerAngle, setSpinnerAngle] = useState(0);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    if (!activeJobId) {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      return;
    }
    const step = () => {
      setSpinnerAngle((a) => (a + 0.12) % (Math.PI * 2));
      rafRef.current = requestAnimationFrame(step);
    };
    rafRef.current = requestAnimationFrame(step);
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, [activeJobId]);

  const sortOp = ops.find((op) => op.type === "sort");
  const sortKeys = sortOp?.type === "sort" ? sortOp.keys : [];
  const activePreds = activePredicates(ops);
  const filteredColumns = new Set(activePreds.flatMap((p) => {
    if (p.op === "and" || p.op === "or" || p.op === "not") return [];
    return [p.column];
  }));

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !source) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = width * dpr;
    canvas.height = HEADER_HEIGHT * dpr;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${HEADER_HEIGHT}px`;

    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);

    const colors = headerColors();

    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, width, HEADER_HEIGHT);

    const colStart = Math.floor(viewport.scrollX / cellW);
    const colEnd = Math.min(source.n_cols, Math.ceil((viewport.scrollX + width) / cellW) + 1);

    ctx.textBaseline = "middle";

    const pinnedColName = pinnedCorrelationCol !== null ? source.columns[pinnedCorrelationCol]?.name : null;
    const corrMatrix = correlations?.sourceId === source.id ? correlations.matrix : null;
    const corrColNames = corrMatrix ? corrMatrix.columns : [];

    for (let col = colStart; col < colEnd; col++) {
      const info = source.columns[col];
      if (!info) continue;
      const x = col * cellW - viewport.scrollX;

      ctx.fillStyle = colors.bg;
      ctx.fillRect(x, 0, cellW, HEADER_HEIGHT);

      ctx.strokeStyle = colors.border;
      ctx.lineWidth = 1;
      ctx.strokeRect(x, 0, cellW, HEADER_HEIGHT);

      const sortKey = sortKeys.find((k) => k.column === info.name);
      const hasFilter = filteredColumns.has(info.name);

      if (hasFilter) {
        ctx.fillStyle = colors.filterBar;
        ctx.fillRect(x, 0, cellW, 3);
      }

      const quickStat = source.quick_stats?.[col];
      const statsKey = getStatsKey(source.id, col);
      const colStats = allStats.get(statsKey);

      const nullRate = quickStat?.null_rate ?? 0;

      if (pinnedColName && corrMatrix && info.name !== pinnedColName) {
        const pinnedIdx = corrColNames.indexOf(pinnedColName);
        const thisIdx = corrColNames.indexOf(info.name);
        if (pinnedIdx >= 0 && thisIdx >= 0) {
          const r = corrMatrix.matrix[pinnedIdx]?.[thisIdx] ?? null;
          if (r !== null) {
            const t = (r + 1) / 2;
            const [cr, cg, cb] = rdbu(1 - t);
            ctx.globalAlpha = 0.25;
            ctx.fillStyle = `rgb(${cr},${cg},${cb})`;
            ctx.fillRect(x, 0, cellW, HEADER_HEIGHT - NULL_BAR_H);
            ctx.globalAlpha = 1;
          }
        }
      }

      if (zoom >= 0.55) {
        drawNullRateBar(ctx, x, cellW, nullRate, hasFilter);

        ctx.font = FONT_BOLD;
        ctx.fillStyle = colors.text;
        ctx.fillText(info.name, x + 6, 22, cellW - 28);

        ctx.font = FONT_TYPE;
        ctx.fillStyle = colors.textType;
        ctx.fillText(info.data_type, x + 6, 38, cellW - 28);

        if (sortKey) {
          if (activeJobId) {
            const cx = x + cellW - 10;
            const cy = 22;
            const r = 5;
            ctx.strokeStyle = colors.sort;
            ctx.lineWidth = 1.5;
            ctx.beginPath();
            ctx.arc(cx, cy, r, spinnerAngle, spinnerAngle + Math.PI * 1.4);
            ctx.stroke();
            ctx.lineWidth = 1;
          } else {
            ctx.fillStyle = colors.sort;
            ctx.font = FONT_BOLD;
            ctx.fillText(sortKey.descending ? "▼" : "▲", x + cellW - 18, 22);
          }
        }

        if (hasFilter) {
          ctx.fillStyle = colors.filterDot;
          ctx.beginPath();
          ctx.arc(x + cellW - 10, 38, 3, 0, Math.PI * 2);
          ctx.fill();
        }

        if (zoom >= 0.55 && colStats?.distinct_count !== null && colStats?.distinct_count !== undefined) {
          const dc = colStats.distinct_count;
          const badge = dc >= 1_000_000 ? `${(dc / 1_000_000).toFixed(1)}M` :
            dc >= 1_000 ? `${(dc / 1_000).toFixed(1)}K` :
            dc === 1 ? "const" :
            dc === 2 ? "bin" :
            String(dc);
          ctx.font = "9px ui-monospace, monospace";
          ctx.fillStyle = colors.textType;
          ctx.textAlign = "right";
          ctx.fillText(badge, x + cellW - 4, 38);
          ctx.textAlign = "left";
        }

        if (colStats?.histogram && colStats.histogram.length > 0) {
          drawSparkline(ctx, colStats.histogram, colStats.null_count, colStats.count, x, HEADER_HEIGHT - SPARKLINE_HEIGHT - NULL_BAR_H, cellW, SPARKLINE_HEIGHT, colors);
        } else if (!allStats.has(statsKey)) {
          fetchStats(source.id, col);
        }

        if (pinnedColName && corrMatrix && info.name !== pinnedColName) {
          const pinnedIdx = corrColNames.indexOf(pinnedColName);
          const thisIdx = corrColNames.indexOf(info.name);
          if (pinnedIdx >= 0 && thisIdx >= 0) {
            const r = corrMatrix.matrix[pinnedIdx]?.[thisIdx] ?? null;
            if (r !== null) {
              const label = `r=${r > 0 ? "+" : ""}${r.toFixed(2)}`;
              ctx.font = "9px ui-monospace, monospace";
              ctx.fillStyle = r > 0.3 ? "#2563eb" : r < -0.3 ? "#dc2626" : colors.textType;
              ctx.textAlign = "right";
              ctx.fillText(label, x + cellW - 4, 22);
              ctx.textAlign = "left";
            }
          }
        }
      } else if (zoom >= 0.28) {
        drawNullRateBar(ctx, x, cellW, nullRate, hasFilter);

        const abbrevName = info.name.length > 8 ? info.name.slice(0, 7) + "…" : info.name;
        ctx.font = FONT_BOLD;
        ctx.fillStyle = colors.text;
        ctx.fillText(abbrevName, x + 4, HEADER_HEIGHT / 2, cellW - 16);

        if (sortKey) {
          if (activeJobId) {
            const cx = x + cellW - 8;
            const cy = HEADER_HEIGHT / 2;
            ctx.strokeStyle = colors.sort;
            ctx.lineWidth = 1.5;
            ctx.beginPath();
            ctx.arc(cx, cy, 4, spinnerAngle, spinnerAngle + Math.PI * 1.4);
            ctx.stroke();
            ctx.lineWidth = 1;
          } else {
            ctx.fillStyle = colors.sort;
            ctx.fillText(sortKey.descending ? "▼" : "▲", x + cellW - 14, HEADER_HEIGHT / 2);
          }
        }
        if (hasFilter) {
          ctx.fillStyle = colors.filterDot;
          ctx.beginPath();
          ctx.arc(x + cellW - 8, HEADER_HEIGHT / 2, 3, 0, Math.PI * 2);
          ctx.fill();
        }
      } else if (zoom >= 0.10) {
        drawNullRateBar(ctx, x, cellW, nullRate, hasFilter);

        const { code, swatchIdx } = dataTypeInfo(info.data_type);
        const [r, g, b] = categoricalColor(swatchIdx);
        const swatchW = Math.min(8, cellW * 0.3);

        ctx.globalAlpha = 0.8;
        ctx.fillStyle = `rgb(${r},${g},${b})`;
        ctx.fillRect(x + 2, HEADER_HEIGHT / 2 - 5, swatchW, 10);
        ctx.globalAlpha = 1;

        if (cellW > 20) {
          ctx.font = FONT_SMALL;
          ctx.fillStyle = colors.text;
          ctx.fillText(code, x + swatchW + 5, HEADER_HEIGHT / 2);
        }

        const rgKey = `${source.id}:${col}`;
        const rgStats = rowGroupStats.get(rgKey);
        if (rgStats && rgStats.length >= 2 && cellW >= 30) {
          const sparkY = HEADER_HEIGHT - 14 - NULL_BAR_H;
          const sparkH = 10;
          const sparkX = x + 2;
          const sparkW = cellW - 4;
          const points = buildDriftSparklinePoints(rgStats, sparkX, sparkY, sparkW, sparkH);
          if (points && points.length >= 2) {
            const firstMean = rgStats[0]?.mean ?? ((rgStats[0]?.min !== null && rgStats[0]?.max !== null) ? ((rgStats[0]?.min ?? 0) + (rgStats[0]?.max ?? 0)) / 2 : null);
            const lastMean = rgStats[rgStats.length - 1]?.mean ?? ((rgStats[rgStats.length - 1]?.min !== null && rgStats[rgStats.length - 1]?.max !== null) ? ((rgStats[rgStats.length - 1]?.min ?? 0) + (rgStats[rgStats.length - 1]?.max ?? 0)) / 2 : null);
            const hasDrift = firstMean !== null && lastMean !== null && Math.abs(lastMean - firstMean) > 0;

            ctx.strokeStyle = hasDrift ? "#f59e0b" : colors.drift;
            ctx.lineWidth = 1;
            ctx.globalAlpha = 0.8;
            ctx.beginPath();
            ctx.moveTo(points[0]!.px, points[0]!.py);
            for (let i = 1; i < points.length; i++) {
              ctx.lineTo(points[i]!.px, points[i]!.py);
            }
            ctx.stroke();
            ctx.globalAlpha = 1;
          }
        }
      } else {
        const { swatchIdx } = dataTypeInfo(info.data_type);
        const [r, g, b] = categoricalColor(swatchIdx);
        ctx.globalAlpha = 0.7;
        ctx.fillStyle = `rgb(${r},${g},${b})`;
        ctx.fillRect(x, 0, cellW, HEADER_HEIGHT);
        ctx.globalAlpha = 1;

        drawNullRateBar(ctx, x, cellW, nullRate, hasFilter);
      }
    }

    ctx.strokeStyle = colors.border;
    ctx.lineWidth = 1;
    ctx.strokeRect(0, HEADER_HEIGHT - 1, width, 1);
  }, [width, source, viewport.scrollX, ops, zoom, allStats, rowGroupStats, correlations, pinnedCorrelationCol, isDark, activeJobId, spinnerAngle]);

  const colAtX = (clientX: number): { col: number; x: number } => {
    const canvas = canvasRef.current!;
    const rect = canvas.getBoundingClientRect();
    const offsetX = clientX - rect.left + viewport.scrollX;
    const col = Math.floor(offsetX / cellW);
    const x = col * cellW - viewport.scrollX + leftOffset;
    return { col, x };
  };

  const handleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!source) return;
    const { col } = colAtX(e.clientX);
    const info = source.columns[col];
    if (!info) return;

    if (e.altKey) {
      if (pinnedCorrelationCol === col) {
        setPinnedCorrelationCol(null);
      } else {
        setPinnedCorrelationCol(col);
      }
      return;
    }

    if (e.shiftKey) {
      const currentSort = ops.find((op) => op.type === "sort");
      const existing = currentSort?.type === "sort" ? currentSort.keys : [];
      const alreadyIdx = existing.findIndex((k) => k.column === info.name);
      if (alreadyIdx >= 0) {
        const updated = existing.map((k, i) =>
          i === alreadyIdx ? { ...k, descending: !k.descending } : k
        );
        setSort(updated);
      } else {
        setSort([...existing, { column: info.name, descending: false, nulls_last: true }]);
      }
      return;
    }

    const currentSort = ops.find((op) => op.type === "sort");
    const singleKey = currentSort?.type === "sort" && currentSort.keys.length === 1
      ? currentSort.keys[0]
      : null;

    if (singleKey?.column === info.name) {
      if (!singleKey.descending) {
        setSort([{ column: info.name, descending: true, nulls_last: true }]);
      } else {
        setSort([]);
      }
    } else {
      setSort([{ column: info.name, descending: false, nulls_last: true }]);
    }
  };

  const handleContextMenu = (e: React.MouseEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    if (!source) return;
    const { col } = colAtX(e.clientX);
    const info = source.columns[col];
    if (!info) return;
    setContextMenu({ kind: "column", x: e.clientX, y: e.clientY, colIndex: col, colName: info.name });
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!source) return;
    const { col } = colAtX(e.clientX);
    if (col >= 0 && col < source.n_cols) {
      setHoveredColumnIndex(col);
    }
  };

  const handleMouseLeave = () => {
    setTimeout(() => setHoveredColumnIndex(null), 200);
  };

  const handleMouseEnter = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!source || !sourceId) return;
    const { col } = colAtX(e.clientX);
    const info = source.columns[col];
    if (!info) return;
    const count = incrementSortAccess(info.name);
    if (count >= 2) {
      const viewExpr = { source_id: sourceId, ops };
      speculativeSort(sourceId, viewExpr, info.name).catch(() => {});
    }
  };

  return (
    <canvas
      ref={canvasRef}
      onClick={handleClick}
      onContextMenu={handleContextMenu}
      onMouseMove={handleMouseMove}
      onMouseLeave={handleMouseLeave}
      onMouseEnter={handleMouseEnter}
      style={{ display: "block", cursor: "pointer" }}
    />
  );
}

function drawSparkline(
  ctx: CanvasRenderingContext2D,
  histogram: Array<{ lo: number; hi: number; count: number }>,
  nullCount: number,
  totalCount: number,
  x: number,
  y: number,
  cellW: number,
  h: number,
  colors: ReturnType<typeof headerColors>
) {
  const maxCount = Math.max(...histogram.map((b) => b.count));
  if (maxCount === 0) return;

  const padding = 3;
  const barAreaW = cellW - padding * 2;
  const barAreaH = h - padding * 2;
  const nBars = histogram.length;
  const barW = Math.max(1, Math.floor(barAreaW / nBars) - 1);

  ctx.globalAlpha = 0.85;

  for (let i = 0; i < nBars; i++) {
    const bucket = histogram[i]!;
    const ratio = bucket.count / maxCount;
    const barH = Math.max(1, Math.round(ratio * barAreaH));
    const bx = x + padding + i * Math.floor(barAreaW / nBars);
    const by = y + padding + barAreaH - barH;

    ctx.fillStyle = colors.sparkline;
    ctx.fillRect(bx, by, barW, barH);
  }

  if (nullCount > 0 && totalCount > 0) {
    const nullRatio = nullCount / totalCount;
    const nullBarH = Math.max(1, Math.round(nullRatio * barAreaH));
    ctx.fillStyle = "#ef4444";
    ctx.globalAlpha = 0.5;
    ctx.fillRect(x + cellW - padding - 3, y + padding + barAreaH - nullBarH, 3, nullBarH);
  }

  ctx.globalAlpha = 1;
}
