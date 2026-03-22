import { useEffect, useState } from "react";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";
import { useViewStore } from "../../stores/view";
import { fetchSources, deleteSource } from "../../lib/api";
import { AddSource } from "./AddSource";
import type { SourceMeta } from "../../lib/types";
import { formatRowCount } from "../../lib/format";

export function SourceManager() {
  const show = useUiStore((s) => s.showSourceManager);
  const toggleSourceManager = useUiStore((s) => s.toggleSourceManager);
  const setSource = useTableStore((s) => s.setSource);
  const activeSource = useTableStore((s) => s.source);
  const setSourceId = useViewStore((s) => s.setSourceId);

  const [sources, setSources] = useState<SourceMeta[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!show) return;
    setLoading(true);
    fetchSources()
      .then(setSources)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [show]);

  if (!show) return null;

  const handleSelect = (source: SourceMeta) => {
    setSource(source);
    setSourceId(source.id);
    toggleSourceManager();
  };

  const handleAdded = (source: SourceMeta) => {
    setSources((prev) => [...prev, source]);
    setSource(source);
    setSourceId(source.id);
    setShowAdd(false);
  };

  const handleDelete = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    await deleteSource(id).catch(console.error);
    setSources((prev) => prev.filter((s) => s.id !== id));
    if (activeSource?.id === id) {
      setSource(null);
      setSourceId(null);
    }
  };

  return (
    <>
      <div
        style={{
          position: "fixed",
          inset: 0,
          background: "rgba(0,0,0,0.3)",
          zIndex: 800,
        }}
        onClick={toggleSourceManager}
      />
      <div
        style={{
          position: "fixed",
          top: 0,
          left: 0,
          bottom: 0,
          width: 320,
          background: "var(--c-bg)",
          borderRight: "1px solid var(--c-border)",
          boxShadow: "4px 0 16px rgba(0,0,0,0.2)",
          zIndex: 801,
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div style={{ padding: "16px", borderBottom: "1px solid var(--c-border)", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h2 style={{ margin: 0, fontSize: 15, color: "var(--c-text)" }}>Data Sources</h2>
          <button onClick={() => setShowAdd(true)} style={{ padding: "5px 12px", fontSize: 12, background: "var(--c-accent)", color: "#fff", border: "none", borderRadius: 4, cursor: "pointer" }}>
            + Add
          </button>
        </div>

        <div style={{ flex: 1, overflowY: "auto" }}>
          {loading && <p style={{ padding: 16, color: "var(--c-text-2)", fontSize: 13 }}>Loading…</p>}
          {!loading && sources.length === 0 && (
            <p style={{ padding: 16, color: "var(--c-text-3)", fontSize: 13 }}>No sources yet. Add a Parquet, CSV, or Arrow file.</p>
          )}
          {sources.map((source) => (
            <div
              key={source.id}
              onClick={() => handleSelect(source)}
              style={{
                padding: "12px 16px",
                borderBottom: "1px solid var(--c-surface-2)",
                cursor: "pointer",
                background: activeSource?.id === source.id ? "var(--c-accent-bg)" : "var(--c-bg)",
                display: "flex",
                justifyContent: "space-between",
                alignItems: "flex-start",
              }}
            >
              <div>
                <div style={{ fontSize: 13, fontWeight: 500, color: "var(--c-text)" }}>{source.name}</div>
                <div style={{ fontSize: 11, color: "var(--c-text-2)", marginTop: 2 }}>
                  {formatRowCount(source.n_rows)} rows · {source.n_cols} cols · {source.format}
                </div>
                <div style={{ fontSize: 10, color: "var(--c-text-3)", marginTop: 1, wordBreak: "break-all" }}>{source.uri}</div>
              </div>
              <button
                onClick={(e) => handleDelete(source.id, e)}
                style={{ fontSize: 11, color: "var(--c-text-3)", background: "none", border: "none", cursor: "pointer", padding: "2px 4px" }}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      </div>

      {showAdd && <AddSource onAdded={handleAdded} onCancel={() => setShowAdd(false)} />}
    </>
  );
}
