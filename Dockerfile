FROM rust:slim AS builder

WORKDIR /app
COPY . .

RUN cargo build --release

# ✅ Reuse the exact same image for runtime
FROM debian:bookworm-slim

WORKDIR /app
COPY --from=builder /app/target/release/prom_otel /usr/local/bin/app

EXPOSE 8888
CMD ["app"]

