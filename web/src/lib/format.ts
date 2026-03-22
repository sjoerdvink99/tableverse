const MAX_CELL_CHARS = 100;

export type CellRenderInfo = {
  text: string;
  align: "left" | "right" | "center";
  color: string;
  isNull: boolean;
};

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function nullColor(): string {
  return cssVar("--canvas-text-null") || "#9ca3af";
}

function textColor(): string {
  return cssVar("--canvas-text") || "#111827";
}

function numberColor(): string {
  return cssVar("--canvas-number") || "#1d4ed8";
}

function boolTrueColor(): string {
  return cssVar("--canvas-bool-true") || "#16a34a";
}

function boolFalseColor(): string {
  return cssVar("--canvas-bool-false") || "#dc2626";
}

export function renderCell(value: unknown, dataType: string): CellRenderInfo {
  if (value === null || value === undefined) {
    return { text: "null", align: "left", color: nullColor(), isNull: true };
  }

  const dt = dataType.toLowerCase();

  if (dt === "boolean" || typeof value === "boolean") {
    const boolVal = typeof value === "boolean" ? value : String(value).toLowerCase() === "true";
    return {
      text: boolVal ? "true" : "false",
      align: "center",
      color: boolVal ? boolTrueColor() : boolFalseColor(),
      isNull: false,
    };
  }

  if (
    dt.includes("int") || dt.includes("uint") || dt.includes("float") ||
    dt.includes("double") || dt.includes("decimal") || typeof value === "number" || typeof value === "bigint"
  ) {
    const num = typeof value === "bigint" ? Number(value) : Number(value);
    if (!isFinite(num)) {
      return { text: String(num), align: "right", color: "#ea580c", isNull: false };
    }
    const text = Number.isInteger(num)
      ? num.toLocaleString()
      : num.toPrecision(6).replace(/\.?0+$/, "");
    return { text, align: "right", color: numberColor(), isNull: false };
  }

  if (dt.includes("date32") || dt.includes("date64")) {
    const str = String(value);
    return { text: str, align: "left", color: textColor(), isNull: false };
  }

  if (dt.includes("timestamp")) {
    const str = String(value).replace("T", " ").slice(0, 19);
    return { text: str, align: "left", color: textColor(), isNull: false };
  }

  const str = String(value);
  const MAX_CHARS = 200;
  const text = str.length > MAX_CHARS ? str.slice(0, MAX_CHARS) + "…" : str;
  return { text, align: "left", color: textColor(), isNull: false };
}

export function formatCellValue(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") {
    if (!isFinite(value)) return String(value);
    if (Number.isInteger(value)) return value.toLocaleString();
    return value.toPrecision(6).replace(/\.?0+$/, "");
  }
  if (value instanceof Date) return value.toISOString().replace("T", " ").slice(0, 19);
  if (typeof value === "bigint") return value.toLocaleString();
  const str = String(value);
  return str.length > MAX_CELL_CHARS ? str.slice(0, MAX_CELL_CHARS) + "…" : str;
}

export function formatRowCount(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}
