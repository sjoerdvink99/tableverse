import type { ViewOp } from "./viewExpr";

export function computeViewHash(ops: ViewOp[], sourceId?: string | null): string {
  const canonical = (sourceId ?? "") + "\0" + JSON.stringify(ops);
  let hash = 0xcbf29ce484222325n;
  for (let i = 0; i < canonical.length; i++) {
    hash ^= BigInt(canonical.charCodeAt(i));
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return hash.toString(16).padStart(16, "0");
}
