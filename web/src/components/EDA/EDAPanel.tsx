import { useEffect, useRef, useMemo } from "react";
import vegaEmbed from "vega-embed";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";
import { useStatsStore, getStatsKey } from "../../stores/stats";
import { fetchCorrelations } from "../../lib/api";
import type { ColumnStats, CorrelationMatrix } from "../../lib/types";
import { useState } from "react";

type Tab = "distributions" | "correlations" | "nullity";

const OVERLAY: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(15,23,42,0.4)",
  zIndex: 2000,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const PANEL: React.CSSProperties = {
  background: "var(--c-bg)",
  borderRadius: 12,
  width: "90%",
  maxWidth: 960,
  maxHeight: "88vh",
  display: "flex",
  flexDirection: "column",
  overflow: "hidden",
  border: "1px solid var(--c-border)",
  boxShadow: "0 16px 48px rgba(0,0,0,0.4)",
};

function HistogramChart({ stats }: { stats: ColumnStats }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!ref.current || !stats.histogram) return;
    const spec = {
      $schema: "https://vega.github.io/schema/vega-lite/v5.json",
      width: 200,
      height: 90,
      data: { values: stats.histogram },
      mark: { type: "bar", tooltip: true },
      encoding: {
        x: { field: "lo", type: "quantitative", title: null, axis: { tickCount: 4, labelFontSize: 10 } },
        x2: { field: "hi" },
        y: { field: "count", type: "quantitative", title: null, axis: { tickCount: 3, labelFontSize: 10 } },
        tooltip: [
          { field: "lo", type: "quantitative", title: "From", format: ".3g" },
          { field: "hi", type: "quantitative", title: "To", format: ".3g" },
          { field: "count", type: "quantitative", title: "Count" },
        ],
        color: { value: "#3b82f6" },
      },
      config: { view: { stroke: null }, background: "transparent" },
    };
    let result: { finalize: () => void } | null = null;
    vegaEmbed(ref.current, spec as never, { actions: false, renderer: "svg" })
      .then((r) => { result = r; })
      .catch(() => {});
    return () => { result?.finalize(); };
  }, [stats]);

  return <div ref={ref} />;
}

function TopValuesChart({ stats }: { stats: ColumnStats }) {
  const top = stats.top_values ?? [];
  if (top.length === 0) return null;
  const maxCount = Math.max(...top.map((v) => v.count));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 3, marginTop: 4 }}>
      {top.slice(0, 8).map((v, i) => {
        const label = v.value === null || v.value === undefined ? "(null)" : String(v.value).slice(0, 20);
        const barW = maxCount > 0 ? (v.count / maxCount) * 100 : 0;
        return (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <div style={{ width: 80, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontSize: 10.5, color: "var(--c-text-2)", flexShrink: 0 }}>
              {label}
            </div>
            <div style={{ flex: 1, height: 8, background: "var(--c-surface-2)", borderRadius: 4, overflow: "hidden" }}>
              <div style={{ width: `${barW}%`, height: "100%", background: "#3b82f6", borderRadius: 4 }} />
            </div>
            <span style={{ fontSize: 10, color: "var(--c-text-3)", width: 40, textAlign: "right", flexShrink: 0, fontVariantNumeric: "tabular-nums" }}>
              {(v.rate * 100).toFixed(1)}%
            </span>
          </div>
        );
      })}
    </div>
  );
}

function CorrelationHeatmap({ matrix }: { matrix: CorrelationMatrix }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!ref.current) return;
    const values: { x: string; y: string; value: number | null }[] = [];
    for (let i = 0; i < matrix.columns.length; i++) {
      for (let j = 0; j < matrix.columns.length; j++) {
        values.push({
          x: matrix.columns[j] ?? "",
          y: matrix.columns[i] ?? "",
          value: matrix.matrix[i]?.[j] ?? null,
        });
      }
    }
    const cellSize = Math.max(20, Math.min(40, Math.floor(520 / matrix.columns.length)));
    const size = cellSize * matrix.columns.length;
    const spec = {
      $schema: "https://vega.github.io/schema/vega-lite/v5.json",
      width: size,
      height: size,
      data: { values },
      mark: { type: "rect", tooltip: true },
      encoding: {
        x: { field: "x", type: "ordinal", title: null, axis: { labelAngle: -45, labelLimit: 100, labelFontSize: 11 } },
        y: { field: "y", type: "ordinal", title: null, axis: { labelLimit: 120, labelFontSize: 11 } },
        color: {
          field: "value",
          type: "quantitative",
          scale: { scheme: "redblue", domain: [-1, 1] },
          legend: { title: "r", orient: "right" },
        },
        tooltip: [
          { field: "x", type: "nominal", title: "Column X" },
          { field: "y", type: "nominal", title: "Column Y" },
          { field: "value", type: "quantitative", title: "Pearson r", format: ".3f" },
        ],
      },
      config: { view: { stroke: null }, background: "transparent" },
    };
    let result: { finalize: () => void } | null = null;
    vegaEmbed(ref.current, spec as never, { actions: false, renderer: "svg" })
      .then((r) => { result = r; })
      .catch(() => {});
    return () => { result?.finalize(); };
  }, [matrix]);

  return <div ref={ref} />;
}

