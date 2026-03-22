import { useEffect } from "react";
import { useTableStore } from "../stores/table";
import { useUiStore } from "../stores/ui";
import { useLandmarkStore } from "../stores/landmarkStore";
import { useBookmarkStore, BOOKMARK_COLORS } from "../stores/bookmarkStore";
import { useZoom } from "./useZoom";
import { useNavigation } from "./useNavigation";
import { DEFAULT_CELL_W, DEFAULT_CELL_H, scrollBounds } from "../lib/viewport";

export function useKeyboard() {
  const source = useTableStore((s) => s.source);
  const viewport = useTableStore((s) => s.viewport);
  const setShowCommandPalette = useUiStore((s) => s.setShowCommandPalette);
  const setShowFilterBuilder = useUiStore((s) => s.setShowFilterBuilder);
  const toggleColumnPanel = useUiStore((s) => s.toggleColumnPanel);
  const setShowExportPanel = useUiStore((s) => s.setShowExportPanel);
  const setMinimapVisible = useUiStore((s) => s.setMinimapVisible);
  const minimapVisible = useUiStore((s) => s.minimapVisible);
  const setShowLandmarkPanel = useUiStore((s) => s.setShowLandmarkPanel);
  const setShowBookmarkPanel = useUiStore((s) => s.setShowBookmarkPanel);
  const setShowKeyboardShortcuts = useUiStore((s) => s.setShowKeyboardShortcuts);
  const { zoomIn, zoomOut } = useZoom();
  const { navigateTo, teleportTo } = useNavigation();

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      const tag = el.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable) return;

      const nRows = source?.n_rows ?? 0;
      const nCols = source?.n_cols ?? 0;
      const { zoom, navHistoryBack, navHistoryForward } = useUiStore.getState();
      const cellW = DEFAULT_CELL_W * zoom;
      const cellH = DEFAULT_CELL_H * zoom;
      const { maxX, maxY } = scrollBounds(nRows, nCols, cellW, cellH, viewport.width, viewport.height);
      const { landmarks } = useLandmarkStore.getState();
      const { bookmarks } = useBookmarkStore.getState();

      switch (e.key) {
        case "p":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            setShowCommandPalette(true);
          }
          break;
        case "f":
          if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
            e.preventDefault();
            setShowFilterBuilder(true);
          }
          break;
        case "c":
          if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
            e.preventDefault();
            toggleColumnPanel();
          }
          break;
        case "e":
          if ((e.ctrlKey || e.metaKey) && source) {
            e.preventDefault();
            setShowExportPanel(true);
          }
          break;
        case "m":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            setMinimapVisible(!minimapVisible);
          }
          break;
        case "b":
          if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
            e.preventDefault();
            setShowBookmarkPanel(true);
          } else if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            const { addBookmark, bookmarks: bms } = useBookmarkStore.getState();
            const { zoom: z } = useUiStore.getState();
            const color = BOOKMARK_COLORS[bms.length % BOOKMARK_COLORS.length] ?? "#6b7280";
            const row = Math.round(viewport.scrollY / (DEFAULT_CELL_H * z));
            addBookmark({ scrollX: viewport.scrollX, scrollY: viewport.scrollY, label: `Row ${row.toLocaleString()}`, color });
          }
          break;
        case "l":
          if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
            e.preventDefault();
            setShowLandmarkPanel(true);
          }
          break;
        case "ArrowRight":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            navigateTo(Math.min(maxX, viewport.scrollX + cellW * 10), viewport.scrollY, false);
            break;
          }
          if (e.altKey) {
            e.preventDefault();
            const pos = navHistoryForward();
            if (pos) navigateTo(pos.scrollX, pos.scrollY, false);
            break;
          }
          navigateTo(Math.min(maxX, viewport.scrollX + cellW), viewport.scrollY, false);
          break;
        case "ArrowLeft":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            navigateTo(Math.max(0, viewport.scrollX - cellW * 10), viewport.scrollY, false);
            break;
          }
          if (e.altKey) {
            e.preventDefault();
            const pos = navHistoryBack();
            if (pos) navigateTo(pos.scrollX, pos.scrollY, false);
            break;
          }
          navigateTo(Math.max(0, viewport.scrollX - cellW), viewport.scrollY, false);
          break;
        case "ArrowDown":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            const next = landmarks.find((l) => l.rowOffset * cellH > viewport.scrollY + cellH);
            if (next) {
              navigateTo(viewport.scrollX, next.rowOffset * cellH, true);
            }
            break;
          }
          navigateTo(viewport.scrollX, Math.min(maxY, viewport.scrollY + cellH), false);
          break;
        case "ArrowUp":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            const prev = [...landmarks].reverse().find((l) => l.rowOffset * cellH < viewport.scrollY - cellH);
            if (prev) {
              navigateTo(viewport.scrollX, prev.rowOffset * cellH, true);
            }
            break;
          }
          navigateTo(viewport.scrollX, Math.max(0, viewport.scrollY - cellH), false);
          break;
        case "Home":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            navigateTo(0, 0, true);
          } else {
            navigateTo(0, viewport.scrollY, false);
          }
          break;
        case "End":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            navigateTo(maxX, maxY, true);
          } else {
            navigateTo(maxX, viewport.scrollY, false);
          }
          break;
        case "PageDown":
          if (e.shiftKey) {
            navigateTo(viewport.scrollX, Math.min(maxY, viewport.scrollY + 10 * viewport.height), true);
          } else {
            navigateTo(viewport.scrollX, Math.min(maxY, viewport.scrollY + viewport.height), false);
          }
          break;
        case "PageUp":
          if (e.shiftKey) {
            navigateTo(viewport.scrollX, Math.max(0, viewport.scrollY - 10 * viewport.height), true);
          } else {
            navigateTo(viewport.scrollX, Math.max(0, viewport.scrollY - viewport.height), false);
          }
          break;
        case "g":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            useUiStore.getState().setShowJumpToRow(true);
          }
          break;
        case "+":
        case "=":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            zoomIn();
          }
          break;
        case "-":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            zoomOut();
          }
          break;
        case "1": case "2": case "3": case "4":
        case "5": case "6": case "7": case "8":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            const idx = parseInt(e.key, 10) - 1;
            const bm = bookmarks[idx];
            if (bm) navigateTo(bm.scrollX, bm.scrollY, true);
          }
          break;
        case "?":
          e.preventDefault();
          setShowKeyboardShortcuts(!useUiStore.getState().showKeyboardShortcuts);
          break;
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [
    source,
    viewport,
    setShowCommandPalette,
    setShowFilterBuilder,
    toggleColumnPanel,
    setShowExportPanel,
    setMinimapVisible,
    minimapVisible,
    setShowLandmarkPanel,
    setShowBookmarkPanel,
    setShowKeyboardShortcuts,
    navigateTo,
    teleportTo,
    zoomIn,
    zoomOut,
  ]);
}
