import { useRef, useCallback, useEffect } from "react";
import { useTableStore } from "../../stores/table";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useStatsStore, getStatsKey } from "../../stores/stats";
import { useZoom } from "../../hooks/useZoom";
import { useNavigation } from "../../hooks/useNavigation";
import { DEFAULT_CELL_W, DEFAULT_CELL_H, scrollBounds, cellAtPixel, tileKey, ROW_HEADER_W, HEADER_HEIGHT, SCROLLBAR_SIZE, tileRowsForZoom, tileColsForZoom } from "../../lib/viewport";
import { GridCanvas } from "./GridCanvas";
import { HorizontalScrollbar } from "./HorizontalScrollbar";
import { formatCellValue } from "../../lib/format";
import { resolveRenderMode } from "../../lib/semantic-render";

type Props = {
  width: number;
  height: number;
};

export function ScrollContainer({ width, height }: Props) {
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const setViewport = useTableStore((s) => s.setViewport);
  const setSelection = useTableStore((s) => s.setSelection);
  const selection = useTableStore((s) => s.selection);
  const tiles = useTableStore((s) => s.tiles);
  const zoom = useUiStore((s) => s.zoom);
  const isDark = useUiStore((s) => s.isDark);
  const setTooltip = useUiStore((s) => s.setTooltip);
  const setContextMenu = useUiStore((s) => s.setContextMenu);
  const semanticZoomEnabled = useUiStore((s) => s.semanticZoomEnabled);
  const viewHash = useViewStore((s) => s.viewHash);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const virtualSchema = useViewStore((s) => s.virtualSchema);
  const allStats = useStatsStore((s) => s.stats);
  const { applyZoom } = useZoom();
  const { navigateTo } = useNavigation();

  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const cellW = DEFAULT_CELL_W * zoom;
  const cellH = DEFAULT_CELL_H * zoom;
  const gridW = width;
  const gridH = height - SCROLLBAR_SIZE;

  const effectiveRowCount = virtualRowCount ?? source?.n_rows ?? 0;
  const nCols = source?.n_cols ?? 0;
  const { maxX, maxY } = scrollBounds(effectiveRowCount, nCols, cellW, cellH, gridW, gridH);

  const totalCols = nCols;
  const viewportCols = Math.ceil(gridW / cellW);
  const scrollCol = Math.round(viewport.scrollX / cellW);

  const wheelHandler = useCallback(
    (e: WheelEvent) => {
      e.preventDefault();
      if (!source) return;

      if (e.ctrlKey) {
        const zoomDelta = -e.deltaY * 0.01;
        applyZoom(zoom + zoomDelta, {
          x: e.offsetX - ROW_HEADER_W,
          y: e.offsetY - HEADER_HEIGHT,
        });
        return;
      }

      let dx = e.deltaX;
      let dy = e.deltaY;
      if (e.deltaMode === 1) {
        dx *= cellW;
        dy *= cellH;
      } else if (e.deltaMode === 2) {
        dx *= gridW;
        dy *= gridH;
      }
      navigateTo(
        Math.max(0, Math.min(maxX, viewport.scrollX + dx)),
        Math.max(0, Math.min(maxY, viewport.scrollY + dy)),
        false
      );
    },
    [source, viewport, cellW, cellH, gridW, gridH, navigateTo, effectiveRowCount, zoom, applyZoom, maxX, maxY]
  );

  const wheelHandlerRef = useRef(wheelHandler);
  useEffect(() => { wheelHandlerRef.current = wheelHandler; }, [wheelHandler]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => wheelHandlerRef.current(e);
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, []);

  const offsetFromEvent = (e: React.MouseEvent) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  const getCellValue = (row: number, col: number): { value: unknown; column: string } | null => {
    if (!source) return null;
    const tileRows = tileRowsForZoom(zoom);
    const tileCols = tileColsForZoom(zoom);
    const tileRow = Math.floor(row / tileRows);
    const tileCol = Math.floor(col / tileCols);
    const localRow = row % tileRows;
    const localCol = col % tileCols;
    const tile = tiles.get(tileKey(tileRow, tileCol, viewHash));
    if (!tile) return null;
    const column = tile.getChildAt(localCol);
    if (!column || localRow >= column.length) return null;
    const columnInfo = virtualSchema?.[col] ?? source.columns[col];
    return { value: column.get(localRow), column: columnInfo?.name ?? String(col) };
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const { x, y } = offsetFromEvent(e);
    const cell = cellAtPixel(x, y, viewport.scrollX, viewport.scrollY, cellW, cellH);
    setSelection({ anchor: cell, active: cell });
    dragging.current = true;
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    const { x, y } = offsetFromEvent(e);
    const cell = cellAtPixel(x, y, viewport.scrollX, viewport.scrollY, cellW, cellH);

    if (dragging.current && selection) {
      setSelection({ anchor: selection.anchor, active: cell });
    }

    if (source) {
      const modeRes = semanticZoomEnabled ? resolveRenderMode(zoom) : { primary: "read" as const, secondary: null, blend: 0 };

      if ((modeRes.primary === "satellite" || modeRes.primary === "profile") && zoom < 0.28) {
        const col = cell.col;
        const colInfo = virtualSchema?.[col] ?? source.columns[col];
        if (colInfo) {
          const statsKey = getStatsKey(source.id, col);
          const fs = allStats.get(statsKey);
          if (fs) {
            const mean = fs.mean !== null ? `mean: ${fs.mean.toPrecision(4)}` : null;
            const parts = [
              `${colInfo.name}`,
              `min: ${fs.min}`,
              `max: ${fs.max}`,
              ...(mean ? [mean] : []),
              `null: ${Math.round(fs.null_rate * 100)}%`,
            ];
            setTooltip({ x: e.clientX + 12, y: e.clientY + 12, value: parts.join(" · ") });
            return;
          }
        }
        setTooltip(null);
        return;
      }

      const cv = getCellValue(cell.row, cell.col);
      if (cv) {
        if (zoom >= 0.6) {
          setTooltip({ x: e.clientX + 12, y: e.clientY + 12, value: formatCellValue(cv.value) });
        } else {
          const colInfo = virtualSchema?.[cell.col] ?? source.columns[cell.col];
          const typePart = colInfo ? ` (${colInfo.data_type})` : "";
          setTooltip({ x: e.clientX + 12, y: e.clientY + 12, value: `${cv.column}: ${formatCellValue(cv.value)}${typePart}` });
        }
        return;
      }
    }
    setTooltip(null);
  };

  const handleMouseUp = () => { dragging.current = false; };
  const handleMouseLeave = () => { dragging.current = false; setTooltip(null); };

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    const { x, y } = offsetFromEvent(e);
    const cell = cellAtPixel(x, y, viewport.scrollX, viewport.scrollY, cellW, cellH);
    const cv = getCellValue(cell.row, cell.col);
    if (cv) {
      setContextMenu({ kind: "cell", x: e.clientX, y: e.clientY, cell, value: cv.value, column: cv.column });
    }
  };

  const onScrollCol = useCallback((col: number) => {
    navigateTo(Math.round(col * cellW), viewport.scrollY, false);
  }, [navigateTo, viewport.scrollY, cellW]);

  return (
    <div style={{ position: "relative", width, height, overflow: "hidden" }}>
      <div
        ref={containerRef}
        style={{ position: "absolute", left: 0, top: 0, right: 0, bottom: SCROLLBAR_SIZE, cursor: "cell" }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseLeave}
        onContextMenu={handleContextMenu}
      >
        <GridCanvas width={gridW} height={gridH} />
      </div>
      <div style={{ position: "absolute", left: 0, right: 0, bottom: 0 }}>
        <HorizontalScrollbar
          totalCols={totalCols}
          viewportCols={viewportCols}
          scrollCol={scrollCol}
          onScrollCol={onScrollCol}
          width={width}
          columns={source?.columns ?? []}
          isDark={isDark}
        />
      </div>
    </div>
  );
}
