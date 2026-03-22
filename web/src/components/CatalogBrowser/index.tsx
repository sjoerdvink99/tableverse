import React, { useCallback, useState } from "react";
import { registerSource } from "../../lib/api";
import type { SourceMeta } from "../../lib/types";
import { useUiStore } from "../../stores/ui";
import { useTableStore } from "../../stores/table";

type CatalogType =
  | "iceberg_rest"
  | "delta"
  | "huggingface"
  | "glue"
  | "clickhouse"
  | "s3";

type CatalogEntry = {
  name: string;
  namespace: string;
  uri: string;
  n_rows?: number;
  columns?: string[];
};

type BrowseResult = {
  entries: CatalogEntry[];
  error?: string;
};

const CATALOG_LABELS: Record<CatalogType, string> = {
  iceberg_rest: "Apache Iceberg (REST Catalog)",
  delta: "Delta Lake",
  huggingface: "HuggingFace Datasets",
  glue: "AWS Glue Catalog",
  clickhouse: "ClickHouse",
  s3: "Amazon S3",
};

const CATALOG_PLACEHOLDERS: Record<CatalogType, Record<string, string>> = {
  iceberg_rest: {
    endpoint: "https://catalog.example.com",
    warehouse: "my_warehouse",
    namespace: "analytics",
    token: "Bearer token (optional)",
  },
  delta: {
    path: "/data/delta-tables/my_table  or  s3://bucket/path",
  },
  huggingface: {
    dataset: "owner/dataset-name",
    split: "train",
    token: "HuggingFace token (optional, for private datasets)",
  },
  glue: {
    database: "my_database",
    table: "my_table",
    region: "us-east-1",
  },
  clickhouse: {
    host: "localhost",
    database: "default",
    query: "SELECT * FROM my_table",
  },
  s3: {
    bucket: "my-bucket",
    prefix: "data/parquet/",
    region: "us-east-1",
  },
};

