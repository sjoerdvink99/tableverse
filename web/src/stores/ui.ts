import { create } from "zustand";
import type { CellAddress } from "../lib/types";
import type { SatelliteEncoding } from "../lib/profile-render";

export type Theme = "light" | "dark" | "system";

type ContextMenuTarget =
  | { kind: "cell"; x: number; y: number; cell: CellAddress; value: unknown; column: string }
  | { kind: "column"; x: number; y: number; colIndex: number; colName: string };

export type NavHistoryEntry = { scrollX: number; scrollY: number };

type UiState = {
  zoom: number;
  theme: Theme;
  isDark: boolean;
  semanticZoomEnabled: boolean;
  showSourceManager: boolean;
  showJumpToRow: boolean;
  showKeyboardShortcuts: boolean;
  showExportPanel: boolean;
  showEDAPanel: boolean;
  showColumnPanel: boolean;
  showFilterBuilder: boolean;
  showCommandPalette: boolean;
  contextMenu: ContextMenuTarget | null;
  hoveredCell: CellAddress | null;
  hoveredColumnIndex: number | null;
  tooltip: { x: number; y: number; value: string } | null;
  minimapVisible: boolean;
  minimapWidth: number;
  navHistory: NavHistoryEntry[];
  navHistoryIdx: number;
  showLandmarkPanel: boolean;
  showBookmarkPanel: boolean;
  showCatalogBrowser: boolean;
  activeJobId: string | null;
  sortAccessCounts: Record<string, number>;
  satelliteEncoding: SatelliteEncoding;
  pinnedCorrelationCol: number | null;

  setZoom: (zoom: number) => void;
  zoomIn: () => void;
  zoomOut: () => void;
  resetZoom: () => void;
  setSemanticZoomEnabled: (enabled: boolean) => void;
  setTheme: (theme: Theme) => void;
  toggleTheme: () => void;
  toggleSourceManager: () => void;
  setShowJumpToRow: (show: boolean) => void;
  setShowKeyboardShortcuts: (show: boolean) => void;
  setShowExportPanel: (show: boolean) => void;
  setShowEDAPanel: (show: boolean) => void;
  toggleColumnPanel: () => void;
  setShowFilterBuilder: (show: boolean) => void;
  setShowCommandPalette: (show: boolean) => void;
  setContextMenu: (menu: ContextMenuTarget | null) => void;
  setHoveredCell: (cell: CellAddress | null) => void;
  setHoveredColumnIndex: (index: number | null) => void;
  setTooltip: (tooltip: { x: number; y: number; value: string } | null) => void;
  setMinimapVisible: (v: boolean) => void;
  setMinimapWidth: (w: number) => void;
  pushNavHistory: (pos: NavHistoryEntry) => void;
  navHistoryBack: () => NavHistoryEntry | undefined;
  navHistoryForward: () => NavHistoryEntry | undefined;
  setShowLandmarkPanel: (show: boolean) => void;
  setShowBookmarkPanel: (show: boolean) => void;
  setShowCatalogBrowser: (show: boolean) => void;
  setActiveJobId: (id: string | null) => void;
  sortAccessCount: (col: string) => number;
  incrementSortAccess: (col: string) => number;
  setSatelliteEncoding: (enc: SatelliteEncoding) => void;
  setPinnedCorrelationCol: (col: number | null) => void;
};

export const ZOOM_MIN = 0.05;
export const ZOOM_MAX = 3;

export function adaptiveZoomStep(zoom: number): number {
  return zoom < 0.5 ? 0.05 : 0.1;
}

