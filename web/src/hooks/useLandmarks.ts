import { useEffect } from "react";
import { useTableStore } from "../stores/table";
import { useStatsStore } from "../stores/stats";
import { useLandmarkStore, type Landmark } from "../stores/landmarkStore";
import type { SourceMeta, RowGroupStat, ColumnStats } from "../lib/types";

function detectLandmarks(
  source: SourceMeta,
  rowGroupStatsByCol: Map<string, RowGroupStat[]>,
  columnStats: Map<string, ColumnStats>
): Landmark[] {
  const nCols = source.n_cols;
  const rgMap = new Map<number, { colIdx: number; stat: RowGroupStat }[]>();

  for (const [key, stats] of rowGroupStatsByCol) {
    const colIdx = parseInt(key.split(":")[1] ?? "0", 10);
    for (const stat of stats) {
      const entries = rgMap.get(stat.rg_index) ?? [];
      entries.push({ colIdx, stat });
      rgMap.set(stat.rg_index, entries);
    }
  }

  const modalRowCount = computeModalRowCount(rgMap);
  const results: Landmark[] = [];

  for (const [rgIndex, entries] of rgMap) {
    const firstEntry = entries[0];
    if (!firstEntry) continue;
    const rowOffset = firstEntry.stat.row_offset;
    const rowCount = firstEntry.stat.row_count;

    const nullSurgeCols = entries.filter(
      ({ stat }) => stat.row_count > 0 && stat.null_count / stat.row_count > 0.5
    );
    if (nullSurgeCols.length >= 3) {
      results.push({
        rowOffset,
        rowCount,
        type: "null_surge",
        severity: nullSurgeCols.length / nCols,
        affectedCols: nullSurgeCols.map(({ colIdx }) => colIdx),
      });
    }

    let maxOutlierScore = 0;
    const outlierCols: number[] = [];
    for (const { colIdx, stat } of entries) {
      const statsKey = `${source.id}:${colIdx}`;
      const cs = columnStats.get(statsKey);
      if (!cs || cs.mean === null) continue;
      const mean = cs.mean;
      const stdDev = cs.quantiles ? (cs.quantiles.p75 - cs.quantiles.p25) / 1.35 : null;
      if (stdDev === null || stdDev === 0) continue;
      const minVal = stat.min ?? mean;
      const maxVal = stat.max ?? mean;
      if (minVal < mean - 3 * stdDev || maxVal > mean + 3 * stdDev) {
        const score = Math.max(
          Math.abs(minVal - mean) / stdDev,
          Math.abs(maxVal - mean) / stdDev
        ) / 10;
        maxOutlierScore = Math.max(maxOutlierScore, score);
        outlierCols.push(colIdx);
      }
    }
    if (outlierCols.length > 0) {
      results.push({
        rowOffset,
        rowCount,
        type: "outlier",
        severity: Math.min(1, maxOutlierScore),
        affectedCols: outlierCols,
      });
    }

    if (
      modalRowCount !== null &&
      rowCount !== firstEntry.stat.row_count &&
      Math.abs(rowCount - modalRowCount) / modalRowCount > 0.1
    ) {
      const isLast = !Array.from(rgMap.keys()).some((k) => k > rgIndex);
      if (!isLast) {
        results.push({
          rowOffset,
          rowCount,
          type: "boundary",
          severity: 0.3,
          affectedCols: [],
        });
      }
    }
  }

  return results.sort((a, b) => b.severity - a.severity).slice(0, 200);
}

function computeModalRowCount(rgMap: Map<number, { colIdx: number; stat: RowGroupStat }[]>): number | null {
  const counts = new Map<number, number>();
  for (const entries of rgMap.values()) {
    const rowCount = entries[0]?.stat.row_count;
    if (rowCount === undefined) continue;
    counts.set(rowCount, (counts.get(rowCount) ?? 0) + 1);
  }
  if (counts.size === 0) return null;
  let modal = 0;
  let maxFreq = 0;
  for (const [val, freq] of counts) {
    if (freq > maxFreq) {
      maxFreq = freq;
      modal = val;
    }
  }
  return modal;
}

export function useLandmarks(): void {
  const source = useTableStore((s) => s.source);
  const rowGroupStats = useStatsStore((s) => s.rowGroupStats);
  const columnStats = useStatsStore((s) => s.stats);
  const setLandmarks = useLandmarkStore((s) => s.setLandmarks);

  useEffect(() => {
    if (!source) {
      setLandmarks([]);
      return;
    }
    const landmarks = detectLandmarks(source, rowGroupStats, columnStats);
    setLandmarks(landmarks);
  }, [source?.id, rowGroupStats, columnStats]);
}
