import { useEffect, useRef } from "react";
import { useTableStore } from "../../stores/table";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { DEFAULT_CELL_H, ROW_HEADER_W } from "../../lib/viewport";

type Props = {
  height: number;
};

const FONT = "11px ui-monospace, monospace";

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function RowHeader({ height }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const zoom = useUiStore((s) => s.zoom);
  const isDark = useUiStore((s) => s.isDark);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);

  const cellH = DEFAULT_CELL_H * zoom;
  const effectiveRowCount = virtualRowCount ?? source?.n_rows ?? 0;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !source) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = ROW_HEADER_W * dpr;
    canvas.height = height * dpr;
    canvas.style.width = `${ROW_HEADER_W}px`;
    canvas.style.height = `${height}px`;

    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);

    const bg = cssVar("--canvas-header-bg") || "#f9fafb";
    const border = cssVar("--canvas-grid") || "#e5e7eb";
    const textColor = cssVar("--canvas-text-null") || "#9ca3af";

    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, ROW_HEADER_W, height);

    const rowStart = Math.floor(viewport.scrollY / cellH);
    const rowEnd = Math.min(effectiveRowCount, Math.ceil((viewport.scrollY + height) / cellH) + 1);

    ctx.font = FONT;
    ctx.textBaseline = "middle";
    ctx.textAlign = "right";

    const step = zoom >= 0.6 ? 1 : zoom >= 0.35 ? 10 : 100;

    for (let row = rowStart; row < rowEnd; row++) {
      const y = row * cellH - viewport.scrollY;
      ctx.fillStyle = bg;
      ctx.fillRect(0, y, ROW_HEADER_W, cellH);
      ctx.strokeStyle = border;
      ctx.strokeRect(0, y, ROW_HEADER_W, cellH);

      if (cellH < 4) continue;

      if (step === 1) {
        ctx.fillStyle = textColor;
        ctx.fillText(String(row + 1), ROW_HEADER_W - 6, y + cellH / 2);
      } else if (row % step === 0) {
        ctx.fillStyle = textColor;
        ctx.fillRect(ROW_HEADER_W - 5, y, 4, Math.max(1, Math.min(2, cellH)));
        if (cellH >= 6) {
          ctx.fillText(String(row + 1), ROW_HEADER_W - 8, y + cellH / 2);
        }
      }
    }

    ctx.textAlign = "left";
  }, [height, source, viewport.scrollY, zoom, effectiveRowCount, isDark]);

  return <canvas ref={canvasRef} style={{ display: "block" }} />;
}
