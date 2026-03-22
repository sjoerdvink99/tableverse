export type ViewExpr = {
  source_id: string;
  ops: ViewOp[];
};

export type ViewOp =
  | { type: "filter"; predicate: Predicate }
  | { type: "select"; columns: string[] }
  | { type: "drop"; columns: string[] }
  | { type: "sort"; keys: SortKey[] }
  | { type: "derive"; name: string; expr: ScalarExpr }
  | { type: "deduplicate"; columns: string[] | null }
  | { type: "sample"; n: number; strategy: SampleStrategy; seed: number | null }
  | { type: "group_by"; keys: string[]; aggs: AggExpr[] }
  | { type: "rename"; mappings: [string, string][] }
  | { type: "limit"; n: number };

export type Predicate =
  | { op: "eq"; column: string; value: Literal }
  | { op: "ne"; column: string; value: Literal }
  | { op: "gt"; column: string; value: Literal }
  | { op: "gte"; column: string; value: Literal }
  | { op: "lt"; column: string; value: Literal }
  | { op: "lte"; column: string; value: Literal }
  | { op: "between"; column: string; lo: Literal; hi: Literal }
  | { op: "in"; column: string; values: Literal[] }
  | { op: "not_in"; column: string; values: Literal[] }
  | { op: "contains"; column: string; value: string }
  | { op: "starts_with"; column: string; value: string }
  | { op: "ends_with"; column: string; value: string }
  | { op: "regex"; column: string; pattern: string }
  | { op: "is_null"; column: string }
  | { op: "is_not_null"; column: string }
  | { op: "and"; exprs: Predicate[] }
  | { op: "or"; exprs: Predicate[] }
  | { op: "not"; expr: Predicate };

export type ScalarExpr =
  | { kind: "column"; name: string }
  | { kind: "literal"; value: Literal }
  | { kind: "bin_op"; op: BinOp; left: ScalarExpr; right: ScalarExpr }
  | { kind: "abs"; 0: ScalarExpr }
  | { kind: "round"; expr: ScalarExpr; decimals: number }
  | { kind: "floor"; 0: ScalarExpr }
  | { kind: "ceil"; 0: ScalarExpr }
  | { kind: "upper"; 0: ScalarExpr }
  | { kind: "lower"; 0: ScalarExpr }
  | { kind: "trim"; 0: ScalarExpr }
  | { kind: "length"; 0: ScalarExpr }
  | { kind: "substr"; expr: ScalarExpr; start: number; len: number | null }
  | { kind: "concat"; parts: ScalarExpr[] }
  | { kind: "year"; 0: ScalarExpr }
  | { kind: "month"; 0: ScalarExpr }
  | { kind: "day"; 0: ScalarExpr }
  | { kind: "case"; whens: [Predicate, ScalarExpr][]; else_expr: ScalarExpr | null }
  | { kind: "coalesce"; exprs: ScalarExpr[] }
  | { kind: "rank"; order: SortKey[] }
  | { kind: "n_tile"; n: number }
  | { kind: "cast"; expr: ScalarExpr; to_type: DataType };

export type BinOp = "add" | "sub" | "mul" | "div" | "mod";

export type DataType =
  | "int32" | "int64" | "float32" | "float64"
  | "text" | "boolean" | "date" | "timestamp";

export type AggExpr =
  | { fn: "count"; alias: string }
  | { fn: "count_distinct"; column: string; alias: string }
  | { fn: "sum"; column: string; alias: string }
  | { fn: "min"; column: string; alias: string }
  | { fn: "max"; column: string; alias: string }
  | { fn: "mean"; column: string; alias: string }
  | { fn: "median"; column: string; alias: string }
  | { fn: "std_dev"; column: string; alias: string }
  | { fn: "percentile"; column: string; p: number; alias: string };

export type SortKey = {
  column: string;
  descending: boolean;
  nulls_last: boolean;
};

export type SampleStrategy = "bernoulli" | "system";

export type Literal = null | boolean | number | string;

export function getAggAlias(agg: AggExpr): string {
  return agg.alias;
}

export function predicateColumns(pred: Predicate): string[] {
  switch (pred.op) {
    case "and":
    case "or":
      return pred.exprs.flatMap(predicateColumns);
    case "not":
      return predicateColumns(pred.expr);
    default:
      return [pred.column];
  }
}

function formatLit(value: Literal): string {
  if (value === null) return "null";
  if (typeof value === "string") return `"${value}"`;
  return String(value);
}

function predLabel(pred: Predicate): string {
  switch (pred.op) {
    case "eq": return `${pred.column} = ${formatLit(pred.value)}`;
    case "ne": return `${pred.column} ≠ ${formatLit(pred.value)}`;
    case "gt": return `${pred.column} > ${formatLit(pred.value)}`;
    case "gte": return `${pred.column} ≥ ${formatLit(pred.value)}`;
    case "lt": return `${pred.column} < ${formatLit(pred.value)}`;
    case "lte": return `${pred.column} ≤ ${formatLit(pred.value)}`;
    case "between": return `${pred.column} between ${formatLit(pred.lo)} and ${formatLit(pred.hi)}`;
    case "in": return `${pred.column} in [${pred.values.map(formatLit).join(", ")}]`;
    case "not_in": return `${pred.column} not in [${pred.values.map(formatLit).join(", ")}]`;
    case "contains": return `${pred.column} contains "${pred.value}"`;
    case "starts_with": return `${pred.column} starts with "${pred.value}"`;
    case "ends_with": return `${pred.column} ends with "${pred.value}"`;
    case "regex": return `${pred.column} ~ /${pred.pattern}/`;
    case "is_null": return `${pred.column} is null`;
    case "is_not_null": return `${pred.column} is not null`;
    case "and": return pred.exprs.map(predLabel).join(" AND ");
    case "or": return pred.exprs.map(predLabel).join(" OR ");
    case "not": return `NOT (${predLabel(pred.expr)})`;
  }
}

export function opLabel(op: ViewOp): string {
  switch (op.type) {
    case "filter":
      return predLabel(op.predicate);
    case "select":
      return `select ${op.columns.length} col${op.columns.length !== 1 ? "s" : ""}`;
    case "drop":
      return `drop ${op.columns.map(c => `"${c}"`).join(", ")}`;
    case "sort":
      return op.keys.map(k => `${k.descending ? "↓" : "↑"} ${k.column}`).join(", ");
    case "derive":
      return `derive "${op.name}"`;
    case "deduplicate":
      return op.columns ? `dedup by ${op.columns.join(", ")}` : "deduplicate";
    case "sample":
      return `sample ${op.n.toLocaleString()} rows`;
    case "group_by":
      return `group by ${op.keys.join(", ")}`;
    case "rename":
      return op.mappings.map(([from, to]) => `"${from}" → "${to}"`).join(", ");
    case "limit":
      return `limit ${op.n.toLocaleString()}`;
  }
}

