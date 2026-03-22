import { useRef } from "react";
import { useTableStore } from "../stores/table";
import { useUiStore } from "../stores/ui";
import { useViewStore } from "../stores/view";
import { DEFAULT_CELL_W, DEFAULT_CELL_H, scrollBounds } from "../lib/viewport";

export function useNavigation() {
  const cancelRef = useRef<number | null>(null);

  const navigateTo = (scrollX: number, scrollY: number, pushHistory = false) => {
    const { viewport, source, setViewport } = useTableStore.getState();
    const { virtualRowCount } = useViewStore.getState();
    const { zoom, pushNavHistory } = useUiStore.getState();

    const effectiveRowCount = virtualRowCount ?? source?.n_rows ?? 0;
    const nCols = source?.n_cols ?? 0;
    const cellW = DEFAULT_CELL_W * zoom;
    const cellH = DEFAULT_CELL_H * zoom;
    const { maxX, maxY } = scrollBounds(effectiveRowCount, nCols, cellW, cellH, viewport.width, viewport.height);

    const clampedX = Math.max(0, Math.min(maxX, scrollX));
    const clampedY = Math.max(0, Math.min(maxY, scrollY));

    if (pushHistory) {
      const dx = Math.abs(clampedX - viewport.scrollX);
      const dy = Math.abs(clampedY - viewport.scrollY);
      if (dx + dy > viewport.height * 2) {
        pushNavHistory({ scrollX: viewport.scrollX, scrollY: viewport.scrollY });
      }
    }

    setViewport({ scrollX: clampedX, scrollY: clampedY });
  };

  const teleportTo = (scrollX: number, scrollY: number) => {
    if (cancelRef.current !== null) {
      cancelAnimationFrame(cancelRef.current);
      cancelRef.current = null;
    }

    const { viewport } = useTableStore.getState();
    const dx = Math.abs(scrollX - viewport.scrollX);
    const dy = Math.abs(scrollY - viewport.scrollY);

    if (dx + dy < viewport.height * 2) {
      navigateTo(scrollX, scrollY, true);
      return;
    }

    navigateTo(viewport.scrollX, viewport.scrollY, true);

    const startX = viewport.scrollX;
    const startY = viewport.scrollY;
    const startTime = performance.now();
    const duration = 200;

    const animate = (now: number) => {
      const t = Math.min(1, (now - startTime) / duration);
      const eased = 1 - (1 - t) * (1 - t);
      const { setViewport } = useTableStore.getState();
      setViewport({
        scrollX: startX + (scrollX - startX) * eased,
        scrollY: startY + (scrollY - startY) * eased,
      });
      if (t < 1) {
        cancelRef.current = requestAnimationFrame(animate);
      } else {
        cancelRef.current = null;
      }
    };

    cancelRef.current = requestAnimationFrame(animate);
  };

  return { navigateTo, teleportTo };
}
