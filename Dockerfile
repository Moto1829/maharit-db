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

# Data file inside the volume (read by `maharit server`; --data overrides it)
ENV MAHARIT_DATA=/data/maharit.db

# ヘルスチェック: 監視用 HTTP（port 9090）の /health に GET
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -qO- http://localhost:9090/health || exit 1

# Default command (server mode). Bind to all interfaces: 127.0.0.1 inside a
# container is unreachable from outside it.
CMD ["/app/maharit", "server", "--host", "0.0.0.0", "--metrics-bind", "0.0.0.0:9090"]
