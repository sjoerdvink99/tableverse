import { useState, useEffect } from "react";
import { subscribeColumnStats } from "../lib/api";
import type { ColumnStats, QuickColumnStats } from "../lib/types";
import { useTableStore } from "../stores/table";

export type ProgressiveStatsResult = {
  quick: QuickColumnStats | null;
  coarse: ColumnStats | null;
  full: ColumnStats | null;
  isLoading: boolean;
};

export function useProgressiveStats(
  sourceId: string,
  colIdx: number | null,
): ProgressiveStatsResult {
  const [coarse, setCoarse] = useState<ColumnStats | null>(null);
  const [full, setFull] = useState<ColumnStats | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const source = useTableStore((s) => s.source);

  const quick =
    colIdx !== null && source?.id === sourceId
      ? (source.quick_stats?.[colIdx] ?? null)
      : null;

  useEffect(() => {
    if (colIdx === null) {
      setCoarse(null);
      setFull(null);
      setIsLoading(false);
      return;
    }

    setCoarse(null);
    setFull(null);
    setIsLoading(true);

    const unsubscribe = subscribeColumnStats(
      sourceId,
      colIdx,
      50,
      (event) => {
        if (event.type === "stats") {
          setFull(event.data);
          setIsLoading(false);
        } else if (event.type === "done") {
          setIsLoading(false);
        } else if (event.type === "error") {
          setIsLoading(false);
        }
      },
      (stats) => {
        setCoarse(stats);
      },
    );

    return () => {
      unsubscribe();
      setIsLoading(false);
    };
  }, [sourceId, colIdx]);

  return { quick, coarse, full, isLoading };
}