function NullityChart({ profile }: { profile: ColumnStats[] }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!ref.current) return;
    const values = profile
      .filter((s) => s.null_rate > 0)
      .sort((a, b) => b.null_rate - a.null_rate)
      .map((s) => ({ column: s.column, null_pct: s.null_rate * 100 }));

    if (values.length === 0) return;

    const height = Math.max(160, values.length * 22);
    const spec = {
      $schema: "https://vega.github.io/schema/vega-lite/v5.json",
      width: 540,
      height,
      data: { values },
      mark: { type: "bar", tooltip: true },
      encoding: {
        y: { field: "column", type: "ordinal", title: null, sort: "-x", axis: { labelLimit: 160, labelFontSize: 11 } },
        x: { field: "null_pct", type: "quantitative", title: "Null %", scale: { domain: [0, 100] }, axis: { labelFontSize: 11 } },
        color: {
          field: "null_pct",
          type: "quantitative",
          scale: { scheme: "reds", domain: [0, 100] },
          legend: null,
        },
        tooltip: [
          { field: "column", type: "nominal", title: "Column" },
          { field: "null_pct", type: "quantitative", title: "Null %", format: ".2f" },
        ],
      },
      config: { view: { stroke: null }, background: "transparent" },
    };
    let result: { finalize: () => void } | null = null;
    vegaEmbed(ref.current, spec as never, { actions: false, renderer: "svg" })
      .then((r) => { result = r; })
      .catch(() => {});
    return () => { result?.finalize(); };
  }, [profile]);

  return <div ref={ref} />;
}

