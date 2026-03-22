import React, { useState, useMemo, useEffect } from "react";
import { useViewStore } from "../../stores/view";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";
import { useStatsStore, getStatsKey } from "../../stores/stats";
import { DEFAULT_CELL_W } from "../../lib/viewport";
import type { CardinalityCategory, ColumnStats } from "../../lib/types";

type Tab = "overview" | "manage";
type SortKey = "name" | "null_rate" | "distinct" | "index";

function cardinalityColor(cat: CardinalityCategory): { bg: string; text: string } {
  switch (cat) {
    case "unique": return { bg: "var(--c-accent-bg)", text: "var(--c-accent)" };
    case "low_cardinality": return { bg: "var(--c-green-bg)", text: "var(--c-green)" };
    case "binary": return { bg: "var(--c-surface-2)", text: "var(--c-text-2)" };
    case "constant": return { bg: "var(--c-red-bg)", text: "var(--c-red)" };
    case "categorical": return { bg: "var(--c-purple-bg)", text: "var(--c-purple)" };
    case "high_cardinality": return { bg: "var(--c-orange-bg)", text: "var(--c-orange)" };
    default: return { bg: "var(--c-surface-2)", text: "var(--c-text-3)" };
  }
}

function NullBar({ rate }: { rate: number }) {
  if (rate <= 0) return null;
  const filled = 1 - rate;
  const filledColor = rate < 0.05 ? "#16a34a" : rate < 0.2 ? "#d97706" : "#dc2626";
  return (
    <div style={{ display: "flex", height: 5, width: 60, borderRadius: 3, overflow: "hidden", background: "var(--c-surface-2)", flexShrink: 0 }}>
      <div style={{ width: `${filled * 100}%`, background: filledColor, borderRadius: "3px 0 0 3px" }} />
      <div style={{ flex: 1, background: rate >= 1 ? "#dc2626" : "rgba(220,38,38,0.35)" }} />
    </div>
  );
}

function CardinalityBadge({ cat }: { cat: CardinalityCategory }) {
  const { bg, text } = cardinalityColor(cat);
  const label: Record<CardinalityCategory, string> = {
    unique: "unique",
    low_cardinality: "low",
    binary: "binary",
    constant: "constant",
    categorical: "categ.",
    high_cardinality: "high",
    unknown: "—",
  };
  return (
    <span style={{
      fontSize: 10,
      fontWeight: 600,
      padding: "1px 5px",
      borderRadius: 4,
      background: bg,
      color: text,
      flexShrink: 0,
      fontFamily: "inherit",
      letterSpacing: "-0.01em",
    }}>
      {label[cat]}
    </span>
  );
}

function formatCompact(n: number | null): string {
  if (n === null) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function formatValue(v: unknown): string {
  if (v === null || v === undefined) return "—";
  if (typeof v === "number") {
    if (Math.abs(v) >= 1e6) return v.toExponential(2);
    if (!Number.isInteger(v)) return v.toPrecision(4);
    return String(v);
  }
  return String(v).slice(0, 18);
}

function OverviewRow({
  col,
  info,
  quickStat,
  fullStats,
  onJump,
}: {
  col: number;
  info: { name: string; data_type: string };
  quickStat: { null_rate: number; min: unknown; max: unknown } | undefined;
  fullStats: ColumnStats | undefined;
  onJump: (col: number) => void;
}) {
  const nullRate = fullStats?.null_rate ?? quickStat?.null_rate ?? 0;
  const distinct = fullStats?.distinct_count ?? null;
  const cat = fullStats?.cardinality_category ?? "unknown";
  const minVal = fullStats?.min ?? quickStat?.min ?? null;
  const maxVal = fullStats?.max ?? quickStat?.max ?? null;

  return (
    <div
      onClick={() => onJump(col)}
      style={{
        display: "grid",
        gridTemplateColumns: "1fr auto",
        gap: 0,
        padding: "8px 14px",
        borderBottom: "1px solid var(--c-surface)",
        cursor: "pointer",
        transition: "background 80ms",
      }}
      onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = "var(--c-surface)"; }}
      onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = ""; }}
    >
      <div style={{ minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 3 }}>
          <span style={{
            fontSize: 10,
            color: "var(--c-text-3)",
            background: "var(--c-surface-2)",
            padding: "1px 4px",
            borderRadius: 3,
            fontFamily: "ui-monospace, monospace",
            flexShrink: 0,
          }}>
            {info.data_type}
          </span>
          <span style={{
            fontSize: 12.5,
            fontWeight: 500,
            color: "var(--c-text)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}>
            {info.name}
          </span>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
          <NullBar rate={nullRate} />
          <span style={{ fontSize: 10.5, color: "var(--c-text-3)", fontVariantNumeric: "tabular-nums" }}>
            {nullRate > 0 ? `${(nullRate * 100).toFixed(1)}% null` : "no nulls"}
          </span>
          {minVal !== null && maxVal !== null && (
            <span style={{ fontSize: 10.5, color: "var(--c-text-3)" }}>
              {formatValue(minVal)}–{formatValue(maxVal)}
            </span>
          )}
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 4, paddingLeft: 10 }}>
        {distinct !== null && (
          <span style={{ fontSize: 11, color: "var(--c-text-2)", fontVariantNumeric: "tabular-nums" }}>
            {formatCompact(distinct)}
          </span>
        )}
        {cat !== "unknown" && <CardinalityBadge cat={cat} />}
        {!fullStats && (
          <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>…</span>
        )}
      </div>
    </div>
  );
}

