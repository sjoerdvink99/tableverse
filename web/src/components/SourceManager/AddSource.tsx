import { useEffect, useRef, useState } from "react";
import { fetchProfiles, registerSource, uploadSource } from "../../lib/api";
import type { Credentials, SourceMeta, SourceRecommendation } from "../../lib/types";
import { RecommendationBanner } from "./RecommendationBanner";

const MAX_UPLOAD_BYTES = 512 * 1024 * 1024;

type Props = {
  onAdded: (source: SourceMeta) => void;
  onCancel: () => void;
};

type Tab = "file" | "cloud" | "database" | "huggingface";

const TAB_LABELS: Record<Tab, string> = {
  file: "File / URI",
  cloud: "Cloud Storage",
  database: "Database",
  huggingface: "HuggingFace",
};

const INPUT_STYLE: React.CSSProperties = {
  display: "block",
  width: "100%",
  marginTop: 4,
  padding: "7px 10px",
  fontSize: 13,
  border: "1px solid var(--c-border)",
  borderRadius: 4,
  boxSizing: "border-box",
  background: "var(--c-bg)",
  color: "var(--c-text)",
  outline: "none",
};

const LABEL_STYLE: React.CSSProperties = { fontSize: 13, color: "var(--c-text-2)" };

const SELECT_STYLE: React.CSSProperties = {
  display: "block",
  width: "100%",
  marginTop: 4,
  padding: "7px 10px",
  fontSize: 13,
  border: "1px solid var(--c-border)",
  borderRadius: 4,
  boxSizing: "border-box",
  background: "var(--c-bg)",
  color: "var(--c-text)",
};

