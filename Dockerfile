# Build stage
FROM rust:1.88-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build the application
RUN cargo build --release --package maharit-server

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies (including wget for HEALTHCHECK)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r maharit && useradd -r -g maharit maharit

# Create data directory
RUN mkdir -p /data && chown maharit:maharit /data

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/maharit /app/maharit

# Set ownership
RUN chown maharit:maharit /app/maharit

# Switch to non-root user
USER maharit

# Data volume
VOLUME ["/data"]

# Expose TCP server port and metrics/health HTTP port
EXPOSE 7687
EXPOSE 9090

# Environment variables
ENV MAHARIT_DATA_DIR=/data

# ヘルスチェック: /health エンドポイントに HTTP GET
# /health エンドポイントは Task 25 (metrics) で実装済み (port 9090)
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -qO- http://localhost:9090/health || exit 1

# Default command (server mode)
CMD ["/app/maharit", "server"]
