import { useCallback } from "react";
import { useTableStore } from "../stores/table";
import { useViewStore } from "../stores/view";
import { useUiStore, ZOOM_MIN, ZOOM_MAX, adaptiveZoomStep } from "../stores/ui";
import { scrollBounds, DEFAULT_CELL_W, DEFAULT_CELL_H } from "../lib/viewport";

export function useZoom() {
  const setZoom = useUiStore((s) => s.setZoom);
  const setViewport = useTableStore((s) => s.setViewport);

  const applyZoom = useCallback(
    (newZoom: number, cursor?: { x: number; y: number }) => {
      const { zoom: oldZoom } = useUiStore.getState();
      const { viewport, source } = useTableStore.getState();
      const { virtualRowCount } = useViewStore.getState();

      const clampedZoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, +newZoom.toFixed(2)));
      if (clampedZoom === oldZoom) return;

      const ratio = clampedZoom / oldZoom;
      let newScrollX: number;
      let newScrollY: number;

      if (cursor) {
        newScrollX = (cursor.x + viewport.scrollX) * ratio - cursor.x;
        newScrollY = (cursor.y + viewport.scrollY) * ratio - cursor.y;
      } else {
        newScrollX = viewport.scrollX * ratio;
        newScrollY = viewport.scrollY * ratio;
      }

      const effectiveRowCount = virtualRowCount ?? source?.n_rows ?? 0;
      const nCols = source?.n_cols ?? 0;
      const cellW = DEFAULT_CELL_W * clampedZoom;
      const cellH = DEFAULT_CELL_H * clampedZoom;
      const { maxX, maxY } = scrollBounds(effectiveRowCount, nCols, cellW, cellH, viewport.width, viewport.height);

      setZoom(clampedZoom);
      setViewport({
        scrollX: Math.max(0, Math.min(maxX, newScrollX)),
        scrollY: Math.max(0, Math.min(maxY, newScrollY)),
      });
    },
    [setZoom, setViewport]
  );

  const zoomIn = useCallback(() => {
    const { zoom } = useUiStore.getState();
    applyZoom(+(zoom + adaptiveZoomStep(zoom)).toFixed(2));
  }, [applyZoom]);

  const zoomOut = useCallback(() => {
    const { zoom } = useUiStore.getState();
    applyZoom(+(zoom - adaptiveZoomStep(zoom)).toFixed(2));
  }, [applyZoom]);

  const resetZoom = useCallback(() => {
    applyZoom(1);
  }, [applyZoom]);

  return { applyZoom, zoomIn, zoomOut, resetZoom };
}
