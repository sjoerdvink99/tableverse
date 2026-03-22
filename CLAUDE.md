# Tableverse — CLAUDE.md

Tableverse is the go-to high-performance table viewer for data engineers and ML engineers who need to inspect massive datasets (millions of rows, thousands of columns) instantly — no memory limits, no waiting, no sampling. It uses a tile-based rendering architecture: data is divided into spatial tiles and only the visible region is fetched and rendered, enabling seamless pan and scroll through arbitrarily large tables.

## Architecture

```
Browser (React + Canvas) ↔ Server (Rust/Axum) ↔ Engine (DuckDB)
                                   ↓                    ↓
                               In-memory cache     Apache Arrow IPC
                               Redis (optional)
```

**Crates:**
- `tv-core` — shared types, tile coordinate math, error types, Arrow serde helpers
- `tv-engine` — DuckDB-backed query engine: source catalog, connection pool, query builder, column stats
- `tv-server` — Axum REST API: tile serving, caching, route handlers, AppState
- `tv-cli` — Clap-based CLI; standalone `tableverse serve <file>` entrypoint

**Frontend (`web/`):**
- Canvas-based grid rendering (no DOM table elements)
- Zustand stores for table and UI state
- Client-side LRU tile cache + prefetching
- Apache Arrow (arrow-js) for binary IPC deserialization

## Project Structure

```
Tableverse/
├── crates/
│   ├── tv-core/src/         # Tile math, types, error, arrow serde
│   ├── tv-engine/src/       # catalog, pool, query, stats
│   ├── tv-server/src/       # lib, state, cache, routes/
│   └── tv-cli/src/          # main.rs (Clap entrypoint)
├── web/src/
│   ├── components/
│   │   ├── TableViewer/     # GridCanvas, ScrollContainer, headers
│   │   ├── Toolbar/         # SearchBar, ZoomControls
│   │   ├── Overlays/        # Tooltip, ContextMenu, JumpToRow
│   │   └── SourceManager/   # AddSource
│   ├── hooks/               # useKeyboard, useViewport, useTiles, useSelection
│   ├── stores/              # table.ts, ui.ts (Zustand)
│   └── lib/                 # api.ts, tile-manager, tile-cache, viewport, dsl, export, format
├── deployment/dockercompose/
├── docs/
├── examples/
├── Cargo.toml               # Rust workspace
├── docker-compose.yml
├── Makefile
└── .env.example
```

## Tech Stack

| Layer | Technology |
|---|---|
| Backend language | Rust 1.75+ |
| Web framework | Axum 0.8 |
| Query engine | DuckDB (Parquet, CSV, Arrow, JSON) |
| Data transfer | Apache Arrow IPC |
| Caching | In-memory + Redis (Fred 9) |
| Frontend framework | React 19 |
| Build tool | Vite 8 |
| Frontend state | Zustand 5 |
| Arrow client | arrow-js 21 |
| JS runtime | Bun 1.0+ |
| Async runtime | Tokio |
| CLI parsing | Clap 4 |
| Logging | Tracing / Tracing-Subscriber |

Supported data formats: **Parquet** (primary), CSV, Arrow, JSON — all via DuckDB.

## Key Commands

```bash
# Infrastructure (Redis, MinIO)
cd deployment/dockercompose/infrastructure && docker compose up -d

# Backend
make serve                     # cargo run --release
cargo test --all               # run all tests
cargo fmt && cargo clippy      # format + lint

# Frontend
cd web && bun install
bun run dev                    # Vite dev server (hot reload)
bun run build                  # production build

# Full stack
make docker-up                 # Docker Compose full stack
make example                   # generate sample tiles
```

## Core Patterns

### Tile System
- Each tile is addressed by `(row_offset, col_offset, rows, cols)` — defaults 256 rows × 64 columns
- Cache key is a hash of `(source_id, tile_coords, view_descriptor)` where view = sort + filter state
- Server uses cache-aside: check in-memory → check Redis → query DuckDB
- Client prefetches adjacent tiles before they enter the viewport

### Data Flow (scroll event → rendered cell)
1. Viewport math determines which tiles are visible
2. `useTiles` requests missing tiles from the server
3. Server checks in-memory cache by view hash
4. On miss: `tv-engine` builds a SQL query against the DuckDB view and executes it
5. Result serialized as Arrow IPC binary and cached
6. Client deserializes Arrow binary and hands the `Table` to `GridCanvas`
7. `GridCanvas` draws cells on a `<canvas>` via 2D context

### Query Model
- Each registered source becomes a DuckDB view (managed in `tv-engine/catalog.rs`)
- SQL queries are constructed dynamically in `tv-engine/query.rs` with row/col offsets, sort, and filter
- Arrow IPC is the wire format — no JSON for data payloads

### Statistics
- Computed on demand per column: min, max, mean, null count, distinct count, histogram
- Triggered by the `GET /api/v1/sources/{id}/columns/{idx}/stats` endpoint

## ViewExpr DSL

`ViewExpr` is the serializable intermediate representation of a data pipeline. It compiles to a DuckDB CTE chain at query time.

