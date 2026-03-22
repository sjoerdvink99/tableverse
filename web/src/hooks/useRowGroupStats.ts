import { useEffect, useRef } from "react";
import { useTableStore } from "../stores/table";
import { useUiStore } from "../stores/ui";
import { useStatsStore } from "../stores/stats";
import { fetchRowGroupStatsBatch } from "../lib/api";
import { DEFAULT_CELL_W } from "../lib/viewport";

export function useRowGroupStats(): void {
  const zoom = useUiStore((s) => s.zoom);
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const setRowGroupStats = useStatsStore((s) => s.setRowGroupStats);
  const rowGroupStats = useStatsStore((s) => s.rowGroupStats);
  const pendingRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!source || zoom >= 0.28) return;

    const cellW = DEFAULT_CELL_W * zoom;
    const colStart = Math.max(0, Math.floor(viewport.scrollX / cellW));
    const colEnd = Math.min(source.n_cols, Math.ceil((viewport.scrollX + viewport.width) / cellW) + 2);

    const missing: number[] = [];
    for (let col = colStart; col < colEnd; col++) {
      const key = `${source.id}:${col}`;
      if (!rowGroupStats.has(key) && !pendingRef.current.has(key)) {
        missing.push(col);
      }
    }

    if (missing.length === 0) return;

    const sourceId = source.id;
    for (const col of missing) {
      pendingRef.current.add(`${sourceId}:${col}`);
    }

    fetchRowGroupStatsBatch(sourceId, missing)
      .then((result) => {
        for (const [colIdxStr, stats] of Object.entries(result)) {
          const key = `${sourceId}:${colIdxStr}`;
          setRowGroupStats(key, stats);
          pendingRef.current.delete(key);
        }
      })
      .catch(() => {
        for (const col of missing) {
          pendingRef.current.delete(`${sourceId}:${col}`);
        }
      });
  }, [zoom, source?.id, viewport.scrollX, viewport.width, rowGroupStats, setRowGroupStats]);
}