function StatCard({ stats }: { stats: ColumnStats }) {
  const hasHistogram = stats.histogram && stats.histogram.length > 0;
  const hasTopValues = stats.top_values && stats.top_values.length > 0;
  const showTopValues = !hasHistogram && hasTopValues;

  return (
    <div style={{
      border: "1px solid var(--c-border)",
      borderRadius: 8,
      padding: "12px 14px",
      minWidth: 240,
      maxWidth: 280,
    }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: "var(--c-text)", marginBottom: 2, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {stats.column}
      </div>
      <div style={{ fontSize: 11, color: "var(--c-text-3)", marginBottom: 8, display: "flex", gap: 6, flexWrap: "wrap" }}>
        <span style={{ fontFamily: "ui-monospace, monospace", background: "var(--c-surface-2)", padding: "0 3px", borderRadius: 3 }}>{stats.data_type}</span>
        {stats.null_rate > 0 && <span>{(stats.null_rate * 100).toFixed(1)}% null</span>}
        {stats.mean !== null && <span>mean {stats.mean.toPrecision(4)}</span>}
        {stats.distinct_count !== null && (
          <span>
            {stats.distinct_count >= 1_000_000
              ? `~${(stats.distinct_count / 1_000_000).toFixed(1)}M distinct`
              : stats.distinct_count >= 1_000
              ? `~${(stats.distinct_count / 1_000).toFixed(1)}k distinct`
              : `${stats.distinct_count} distinct`}
          </span>
        )}
      </div>
      {hasHistogram && <HistogramChart stats={stats} />}
      {showTopValues && <TopValuesChart stats={stats} />}
    </div>
  );
}

export function EDAPanel() {
  const showEDAPanel = useUiStore((s) => s.showEDAPanel);
  const setShowEDAPanel = useUiStore((s) => s.setShowEDAPanel);
  const source = useTableStore((s) => s.source);
  const allStats = useStatsStore((s) => s.stats);
  const fetchStats = useStatsStore((s) => s.fetchStats);
  const pending = useStatsStore((s) => s.pending);

  const [tab, setTab] = useState<Tab>("distributions");
  const [correlations, setCorrelations] = useState<CorrelationMatrix | null>(null);
  const [corrLoading, setCorrLoading] = useState(false);
  const [corrError, setCorrError] = useState<string | null>(null);

  useEffect(() => {
    if (!showEDAPanel || !source) return;
    source.columns.forEach((_, i) => {
      fetchStats(source.id, i);
    });
  }, [showEDAPanel, source?.id]);

  useEffect(() => {
    if (!showEDAPanel || !source || tab !== "correlations" || correlations) return;
    setCorrLoading(true);
    setCorrError(null);
    fetchCorrelations(source.id)
      .then(setCorrelations)
      .catch((e) => setCorrError(String(e)))
      .finally(() => setCorrLoading(false));
  }, [showEDAPanel, source?.id, tab]);

  const profile = useMemo(() => {
    if (!source) return [];
    return source.columns
      .map((_, i) => allStats.get(getStatsKey(source.id, i)))
      .filter((s): s is ColumnStats => s !== undefined);
  }, [source, allStats]);

  const isLoading = source ? pending.size > 0 && profile.length < source.n_cols : false;

  if (!showEDAPanel) return null;

  const displayable = profile.filter((s) => (s.histogram && s.histogram.length > 0) || (s.top_values && s.top_values.length > 0));
  const columnsWithNulls = profile.filter((s) => s.null_rate > 0);

  return (
    <div style={OVERLAY} onClick={() => setShowEDAPanel(false)}>
      <div style={PANEL} onClick={(e) => e.stopPropagation()}>
        <div style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "16px 20px 12px",
          borderBottom: "1px solid var(--c-border)",
          flexShrink: 0,
        }}>
          <div>
            <div style={{ fontSize: 15, fontWeight: 600, color: "var(--c-text)" }}>Explore</div>
            {source && (
              <div style={{ fontSize: 12, color: "var(--c-text-2)", marginTop: 2 }}>
                {source.n_rows.toLocaleString()} rows · {source.n_cols.toLocaleString()} columns · {source.name}
                {isLoading && <span style={{ marginLeft: 8, color: "var(--c-text-3)" }}>loading stats…</span>}
              </div>
            )}
          </div>
          <button
            onClick={() => setShowEDAPanel(false)}
            style={{ background: "none", border: "none", fontSize: 18, cursor: "pointer", color: "var(--c-text-3)", lineHeight: 1, padding: "2px 6px" }}
          >
            ×
          </button>
        </div>

        <div style={{ display: "flex", gap: 0, padding: "0 20px", flexShrink: 0, borderBottom: "1px solid var(--c-border)" }}>
          {(["distributions", "correlations", "nullity"] as Tab[]).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              style={{
                padding: "10px 16px",
                border: "none",
                background: "none",
                cursor: "pointer",
                fontSize: 13,
                fontWeight: 500,
                color: tab === t ? "var(--c-accent)" : "var(--c-text-2)",
                borderBottom: tab === t ? "2px solid var(--c-accent)" : "2px solid transparent",
                marginBottom: -1,
                fontFamily: "inherit",
              }}
            >
              {t.charAt(0).toUpperCase() + t.slice(1)}
            </button>
          ))}
        </div>

        <div style={{ flex: 1, overflowY: "auto", overflowX: "auto", padding: "20px" }}>
          {tab === "distributions" && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 14 }}>
              {isLoading && displayable.length === 0 && (
                <div style={{ color: "var(--c-text-2)", fontSize: 13 }}>Computing distributions…</div>
              )}
              {!isLoading && displayable.length === 0 && (
                <div style={{ color: "var(--c-text-2)", fontSize: 13 }}>No distribution data available.</div>
              )}
              {displayable.map((s) => <StatCard key={s.column} stats={s} />)}
            </div>
          )}

          {tab === "correlations" && (
            <div>
              {corrLoading && <div style={{ color: "var(--c-text-2)", fontSize: 13 }}>Computing correlations…</div>}
              {corrError && <div style={{ color: "#ef4444", fontSize: 13 }}>{corrError}</div>}
              {!corrLoading && !corrError && (!correlations || correlations.columns.length < 2) && (
                <div style={{ color: "var(--c-text-2)", fontSize: 13 }}>
                  Not enough numeric columns for correlation analysis (need ≥ 2).
                </div>
              )}
              {!corrLoading && !corrError && correlations && correlations.columns.length >= 2 && (
                <CorrelationHeatmap matrix={correlations} />
              )}
            </div>
          )}

          {tab === "nullity" && (
            <div>
              {isLoading && profile.length === 0 && (
                <div style={{ color: "var(--c-text-2)", fontSize: 13 }}>Computing null rates…</div>
              )}
              {!isLoading && columnsWithNulls.length === 0 && profile.length > 0 && (
                <div style={{ color: "var(--c-text-2)", fontSize: 13 }}>No columns with null values.</div>
              )}
              {profile.length > 0 && <NullityChart profile={profile} />}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