export function CatalogBrowser() {
  const showCatalogBrowser = useUiStore((s) => s.showCatalogBrowser);
  const setShowCatalogBrowser = useUiStore((s) => s.setShowCatalogBrowser);
  const setSource = useTableStore((s) => s.setSource);

  const [catalogType, setCatalogType] = useState<CatalogType>("huggingface");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [entries, setEntries] = useState<CatalogEntry[]>([]);
  const [openingId, setOpeningId] = useState<string | null>(null);

  const setField = useCallback((key: string, value: string) => {
    setFields((prev) => ({ ...prev, [key]: value }));
  }, []);

  const handleBrowse = useCallback(async () => {
    setLoading(true);
    setError(null);
    setEntries([]);

    try {
      const result = await browseCatalog(catalogType, fields);
      if (result.error) {
        setError(result.error);
      } else {
        setEntries(result.entries);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [catalogType, fields]);

  const handleOpen = useCallback(
    async (entry: CatalogEntry) => {
      setOpeningId(entry.uri);
      try {
        const credentials = buildCredentials(catalogType, fields);
        const meta: SourceMeta = await registerSource(
          entry.uri,
          entry.name,
          undefined,
          credentials
        );
        setSource(meta);
        setShowCatalogBrowser(false);
      } catch (e) {
        setError(`Failed to open ${entry.name}: ${String(e)}`);
      } finally {
        setOpeningId(null);
      }
    },
    [catalogType, fields, setSource, setShowCatalogBrowser]
  );

  const handleDirectOpen = useCallback(async () => {
    const uri = buildDirectUri(catalogType, fields);
    if (!uri) {
      setError("Please fill in required fields");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const credentials = buildCredentials(catalogType, fields);
      const meta: SourceMeta = await registerSource(uri, undefined, undefined, credentials);
      setSource(meta);
      setShowCatalogBrowser(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [catalogType, fields, setSource, setShowCatalogBrowser]);

  if (!showCatalogBrowser) return null;

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
      }}
      onClick={(e) => e.target === e.currentTarget && setShowCatalogBrowser(false)}
    >
      <div
        style={{
          background: "var(--c-bg)",
          border: "1px solid var(--c-border)",
          borderRadius: 10,
          width: 600,
          maxHeight: "80vh",
          display: "flex",
          flexDirection: "column",
          boxShadow: "0 20px 60px rgba(0,0,0,0.4)",
        }}
      >
        <div
          style={{
            padding: "16px 20px",
            borderBottom: "1px solid var(--c-border)",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <span style={{ fontWeight: 600, fontSize: 14 }}>Connect to Data Source</span>
          <button
            onClick={() => setShowCatalogBrowser(false)}
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              color: "var(--c-muted)",
              fontSize: 18,
              lineHeight: 1,
              padding: "0 4px",
            }}
          >
            ×
          </button>
        </div>

        <div style={{ padding: "16px 20px", overflowY: "auto", flex: 1 }}>
          <div style={{ marginBottom: 16 }}>
            <label style={{ display: "block", fontSize: 11, color: "var(--c-muted)", marginBottom: 6, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.05em" }}>
              Catalog Type
            </label>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
              {(Object.keys(CATALOG_LABELS) as CatalogType[]).map((t) => (
                <button
                  key={t}
                  onClick={() => { setCatalogType(t); setFields({}); setEntries([]); setError(null); }}
                  style={{
                    padding: "4px 10px",
                    borderRadius: 5,
                    border: "1px solid",
                    borderColor: catalogType === t ? "var(--c-accent)" : "var(--c-border)",
                    background: catalogType === t ? "var(--c-accent-subtle)" : "transparent",
                    color: catalogType === t ? "var(--c-accent)" : "var(--c-text)",
                    cursor: "pointer",
                    fontSize: 12,
                    fontWeight: catalogType === t ? 600 : 400,
                  }}
                >
                  {CATALOG_LABELS[t]}
                </button>
              ))}
            </div>
          </div>

          <CatalogFields
            type={catalogType}
            fields={fields}
            placeholders={CATALOG_PLACEHOLDERS[catalogType]}
            onChange={setField}
          />

          {error && (
            <div
              style={{
                marginTop: 12,
                padding: "8px 12px",
                background: "var(--c-error-subtle, #fef2f2)",
                border: "1px solid var(--c-error-border, #fecaca)",
                borderRadius: 6,
                fontSize: 12,
                color: "var(--c-error, #dc2626)",
              }}
            >
              {error}
            </div>
          )}

          {entries.length > 0 && (
            <div style={{ marginTop: 16 }}>
              <div style={{ fontSize: 11, color: "var(--c-muted)", marginBottom: 8, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.05em" }}>
                Available Tables ({entries.length})
              </div>
              <div style={{ border: "1px solid var(--c-border)", borderRadius: 6, overflow: "hidden" }}>
                {entries.map((entry, i) => (
                  <div
                    key={entry.uri}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      padding: "8px 12px",
                      borderBottom: i < entries.length - 1 ? "1px solid var(--c-border)" : "none",
                      background: "var(--c-surface)",
                    }}
                  >
                    <div>
                      <div style={{ fontSize: 13, fontWeight: 500 }}>{entry.name}</div>
                      {entry.namespace && (
                        <div style={{ fontSize: 11, color: "var(--c-muted)", marginTop: 1 }}>
                          {entry.namespace}
                        </div>
                      )}
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                      {entry.n_rows != null && (
                        <span style={{ fontSize: 11, color: "var(--c-muted)" }}>
                          {entry.n_rows.toLocaleString()} rows
                        </span>
                      )}
                      <button
                        className="tv-btn"
                        disabled={openingId === entry.uri}
                        onClick={() => handleOpen(entry)}
                        style={{ fontSize: 12 }}
                      >
                        {openingId === entry.uri ? "Opening…" : "Open"}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        <div
          style={{
            padding: "12px 20px",
            borderTop: "1px solid var(--c-border)",
            display: "flex",
            gap: 8,
            justifyContent: "flex-end",
          }}
        >
          <button
            className="tv-btn"
            onClick={handleBrowse}
            disabled={loading}
          >
            {loading ? "Browsing…" : "Browse"}
          </button>
          <button
            className="tv-btn tv-btn-primary"
            onClick={handleDirectOpen}
            disabled={loading}
          >
            {loading ? "Connecting…" : "Open Directly"}
          </button>
        </div>
      </div>
    </div>
  );
}

function CatalogFields({
  type,
  fields,
  placeholders,
  onChange,
}: {
  type: CatalogType;
  fields: Record<string, string>;
  placeholders: Record<string, string>;
  onChange: (key: string, value: string) => void;
}) {
  const fieldEntries = Object.entries(placeholders);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {fieldEntries.map(([key, placeholder]) => (
        <div key={key}>
          <label
            style={{
              display: "block",
              fontSize: 11,
              color: "var(--c-muted)",
              marginBottom: 4,
              fontWeight: 500,
              textTransform: "capitalize",
            }}
          >
            {key.replace(/_/g, " ")}
          </label>
          <input
            type={key === "token" || key === "password" ? "password" : "text"}
            value={fields[key] ?? ""}
            placeholder={placeholder}
            onChange={(e) => onChange(key, e.target.value)}
            style={{
              width: "100%",
              padding: "6px 10px",
              border: "1px solid var(--c-border)",
              borderRadius: 5,
              background: "var(--c-surface)",
              color: "var(--c-text)",
              fontSize: 13,
              boxSizing: "border-box",
            }}
          />
        </div>
      ))}
    </div>
  );
}

async function browseCatalog(
  type: CatalogType,
  fields: Record<string, string>
): Promise<BrowseResult> {
  const response = await fetch("/api/v1/catalog/browse", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ type, ...fields }),
  });

  if (!response.ok) {
    const body = await response.json().catch(() => ({ error: response.statusText }));
    return { entries: [], error: body.error ?? "Browse failed" };
  }

  return response.json();
}

function buildDirectUri(type: CatalogType, fields: Record<string, string>): string | null {
  switch (type) {
    case "delta":
      return fields.path ? `delta://${fields.path}` : null;
    case "huggingface":
      return fields.dataset
        ? `hf://datasets/${fields.dataset}${fields.split ? `/${fields.split}` : ""}`
        : null;
    case "iceberg_rest":
      return fields.endpoint && fields.warehouse && fields.namespace && fields.table
        ? `iceberg://${fields.endpoint.replace(/^https?:\/\//, "")}/${fields.warehouse}/${fields.namespace}/${fields.table}`
        : null;
    case "glue":
      return fields.database && fields.table
        ? `glue://${fields.database}/${fields.table}${fields.region ? `?region=${fields.region}` : ""}`
        : null;
    case "s3":
      return fields.bucket
        ? `s3://${fields.bucket}/${fields.prefix ?? ""}`
        : null;
    default:
      return null;
  }
}

function buildCredentials(
  type: CatalogType,
  fields: Record<string, string>
): Record<string, string> | undefined {
  if (type === "huggingface" && fields.token) {
    return { access_key: fields.token };
  }
  if (type === "iceberg_rest" && fields.token) {
    return { access_key: fields.token };
  }
  if (fields.access_key && fields.secret_key) {
    return { access_key: fields.access_key, secret_key: fields.secret_key };
  }
  return undefined;
}