```typescript
type ViewExpr = { source_id: string; ops: ViewOp[]; };
```

### ViewOp variants

| type | fields | description |
|---|---|---|
| `filter` | `predicate: Predicate` | Keep rows matching the predicate tree |
| `select` | `columns: string[]` | Keep only these columns |
| `drop` | `columns: string[]` | Remove these columns |
| `sort` | `keys: SortKey[]` | Sort rows — always compiled last |
| `derive` | `name: string, expr: ScalarExpr` | Add a computed column |
| `deduplicate` | `columns: string[] \| null` | Remove duplicate rows |
| `sample` | `n: number, strategy: "bernoulli"\|"system", seed?` | Random sample |
| `group_by` | `keys: string[], aggs: AggExpr[]` | Aggregate by key columns |

### Predicate

Leaf predicates: `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `between`, `in`, `not_in`, `contains`, `starts_with`, `ends_with`, `regex`, `is_null`, `is_not_null`

Combinators: `and { exprs }`, `or { exprs }`, `not { expr }`

Literals: `null | boolean | number | string`

### ScalarExpr (for derive ops)

`column`, `literal`, `bin_op` (add/sub/mul/div/mod), `abs`, `round`, `floor`, `ceil`, `upper`, `lower`, `trim`, `length`, `substr`, `concat`, `year`, `month`, `day`, `case`, `coalesce`, `rank`, `ntile`, `cast`

### AggExpr (for group_by ops)

`count`, `count_distinct`, `sum`, `min`, `max`, `mean`, `median`, `std_dev`, `percentile` — each requires an `alias` field.

### CTE Compiler

- Each `ViewOp` → one CTE step (`step_0`, `step_1`, …)
- `sort` is always reordered to last before compilation
- Tile queries: `LIMIT rows OFFSET row` appended to final SELECT
- Count queries: `SELECT COUNT(*) FROM (<chain>)`
- Download queries: full chain piped to DuckDB `COPY ... TO`

### View Hash

16-char hex FNV-1a over canonical JSON of `ops`. Used as tile cache key dimension — ops change → new hash → old tiles automatically ignored.

### Interaction → DSL flow

All operations are created through direct table interaction, never via form builders:

- **Column header click** → sort asc → desc → clear
- **Column header shift+click** → multi-sort
- **Column header right-click** → ColumnContextMenu (sort, hide, group by, derive)
- **Column header hover** → distribution popover (histogram click/drag → between filter; null band → is_null)
- **Cell right-click** → CellContextMenu (eq / ne / gt / lt / not_in / is_null / copy)
- **PipelineBar** → read-only op chips with × remove

### Export

- `POST /query/export` — returns code string (SQL, DuckDB Python, Polars, Pandas)
- `GET /query/download?format=parquet|csv&view_expr=<base64>` — file download

## API Endpoints

```
GET    /healthz
GET    /api/v1/sources
POST   /api/v1/sources
GET    /api/v1/sources/:id
DELETE /api/v1/sources/:id
GET    /api/v1/sources/:id/tiles?row=&col=&rows=&cols=&sort=&filter=
POST   /api/v1/sources/:id/query/tiles          { view_expr, row, col, rows?, cols? } → Arrow IPC
POST   /api/v1/sources/:id/query/count          { view_expr } → { count }
POST   /api/v1/sources/:id/query/schema         { view_expr } → { columns }
POST   /api/v1/sources/:id/query/export         { view_expr, format } → { code }
GET    /api/v1/sources/:id/query/download       ?format=parquet|csv&view_expr=<base64>
GET    /api/v1/sources/:id/columns/:idx/stats
POST   /api/v1/sources/:id/search
```

## Coding Conventions

- **No comments** in code — self-documenting names only
- **Rust**: follow `cargo fmt` defaults; all warnings treated as errors in CI; use `thiserror` for error types
- **TypeScript**: strict mode, no `any`, no implicit returns
- **No ORM** — raw SQL built in `tv-engine/query.rs`
- **No Redux** — Zustand only
- **No DOM table elements** — Canvas rendering only for the grid
- Errors propagate via typed error enums (`tv-core/error.rs`, crate-local `error.rs` files)
- AppState is shared via Axum `Extension` / `State` — no globals

## Environment Variables

See `.env.example`. Key vars:

```
SERVER_PORT=3000
REDIS_URL=redis://localhost:6379
MINIO_ENDPOINT=http://localhost:9000
MINIO_ACCESS_KEY=...
MINIO_SECRET_KEY=...
TILE_CACHE_TTL_SECS=3600
```

## Testing

- Unit tests live alongside source files (`#[cfg(test)]` modules in Rust)
- Integration tests in `tests/` directories per crate
- `cargo test --all` runs everything
- Frontend: `bun run test` (Vitest)

## Product Vision

Tableverse targets data engineers and ML engineers as users. The north-star metric is **time to insight**: a user should be able to drop a 1B-row Parquet file and start inspecting real cells in under 3 seconds. Every architectural decision — tile caching, Arrow IPC, canvas rendering, DuckDB — exists to serve this goal.
