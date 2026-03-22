import { useEffect } from "react";
import { useTableStore } from "../stores/table";
import { useViewStore } from "../stores/view";
import { useUiStore } from "../stores/ui";
import { scrollBounds, DEFAULT_CELL_W, DEFAULT_CELL_H } from "../lib/viewport";

export function useViewport(containerRef: React.RefObject<HTMLElement | null>) {
  const setViewport = useTableStore((s) => s.setViewport);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        const { viewport, source } = useTableStore.getState();
        const { virtualRowCount } = useViewStore.getState();
        const { zoom } = useUiStore.getState();
        if (source) {
          const cellW = DEFAULT_CELL_W * zoom;
          const cellH = DEFAULT_CELL_H * zoom;
          const effectiveRowCount = virtualRowCount ?? source.n_rows;
          const { maxX, maxY } = scrollBounds(effectiveRowCount, source.n_cols, cellW, cellH, width, height);
          setViewport({
            width,
            height,
            scrollX: Math.min(viewport.scrollX, maxX),
            scrollY: Math.min(viewport.scrollY, maxY),
          });
        } else {
          setViewport({ width, height });
        }
      }
    });

    observer.observe(el);
    const { width, height } = el.getBoundingClientRect();
    setViewport({ width, height });

    return () => observer.disconnect();
  }, [containerRef, setViewport]);
}
