# Tableverse

[![CI](https://github.com/sjoerdvink99/tableverse/actions/workflows/release.yml/badge.svg)](https://github.com/sjoerdvink99/tableverse/actions)
[![Release](https://img.shields.io/github/v/release/sjoerdvink99/tableverse)](https://github.com/sjoerdvink99/tableverse/releases/latest)
[![Docker](https://img.shields.io/badge/docker-ghcr.io-blue)](https://github.com/sjoerdvink99/tableverse/pkgs/container/tableverse)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

Inspect any Parquet, CSV, or Arrow file in your browser, no matter the size. No memory limits.

---

## Why not just use...

|                     | pandas                 | DuckDB CLI | VisiData      | **Tableverse**       |
| ------------------- | ---------------------- | ---------- | ------------- | -------------------- |
| Large Parquet files | crashes on large files | needs SQL  | terminal only | **✅ web UI**        |
| Column histograms   | `.describe()`          | ❌         | ✅            | **on hover**         |
| Filter without SQL  | ❌                     | ❌         | ✅            | **click**            |
| No local install    | ❌                     | ❌         | ❌            | **docker one-liner** |

---

## Install

### Docker — no dependencies required

```bash
docker run --rm -v $(pwd):/data -p 8080:8080 \
  ghcr.io/sjoerdvink99/tableverse:latest /data/file.parquet
```

Open [http://localhost:8080](http://localhost:8080).

Works on macOS (including Apple Silicon), Linux, and Windows. Replace `file.parquet` with any Parquet, CSV, Arrow IPC, or NDJSON file.

### Rust

```bash
cargo install tv-cli
```

---

## Quick start

```bash
tableverse serve data.parquet   # opens in browser automatically
tableverse inspect data.parquet   # print column overview to terminal
tableverse profile data.parquet   # output full column stats as JSON
```

`serve` works with local paths and remote URIs:

```bash
tableverse serve s3://my-bucket/dataset.parquet
tableverse serve gs://my-bucket/dataset.parquet
```

---

## What you get

```
$ tableverse inspect orders.parquet

  orders.parquet  18,432,917 rows × 47 columns

  Column             Type        Null%    Min                 Max
  ─────────────────  ──────────  ───────  ──────────────────  ──────────────────
  order_id           Int64         0.0%   1                   18432917
  customer_id        Int64         0.0%   1001                3099234
  status             Utf8          0.2%   —                   —
  amount             Float64       0.8%   0.01                9999.99
  country            Utf8         12.1%   —                   —
  created_at         Timestamp     0.0%   2020-01-01          2024-12-31
```

Schema, null rates, and min/max are read from Parquet footer metadata. No rows are scanned.

---

## Features

- **Tile-based grid**: only visible tiles are fetched and rendered. Scroll through millions of rows at 60 fps with no DOM table elements.
- **Distribution histograms**: hover any column header to see the value distribution. Click or drag a bar to create a range filter. Click the null band to filter on nulls.
- **Correlation matrix**: pairwise Pearson correlations across all numeric columns in a single streaming pass.
- **ViewExpr pipeline**: sort, filter, group by, derive new columns, deduplicate. All ops are composable and reversible. Pipelines build through direct table interaction: click a header to sort, right-click a cell to filter on its value.
- **Export**: generate equivalent SQL, DuckDB Python, Polars, or pandas code from any pipeline state. Download filtered results as Parquet or CSV.
- **Remote files**: pass an `s3://`, `gs://`, or `az://` URI directly. No local copy needed.

---

## Supported formats

| Format                | Read | Write | S3 / GCS / Azure |
| --------------------- | ---- | ----- | ---------------- |
| Parquet               | ✅   | ✅    | ✅               |
| CSV                   | ✅   | —     | ✅               |
| Arrow IPC             | ✅   | —     | ✅               |
| JSON (line-delimited) | ✅   | —     | ✅               |

---

## Architecture

**Tile-based reads.** The grid is divided into tiles of 256 rows × 64 columns. The server fetches only the tile the user is looking at, plus adjacent tiles for prefetch. A 1B-row table never requires a full scan to render.

**Parquet row group pruning.** Filtered queries skip row groups whose min/max metadata excludes the predicate. For selective filters on sorted columns, this means reading a tiny fraction of the file.

**Arrow IPC wire format.** Data moves from Rust to the browser as binary Arrow IPC with no JSON serialization overhead on either end. The browser deserializes directly into typed arrays.

**Materialization cache for stateful ops.** Sorts, group-bys, and deduplicates materialize once into a 2 GB LRU in-memory cache keyed by a hash of the pipeline. Subsequent tile requests are O(1) slices.

```
Browser (React + Canvas)
        │  Arrow IPC (binary)
Axum REST API
        │  in-memory LRU
tv-engine (Parquet / Arrow / CSV / JSON reader)
        │  object_store (local, S3, GCS, Azure)
File system / object storage
```

---

## Build from source

```bash
git clone https://github.com/sjoerdvink99/tableverse
cd tableverse
cd web && bun install && bun run build && cd ..
cargo build --release -p tv-cli
./target/release/tableverse serve path/to/file.parquet
```

Requirements: Rust 1.75+, Bun 1.0+.

---

## License

MIT
