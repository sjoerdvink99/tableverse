import { useEffect, useRef, useState } from "react";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";
import { useViewStore } from "../../stores/view";
import { useStatsStore, getStatsKey } from "../../stores/stats";
import { useProgressiveStats } from "../../hooks/useProgressiveStats";
import { DEFAULT_CELL_W, HEADER_HEIGHT, ROW_HEADER_W } from "../../lib/viewport";
import type { ColumnStats } from "../../lib/types";

const POPOVER_WIDTH = 240;
const BAR_HEIGHT = 72;

type Props = Record<string, never>;

type HistogramPanelProps = {
  stats: ColumnStats;
  isApproximate: boolean;
  dragStart: number | null;
  dragEnd: number | null;
  onBucketClick: (bucket: { lo: number; hi: number }, idx: number) => void;
  onDragEnd: () => void;
  onDragStart: (idx: number) => void;
  onDragOver: (idx: number) => void;
};

function HistogramPanel({
  stats,
  isApproximate,
  dragStart,
  dragEnd,
  onBucketClick,
  onDragEnd,
  onDragStart,
  onDragOver,
}: HistogramPanelProps) {
  if (!stats.histogram || stats.histogram.length === 0) return null;
  const maxCount = Math.max(...stats.histogram.map((b) => b.count), 1);
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-end",
        height: BAR_HEIGHT,
        gap: 1,
        marginBottom: 4,
        opacity: isApproximate ? 0.55 : 1,
        transition: "opacity 0.2s",
      }}
      onMouseUp={onDragEnd}
      title={isApproximate ? "Approximate histogram — full stats loading…" : undefined}
    >
      {stats.histogram.map((bucket, i) => {
        const heightPct = (bucket.count / maxCount) * 100;
        const isSelected =
          dragStart !== null &&
          dragEnd !== null &&
          i >= Math.min(dragStart, dragEnd) &&
          i <= Math.max(dragStart, dragEnd);
        return (
          <div
            key={i}
            style={{
              flex: 1,
              height: `${Math.max(heightPct, 2)}%`,
              background: isSelected ? "var(--c-accent)" : "var(--canvas-sparkline)",
              borderRadius: "2px 2px 0 0",
              cursor: "pointer",
              transition: "background 0.1s",
            }}
            title={`${bucket.lo.toFixed(2)}–${bucket.hi.toFixed(2)}: ${bucket.count.toLocaleString()}`}
            onClick={() => onBucketClick(bucket, i)}
            onMouseDown={() => onDragStart(i)}
            onMouseEnter={() => onDragOver(i)}
          />
        );
      })}
    </div>
  );
}

