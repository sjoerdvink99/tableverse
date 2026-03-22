import type { Table } from "apache-arrow";
import type { Predicate, Literal } from "./viewExpr";

export const CLIENT_INDEX_MAX_ROWS = 10_000_000;

type ColData = Float64Array | string[];

export class ClientIndex {
  private cols = new Map<string, ColData>();
  private sortedIdx = new Map<string, Uint32Array>();
  private colNames: string[] = [];
  private _totalRows = 0;

  get totalRows(): number {
    return this._totalRows;
  }

  build(tables: Table[]): void {
    this.cols.clear();
    this.sortedIdx.clear();
    this.colNames = [];
    this._totalRows = 0;

    if (tables.length === 0) return;

    const first = tables[0]!;
    this.colNames = first.schema.fields.map((f) => f.name);
    this._totalRows = tables.reduce((s, t) => s + t.numRows, 0);

    for (const name of this.colNames) {
      const firstField = first.schema.fields.find((f) => f.name === name);
      const typeStr = firstField ? String(firstField.type) : "";
      const isNum = isNumericTypeStr(typeStr);

      if (isNum) {
        const data = new Float64Array(this._totalRows);
        let off = 0;
        for (const table of tables) {
          const col = table.getChild(name);
          if (col) {
            for (let i = 0; i < col.length; i++) {
              const v = col.get(i);
              data[off++] = v === null || v === undefined ? NaN : Number(v);
            }
          } else {
            off += table.numRows;
          }
        }
        this.cols.set(name, data);
        this.sortedIdx.set(name, buildSortedIndicesNum(data));
      } else {
        const data: string[] = new Array(this._totalRows);
        let off = 0;
        for (const table of tables) {
          const col = table.getChild(name);
          if (col) {
            for (let i = 0; i < col.length; i++) {
              const v = col.get(i);
              data[off++] = v === null || v === undefined ? "" : String(v);
            }
          } else {
            for (let i = 0; i < table.numRows; i++) data[off++] = "";
          }
        }
        this.cols.set(name, data);
        this.sortedIdx.set(name, buildSortedIndicesStr(data));
      }
    }
  }

  filter(pred: Predicate): Uint32Array {
    return this.evalPred(pred);
  }

  sliceRows(rowIndices: Uint32Array, rowOffset: number, rowCount: number): Map<string, unknown[]> {
    const end = Math.min(rowOffset + rowCount, rowIndices.length);
    const result = new Map<string, unknown[]>();
    for (const name of this.colNames) {
      const col = this.cols.get(name)!;
      const vals: unknown[] = [];
      for (let i = rowOffset; i < end; i++) {
        const idx = rowIndices[i]!;
        vals.push(col instanceof Float64Array ? col[idx] : (col as string[])[idx]);
      }
      result.set(name, vals);
    }
    return result;
  }

  private evalPred(pred: Predicate): Uint32Array {
    switch (pred.op) {
      case "and": {
        let r = this.allRows();
        for (const sub of pred.exprs) r = intersect(r, this.evalPred(sub));
        return r;
      }
      case "or": {
        return union(pred.exprs.map((sub) => this.evalPred(sub)));
      }
      case "not":
        return complement(this.evalPred(pred.expr), this._totalRows);
      case "eq":
        return this.evalEq(pred.column, pred.value);
      case "ne":
        return complement(this.evalEq(pred.column, pred.value), this._totalRows);
      case "gt":
        return this.evalCmp(pred.column, pred.value, "gt");
      case "gte":
        return this.evalCmp(pred.column, pred.value, "gte");
      case "lt":
        return this.evalCmp(pred.column, pred.value, "lt");
      case "lte":
        return this.evalCmp(pred.column, pred.value, "lte");
      case "between":
        return intersect(
          this.evalCmp(pred.column, pred.lo, "gte"),
          this.evalCmp(pred.column, pred.hi, "lte")
        );
      case "in":
        return union(pred.values.map((v) => this.evalEq(pred.column, v)));
      case "not_in":
        return complement(
          union(pred.values.map((v) => this.evalEq(pred.column, v))),
          this._totalRows
        );
      case "contains":
        return this.evalStrMatch(pred.column, (s) => s.includes(pred.value));
      case "starts_with":
        return this.evalStrMatch(pred.column, (s) => s.startsWith(pred.value));
      case "ends_with":
        return this.evalStrMatch(pred.column, (s) => s.endsWith(pred.value));
      case "regex": {
        let re: RegExp;
        try {
          re = new RegExp(pred.pattern);
        } catch {
          return this.allRows();
        }
        return this.evalStrMatch(pred.column, (s) => re.test(s));
      }
      case "is_null":
        return this.evalNull(pred.column, true);
      case "is_not_null":
        return this.evalNull(pred.column, false);
      default:
        return this.allRows();
    }
  }

