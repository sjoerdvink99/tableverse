pub const DEFAULT_TILE_ROWS: u64 = 256;
pub const DEFAULT_TILE_COLS: usize = 64;

pub fn optimal_tile_rows(rg_rows: u64) -> u32 {
    if rg_rows <= 1024 {
        rg_rows as u32
    } else if rg_rows <= 131_072 {
        (rg_rows / 16) as u32
    } else {
        8192
    }
}

pub fn tile_rows_for_zoom(zoom: f32) -> u64 {
    if zoom >= 1.0 {
        256
    } else if zoom >= 0.5 {
        512
    } else if zoom >= 0.25 {
        1024
    } else if zoom >= 0.1 {
        2048
    } else {
        4096
    }
}

pub fn tile_cols_for_zoom(zoom: f32) -> usize {
    if zoom >= 1.0 {
        64
    } else if zoom >= 0.5 {
        128
    } else {
        256
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileCoord {
    pub row_tile: u64,
    pub col_tile: usize,
}

pub struct ViewportParams {
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub viewport_w: f64,
    pub viewport_h: f64,
    pub cell_w: f64,
    pub cell_h: f64,
    pub tile_rows: u64,
    pub tile_cols: usize,
    pub prefetch: u64,
}

pub fn tile_key(source_id: &str, view_hash: &str, row_tile: u64, col_tile: usize) -> String {
    format!("tt:tile:{source_id}:{view_hash}:{row_tile}:{col_tile}")
}

pub fn tile_range(
    row_tile: u64,
    col_tile: usize,
    tile_rows: u64,
    tile_cols: usize,
    n_rows: u64,
    n_cols: usize,
) -> (u64, usize, u64, usize) {
    let row_start = row_tile * tile_rows;
    let col_start = col_tile * tile_cols;
    let row_end = (row_start + tile_rows).min(n_rows);
    let col_end = (col_start + tile_cols).min(n_cols);
    (
        row_start,
        col_start,
        row_end - row_start,
        col_end - col_start,
    )
}

pub fn visible_tiles(p: &ViewportParams) -> Vec<TileCoord> {
    let tile_px_w = p.cell_w * p.tile_cols as f64;
    let tile_px_h = p.cell_h * p.tile_rows as f64;

    let col_start = ((p.scroll_x / tile_px_w) as i64 - p.prefetch as i64).max(0) as usize;
    let col_end = ((p.scroll_x + p.viewport_w) / tile_px_w).ceil() as usize + p.prefetch as usize;
    let row_start = ((p.scroll_y / tile_px_h) as i64 - p.prefetch as i64).max(0) as u64;
    let row_end = ((p.scroll_y + p.viewport_h) / tile_px_h).ceil() as u64 + p.prefetch;

    let mut tiles = Vec::new();
    for row_tile in row_start..row_end {
        for col_tile in col_start..col_end {
            tiles.push(TileCoord { row_tile, col_tile });
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_range_normal() {
        let (rs, cs, rlen, clen) = tile_range(0, 0, 256, 64, 1000, 200);
        assert_eq!(rs, 0);
        assert_eq!(cs, 0);
        assert_eq!(rlen, 256);
        assert_eq!(clen, 64);
    }

    #[test]
    fn tile_range_clamps_to_bounds() {
        let (rs, cs, rlen, clen) = tile_range(3, 2, 256, 64, 900, 150);
        assert_eq!(rs, 768);
        assert_eq!(cs, 128);
        assert_eq!(rlen, 132);
        assert_eq!(clen, 22);
    }

    #[test]
    fn visible_tiles_basic() {
        let tiles = visible_tiles(&ViewportParams {
            scroll_x: 0.0,
            scroll_y: 0.0,
            viewport_w: 800.0,
            viewport_h: 600.0,
            cell_w: 120.0,
            cell_h: 32.0,
            tile_rows: 256,
            tile_cols: 64,
            prefetch: 0,
        });
        assert!(!tiles.is_empty());
        assert!(tiles.contains(&TileCoord {
            row_tile: 0,
            col_tile: 0
        }));
    }

    #[test]
    fn tile_key_format() {
        let key = tile_key("src1", "abc123", 2, 3);
        assert_eq!(key, "tt:tile:src1:abc123:2:3");
    }

    #[test]
    fn tile_range_zero_rows() {
        let (rs, cs, rlen, clen) = tile_range(0, 0, 256, 64, 0, 0);
        assert_eq!(rlen, 0);
        assert_eq!(clen, 0);
        assert_eq!(rs, 0);
        assert_eq!(cs, 0);
    }

    #[test]
    fn tile_range_single_tile() {
        let (rs, cs, rlen, clen) = tile_range(0, 0, 256, 64, 100, 10);
        assert_eq!(rs, 0);
        assert_eq!(cs, 0);
        assert_eq!(rlen, 100);
        assert_eq!(clen, 10);
    }

    #[test]
    fn visible_tiles_with_prefetch() {
        let tiles = visible_tiles(&ViewportParams {
            scroll_x: 0.0,
            scroll_y: 0.0,
            viewport_w: 800.0,
            viewport_h: 600.0,
            cell_w: 120.0,
            cell_h: 32.0,
            tile_rows: 256,
            tile_cols: 64,
            prefetch: 1,
        });
        assert!(tiles.len() > 1);
    }

    #[test]
    fn visible_tiles_at_scroll_offset() {
        let tiles = visible_tiles(&ViewportParams {
            scroll_x: 64.0 * 120.0,
            scroll_y: 256.0 * 32.0,
            viewport_w: 800.0,
            viewport_h: 600.0,
            cell_w: 120.0,
            cell_h: 32.0,
            tile_rows: 256,
            tile_cols: 64,
            prefetch: 0,
        });
        assert!(tiles.iter().any(|t| t.row_tile >= 1));
        assert!(tiles.iter().any(|t| t.col_tile >= 1));
    }
}