export function ColumnDistributionPopover(_props: Props) {
  const hoveredColIdx = useUiStore((s) => s.hoveredColumnIndex);
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const zoom = useUiStore((s) => s.zoom);
  const addPredicate = useViewStore((s) => s.addPredicate);
  const allStats = useStatsStore((s) => s.stats);
  const pending = useStatsStore((s) => s.pending);
  const fetchStats = useStatsStore((s) => s.fetchStats);
  const [customInput, setCustomInput] = useState("");
  const [dragStart, setDragStart] = useState<number | null>(null);
  const [dragEnd, setDragEnd] = useState<number | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const prevColIdx = useRef<number | null>(null);

  const cellW = DEFAULT_CELL_W * zoom;

  const progressive = useProgressiveStats(source?.id ?? "", hoveredColIdx);

  useEffect(() => {
    if (hoveredColIdx === null || !source) return;
    if (hoveredColIdx === prevColIdx.current) return;

    prevColIdx.current = hoveredColIdx;
    setCustomInput("");
    setDragStart(null);
    setDragEnd(null);

    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = setTimeout(() => {
      fetchStats(source.id, hoveredColIdx);
    }, 120);

    return () => {
      if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    };
  }, [hoveredColIdx, source?.id]);

  if (hoveredColIdx === null || !source) return null;

  const colInfo = source.columns[hoveredColIdx];
  if (!colInfo) return null;

  const statsKey = getStatsKey(source.id, hoveredColIdx);
  const storeStats = allStats.get(statsKey) ?? null;
  const loading = pending.has(statsKey);

  const stats = storeStats ?? progressive.full ?? progressive.coarse ?? null;
  const isApproximate = stats !== null && storeStats === null && progressive.full === null;

  const colPixelX = hoveredColIdx * cellW - viewport.scrollX + ROW_HEADER_W;
  const popoverLeft = Math.max(4, Math.min(colPixelX, window.innerWidth - POPOVER_WIDTH - 8));
  const popoverTop = HEADER_HEIGHT;

  const nullFraction = stats ? stats.null_rate : 0;
  const nonNullCount = stats ? stats.count - stats.null_count : 0;

  const handleHistogramClick = (bucket: { lo: number; hi: number }, _bucketIdx: number) => {
    if (dragStart !== null && dragEnd !== null) return;
    addPredicate({ op: "between", column: colInfo.name, lo: bucket.lo, hi: bucket.hi });
  };

  const handleHistogramDragEnd = () => {
    if (dragStart === null || dragEnd === null || !stats?.histogram) return;
    const lo = Math.min(dragStart, dragEnd);
    const hi = Math.max(dragStart, dragEnd);
    const buckets = stats.histogram;
    const selectedLo = buckets[lo]!.lo;
    const selectedHi = buckets[hi]!.hi;
    addPredicate({ op: "between", column: colInfo.name, lo: selectedLo, hi: selectedHi });
    setDragStart(null);
    setDragEnd(null);
  };

  const submitCustom = (raw: string) => {
    const trimmed = raw.trim();
    if (!trimmed) return;
    const match = trimmed.match(/^(>=|<=|>|<|!=|=|contains?)\s*(.+)$/i);
    if (!match) return;
    const [, opStr, val] = match;
    if (!opStr || val === undefined) return;
    const parsed: string | number = isNaN(Number(val)) ? val : Number(val);
    const col = colInfo.name;
    switch (opStr.toLowerCase()) {
      case ">":  addPredicate({ op: "gt", column: col, value: parsed }); break;
      case ">=": addPredicate({ op: "gte", column: col, value: parsed }); break;
      case "<":  addPredicate({ op: "lt", column: col, value: parsed }); break;
      case "<=": addPredicate({ op: "lte", column: col, value: parsed }); break;
      case "!=": addPredicate({ op: "ne", column: col, value: parsed }); break;
      case "=":  addPredicate({ op: "eq", column: col, value: parsed }); break;
      case "contains":
      case "contain": addPredicate({ op: "contains", column: col, value: String(val) }); break;
    }
    setCustomInput("");
  };

  return (
    <div
      style={{
        position: "absolute",
        left: popoverLeft,
        top: popoverTop,
        width: POPOVER_WIDTH,
        background: "var(--c-bg)",
        border: "1px solid var(--c-border)",
        borderRadius: 8,
        boxShadow: "0 8px 24px rgba(0,0,0,0.2)",
        zIndex: 500,
        padding: "10px 12px 10px",
        pointerEvents: "auto",
      }}
      onMouseLeave={() => {}}
    >
      <div style={{ fontSize: 12, fontWeight: 600, color: "var(--c-text)", marginBottom: 2 }}>
        {colInfo.name}
      </div>
      <div style={{ fontSize: 11, color: "var(--c-text-3)", marginBottom: 8, fontFamily: "ui-monospace, monospace" }}>
        {colInfo.data_type}
      </div>

      {stats && (
        <>
          <div
            style={{ height: 6, borderRadius: 3, background: "var(--c-surface-2)", marginBottom: 8, cursor: "pointer", overflow: "hidden" }}
            title={`${((1 - nullFraction) * 100).toFixed(1)}% non-null · ${(nullFraction * 100).toFixed(1)}% null`}
          >
            <div style={{ display: "flex", height: "100%" }}>
              <div
                style={{ width: `${(1 - nullFraction) * 100}%`, background: "var(--c-accent)", borderRadius: "3px 0 0 3px" }}
                onClick={() => addPredicate({ op: "is_not_null", column: colInfo.name })}
                title="Click to filter: is not null"
              />
              {nullFraction > 0 && (
                <div
                  style={{ flex: 1, background: "#fca5a5", borderRadius: "0 3px 3px 0" }}
                  onClick={() => addPredicate({ op: "is_null", column: colInfo.name })}
                  title="Click to filter: is null"
                />
              )}
            </div>
          </div>

          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 10, color: "var(--c-text-3)", marginBottom: 4 }}>
            <span>{nonNullCount.toLocaleString()} non-null</span>
            {stats.null_count > 0 && <span>{stats.null_count.toLocaleString()} null</span>}
          </div>

          <HistogramPanel
            stats={stats}
            isApproximate={isApproximate}
            dragStart={dragStart}
            dragEnd={dragEnd}
            onBucketClick={handleHistogramClick}
            onDragEnd={handleHistogramDragEnd}
            onDragStart={setDragStart}
            onDragOver={(i) => { if (dragStart !== null) setDragEnd(i); }}
          />

          {isApproximate && (
            <div style={{ fontSize: 10, color: "var(--c-text-3)", marginBottom: 4, fontStyle: "italic" }}>
              approximate
            </div>
          )}

          {stats.min !== null && stats.max !== null && (
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: 10, color: "var(--c-text-3)", marginBottom: 6 }}>
              <span>{String(stats.min)}</span>
              <span>{String(stats.max)}</span>
            </div>
          )}
        </>
      )}

      {(loading || progressive.isLoading) && !stats && (
        <div style={{ fontSize: 11, color: "var(--c-text-3)", padding: "4px 0" }}>Loading…</div>
      )}

      <input
        value={customInput}
        onChange={(e) => setCustomInput(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter") submitCustom(customInput); }}
        placeholder="> 100 · contains foo · = active"
        style={{
          width: "100%",
          padding: "5px 7px",
          border: "1px solid var(--c-border)",
          borderRadius: 5,
          fontSize: 11,
          outline: "none",
          boxSizing: "border-box",
          marginTop: 2,
          color: "var(--c-text)",
          background: "var(--c-bg)",
          fontFamily: "ui-monospace, monospace",
        }}
      />
    </div>
  );
}