export function ColumnPanel() {
  const showColumnPanel = useUiStore((s) => s.showColumnPanel);
  const toggleColumnPanel = useUiStore((s) => s.toggleColumnPanel);
  const zoom = useUiStore((s) => s.zoom);
  const source = useTableStore((s) => s.source);
  const setViewport = useTableStore((s) => s.setViewport);
  const columnViews = useViewStore((s) => s.columnViews);
  const initColumnViews = useViewStore((s) => s.initColumnViews);
  const toggleColumnVisibility = useViewStore((s) => s.toggleColumnVisibility);
  const setColumnPinned = useViewStore((s) => s.setColumnPinned);
  const reorderColumn = useViewStore((s) => s.reorderColumn);
  const allStats = useStatsStore((s) => s.stats);
  const fetchStats = useStatsStore((s) => s.fetchStats);

  const [tab, setTab] = useState<Tab>("overview");
  const [search, setSearch] = useState("");
  const [sortKey, setSortKey] = useState<SortKey>("index");

  React.useEffect(() => {
    if (source && columnViews.length === 0) {
      initColumnViews(source.columns);
    }
  }, [source, columnViews.length, initColumnViews]);

  useEffect(() => {
    if (!showColumnPanel || !source || tab !== "overview") return;
    source.columns.forEach((_, i) => {
      const key = getStatsKey(source.id, i);
      if (!allStats.has(key)) {
        fetchStats(source.id, i);
      }
    });
  }, [showColumnPanel, source?.id, tab]);

  const columns = useMemo(() => {
    if (!source) return [];
    return source.columns.map((col, i) => {
      const key = getStatsKey(source.id, i);
      const quick = source.quick_stats?.[i];
      const full = allStats.get(key);
      return { col: i, info: col, quickStat: quick, fullStats: full };
    });
  }, [source, allStats]);

  const filtered = useMemo(() => {
    const q = search.toLowerCase();
    const result = q ? columns.filter((c) => c.info.name.toLowerCase().includes(q)) : [...columns];
    result.sort((a, b) => {
      switch (sortKey) {
        case "name": return a.info.name.localeCompare(b.info.name);
        case "null_rate": {
          const ar = a.fullStats?.null_rate ?? a.quickStat?.null_rate ?? 0;
          const br = b.fullStats?.null_rate ?? b.quickStat?.null_rate ?? 0;
          return br - ar;
        }
        case "distinct": {
          const ad = a.fullStats?.distinct_count ?? -1;
          const bd = b.fullStats?.distinct_count ?? -1;
          return bd - ad;
        }
        default: return a.col - b.col;
      }
    });
    return result;
  }, [columns, search, sortKey]);

  const handleJump = (colIndex: number) => {
    const cellW = DEFAULT_CELL_W * zoom;
    setViewport({ scrollX: colIndex * cellW });
    toggleColumnPanel();
  };

  if (!showColumnPanel) return null;

  const sorted = [...columnViews].sort((a, b) => a.displayIndex - b.displayIndex);
  const allVisible = sorted.every((cv) => cv.visible);
  const allHidden = sorted.every((cv) => !cv.visible);

  const hideAll = () => sorted.forEach((cv) => { if (cv.visible) toggleColumnVisibility(cv.name); });
  const showAll = () => sorted.forEach((cv) => { if (!cv.visible) toggleColumnVisibility(cv.name); });

  return (
    <>
      <div style={{ position: "fixed", inset: 0, zIndex: 1200 }} onClick={toggleColumnPanel} />
      <div
        style={{
          position: "fixed",
          top: 0,
          left: 0,
          bottom: 0,
          width: 320,
          background: "var(--c-bg)",
          borderRight: "1px solid var(--c-border)",
          zIndex: 1201,
          display: "flex",
          flexDirection: "column",
          boxShadow: "4px 0 20px rgba(0,0,0,0.15)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "14px 16px 0",
          flexShrink: 0,
        }}>
          <span style={{ fontSize: 14, fontWeight: 600, color: "var(--c-text)" }}>
            Columns
            {source && (
              <span style={{ fontSize: 12, fontWeight: 400, color: "var(--c-text-3)", marginLeft: 6 }}>
                {source.n_cols}
              </span>
            )}
          </span>
          <button
            onClick={toggleColumnPanel}
            style={{ background: "none", border: "none", fontSize: 18, cursor: "pointer", color: "var(--c-text-3)", lineHeight: 1, padding: "2px 4px" }}
          >
            ×
          </button>
        </div>

        <div style={{ display: "flex", padding: "10px 16px 0", gap: 0, flexShrink: 0 }}>
          {(["overview", "manage"] as Tab[]).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              style={{
                padding: "6px 12px",
                border: "none",
                background: "none",
                cursor: "pointer",
                fontSize: 12.5,
                fontWeight: 500,
                color: tab === t ? "var(--c-accent)" : "var(--c-text-3)",
                borderBottom: tab === t ? "2px solid var(--c-accent)" : "2px solid transparent",
                marginBottom: -1,
                fontFamily: "inherit",
              }}
            >
              {t.charAt(0).toUpperCase() + t.slice(1)}
            </button>
          ))}
        </div>

        <div style={{ height: 1, background: "var(--c-border)", flexShrink: 0 }} />

        {tab === "overview" && (
          <>
            <div style={{ padding: "10px 14px", display: "flex", gap: 8, flexShrink: 0, borderBottom: "1px solid var(--c-border)" }}>
              <input
                type="text"
                placeholder="Search columns…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                style={{
                  flex: 1,
                  height: 27,
                  padding: "0 8px",
                  fontSize: 12.5,
                  border: "1px solid var(--c-border)",
                  borderRadius: 5,
                  background: "var(--c-surface)",
                  color: "var(--c-text)",
                  outline: "none",
                  fontFamily: "inherit",
                }}
              />
              <select
                value={sortKey}
                onChange={(e) => setSortKey(e.target.value as SortKey)}
                style={{
                  height: 27,
                  padding: "0 4px",
                  fontSize: 11.5,
                  border: "1px solid var(--c-border)",
                  borderRadius: 5,
                  background: "var(--c-surface)",
                  color: "var(--c-text-2)",
                  outline: "none",
                  cursor: "pointer",
                  fontFamily: "inherit",
                }}
              >
                <option value="index">Index</option>
                <option value="name">Name</option>
                <option value="null_rate">Null %</option>
                <option value="distinct">Distinct</option>
              </select>
            </div>
            <div style={{ flex: 1, overflowY: "auto" }}>
              {filtered.map(({ col, info, quickStat, fullStats }) => (
                <OverviewRow
                  key={col}
                  col={col}
                  info={info}
                  quickStat={quickStat}
                  fullStats={fullStats}
                  onJump={handleJump}
                />
              ))}
              {filtered.length === 0 && (
                <div style={{ padding: "24px 16px", fontSize: 13, color: "var(--c-text-3)", textAlign: "center" }}>
                  No columns match.
                </div>
              )}
            </div>
          </>
        )}

        {tab === "manage" && (
          <>
            <div style={{ display: "flex", gap: 6, padding: "10px 14px", borderBottom: "1px solid var(--c-border)", flexShrink: 0 }}>
              <button
                onClick={showAll}
                disabled={allVisible}
                style={{
                  padding: "4px 10px",
                  fontSize: 11.5,
                  border: "1px solid var(--c-border)",
                  borderRadius: 4,
                  background: "var(--c-surface)",
                  cursor: allVisible ? "default" : "pointer",
                  color: allVisible ? "var(--c-text-3)" : "var(--c-text-2)",
                  fontWeight: 500,
                  fontFamily: "inherit",
                }}
              >
                Show all
              </button>
              <button
                onClick={hideAll}
                disabled={allHidden}
                style={{
                  padding: "4px 10px",
                  fontSize: 11.5,
                  border: "1px solid var(--c-border)",
                  borderRadius: 4,
                  background: "var(--c-surface)",
                  cursor: allHidden ? "default" : "pointer",
                  color: allHidden ? "var(--c-text-3)" : "var(--c-text-2)",
                  fontWeight: 500,
                  fontFamily: "inherit",
                }}
              >
                Hide all
              </button>
            </div>
            <div style={{ flex: 1, overflowY: "auto" }}>
              {sorted.map((cv, i) => {
                const sourceCol = source?.columns.find((c) => c.name === cv.name);
                return (
                  <div
                    key={cv.name}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      padding: "7px 12px",
                      borderBottom: "1px solid var(--c-surface)",
                      background: cv.visible ? "var(--c-bg)" : "var(--c-surface)",
                    }}
                  >
                    <input
                      type="checkbox"
                      checked={cv.visible}
                      onChange={() => toggleColumnVisibility(cv.name)}
                      style={{ flexShrink: 0, cursor: "pointer" }}
                    />
                    <span style={{
                      fontSize: 10,
                      color: "var(--c-text-3)",
                      background: "var(--c-surface-2)",
                      borderRadius: 3,
                      padding: "1px 4px",
                      flexShrink: 0,
                      fontFamily: "ui-monospace, monospace",
                    }}>
                      {sourceCol?.data_type ?? ""}
                    </span>
                    <span style={{
                      flex: 1,
                      fontSize: 12.5,
                      color: cv.visible ? "var(--c-text)" : "var(--c-text-3)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}>
                      {cv.name}
                    </span>
                    <div style={{ display: "flex", gap: 2, flexShrink: 0 }}>
                      <button
                        onClick={() => setColumnPinned(cv.name, cv.pinned === "left" ? null : "left")}
                        title="Pin left"
                        style={{
                          padding: "2px 5px",
                          fontSize: 10,
                          border: "1px solid var(--c-border)",
                          borderRadius: 3,
                          background: cv.pinned === "left" ? "var(--c-accent-bg)" : "var(--c-surface)",
                          color: cv.pinned === "left" ? "var(--c-accent)" : "var(--c-text-3)",
                          cursor: "pointer",
                          fontFamily: "inherit",
                        }}
                      >
                        L
                      </button>
                      <button
                        onClick={() => setColumnPinned(cv.name, cv.pinned === "right" ? null : "right")}
                        title="Pin right"
                        style={{
                          padding: "2px 5px",
                          fontSize: 10,
                          border: "1px solid var(--c-border)",
                          borderRadius: 3,
                          background: cv.pinned === "right" ? "var(--c-accent-bg)" : "var(--c-surface)",
                          color: cv.pinned === "right" ? "var(--c-accent)" : "var(--c-text-3)",
                          cursor: "pointer",
                          fontFamily: "inherit",
                        }}
                      >
                        R
                      </button>
                      <button
                        onClick={() => i > 0 && reorderColumn(i, i - 1)}
                        disabled={i === 0}
                        title="Move up"
                        style={{
                          padding: "2px 5px",
                          fontSize: 10,
                          border: "1px solid var(--c-border)",
                          borderRadius: 3,
                          background: "var(--c-surface)",
                          color: i === 0 ? "var(--c-border)" : "var(--c-text-3)",
                          cursor: i === 0 ? "default" : "pointer",
                          fontFamily: "inherit",
                        }}
                      >
                        ↑
                      </button>
                      <button
                        onClick={() => i < sorted.length - 1 && reorderColumn(i, i + 1)}
                        disabled={i === sorted.length - 1}
                        title="Move down"
                        style={{
                          padding: "2px 5px",
                          fontSize: 10,
                          border: "1px solid var(--c-border)",
                          borderRadius: 3,
                          background: "var(--c-surface)",
                          color: i === sorted.length - 1 ? "var(--c-border)" : "var(--c-text-3)",
                          cursor: i === sorted.length - 1 ? "default" : "pointer",
                          fontFamily: "inherit",
                        }}
                      >
                        ↓
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}
      </div>
    </>
  );
}
