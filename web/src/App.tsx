import React, { useState } from "react";
import { TableViewer } from "./components/TableViewer";
import { PipelineBar } from "./components/PipelineBar";
import { Tooltip } from "./components/Overlays/Tooltip";
import { JumpToRow } from "./components/Overlays/JumpToRow";
import { SourceManager } from "./components/SourceManager";
import { CellContextMenu } from "./components/CellContextMenu";
import { ColumnContextMenu } from "./components/ColumnContextMenu";
import { ExportPanel } from "./components/ExportPanel";
import { EDAPanel } from "./components/EDA/EDAPanel";
import { ColumnPanel } from "./components/ColumnPanel";
import { FilterBuilder } from "./components/FilterBuilder";
import { CommandPalette } from "./components/CommandPalette";
import { LandmarkPanel } from "./components/Overlays/LandmarkPanel";
import { BookmarkPanel } from "./components/Overlays/BookmarkPanel";
import { KeyboardShortcutsPanel } from "./components/Overlays/KeyboardShortcutsPanel";
import { CatalogBrowser } from "./components/CatalogBrowser";
import { StatusBar } from "./components/StatusBar";
import { SearchBar } from "./components/Toolbar/SearchBar";
import { ZoomControls } from "./components/Toolbar/ZoomControls";
import { NavigationBar } from "./components/Toolbar/NavigationBar";
import { NavHistoryButtons } from "./components/Toolbar/NavHistoryButtons";
import { useUiStore } from "./stores/ui";
import { useTableStore } from "./stores/table";
import { useUrlState } from "./hooks/useUrlState";
import "./styles.css";

class TableErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { hasError: boolean; error: string }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { hasError: false, error: "" };
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error: error.message };
  }

  override render() {
    if (this.state.hasError) {
      return (
        <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", flexDirection: "column", gap: 8, color: "#6b7280" }}>
          <span style={{ fontSize: 14 }}>Something went wrong rendering the table.</span>
          <button
            onClick={() => this.setState({ hasError: false, error: "" })}
            style={{ fontSize: 12, padding: "4px 12px", border: "1px solid #e5e7eb", borderRadius: 4, cursor: "pointer", background: "white" }}
          >
            Retry
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

function LogoMark() {
  return (
    <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
      <rect width="20" height="20" rx="4.5" fill="#1e40af" />
      <rect x="3.5" y="4" width="13" height="2.5" rx="1.25" fill="white" />
      <rect x="3.5" y="8.75" width="13" height="2" rx="1" fill="white" fillOpacity="0.65" />
      <rect x="3.5" y="13.5" width="13" height="2" rx="1" fill="white" fillOpacity="0.35" />
    </svg>
  );
}

function DatabaseIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <ellipse cx="6.5" cy="3.25" rx="4" ry="1.75" stroke="currentColor" strokeWidth="1.25" />
      <path
        d="M2.5 3.25v6.5c0 .97 1.79 1.75 4 1.75s4-.78 4-1.75V3.25"
        stroke="currentColor"
        strokeWidth="1.25"
        fill="none"
      />
      <path
        d="M2.5 7c0 .97 1.79 1.75 4 1.75S10.5 7.97 10.5 7"
        stroke="currentColor"
        strokeWidth="1.25"
        fill="none"
      />
    </svg>
  );
}

function ColumnsIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <rect x="1" y="1" width="3.5" height="11" rx="1" stroke="currentColor" strokeWidth="1.25" />
      <rect x="5.25" y="1" width="3.5" height="11" rx="1" stroke="currentColor" strokeWidth="1.25" />
      <rect x="9.5" y="1" width="2.5" height="11" rx="1" stroke="currentColor" strokeWidth="1.25" />
    </svg>
  );
}

function FilterIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <path d="M1.5 2.5h10M3.5 6.5h6M5.5 10.5h2" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" />
    </svg>
  );
}

function BarChartIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <rect x="1" y="7" width="3" height="5" rx="0.75" stroke="currentColor" strokeWidth="1.25" />
      <rect x="5" y="4" width="3" height="8" rx="0.75" stroke="currentColor" strokeWidth="1.25" />
      <rect x="9" y="1" width="3" height="11" rx="0.75" stroke="currentColor" strokeWidth="1.25" />
    </svg>
  );
}

function ExportIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <path d="M2 9v2h9V9" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M6.5 1v7M4 5.5l2.5 2.5 2.5-2.5" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ShareIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <circle cx="10" cy="2.5" r="1.5" stroke="currentColor" strokeWidth="1.25" />
      <circle cx="10" cy="10.5" r="1.5" stroke="currentColor" strokeWidth="1.25" />
      <circle cx="3" cy="6.5" r="1.5" stroke="currentColor" strokeWidth="1.25" />
      <path d="M4.4 5.7l4.2-2.4M4.4 7.3l4.2 2.4" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" />
    </svg>
  );
}

function SunIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <circle cx="6.5" cy="6.5" r="2.5" stroke="currentColor" strokeWidth="1.25" />
      <path d="M6.5 1v1.5M6.5 10.5V12M1 6.5h1.5M10.5 6.5H12M2.9 2.9l1.05 1.05M9.05 9.05l1.05 1.05M2.9 10.1l1.05-1.05M9.05 3.95l1.05-1.05" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
      <path d="M10.5 7.5A5 5 0 015.5 2.5a5 5 0 100 8 4.9 4.9 0 005-3z" stroke="currentColor" strokeWidth="1.25" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function App() {
  const toggleSourceManager = useUiStore((s) => s.toggleSourceManager);
  const setShowExportPanel = useUiStore((s) => s.setShowExportPanel);
  const setShowEDAPanel = useUiStore((s) => s.setShowEDAPanel);
  const toggleColumnPanel = useUiStore((s) => s.toggleColumnPanel);
  const setShowFilterBuilder = useUiStore((s) => s.setShowFilterBuilder);
  const setShowCatalogBrowser = useUiStore((s) => s.setShowCatalogBrowser);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const isDark = useUiStore((s) => s.isDark);
  const setShowKeyboardShortcuts = useUiStore((s) => s.setShowKeyboardShortcuts);
  const source = useTableStore((s) => s.source);
  const [shareCopied, setShareCopied] = useState(false);
  useUrlState();

  const handleShare = () => {
    navigator.clipboard.writeText(window.location.href).then(() => {
      setShareCopied(true);
      setTimeout(() => setShareCopied(false), 1800);
    });
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        overflow: "hidden",
        fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif",
        background: "var(--c-surface)",
        color: "var(--c-text)",
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "0 12px",
          height: 40,
          background: "var(--c-bg)",
          borderBottom: "1px solid var(--c-border)",
          flexShrink: 0,
          gap: 8,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
          <LogoMark />
          <span
            style={{
              fontWeight: 600,
              fontSize: 14,
              color: "var(--c-text)",
              letterSpacing: "-0.3px",
            }}
          >
            Tableverse
          </span>
          <span
            style={{
              color: "var(--c-border)",
              fontSize: 16,
              fontWeight: 300,
              margin: "0 1px",
              userSelect: "none",
            }}
          >
            /
          </span>
          {source ? (
            <button className="tv-btn" onClick={toggleSourceManager} style={{ maxWidth: 260 }}>
              <span
                style={{
                  display: "inline-block",
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: "#22c55e",
                  flexShrink: 0,
                }}
              />
              <span
                style={{
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {source.name}
              </span>
            </button>
          ) : (
            <button className="tv-btn" onClick={toggleSourceManager}>
              <DatabaseIcon />
              Sources
            </button>
          )}
          <button className="tv-btn tv-btn-ghost" onClick={() => setShowCatalogBrowser(true)} title="Browse external catalogs">
            Catalogs
          </button>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <NavHistoryButtons />
          <NavigationBar />
          <SearchBar />
          <ZoomControls />
          <button className="tv-btn tv-btn-ghost" onClick={toggleTheme} title={isDark ? "Switch to light mode" : "Switch to dark mode"}>
            {isDark ? <SunIcon /> : <MoonIcon />}
          </button>
          <button
            className="tv-btn tv-btn-ghost"
            onClick={() => setShowKeyboardShortcuts(true)}
            title="Keyboard shortcuts (?)"
            style={{ fontWeight: 600, fontSize: 13, minWidth: 26 }}
          >
            ?
          </button>
          {source && (
            <>
              <div className="tv-sep" />
              <button className="tv-btn" onClick={toggleColumnPanel}>
                <ColumnsIcon />
                Columns
              </button>
              <button className="tv-btn" onClick={() => setShowFilterBuilder(true)}>
                <FilterIcon />
                Filter
              </button>
              <div className="tv-sep" />
              <button className="tv-btn" onClick={() => setShowEDAPanel(true)}>
                <BarChartIcon />
                Explore
              </button>
              <button className="tv-btn" onClick={() => setShowExportPanel(true)}>
                <ExportIcon />
                Export
              </button>
              <button className="tv-btn" onClick={handleShare}>
                <ShareIcon />
                {shareCopied ? "Copied!" : "Share"}
              </button>
            </>
          )}
        </div>
      </header>

      <PipelineBar />

      <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
        <TableErrorBoundary>
          <TableViewer />
        </TableErrorBoundary>
      </div>

      <StatusBar />

      <Tooltip />
      <JumpToRow />
      <SourceManager />
      <CellContextMenu />
      <ColumnContextMenu />
      <ExportPanel />
      <EDAPanel />
      <ColumnPanel />
      <FilterBuilder />
      <CommandPalette />
      <LandmarkPanel />
      <BookmarkPanel />
      <KeyboardShortcutsPanel />
      <CatalogBrowser />
    </div>
  );
}
