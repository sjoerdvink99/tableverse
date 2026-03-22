import type { Predicate, ViewOp } from "./viewExpr";

export function addPredicate(ops: ViewOp[], pred: Predicate): ViewOp[] {
  const filterIdx = ops.findIndex((op) => op.type === "filter");
  if (filterIdx === -1) {
    return [{ type: "filter", predicate: pred }, ...ops];
  }
  const existing = ops[filterIdx] as Extract<ViewOp, { type: "filter" }>;
  const merged = mergePredicate(existing.predicate, pred);
  return ops.map((op, i) =>
    i === filterIdx ? { type: "filter" as const, predicate: merged } : op
  );
}

export function removePredicate(ops: ViewOp[], column: string): ViewOp[] {
  return ops
    .map((op) => {
      if (op.type !== "filter") return op;
      const pruned = pruneColumn(op.predicate, column);
      if (pruned === null) return null;
      return { type: "filter" as const, predicate: pruned };
    })
    .filter((op): op is ViewOp => op !== null);
}

export function activePredicates(ops: ViewOp[]): Predicate[] {
  const filterOp = ops.find((op): op is Extract<ViewOp, { type: "filter" }> => op.type === "filter");
  if (!filterOp) return [];
  if (filterOp.predicate.op === "and") return filterOp.predicate.exprs;
  return [filterOp.predicate];
}

export function predicatesForColumn(ops: ViewOp[], column: string): Predicate[] {
  return activePredicates(ops).filter((p) => isSingleColumn(p, column));
}

function mergePredicate(existing: Predicate, incoming: Predicate): Predicate {
  if (existing.op === "and") {
    const col = incomingColumn(incoming);
    if (col) {
      const filtered = existing.exprs.filter((e) => !isSingleColumn(e, col));
      return { op: "and", exprs: [...filtered, incoming] };
    }
    return { op: "and", exprs: [...existing.exprs, incoming] };
  }
  const col = incomingColumn(incoming);
  if (col && isSingleColumn(existing, col)) {
    return incoming;
  }
  return { op: "and", exprs: [existing, incoming] };
}

function pruneColumn(pred: Predicate, column: string): Predicate | null {
  if (pred.op === "and") {
    const remaining = pred.exprs
      .map((e) => pruneColumn(e, column))
      .filter((e): e is Predicate => e !== null);
    if (remaining.length === 0) return null;
    if (remaining.length === 1) return remaining[0]!;
    return { op: "and", exprs: remaining };
  }
  if (pred.op === "or") {
    const remaining = pred.exprs
      .map((e) => pruneColumn(e, column))
      .filter((e): e is Predicate => e !== null);
    if (remaining.length === 0) return null;
    if (remaining.length === 1) return remaining[0]!;
    return { op: "or", exprs: remaining };
  }
  if (pred.op === "not") {
    const inner = pruneColumn(pred.expr, column);
    if (inner === null) return null;
    return { op: "not", expr: inner };
  }
  return isSingleColumn(pred, column) ? null : pred;
}

function isSingleColumn(pred: Predicate, column: string): boolean {
  if (pred.op === "and" || pred.op === "or" || pred.op === "not") return false;
  return pred.column === column;
}

function incomingColumn(pred: Predicate): string | null {
  if (pred.op === "and" || pred.op === "or" || pred.op === "not") return null;
  return pred.column;
}

export function appendNotIn(ops: ViewOp[], column: string, value: unknown): ViewOp[] {
  const existing = activePredicates(ops).find(
    (p): p is Extract<Predicate, { op: "not_in" }> =>
      p.op === "not_in" && p.column === column
  );
  const nextPred: Predicate = existing
    ? { op: "not_in", column, values: [...existing.values, value as never] }
    : { op: "not_in", column, values: [value as never] };
  return addPredicate(removePredicate(ops, column), nextPred);
}
