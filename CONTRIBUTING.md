# Contributing to Tableverse

## Prerequisites

| Tool | Version |
|------|---------|
| Rust | 1.75+ |
| Bun  | 1.0+   |
| Python | 3.8+ (optional, for Python package work) |

## Getting started

```bash
git clone https://github.com/sjoerdvink99/tableverse
cd tableverse

# Install git hooks
make install-hooks

# Build everything
make build

# Run the dev stack (two terminals)
make serve          # backend on :8080, FILE=examples/data/sheet1.parquet
make dev-web        # Vite dev server on :5173 with hot reload
```

The backend embeds the frontend via `rust-embed`. For development, Vite proxies `/api` to `:8080`, so the dev server hot-reloads the frontend while talking to a real backend.

## Project structure

```
crates/
  tv-core/     Shared types, tile math, ViewExpr DSL, optimizer
  tv-engine/   Parquet reader, Arrow executor, stats, indexes, materializer
  tv-server/   Axum routes, tile cache, AppState
  tv-cli/      Clap entrypoint — the `tableverse` binary
  tv-bench/    Criterion benchmarks
web/
  src/
    components/   Canvas grid, headers, overlays, panels
    hooks/        Data fetching, zoom, navigation, selection
    lib/          API client, tile manager, rendering, ViewExpr helpers
    stores/       Zustand stores (table, ui, view, stats, bookmarks)
py/
  tableverse/   Python wrapper — show(), inspect(), Jupyter magic
```

## Development workflow

### Backend changes

```bash
cargo build          # debug build (fast)
cargo test --all     # run all tests
cargo clippy --all -- -D warnings   # must pass with zero warnings
cargo fmt --all      # format
```

Tests live alongside source files in `#[cfg(test)]` modules and in `crates/*/tests/`.

### Frontend changes

```bash
cd web
bun install
bun run dev          # hot-reload dev server
bunx tsc --noEmit    # type check
bun run lint         # ESLint
bun run format       # Prettier
```

### Python package changes

```bash
cd py
# Syntax check
python -m py_compile tableverse/__init__.py tableverse/_cli.py tableverse/magic.py
```

The Python package is a thin wrapper around the `tableverse` binary. To test it end-to-end, build the binary first (`cargo build --release -p tv-cli`), put it on PATH, then call `tv.show()`.

## Code conventions

These are enforced in CI and reviewed in PRs.

**Rust**
- Zero comments — names must be self-documenting
- All warnings treated as errors (`-D warnings` in clippy)
- Use `thiserror` for error types; propagate with `?`
- No `unwrap()` outside of tests — use `expect("reason")` if you must
- `async` at the route/engine boundary; synchronous Arrow compute inside

**TypeScript**
- Strict mode, no `any`, no implicit returns
- State via Zustand only — no Context, no Redux
- Grid rendering via Canvas only — no DOM `<table>` elements
- No `console.log` in committed code

**General**
- Do not add features or refactor code beyond the scope of the PR
- Do not add comments, docstrings, or type annotations to code you did not change
- Keep solutions simple — three similar lines is better than a premature abstraction

## Architecture notes for contributors

**Tile system** — Data is divided into `(row_offset, col_offset, rows, cols)` tiles. The client requests only visible tiles; the server checks an LRU cache before executing. Tile cache keys are `(source_id, tile_coords, view_hash)` where `view_hash` is an FNV-1a hash of the serialised `ViewExpr`.

**ViewExpr** — The pipeline DSL. Each op (`filter`, `sort`, `group_by`, `derive`, `deduplicate`, `sample`) becomes a step in the executor. The optimizer runs 5 passes before execution. When adding a new op, touch: `tv-core/src/expr.rs`, `tv-core/src/optimizer.rs`, `tv-engine/src/executor.rs`, `tv-engine/src/compiler/`, `tv-engine/src/export/`.

**Materializer** — Ops that require a full scan (sort, group_by, deduplicate, sample, top_k) materialise the result once and serve tiles from the materialised view. Large materialised views spill to disk via `spill_pipeline.rs`.

**Semantic zoom** — Five render modes keyed by zoom level: `satellite` (<0.10), `profile` (0.10–0.28), `heatmap` (0.28–0.55), `scan` (0.55–0.85), `read` (>0.85). Rendering logic lives in `web/src/lib/semantic-render.ts` and `profile-render.ts`.

## Submitting a pull request

1. Fork the repository and create a branch from `main`
2. Make your changes — keep the diff focused on one thing
3. Ensure `cargo fmt --all`, `cargo clippy --all -- -D warnings`, and `cargo test --all` all pass
4. Ensure `bunx tsc --noEmit` passes
5. Open a PR with a clear title and a short description of *why* the change is needed
6. Link any related issues

PRs that touch the tile protocol, ViewExpr schema, or public API should include a brief description of the wire format impact.
