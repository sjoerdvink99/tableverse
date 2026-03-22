FROM oven/bun:1 AS frontend
WORKDIR /app/web
COPY web/package.json web/bunfig.toml ./
RUN bun install
COPY web/ ./
RUN bunx vite build

FROM rust:1.85 AS builder
WORKDIR /app
COPY . .
COPY --from=frontend /app/web/dist ./web/dist
RUN cargo build -p tv-cli --release

FROM gcr.io/distroless/cc-debian12
LABEL org.opencontainers.image.title="Tableverse" \
      org.opencontainers.image.description="High-performance table viewer for massive datasets" \
      org.opencontainers.image.source="https://github.com/sjoerdvink99/tableverse" \
      org.opencontainers.image.licenses="MIT"
WORKDIR /app
COPY --from=builder /app/target/release/tableverse /usr/local/bin/tableverse
ENV RUST_LOG=info
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/tableverse", "serve", "--port", "8080", "--no-open"]
