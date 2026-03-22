import { useEffect, useRef, useState } from "react";
import { useUiStore } from "../../stores/ui";
import { useViewStore } from "../../stores/view";
import { useTableStore } from "../../stores/table";
import { fetchExportCode, buildDownloadUrl, type ExportFormat } from "../../lib/api";

type Dialect = "sql" | "duckdb" | "polars" | "pandas" | "shell" | "shell_csv" | "ansi_sql" | "dbt";

const DIALECT_MAP: Record<Dialect, ExportFormat> = {
  sql: "sql",
  duckdb: "python_duckdb",
  polars: "python_polars",
  pandas: "python_pandas",
  shell: "shell",
  shell_csv: "shell_csv",
  ansi_sql: "ansi_sql",
  dbt: "dbt",
};

const DIALECT_LABELS: Record<Dialect, string> = {
  sql: "SQL",
  duckdb: "DuckDB",
  polars: "Polars",
  pandas: "Pandas",
  shell: "Shell",
  shell_csv: "Shell CSV",
  ansi_sql: "ANSI SQL",
  dbt: "dbt",
};

const OVERLAY: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.4)",
  zIndex: 2000,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

export function ExportPanel() {
  const showExportPanel = useUiStore((s) => s.showExportPanel);
  const setShowExportPanel = useUiStore((s) => s.setShowExportPanel);
  const viewHash = useViewStore((s) => s.viewHash);
  const sourceId = useViewStore((s) => s.sourceId);
  const virtualRowCount = useViewStore((s) => s.virtualRowCount);
  const source = useTableStore((s) => s.source);

  const [dialect, setDialect] = useState<Dialect>("duckdb");
  const [code, setCode] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    if (!showExportPanel || !sourceId) return;
    const viewExpr = useViewStore.getState().buildViewExpr();
    if (!viewExpr) return;
    abortRef.current?.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    setLoading(true);
    setCode("");
    fetchExportCode(viewExpr, DIALECT_MAP[dialect])
      .then((c) => { if (!ctrl.signal.aborted) setCode(c); })
      .catch(() => { if (!ctrl.signal.aborted) setCode("// Error generating code"); })
      .finally(() => { if (!ctrl.signal.aborted) setLoading(false); });
    return () => ctrl.abort();
  }, [showExportPanel, viewHash, sourceId, dialect]);

  if (!showExportPanel) return null;

  const viewExpr = useViewStore.getState().buildViewExpr();

  const copy = async () => {
    await navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  const rowLabel = source
    ? virtualRowCount !== null
      ? `${source.n_rows.toLocaleString()} → ${virtualRowCount.toLocaleString()} rows`
      : `${source.n_rows.toLocaleString()} rows`
    : "";

  const codeBlockStyle: React.CSSProperties = {
    background: "var(--c-surface-2)",
    color: "var(--c-text)",
    borderRadius: 8,
    padding: "14px 16px",
    fontSize: 12,
    fontFamily: "ui-monospace, monospace",
    overflowX: "auto",
    whiteSpace: "pre",
    flex: 1,
    minHeight: 100,
    border: "1px solid var(--c-border)",
  };

  return (
    <div style={OVERLAY} onClick={() => setShowExportPanel(false)}>
      <div
        style={{
          background: "var(--c-bg)",
          borderRadius: 12,
          width: "90%",
          maxWidth: 760,
          maxHeight: "80vh",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          border: "1px solid var(--c-border)",
          boxShadow: "0 16px 48px rgba(0,0,0,0.4)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "16px 20px 12px",
            borderBottom: "1px solid var(--c-border)",
            flexShrink: 0,
          }}
        >
          <div>
            <div style={{ fontSize: 15, fontWeight: 600, color: "var(--c-text)" }}>Export / Use</div>
            {rowLabel && (
              <div style={{ fontSize: 12, color: "var(--c-text-2)", marginTop: 2 }}>{rowLabel}</div>
            )}
          </div>
          <button
            onClick={() => setShowExportPanel(false)}
            style={{
              background: "none",
              border: "none",
              fontSize: 18,
              cursor: "pointer",
              color: "var(--c-text-3)",
              lineHeight: 1,
              padding: "2px 6px",
            }}
          >
            ×
          </button>
        </div>

        <div style={{ flex: 1, overflowY: "auto", padding: "16px 20px" }}>
          <div style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 12, fontWeight: 600, color: "var(--c-text-2)", marginBottom: 8, textTransform: "uppercase", letterSpacing: "0.04em" }}>
              Use in code
            </div>
            <div style={{ display: "flex", gap: 4, marginBottom: 10, flexWrap: "wrap" }}>
              {(["duckdb", "polars", "pandas", "sql", "shell", "shell_csv", "ansi_sql", "dbt"] as Dialect[]).map((d) => (
                <button
                  key={d}
                  onClick={() => setDialect(d)}
                  style={{
                    padding: "5px 12px",
                    borderRadius: 6,
                    border: "1px solid",
                    fontSize: 12,
                    fontWeight: 500,
                    cursor: "pointer",
                    background: dialect === d ? "var(--c-accent)" : "var(--c-bg)",
                    color: dialect === d ? "#fff" : "var(--c-text-2)",
                    borderColor: dialect === d ? "var(--c-accent)" : "var(--c-border)",
                  }}
                >
                  {DIALECT_LABELS[d]}
                </button>
              ))}
            </div>
            <div style={{ position: "relative" }}>
              {loading ? (
                <div style={{ ...codeBlockStyle, color: "var(--c-text-2)" }}>Generating…</div>
              ) : (
                <div style={codeBlockStyle}>{code}</div>
              )}
              <button
                onClick={copy}
                style={{
                  position: "absolute",
                  top: 8,
                  right: 8,
                  padding: "4px 10px",
                  background: copied ? "var(--c-green)" : "var(--c-surface-2)",
                  color: "var(--c-text)",
                  border: "1px solid var(--c-border)",
                  borderRadius: 5,
                  fontSize: 11,
                  cursor: "pointer",
                  fontWeight: 500,
                }}
              >
                {copied ? "Copied!" : "Copy"}
              </button>
            </div>
          </div>

          {viewExpr && (
            <div>
              <div style={{ fontSize: 12, fontWeight: 600, color: "var(--c-text-2)", marginBottom: 8, textTransform: "uppercase", letterSpacing: "0.04em" }}>
                Download
              </div>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                {(
                  [
                    { format: "parquet", label: "Parquet", ext: "parquet" },
                    { format: "csv", label: "CSV", ext: "csv" },
                    { format: "arrow", label: "Arrow", ext: "arrow" },
                    { format: "jsonl", label: "JSONL", ext: "jsonl" },
                  ] as { format: "parquet" | "csv" | "arrow" | "jsonl"; label: string; ext: string }[]
                ).map(({ format, label, ext }) => (
                  <a
                    key={format}
                    href={buildDownloadUrl(viewExpr, format)}
                    download={`${source?.name ?? "data"}.${ext}`}
                    style={{
                      padding: "8px 16px",
                      background: "var(--c-surface)",
                      border: "1px solid var(--c-border)",
                      borderRadius: 6,
                      fontSize: 13,
                      color: "var(--c-text)",
                      textDecoration: "none",
                      fontWeight: 500,
                    }}
                  >
                    ↓ {label}
                  </a>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
