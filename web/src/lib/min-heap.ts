export class MinHeap<T> {
  private data: Array<{ score: number; item: T }> = [];

  get size(): number {
    return this.data.length;
  }

  push(score: number, item: T): void {
    this.data.push({ score, item });
    this.bubbleUp(this.data.length - 1);
  }

  pop(): T | undefined {
    if (this.data.length === 0) return undefined;
    const top = this.data[0]!.item;
    const last = this.data.pop()!;
    if (this.data.length > 0) {
      this.data[0] = last;
      this.sinkDown(0);
    }
    return top;
  }

  peek(): T | undefined {
    return this.data[0]?.item;
  }

  clear(): void {
    this.data = [];
  }

  private bubbleUp(i: number): void {
    while (i > 0) {
      const parent = (i - 1) >>> 1;
      if (this.data[parent]!.score <= this.data[i]!.score) break;
      [this.data[parent], this.data[i]] = [this.data[i]!, this.data[parent]!];
      i = parent;
    }
  }

  private sinkDown(i: number): void {
    const n = this.data.length;
    while (true) {
      let smallest = i;
      const l = 2 * i + 1;
      const r = 2 * i + 2;
      if (l < n && this.data[l]!.score < this.data[smallest]!.score) smallest = l;
      if (r < n && this.data[r]!.score < this.data[smallest]!.score) smallest = r;
      if (smallest === i) break;
      [this.data[smallest], this.data[i]] = [this.data[i]!, this.data[smallest]!];
      i = smallest;
    }
  }
}
