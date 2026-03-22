import { useRef, useState, useLayoutEffect, useEffect, useCallback } from "react";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";
import { useBookmarkStore, BOOKMARK_COLORS } from "../../stores/bookmarkStore";
import { useNavigation } from "../../hooks/useNavigation";
import { useRowGroupStats } from "../../hooks/useRowGroupStats";
import { useLandmarks } from "../../hooks/useLandmarks";
import { HEADER_HEIGHT, DEFAULT_CELL_W, DEFAULT_CELL_H, SCROLLBAR_SIZE } from "../../lib/viewport";
import { MinimapCanvas } from "./MinimapCanvas";
import { BookmarkPins } from "./BookmarkPins";

export function Minimap() {
  const minimapVisible = useUiStore((s) => s.minimapVisible);
  const minimapWidth = useUiStore((s) => s.minimapWidth);
  const setMinimapVisible = useUiStore((s) => s.setMinimapVisible);
  const zoom = useUiStore((s) => s.zoom);

  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);

  const addBookmark = useBookmarkStore((s) => s.addBookmark);
  const bookmarks = useBookmarkStore((s) => s.bookmarks);

  const { teleportTo, navigateTo } = useNavigation();
  const mainRef = useRef<HTMLDivElement>(null);
  const [panelH, setPanelH] = useState(400);
  const [hovered, setHovered] = useState(false);

  useLayoutEffect(() => {
    const el = mainRef.current;
    if (!el) return;
    const obs = new ResizeObserver(() => {
      if (el.clientHeight > 0) setPanelH(el.clientHeight);
    });
    obs.observe(el);
    if (el.clientHeight > 0) setPanelH(el.clientHeight);
    return () => obs.disconnect();
  }, [minimapVisible]);

  useRowGroupStats();
  useLandmarks();

  const nextBookmarkColor = () => BOOKMARK_COLORS[bookmarks.length % BOOKMARK_COLORS.length] ?? "#6b7280";

  const cellW = DEFAULT_CELL_W * zoom;
  const cellH = DEFAULT_CELL_H * zoom;

  const wheelHandler = useCallback(
    (e: WheelEvent) => {
      e.preventDefault();
      if (!source) return;
      let dx = e.deltaX;
      let dy = e.deltaY;
      if (e.deltaMode === 1) {
        dx *= cellW;
        dy *= cellH;
      }
      navigateTo(
        Math.max(0, viewport.scrollX + dx),
        Math.max(0, viewport.scrollY + dy),
        false
      );
    },
    [source, viewport, cellW, cellH, navigateTo]
  );

  const wheelHandlerRef = useRef(wheelHandler);
  useEffect(() => { wheelHandlerRef.current = wheelHandler; }, [wheelHandler]);

  useEffect(() => {
    const el = mainRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => wheelHandlerRef.current(e);
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, [minimapVisible]);

  if (!minimapVisible) {
    return (
      <div
        style={{
          position: "absolute",
          top: HEADER_HEIGHT,
          bottom: SCROLLBAR_SIZE,
          right: 0,
          width: 12,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          pointerEvents: "none",
          zIndex: 40,
        }}
      >
        <button
          onClick={() => setMinimapVisible(true)}
          title="Show minimap (Ctrl+M)"
          style={{
            pointerEvents: "auto",
            width: 12,
            height: 32,
            background: "var(--c-bg)",
            border: "1px solid var(--c-border)",
            borderRight: "none",
            borderRadius: "4px 0 0 4px",
            cursor: "pointer",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontSize: 10,
            color: "var(--c-text-3)",
            padding: 0,
          }}
        >
          ›
        </button>
      </div>
    );
  }

  return (
    <div
      ref={mainRef}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        position: "absolute",
        top: HEADER_HEIGHT,
        right: 0,
        bottom: SCROLLBAR_SIZE,
        width: minimapWidth,
        zIndex: 40,
        background: "var(--c-bg)",
        borderLeft: "1px solid var(--c-border)",
      }}
    >
      <div style={{ position: "relative", width: "100%", height: "100%", overflow: "hidden" }}>
        {source ? (
          <>
            <MinimapCanvas
              panelW={minimapWidth}
              panelH={panelH}
              onNavigate={(x, y) => teleportTo(x, y)}
              onDrag={(x, y) => navigateTo(x, y, false)}
              onBookmark={(x, y) => {
                const row = Math.round(y / 32);
                addBookmark({
                  scrollX: x,
                  scrollY: y,
                  label: `Row ${row.toLocaleString()}`,
                  color: nextBookmarkColor(),
                });
              }}
            />
            <BookmarkPins panelW={minimapWidth} panelH={panelH} />
          </>
        ) : (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", color: "var(--c-text-3)", fontSize: 11 }}>
            No source
          </div>
        )}
      </div>
      <button
        onClick={() => setMinimapVisible(false)}
        title="Hide minimap (Ctrl+M)"
        style={{
          position: "absolute",
          left: -12,
          top: "50%",
          transform: "translateY(-50%)",
          width: 12,
          height: 32,
          background: "var(--c-bg)",
          border: "1px solid var(--c-border)",
          borderRight: "none",
          borderRadius: "4px 0 0 4px",
          cursor: "pointer",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontSize: 10,
          color: "var(--c-text-3)",
          padding: 0,
          opacity: hovered ? 1 : 0,
          transition: "opacity 150ms",
          zIndex: 41,
        }}
      >
        ‹
      </button>
    </div>
  );
}