function resolveIsDark(theme: Theme): boolean {
  if (theme === "dark") return true;
  if (theme === "light") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyTheme(isDark: boolean) {
  if (isDark) {
    document.documentElement.classList.add("dark");
  } else {
    document.documentElement.classList.remove("dark");
  }
}

const savedTheme = (localStorage.getItem("tv-theme") as Theme | null) ?? "system";
const initialIsDark = resolveIsDark(savedTheme);
applyTheme(initialIsDark);

const NAV_HISTORY_MAX = 64;

export const useUiStore = create<UiState>((set, get) => ({
  zoom: 1,
  theme: savedTheme,
  isDark: initialIsDark,
  semanticZoomEnabled: true,
  showSourceManager: false,
  showJumpToRow: false,
  showKeyboardShortcuts: false,
  showExportPanel: false,
  showEDAPanel: false,
  showColumnPanel: false,
  showFilterBuilder: false,
  showCommandPalette: false,
  contextMenu: null,
  hoveredCell: null,
  hoveredColumnIndex: null,
  tooltip: null,
  minimapVisible: true,
  minimapWidth: 100,
  navHistory: [],
  navHistoryIdx: -1,
  showLandmarkPanel: false,
  showBookmarkPanel: false,
  showCatalogBrowser: false,
  activeJobId: null,
  sortAccessCounts: {},
  satelliteEncoding: "null_rate" as SatelliteEncoding,
  pinnedCorrelationCol: null,

  setZoom: (zoom) => set({ zoom: Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom)) }),
  zoomIn: () => set((s) => ({ zoom: Math.min(ZOOM_MAX, +(s.zoom + adaptiveZoomStep(s.zoom)).toFixed(2)) })),
  zoomOut: () => set((s) => ({ zoom: Math.max(ZOOM_MIN, +(s.zoom - adaptiveZoomStep(s.zoom)).toFixed(2)) })),
  resetZoom: () => set({ zoom: 1 }),
  setSemanticZoomEnabled: (semanticZoomEnabled) => set({ semanticZoomEnabled }),

  setTheme: (theme) => {
    const isDark = resolveIsDark(theme);
    applyTheme(isDark);
    localStorage.setItem("tv-theme", theme);
    set({ theme, isDark });
  },

  toggleTheme: () => {
    const { isDark } = get();
    const next: Theme = isDark ? "light" : "dark";
    get().setTheme(next);
  },

  toggleSourceManager: () => set((s) => ({ showSourceManager: !s.showSourceManager })),
  setShowJumpToRow: (show) => set({ showJumpToRow: show }),
  setShowKeyboardShortcuts: (show) => set({ showKeyboardShortcuts: show }),
  setShowExportPanel: (show) => set({ showExportPanel: show }),
  setShowEDAPanel: (show) => set({ showEDAPanel: show }),
  toggleColumnPanel: () => set((s) => ({ showColumnPanel: !s.showColumnPanel })),
  setShowFilterBuilder: (show) => set({ showFilterBuilder: show }),
  setShowCommandPalette: (show) => set({ showCommandPalette: show }),
  setContextMenu: (contextMenu) => set({ contextMenu }),
  setHoveredCell: (hoveredCell) => set({ hoveredCell }),
  setHoveredColumnIndex: (hoveredColumnIndex) => set({ hoveredColumnIndex }),
  setTooltip: (tooltip) => set({ tooltip }),
  setMinimapVisible: (minimapVisible) => set({ minimapVisible }),
  setMinimapWidth: (minimapWidth) => set({ minimapWidth }),

  pushNavHistory: (pos) =>
    set((s) => {
      const truncated = s.navHistory.slice(0, s.navHistoryIdx + 1);
      const next = [...truncated, pos].slice(-NAV_HISTORY_MAX);
      return { navHistory: next, navHistoryIdx: next.length - 1 };
    }),

  navHistoryBack: () => {
    const { navHistory, navHistoryIdx } = get();
    if (navHistoryIdx <= 0) return undefined;
    const newIdx = navHistoryIdx - 1;
    set({ navHistoryIdx: newIdx });
    return navHistory[newIdx];
  },

  navHistoryForward: () => {
    const { navHistory, navHistoryIdx } = get();
    if (navHistoryIdx >= navHistory.length - 1) return undefined;
    const newIdx = navHistoryIdx + 1;
    set({ navHistoryIdx: newIdx });
    return navHistory[newIdx];
  },

  setShowLandmarkPanel: (showLandmarkPanel) => set({ showLandmarkPanel }),
  setShowBookmarkPanel: (showBookmarkPanel) => set({ showBookmarkPanel }),
  setShowCatalogBrowser: (showCatalogBrowser) => set({ showCatalogBrowser }),
  setActiveJobId: (activeJobId) => set({ activeJobId }),
  sortAccessCount: (col) => get().sortAccessCounts[col] ?? 0,
  incrementSortAccess: (col) => {
    const next = (get().sortAccessCounts[col] ?? 0) + 1;
    set((s) => ({ sortAccessCounts: { ...s.sortAccessCounts, [col]: next } }));
    return next;
  },
  setSatelliteEncoding: (satelliteEncoding) => set({ satelliteEncoding }),
  setPinnedCorrelationCol: (pinnedCorrelationCol) => set({ pinnedCorrelationCol }),
}));
