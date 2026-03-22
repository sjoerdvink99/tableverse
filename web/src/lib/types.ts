export type SourceFormat = "parquet" | "csv" | "arrow" | "json";

export type SourceKind =
  | "local_file"
  | "s3"
  | "gcs"
  | "azure_blob"
  | "http"
  | "delta"
  | "iceberg"
  | "postgres"
  | "mysql"
  | "hugging_face";

export type Credentials = {
  access_key?: string;
  secret_key?: string;
  session_token?: string;
  endpoint?: string;
  region?: string;
};

export type ProfileSummary = {
  name: string;
};

export type ColumnInfo = {
  index: number;
  name: string;
  data_type: string;
  nullable: boolean;
};

export type CardinalityCategory =
  | "constant"
  | "binary"
  | "low_cardinality"
  | "categorical"
  | "high_cardinality"
  | "unique"
  | "unknown";

export type Quantiles = {
  p1: number;
  p5: number;
  p25: number;
  p50: number;
  p75: number;
  p95: number;
  p99: number;
};

export type TopValue = {
  value: unknown;
  count: number;
  rate: number;
};

export type QuickColumnStats = {
  index: number;
  null_count: number;
  null_rate: number;
  min: unknown;
  max: unknown;
};

export type SourceRecommendation = {
  kind: string;
  message: string;
};

export type SourceMeta = {
  id: string;
  name: string;
  uri: string;
  files?: string[];
  format: SourceFormat;
  kind?: SourceKind;
  n_rows: number;
  n_cols: number;
  columns: ColumnInfo[];
  quick_stats?: QuickColumnStats[];
  recommendations?: SourceRecommendation[];
  tile_rows?: number;
};

export type SortSpec = {
  column: string;
  descending: boolean;
};

export type FilterExpr =
  | { op: "eq"; column: string; value: unknown }
  | { op: "ne"; column: string; value: unknown }
  | { op: "gt"; column: string; value: unknown }
  | { op: "gte"; column: string; value: unknown }
  | { op: "lt"; column: string; value: unknown }
  | { op: "lte"; column: string; value: unknown }
  | { op: "contains"; column: string; value: string }
  | { op: "is_null"; column: string }
  | { op: "is_not_null"; column: string }
  | { op: "and"; exprs: FilterExpr[] }
  | { op: "or"; exprs: FilterExpr[] }
  | { op: "not"; expr: FilterExpr };

export type TileCoord = {
  row: number;
  col: number;
};

export type TileKey = string;

export type CellAddress = {
  row: number;
  col: number;
};

export type CellRange = {
  anchor: CellAddress;
  active: CellAddress;
};

export type ColumnStats = {
  column: string;
  index: number;
  data_type: string;
  count: number;
  null_count: number;
  null_rate: number;
  distinct_count: number | null;
  min: unknown;
  max: unknown;
  mean: number | null;
  quantiles: Quantiles | null;
  histogram: Array<{ lo: number; hi: number; count: number }> | null;
  top_values: TopValue[] | null;
  cardinality_category: CardinalityCategory;
};

export type CorrelationMatrix = {
  columns: string[];
  matrix: Array<Array<number | null>>;
};

export type SearchResults = {
  rows: number[];
  total: number;
};

export type Viewport = {
  scrollX: number;
  scrollY: number;
  width: number;
  height: number;
};

export type RowGroupStat = {
  rg_index: number;
  row_offset: number;
  row_count: number;
  null_count: number;
  min: number | null;
  max: number | null;
  mean: number | null;
};
