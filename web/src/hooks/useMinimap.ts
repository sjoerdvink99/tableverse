import { useState, useEffect, useRef } from "react";
import { useTableStore } from "../stores/table";
import { useViewStore } from "../stores/view";
import { useStatsStore } from "../stores/stats";
import { useUiStore } from "../stores/ui";

export function useMinimap(panelW: number, panelH: number): ImageData | null {
  const [imageData, setImageData] = useState<ImageData | null>(null);
  const idleRef = useRef<number | null>(null);

  const source = useTableStore((s) => s.source);
  const viewHash = useViewStore((s) => s.viewHash);
  const rowGroupStats = useStatsStore((s) => s.rowGroupStats);
  const isDark = useUiStore((s) => s.isDark);

  useEffect(() => {
    if (!source || panelW <= 0 || panelH <= 0) {
      setImageData(null);
      return;
    }

    const compute = () => {
      const nRows = source.n_rows;
      const sourceId = source.id;
      const quickStats = source.quick_stats ?? [];
      const nCols = source.n_cols;

      const globalNullRate =
        quickStats.length > 0
          ? quickStats.reduce((s, q) => s + (q?.null_rate ?? 0), 0) / quickStats.length
          : 0;

      const data = new Uint8ClampedArray(panelW * panelH * 4);

      for (let y = 0; y < panelH; y++) {
        const rowStart = Math.floor((y / panelH) * nRows);
        const rowEnd = Math.ceil(((y + 1) / panelH) * nRows);

        let rgNullSum = 0;
        let rgCount = 0;
        for (let ci = 0; ci < nCols; ci++) {
          const stats = rowGroupStats.get(`${sourceId}:${ci}`);
          if (!stats) continue;
          for (const rg of stats) {
            if (rg.row_offset < rowEnd && rg.row_offset + rg.row_count > rowStart) {
              rgNullSum += rg.row_count > 0 ? rg.null_count / rg.row_count : 0;
              rgCount++;
            }
          }
        }

        const nullRate = rgCount > 0
          ? globalNullRate * 0.3 + (rgNullSum / rgCount) * 0.7
          : globalNullRate;

        const v = isDark
          ? Math.round((1 - nullRate) * 38 + 26)
          : Math.round((1 - nullRate) * 45 + 185);

        for (let x = 0; x < panelW; x++) {
          const idx = (y * panelW + x) * 4;
          data[idx] = v;
          data[idx + 1] = v;
          data[idx + 2] = v;
          data[idx + 3] = 255;
        }
      }

      setImageData(new ImageData(data, panelW, panelH));
    };

    if (idleRef.current !== null) {
      cancelIdleCallback(idleRef.current);
    }

    if (source.n_rows > 5_000_000) {
      idleRef.current = requestIdleCallback(compute);
    } else {
      compute();
    }

    return () => {
      if (idleRef.current !== null) {
        cancelIdleCallback(idleRef.current);
        idleRef.current = null;
      }
    };
  }, [source?.id, viewHash, panelW, panelH, rowGroupStats, isDark]);

  return imageData;
}
