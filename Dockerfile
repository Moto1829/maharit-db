# Build stage
FROM rust:1.84-slim-bookworm AS builder

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

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
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

# Environment variables
ENV MAHARIT_DATA_DIR=/data

# Default command (REPL mode)
ENTRYPOINT ["/app/maharit"]
