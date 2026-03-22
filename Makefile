.PHONY: build build-dev test clean serve dev-web lint format install-hooks docker-build docker-run docker-up docker-down docker-logs bench bench-data bench-tv bench-competitors bench-report bench-viz

FILE ?= examples/data/sheet1.parquet

BENCH_SCALES ?= 1m 10m

build:
	cd web && bun install && bun run build
	cargo build --release

build-dev:
	cargo build

test:
	cargo test --all

clean:
	cargo clean
	rm -rf web/node_modules web/dist

serve:
	RUST_LOG=info cargo run -p tv-cli -- serve $(FILE)

dev-web:
	cd web && bun install && bun run dev

lint:
	cargo clippy --all-targets --all-features -- -D warnings

format:
	cargo fmt --all
	cd web && bun run prettier --write .

install-hooks:
	git config core.hooksPath .githooks

PORT ?= 8080

docker-build:
	docker compose -f deployment/docker-compose.yml build

docker-run:
	docker run --rm -p $(PORT):8080 -v $(dir $(abspath $(FILE))):/data:ro tableverse /data/$(notdir $(FILE))

docker-up:
	docker compose -f deployment/docker-compose.yml up -d

docker-down:
	docker compose -f deployment/docker-compose.yml down

docker-logs:
	docker compose -f deployment/docker-compose.yml logs -f