  private allRows(): Uint32Array {
    const a = new Uint32Array(this._totalRows);
    for (let i = 0; i < a.length; i++) a[i] = i;
    return a;
  }

  private evalEq(column: string, value: Literal): Uint32Array {
    const data = this.cols.get(column);
    if (!data) return this.allRows();

    if (data instanceof Float64Array && typeof value === "number") {
      const matching: number[] = [];
      for (let i = 0; i < data.length; i++) {
        if (data[i] === value) matching.push(i);
      }
      return new Uint32Array(matching);
    }

    if (Array.isArray(data) && typeof value === "string") {
      const sorted = this.sortedIdx.get(column)!;
      const matching: number[] = [];
      let lo = 0;
      let hi = sorted.length - 1;
      while (lo <= hi) {
        const mid = (lo + hi) >>> 1;
        if ((data[sorted[mid]!]! as string) < value) lo = mid + 1;
        else hi = mid - 1;
      }
      for (let i = lo; i < sorted.length && data[sorted[i]!] === value; i++) {
        matching.push(sorted[i]!);
      }
      return new Uint32Array(matching).sort();
    }

    return this.allRows();
  }

  private evalCmp(
    column: string,
    value: Literal,
    op: "gt" | "gte" | "lt" | "lte"
  ): Uint32Array {
    const data = this.cols.get(column);
    if (!(data instanceof Float64Array) || typeof value !== "number") return this.allRows();
    const matching: number[] = [];
    for (let i = 0; i < data.length; i++) {
      const v = data[i]!;
      const pass =
        op === "gt" ? v > value :
        op === "gte" ? v >= value :
        op === "lt" ? v < value :
        v <= value;
      if (pass) matching.push(i);
    }
    return new Uint32Array(matching);
  }

  private evalStrMatch(column: string, test: (s: string) => boolean): Uint32Array {
    const data = this.cols.get(column);
    if (!(Array.isArray(data))) return this.allRows();
    const matching: number[] = [];
    for (let i = 0; i < data.length; i++) {
      if (test(data[i] as string)) matching.push(i);
    }
    return new Uint32Array(matching);
  }

  private evalNull(column: string, wantNull: boolean): Uint32Array {
    const data = this.cols.get(column);
    if (!data) return this.allRows();
    const matching: number[] = [];
    for (let i = 0; i < data.length; i++) {
      const isNull =
        data instanceof Float64Array
          ? isNaN(data[i]!)
          : (data as string[])[i] === null || (data as string[])[i] === "";
      if (isNull === wantNull) matching.push(i);
    }
    return new Uint32Array(matching);
  }
}

function buildSortedIndicesNum(data: Float64Array): Uint32Array {
  const idx = new Uint32Array(data.length);
  for (let i = 0; i < idx.length; i++) idx[i] = i;
  idx.sort((a, b) => data[a]! - data[b]!);
  return idx;
}

function buildSortedIndicesStr(data: string[]): Uint32Array {
  const idx = new Uint32Array(data.length);
  for (let i = 0; i < idx.length; i++) idx[i] = i;
  idx.sort((a, b) => (data[a]! < data[b]! ? -1 : data[a]! > data[b]! ? 1 : 0));
  return idx;
}

function intersect(a: Uint32Array, b: Uint32Array): Uint32Array {
  const setB = new Set<number>(b);
  const out: number[] = [];
  for (const x of a) if (setB.has(x)) out.push(x);
  return new Uint32Array(out);
}

function union(arrays: Uint32Array[]): Uint32Array {
  const set = new Set<number>();
  for (const arr of arrays) for (const x of arr) set.add(x);
  const out = new Uint32Array(set.size);
  let i = 0;
  for (const x of set) out[i++] = x;
  out.sort();
  return out;
}

function complement(matches: Uint32Array, total: number): Uint32Array {
  const set = new Set<number>(matches);
  const out: number[] = [];
  for (let i = 0; i < total; i++) if (!set.has(i)) out.push(i);
  return new Uint32Array(out);
}

function isNumericTypeStr(typeStr: string): boolean {
  return /int|float|double|decimal|uint|num/i.test(typeStr);
}