export function AddSource({ onAdded, onCancel }: Props) {
  const [tab, setTab] = useState<Tab>("file");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [recommendations, setRecommendations] = useState<SourceRecommendation[]>([]);
  const [showBanner, setShowBanner] = useState(false);
  const [profiles, setProfiles] = useState<string[]>([]);
  const [dragOver, setDragOver] = useState(false);
  const dragDepth = useRef(0);

  const [fileUri, setFileUri] = useState("");
  const [fileName, setFileName] = useState("");

  const [cloudUri, setCloudUri] = useState("");
  const [cloudName, setCloudName] = useState("");
  const [cloudProfile, setCloudProfile] = useState("");
  const [cloudAccessKey, setCloudAccessKey] = useState("");
  const [cloudSecretKey, setCloudSecretKey] = useState("");
  const [cloudRegion, setCloudRegion] = useState("");
  const [cloudEndpoint, setCloudEndpoint] = useState("");

  const [dbConnectionString, setDbConnectionString] = useState("");
  const [dbSchema, setDbSchema] = useState("");
  const [dbTable, setDbTable] = useState("");
  const [dbName, setDbName] = useState("");

  const [hfDataset, setHfDataset] = useState("");
  const [hfSplit, setHfSplit] = useState("");
  const [hfName, setHfName] = useState("");

  useEffect(() => {
    fetchProfiles()
      .then(setProfiles)
      .catch(() => setProfiles([]));
  }, []);

  const onSourceAdded = (source: SourceMeta) => {
    onAdded(source);
    if (source.recommendations && source.recommendations.length > 0) {
      setRecommendations(source.recommendations);
      setShowBanner(true);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");

    try {
      let uri = "";
      let name: string | undefined;
      let profile: string | undefined;
      let credentials: Credentials | undefined;

      if (tab === "file") {
        if (!fileUri.trim()) {
          setError("File path or URI is required");
          return;
        }
        uri = fileUri.trim();
        name = fileName.trim() || undefined;
      } else if (tab === "cloud") {
        if (!cloudUri.trim()) {
          setError("URI is required");
          return;
        }
        uri = cloudUri.trim();
        name = cloudName.trim() || undefined;
        profile = cloudProfile.trim() || undefined;
        const creds: Credentials = {
          ...(cloudAccessKey.trim() && { access_key: cloudAccessKey.trim() }),
          ...(cloudSecretKey.trim() && { secret_key: cloudSecretKey.trim() }),
          ...(cloudRegion.trim() && { region: cloudRegion.trim() }),
          ...(cloudEndpoint.trim() && { endpoint: cloudEndpoint.trim() }),
        };
        if (Object.keys(creds).length > 0) credentials = creds;
      } else if (tab === "database") {
        if (!dbConnectionString.trim()) {
          setError("Connection string is required");
          return;
        }
        const table = dbTable.trim();
        const schema = dbSchema.trim();
        const tableRef = schema && table ? `${schema}.${table}` : table || schema;
        uri = tableRef
          ? `${dbConnectionString.trim()}?table=${encodeURIComponent(tableRef)}`
          : dbConnectionString.trim();
        name = dbName.trim() || undefined;
      } else if (tab === "huggingface") {
        if (!hfDataset.trim()) {
          setError("Dataset identifier is required");
          return;
        }
        const split = hfSplit.trim();
        uri = split
          ? `hf://${hfDataset.trim()}/${split}`
          : `hf://${hfDataset.trim()}`;
        name = hfName.trim() || undefined;
      }

      const source = await registerSource(uri, name, profile, credentials);
      onSourceAdded(source);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const isSubmitDisabled = (): boolean => {
    if (loading) return true;
    if (tab === "file") return !fileUri.trim();
    if (tab === "cloud") return !cloudUri.trim();
    if (tab === "database") return !dbConnectionString.trim();
    if (tab === "huggingface") return !hfDataset.trim();
    return true;
  };

  const handleFileUpload = async (file: File) => {
    if (file.size > MAX_UPLOAD_BYTES) {
      setError(`File too large (max ${MAX_UPLOAD_BYTES / 1024 / 1024} MB). Use a file path instead.`);
      return;
    }
    setLoading(true);
    setError("");
    try {
      const isParquet = file.name.endsWith(".parquet") || file.type === "application/x-parquet";
      const buf = await file.arrayBuffer();
      const source = await uploadSource(buf, fileName.trim() || file.name, isParquet);
      onSourceAdded(source);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleDragEnter = (e: React.DragEvent) => {
    e.preventDefault();
    dragDepth.current += 1;
    setDragOver(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    dragDepth.current -= 1;
    if (dragDepth.current === 0) setDragOver(false);
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    dragDepth.current = 0;
    setDragOver(false);
    const file = e.dataTransfer.files[0];
    if (file) handleFileUpload(file);
  };

  const handleFileInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) handleFileUpload(file);
    e.target.value = "";
  };

  return (
    <>
      {showBanner && (
        <RecommendationBanner
          recommendations={recommendations}
          onDismiss={() => setShowBanner(false)}
        />
      )}
      <div
        style={{
          position: "fixed",
          inset: 0,
          background: "rgba(0,0,0,0.4)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          zIndex: 900,
        }}
        onClick={onCancel}
      >
        <div
          style={{
            background: "var(--c-bg)",
            borderRadius: 8,
            padding: "24px",
            width: 480,
            boxShadow: "0 8px 32px rgba(0,0,0,0.3)",
            border: "1px solid var(--c-border)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <h3 style={{ margin: "0 0 16px", fontSize: 16, color: "var(--c-text)" }}>Add data source</h3>

          <div style={{ display: "flex", gap: 0, marginBottom: 16, borderBottom: "1px solid var(--c-border)" }}>
            {(["file", "cloud", "database", "huggingface"] as Tab[]).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => setTab(t)}
                style={{
                  padding: "6px 12px",
                  fontSize: 12,
                  fontWeight: 500,
                  border: "none",
                  borderBottom: tab === t ? "2px solid var(--c-accent)" : "2px solid transparent",
                  background: "none",
                  cursor: "pointer",
                  color: tab === t ? "var(--c-accent)" : "var(--c-text-2)",
                  marginBottom: -1,
                }}
              >
                {TAB_LABELS[t]}
              </button>
            ))}
          </div>

          <form onSubmit={handleSubmit} style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            {tab === "file" && (
              <>
                <div
                  onDragEnter={handleDragEnter}
                  onDragOver={(e) => e.preventDefault()}
                  onDragLeave={handleDragLeave}
                  onDrop={handleDrop}
                  style={{
                    border: `2px dashed ${dragOver ? "var(--c-accent)" : "var(--c-border)"}`,
                    borderRadius: 6,
                    padding: "20px 16px",
                    textAlign: "center",
                    background: dragOver ? "var(--c-surface)" : "transparent",
                    transition: "border-color 0.15s, background 0.15s",
                    cursor: "pointer",
                  }}
                  onClick={() => document.getElementById("tv-file-input")?.click()}
                >
                  <input
                    id="tv-file-input"
                    type="file"
                    accept=".parquet,.arrow,.csv,.json,.jsonl"
                    style={{ display: "none" }}
                    onChange={handleFileInput}
                  />
                  <div style={{ fontSize: 13, color: "var(--c-text-2)" }}>
                    Drop a file here or{" "}
                    <span style={{ color: "var(--c-accent)", textDecoration: "underline" }}>browse</span>
                  </div>
                  <div style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 4 }}>
                    .parquet · .arrow · .csv · .json · .jsonl — max 512 MB
                  </div>
                </div>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <div style={{ flex: 1, height: 1, background: "var(--c-border)" }} />
                  <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>or enter path</span>
                  <div style={{ flex: 1, height: 1, background: "var(--c-border)" }} />
                </div>
                <label style={LABEL_STYLE}>
                  File path or URI
                  <input
                    value={fileUri}
                    onChange={(e) => setFileUri(e.target.value)}
                    placeholder="/path/to/data.parquet"
                    style={INPUT_STYLE}
                  />
                </label>
                <label style={LABEL_STYLE}>
                  Name (optional)
                  <input
                    value={fileName}
                    onChange={(e) => setFileName(e.target.value)}
                    placeholder="My dataset"
                    style={INPUT_STYLE}
                  />
                </label>
              </>
            )}

            {tab === "cloud" && (
              <>
                <label style={LABEL_STYLE}>
                  URI
                  <input
                    autoFocus
                    value={cloudUri}
                    onChange={(e) => setCloudUri(e.target.value)}
                    placeholder="s3://bucket/data.parquet"
                    style={INPUT_STYLE}
                  />
                </label>
                <label style={LABEL_STYLE}>
                  Name (optional)
                  <input
                    value={cloudName}
                    onChange={(e) => setCloudName(e.target.value)}
                    placeholder="My dataset"
                    style={INPUT_STYLE}
                  />
                </label>
                <label style={LABEL_STYLE}>
                  Profile
                  <select
                    value={cloudProfile}
                    onChange={(e) => setCloudProfile(e.target.value)}
                    style={SELECT_STYLE}
                  >
                    <option value="">— none —</option>
                    {profiles.map((p) => (
                      <option key={p} value={p}>{p}</option>
                    ))}
                  </select>
                </label>
                <div style={{ fontSize: 12, color: "var(--c-text-2)", fontWeight: 500, marginBottom: -4 }}>
                  Inline credentials (optional)
                </div>
                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                  <label style={LABEL_STYLE}>
                    Access key
                    <input
                      value={cloudAccessKey}
                      onChange={(e) => setCloudAccessKey(e.target.value)}
                      placeholder="AKIAIOSFODNN7..."
                      style={INPUT_STYLE}
                    />
                  </label>
                  <label style={LABEL_STYLE}>
                    Secret key
                    <input
                      type="password"
                      value={cloudSecretKey}
                      onChange={(e) => setCloudSecretKey(e.target.value)}
                      placeholder="wJalrXUtnFEMI..."
                      style={INPUT_STYLE}
                    />
                  </label>
                  <label style={LABEL_STYLE}>
                    Region
                    <input
                      value={cloudRegion}
                      onChange={(e) => setCloudRegion(e.target.value)}
                      placeholder="us-east-1"
                      style={INPUT_STYLE}
                    />
                  </label>
                  <label style={LABEL_STYLE}>
                    Endpoint
                    <input
                      value={cloudEndpoint}
                      onChange={(e) => setCloudEndpoint(e.target.value)}
                      placeholder="https://s3.example.com"
                      style={INPUT_STYLE}
                    />
                  </label>
                </div>
              </>
            )}

            {tab === "database" && (
              <>
                <label style={LABEL_STYLE}>
                  Connection string
                  <input
                    autoFocus
                    value={dbConnectionString}
                    onChange={(e) => setDbConnectionString(e.target.value)}
                    placeholder="postgres://user:pass@host/db"
                    style={INPUT_STYLE}
                  />
                </label>
                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                  <label style={LABEL_STYLE}>
                    Schema (optional)
                    <input
                      value={dbSchema}
                      onChange={(e) => setDbSchema(e.target.value)}
                      placeholder="public"
                      style={INPUT_STYLE}
                    />
                  </label>
                  <label style={LABEL_STYLE}>
                    Table
                    <input
                      value={dbTable}
                      onChange={(e) => setDbTable(e.target.value)}
                      placeholder="my_table"
                      style={INPUT_STYLE}
                    />
                  </label>
                </div>
                <label style={LABEL_STYLE}>
                  Name (optional)
                  <input
                    value={dbName}
                    onChange={(e) => setDbName(e.target.value)}
                    placeholder="My table"
                    style={INPUT_STYLE}
                  />
                </label>
              </>
            )}

            {tab === "huggingface" && (
              <>
                <label style={LABEL_STYLE}>
                  Dataset
                  <input
                    autoFocus
                    value={hfDataset}
                    onChange={(e) => setHfDataset(e.target.value)}
                    placeholder="datasets/owner/name"
                    style={INPUT_STYLE}
                  />
                </label>
                <label style={LABEL_STYLE}>
                  Split (optional)
                  <input
                    value={hfSplit}
                    onChange={(e) => setHfSplit(e.target.value)}
                    placeholder="train"
                    style={INPUT_STYLE}
                  />
                </label>
                <label style={LABEL_STYLE}>
                  Name (optional)
                  <input
                    value={hfName}
                    onChange={(e) => setHfName(e.target.value)}
                    placeholder="My dataset"
                    style={INPUT_STYLE}
                  />
                </label>
              </>
            )}

            {error && <p style={{ color: "#ef4444", fontSize: 12, margin: 0 }}>{error}</p>}

            <div style={{ display: "flex", gap: 8, justifyContent: "flex-end", marginTop: 4 }}>
              <button
                type="button"
                onClick={onCancel}
                style={{
                  padding: "7px 14px",
                  fontSize: 13,
                  background: "var(--c-surface)",
                  border: "1px solid var(--c-border)",
                  borderRadius: 4,
                  cursor: "pointer",
                  color: "var(--c-text-2)",
                }}
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={isSubmitDisabled()}
                style={{
                  padding: "7px 14px",
                  fontSize: 13,
                  background: "var(--c-accent)",
                  color: "#fff",
                  border: "none",
                  borderRadius: 4,
                  cursor: isSubmitDisabled() ? "not-allowed" : "pointer",
                  opacity: isSubmitDisabled() ? 0.6 : 1,
                }}
              >
                {loading ? "Loading…" : "Add source"}
              </button>
            </div>
          </form>
        </div>
      </div>
    </>
  );
}
