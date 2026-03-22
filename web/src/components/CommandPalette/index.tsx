import { useState, useEffect, useRef } from "react";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useTableStore } from "../../stores/table";
import { fuzzyMatch } from "../../lib/fuzzyMatch";
import type { Command } from "../../lib/commands";

export function CommandPalette() {
  const show = useUiStore((s) => s.showCommandPalette);
  const setShow = useUiStore((s) => s.setShowCommandPalette);
  const source = useTableStore((s) => s.source);
  const setSort = useViewStore((s) => s.setSort);
  const clearOps = useViewStore((s) => s.clearOps);
  const setShowExportPanel = useUiStore((s) => s.setShowExportPanel);
  const toggleColumnPanel = useUiStore((s) => s.toggleColumnPanel);
  const setShowFilterBuilder = useUiStore((s) => s.setShowFilterBuilder);
  const toggleSourceManager = useUiStore((s) => s.toggleSourceManager);

  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const allCommands: Command[] = [
    {
      id: "open_source",
      label: "Open data source",
      category: "source",
      shortcut: "Ctrl+O",
      action: () => { toggleSourceManager(); setShow(false); },
    },
    {
      id: "export_panel",
      label: "Export / Use code",
      category: "export",
      shortcut: "Ctrl+E",
      action: () => { setShowExportPanel(true); setShow(false); },
    },
    {
      id: "toggle_columns",
      label: "Toggle column panel",
      category: "view",
      shortcut: "Ctrl+Shift+C",
      action: () => { toggleColumnPanel(); setShow(false); },
    },
    {
      id: "filter_builder",
      label: "Open filter builder",
      category: "filter",
      shortcut: "Ctrl+Shift+F",
      action: () => { setShowFilterBuilder(true); setShow(false); },
    },
    {
      id: "clear_ops",
      label: "Clear all filters and sorts",
      category: "filter",
      action: () => { clearOps(); setShow(false); },
    },
    ...(source?.columns ?? []).map((col) => ({
      id: `sort_asc_${col.name}`,
      label: `Sort by ${col.name} ascending`,
      category: "filter" as const,
      action: () => { setSort([{ column: col.name, descending: false, nulls_last: true }]); setShow(false); },
    })),
    ...(source?.columns ?? []).map((col) => ({
      id: `sort_desc_${col.name}`,
      label: `Sort by ${col.name} descending`,
      category: "filter" as const,
      action: () => { setSort([{ column: col.name, descending: true, nulls_last: true }]); setShow(false); },
    })),
  ];

  const filtered = query
    ? allCommands
        .filter((cmd) => fuzzyMatch(query, cmd.label).matched)
        .sort((a, b) => fuzzyMatch(query, b.label).score - fuzzyMatch(query, a.label).score)
    : allCommands;

  useEffect(() => {
    if (show) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [show]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  if (!show) return null;

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") { setShow(false); return; }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
      return;
    }
    if (e.key === "Enter" && filtered[selectedIndex]) {
      filtered[selectedIndex].action();
      return;
    }
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        zIndex: 3000,
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        paddingTop: 80,
      }}
      onClick={() => setShow(false)}
    >
      <div
        style={{
          background: "var(--c-bg)",
          borderRadius: 8,
          width: 560,
          maxHeight: 400,
          overflow: "hidden",
          boxShadow: "0 16px 48px rgba(0,0,0,0.4)",
          display: "flex",
          flexDirection: "column",
          border: "1px solid var(--c-border)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ borderBottom: "1px solid var(--c-border)", padding: "8px 12px" }}>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a command…"
            style={{ width: "100%", border: "none", outline: "none", fontSize: 14, color: "var(--c-text)", background: "transparent" }}
          />
        </div>
        <div style={{ overflowY: "auto", maxHeight: 352 }}>
          {filtered.length === 0 && (
            <div style={{ padding: "12px 16px", fontSize: 13, color: "var(--c-text-3)" }}>No commands found</div>
          )}
          {filtered.map((cmd, i) => (
            <div
              key={cmd.id}
              onClick={() => cmd.action()}
              style={{
                padding: "9px 16px",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                background: i === selectedIndex ? "var(--c-surface-2)" : "transparent",
                fontSize: 13,
                color: "var(--c-text)",
              }}
            >
              <span>{cmd.label}</span>
              <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                {cmd.shortcut && (
                  <span
                    style={{
                      fontSize: 11,
                      color: "var(--c-text-3)",
                      background: "var(--c-surface)",
                      border: "1px solid var(--c-border)",
                      borderRadius: 4,
                      padding: "1px 5px",
                    }}
                  >
                    {cmd.shortcut}
                  </span>
                )}
                <span
                  style={{
                    fontSize: 11,
                    color: "var(--c-text-3)",
                    textTransform: "uppercase",
                    letterSpacing: "0.04em",
                  }}
                >
                  {cmd.category}
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
