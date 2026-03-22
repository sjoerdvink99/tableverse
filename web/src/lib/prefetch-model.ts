const HISTORY_LEN = 20;
const PREFETCH_CANDIDATES = 3;

type TileCoord = { row: number; col: number };

export class PrefetchModel {
  private history: TileCoord[] = [];
  private transitions = new Map<string, Map<string, number>>();

  record(row: number, col: number): void {
    const coord = { row, col };
    if (this.history.length >= 2) {
      const prev2 = this.history[this.history.length - 2]!;
      const prev1 = this.history[this.history.length - 1]!;
      const stateKey = `${prev2.row},${prev2.col}|${prev1.row},${prev1.col}`;
      const nextKey = `${row},${col}`;
      const map = this.transitions.get(stateKey) ?? new Map<string, number>();
      map.set(nextKey, (map.get(nextKey) ?? 0) + 1);
      this.transitions.set(stateKey, map);
    }
    this.history.push(coord);
    if (this.history.length > HISTORY_LEN) {
      this.history.shift();
    }
  }

  predict(): TileCoord[] {
    if (this.history.length < 2) return [];
    const prev2 = this.history[this.history.length - 2]!;
    const prev1 = this.history[this.history.length - 1]!;
    const stateKey = `${prev2.row},${prev2.col}|${prev1.row},${prev1.col}`;
    const map = this.transitions.get(stateKey);
    if (!map) return this.fallbackPredictions();
    const sorted = [...map.entries()].sort((a, b) => b[1] - a[1]);
    return sorted.slice(0, PREFETCH_CANDIDATES).map(([key]) => {
      const [r, c] = key.split(",").map(Number);
      return { row: r!, col: c! };
    });
  }

  private fallbackPredictions(): TileCoord[] {
    if (this.history.length === 0) return [];
    const last = this.history[this.history.length - 1]!;
    if (this.history.length >= 2) {
      const prev = this.history[this.history.length - 2]!;
      const dr = last.row - prev.row;
      const dc = last.col - prev.col;
      if (dr !== 0 || dc !== 0) {
        return [
          { row: last.row + dr, col: last.col + dc },
          { row: last.row + 2 * dr, col: last.col + 2 * dc },
          { row: last.row + dr, col: last.col },
        ];
      }
    }
    return [
      { row: last.row + 1, col: last.col },
      { row: last.row - 1, col: last.col },
      { row: last.row, col: last.col + 1 },
    ];
  }

  reset(): void {
    this.history = [];
    this.transitions.clear();
  }
}
