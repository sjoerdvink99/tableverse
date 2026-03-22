import type { CellAddress, TileCoord, Viewport } from "./types";

export const TILE_ROWS = 256;
export const TILE_COLS = 64;
export const DEFAULT_CELL_W = 140;
export const DEFAULT_CELL_H = 32;
export const HEADER_HEIGHT = 100;
export const ROW_HEADER_W = 60;
export const SCROLLBAR_SIZE = 14;

export function tileRowsForZoom(zoom: number): number {
  if (zoom >= 1.0) return 256;
  if (zoom >= 0.5) return 512;
  if (zoom >= 0.25) return 1024;
  if (zoom >= 0.1) return 2048;
  return 4096;
}

export function tileColsForZoom(zoom: number): number {
  if (zoom >= 1.0) return 64;
  if (zoom >= 0.5) return 128;
  return 256;
}

export function visibleTiles(
  viewport: Viewport,
  cellW: number,
  cellH: number,
  zoom = 1,
  prefetch = 1
): TileCoord[] {
  const tileCols = tileColsForZoom(zoom);
  const tileRows = tileRowsForZoom(zoom);
  const tilePixelW = cellW * tileCols;
  const tilePixelH = cellH * tileRows;

  const colStart = Math.max(0, Math.floor(viewport.scrollX / tilePixelW) - prefetch);
  const colEnd = Math.ceil((viewport.scrollX + viewport.width) / tilePixelW) + prefetch;
  const rowStart = Math.max(0, Math.floor(viewport.scrollY / tilePixelH) - prefetch);
  const rowEnd = Math.ceil((viewport.scrollY + viewport.height) / tilePixelH) + prefetch;

  const tiles: TileCoord[] = [];
  for (let row = rowStart; row < rowEnd; row++) {
    for (let col = colStart; col < colEnd; col++) {
      tiles.push({ row, col });
    }
  }
  return tiles;
}

export function tileKey(row: number, col: number, viewHash = ""): string {
  return `${viewHash}:${row}:${col}`;
}

export function cellAtPixel(
  px: number,
  py: number,
  scrollX: number,
  scrollY: number,
  cellW: number,
  cellH: number
): CellAddress {
  const col = Math.floor((px + scrollX) / cellW);
  const row = Math.floor((py + scrollY) / cellH);
  return { row: Math.max(0, row), col: Math.max(0, col) };
}

export function viewportCenterTile(
  viewport: Viewport,
  cellW: number,
  cellH: number,
  zoom = 1
): { row: number; col: number } {
  const tileRows = tileRowsForZoom(zoom);
  const tileCols = tileColsForZoom(zoom);
  const centerX = viewport.scrollX + viewport.width / 2;
  const centerY = viewport.scrollY + viewport.height / 2;
  return {
    row: Math.max(0, Math.floor(centerY / (cellH * tileRows))),
    col: Math.max(0, Math.floor(centerX / (cellW * tileCols))),
  };
}

export function scrollBounds(nRows: number, nCols: number, cellW: number, cellH: number, vw: number, vh: number) {
  return {
    maxX: Math.max(0, nCols * cellW - vw),
    maxY: Math.max(0, nRows * cellH - vh),
  };
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest;

  describe("visibleTiles", () => {
    it("returns tile at origin for zero scroll", () => {
      const tiles = visibleTiles({ scrollX: 0, scrollY: 0, width: 800, height: 600 }, 140, 32, 1, 0);
      expect(tiles).toContainEqual({ row: 0, col: 0 });
    });

    it("returns multiple tiles when viewport is large", () => {
      const tiles = visibleTiles({ scrollX: 0, scrollY: 0, width: 2000, height: 2000 }, 140, 32, 1, 0);
      expect(tiles.length).toBeGreaterThan(1);
    });

    it("returns fewer tiles at low zoom due to larger tile dims", () => {
      const tilesNormal = visibleTiles({ scrollX: 0, scrollY: 0, width: 800, height: 600 }, 140, 32, 1, 0);
      const tilesZoomedOut = visibleTiles({ scrollX: 0, scrollY: 0, width: 800, height: 600 }, 14, 3.2, 0.1, 0);
      expect(tilesZoomedOut.length).toBeLessThanOrEqual(tilesNormal.length);
    });
  });

  describe("cellAtPixel", () => {
    it("maps pixel to cell correctly", () => {
      const cell = cellAtPixel(150, 64, 0, 0, 140, 32);
      expect(cell).toEqual({ row: 2, col: 1 });
    });
  });

  describe("tileRowsForZoom", () => {
    it("returns 256 at zoom 1", () => expect(tileRowsForZoom(1)).toBe(256));
    it("returns 512 at zoom 0.5", () => expect(tileRowsForZoom(0.5)).toBe(512));
    it("returns 4096 at zoom 0.05", () => expect(tileRowsForZoom(0.05)).toBe(4096));
  });
}
