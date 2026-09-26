# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright 2026 coccinella-labs

# Build stage
FROM rust:1.92.0 AS builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy workspace crates
COPY lib ./lib

# Copy root crate source
COPY src ./src

# Build application
RUN cargo build --release -p harper-ui --bin harper

# Test stage
FROM rust:1.92.0 AS tester

WORKDIR /app

# Install system dependencies first to leverage Docker layer caching.
RUN apt-get update && apt-get install -y --no-install-recommends libsqlite3-dev pkg-config && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Reuse build artifacts from the builder stage to avoid re-downloading dependencies.
COPY --from=builder /usr/local/cargo/registry /usr/local/cargo/registry

# Copy config files needed for tests
COPY config ./config

# Copy workspace and tests
COPY lib ./lib
COPY src ./src
COPY tests ./tests

    # Run E2E tests in debug mode as some tests may not work in release.
    RUN cargo test --workspace --tests

# Runtime stage
FROM debian:bookworm-slim

# Install ca-certificates and bubblewrap for sandbox isolation
RUN apt-get update && apt-get install -y ca-certificates bubblewrap && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy config files
COPY config ./config

# Copy the binary from builder stage
COPY --from=builder /app/target/release/harper .

# Set the binary as executable
RUN chmod +x harper

# Create and switch to a non-root user
RUN useradd --create-home appuser
RUN chown -R appuser:appuser /app
USER appuser

# Set environment variables (can be overridden)
ENV DATABASE_PATH=/app/data/harper.db

EXPOSE 8080

# Set the entrypoint
ENTRYPOINT ["./harper"]
