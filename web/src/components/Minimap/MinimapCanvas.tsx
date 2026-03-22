import { useRef, useEffect } from "react";
import { useTableStore } from "../../stores/table";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useBookmarkStore } from "../../stores/bookmarkStore";
import { useMinimap } from "../../hooks/useMinimap";
import { DEFAULT_CELL_W, DEFAULT_CELL_H, scrollBounds } from "../../lib/viewport";

interface Props {
  panelW: number;
  panelH: number;
  onNavigate: (scrollX: number, scrollY: number) => void;
  onDrag: (scrollX: number, scrollY: number) => void;
  onBookmark: (scrollX: number, scrollY: number) => void;
}

export function MinimapCanvas({ panelW, panelH, onNavigate, onDrag, onBookmark }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const bitmapRef = useRef<ImageData | null>(null);
  const draggingRef = useRef(false);

  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const zoom = useUiStore((s) => s.zoom);
  const isDark = useUiStore((s) => s.isDark);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const bookmarks = useBookmarkStore((s) => s.bookmarks);
  const imageData = useMinimap(panelW, panelH);

  useEffect(() => {
    bitmapRef.current = imageData;
    redraw();
  }, [imageData]);

  useEffect(() => {
    redraw();
  }, [viewport, zoom, bookmarks, isDark]);

  const getViewportBand = () => {
    if (!source) return { vpY: 0, vpH: panelH };
    const nRows = virtualRowCount ?? source.n_rows;
    const nCols = source.n_cols;
    const cellW = DEFAULT_CELL_W * zoom;
    const cellH = DEFAULT_CELL_H * zoom;
    const { maxY } = scrollBounds(nRows, nCols, cellW, cellH, viewport.width, viewport.height);
    const totalH = Math.max(1, nRows * cellH);
    const vpH = Math.max(8, Math.min(panelH, (viewport.height / totalH) * panelH));
    const vpY = maxY > 0 ? (viewport.scrollY / maxY) * (panelH - vpH) : 0;
    return { vpY, vpH };
  };

  const redraw = () => {
    const canvas = canvasRef.current;
    if (!canvas || !source) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.clearRect(0, 0, panelW, panelH);

    if (bitmapRef.current) {
      ctx.putImageData(bitmapRef.current, 0, 0);
    } else {
      ctx.fillStyle = isDark ? "#1a2438" : "#f1f5f9";
      ctx.fillRect(0, 0, panelW, panelH);
    }

    const { vpY, vpH } = getViewportBand();

    ctx.fillStyle = isDark ? "rgba(10,18,36,0.72)" : "rgba(241,245,249,0.75)";
    if (vpY > 0) ctx.fillRect(0, 0, panelW, vpY);
    if (vpY + vpH < panelH) ctx.fillRect(0, vpY + vpH, panelW, panelH - (vpY + vpH));

    ctx.fillStyle = isDark ? "rgba(96,165,250,0.07)" : "rgba(59,130,246,0.05)";
    ctx.fillRect(0, vpY, panelW, vpH);

    ctx.strokeStyle = isDark ? "rgba(96,165,250,0.9)" : "rgba(59,130,246,0.8)";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(0, vpY + 1);
    ctx.lineTo(panelW, vpY + 1);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(0, vpY + vpH - 1);
    ctx.lineTo(panelW, vpY + vpH - 1);
    ctx.stroke();

    if (vpH > 12) {
      const midY = Math.round(vpY + vpH / 2);
      ctx.fillStyle = isDark ? "rgba(96,165,250,0.5)" : "rgba(59,130,246,0.45)";
      for (let d = -3; d <= 3; d += 3) {
        ctx.beginPath();
        ctx.arc(panelW / 2 + d * 3, midY, 1.5, 0, Math.PI * 2);
        ctx.fill();
      }
    }

    const nRows = virtualRowCount ?? source.n_rows;
    const nCols = source.n_cols;
    const cellW = DEFAULT_CELL_W * zoom;
    const cellH = DEFAULT_CELL_H * zoom;
    const { maxY } = scrollBounds(nRows, nCols, cellW, cellH, viewport.width, viewport.height);

    for (const bm of bookmarks) {
      const by = maxY > 0 ? (bm.scrollY / maxY) * (panelH - 8) : 0;
      const s = 4;
      ctx.fillStyle = bm.color;
      ctx.globalAlpha = 0.9;
      ctx.beginPath();
      ctx.moveTo(6, by - s);
      ctx.lineTo(6 + s, by);
      ctx.lineTo(6, by + s);
      ctx.lineTo(6 - s, by);
      ctx.closePath();
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  };

  const toScrollY = (offsetY: number): number => {
    if (!source) return viewport.scrollY;
    const nRows = virtualRowCount ?? source.n_rows;
    const nCols = source.n_cols;
    const cellW = DEFAULT_CELL_W * zoom;
    const cellH = DEFAULT_CELL_H * zoom;
    const { maxY } = scrollBounds(nRows, nCols, cellW, cellH, viewport.width, viewport.height);
    const { vpH } = getViewportBand();
    return Math.max(0, Math.min(maxY, (offsetY / Math.max(1, panelH - vpH)) * maxY));
  };

  const handlePointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const { offsetY } = e.nativeEvent;
    if (e.ctrlKey) {
      const scrollY = toScrollY(offsetY);
      onBookmark(viewport.scrollX, scrollY);
      return;
    }
    draggingRef.current = true;
    (e.currentTarget as HTMLCanvasElement).setPointerCapture(e.pointerId);
    onNavigate(viewport.scrollX, toScrollY(offsetY));
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (!draggingRef.current) return;
    onDrag(viewport.scrollX, toScrollY(e.nativeEvent.offsetY));
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (draggingRef.current) {
      draggingRef.current = false;
      (e.currentTarget as HTMLCanvasElement).releasePointerCapture(e.pointerId);
    }
  };

  return (
    <canvas
      ref={canvasRef}
      width={panelW}
      height={panelH}
      style={{ display: "block", cursor: "ns-resize" }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    />
  );
}
