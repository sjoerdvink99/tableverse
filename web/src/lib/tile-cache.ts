import type { Table } from "apache-arrow";

type CacheEntry = {
  table: Table;
  bytes: number;
};

function tableByteSize(table: Table): number {
  let total = 0;
  for (let i = 0; i < table.numCols; i++) {
    const col = table.getChildAt(i);
    if (col) {
      for (let chunk = 0; chunk < col.data.length; chunk++) {
        const d = col.data[chunk];
        if (d) {
          const buffers = d.buffers as unknown as (ArrayBuffer | null)[];
          for (const buf of buffers) {
            if (buf) total += buf.byteLength;
          }
        }
      }
    }
  }
  return total || 1024;
}

export class TileCache {
  private cache = new Map<string, CacheEntry>();
  private currentBytes = 0;
  private maxBytes: number;

  constructor(maxBytes = 256 * 1024 * 1024) {
    this.maxBytes = maxBytes;
  }

  get(key: string): Table | undefined {
    const entry = this.cache.get(key);
    if (entry) {
      this.cache.delete(key);
      this.cache.set(key, entry);
      return entry.table;
    }
    return undefined;
  }

  set(key: string, table: Table): void {
    const existing = this.cache.get(key);
    if (existing) {
      this.currentBytes -= existing.bytes;
      this.cache.delete(key);
    }
    const bytes = tableByteSize(table);
    while (this.currentBytes + bytes > this.maxBytes && this.cache.size > 0) {
      this.evict();
    }
    this.cache.set(key, { table, bytes });
    this.currentBytes += bytes;
  }

  has(key: string): boolean {
    return this.cache.has(key);
  }

  delete(key: string): void {
    const entry = this.cache.get(key);
    if (entry) {
      this.currentBytes -= entry.bytes;
      this.cache.delete(key);
    }
  }

  clear(): void {
    this.cache.clear();
    this.currentBytes = 0;
  }

  size(): number {
    return this.cache.size;
  }

  private evict(): void {
    const oldest = this.cache.keys().next().value;
    if (oldest !== undefined) {
      const entry = this.cache.get(oldest);
      if (entry) this.currentBytes -= entry.bytes;
      this.cache.delete(oldest);
    }
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest;

  describe("TileCache", () => {
    it("stores and retrieves entries", () => {
      const cache = new TileCache(10 * 1024 * 1024);
      const mockTable = { numCols: 0, getChildAt: () => null } as unknown as Table;
      cache.set("a", mockTable);
      expect(cache.get("a")).toBe(mockTable);
    });

    it("evicts LRU when over byte budget", async () => {
      const cache = new TileCache(2048);
      const makeTable = () => ({ numCols: 0, getChildAt: () => null }) as unknown as Table;
      const t1 = makeTable();
      const t2 = makeTable();
      const t3 = makeTable();
      cache.set("a", t1);
      await new Promise((r) => setTimeout(r, 5));
      cache.set("b", t2);
      await new Promise((r) => setTimeout(r, 5));
      cache.get("a");
      cache.set("c", t3);
      expect(cache.has("c")).toBe(true);
    });

    it("reports correct size", () => {
      const cache = new TileCache(10 * 1024 * 1024);
      const makeTable = () => ({ numCols: 0, getChildAt: () => null }) as unknown as Table;
      cache.set("x", makeTable());
      cache.set("y", makeTable());
      expect(cache.size()).toBe(2);
      cache.delete("x");
      expect(cache.size()).toBe(1);
    });
  });
}
