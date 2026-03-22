import { useEffect } from "react";
import { useTableStore } from "../stores/table";
import { useUiStore } from "../stores/ui";
import { useStatsStore } from "../stores/stats";
import { DEFAULT_CELL_W } from "../lib/viewport";

export function useSemanticStats(): void {
  const zoom = useUiStore((s) => s.zoom);
  const semanticZoomEnabled = useUiStore((s) => s.semanticZoomEnabled);
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const fetchStats = useStatsStore((s) => s.fetchStats);
  const allStats = useStatsStore((s) => s.stats);

  useEffect(() => {
    if (!source || !semanticZoomEnabled || zoom >= 0.85) return;

    const cellW = DEFAULT_CELL_W * zoom;
    const colStart = Math.max(0, Math.floor(viewport.scrollX / cellW));
    const colEnd = Math.min(source.n_cols, Math.ceil((viewport.scrollX + viewport.width) / cellW) + 1);

    for (let col = colStart; col < colEnd; col++) {
      const key = `${source.id}:${col}`;
      if (!allStats.has(key)) {
        fetchStats(source.id, col);
      }
    }
  }, [zoom, semanticZoomEnabled, source?.id, viewport.scrollX, viewport.width, allStats, fetchStats]);
}
