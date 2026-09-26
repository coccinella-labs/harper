# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright 2026 coccinella-labs

# Harper task runner
# Run `just --list` to see all recipes

set dotenv-filename := ".env"

# Default recipe
default: build

# === Build Commands ===

# Build all targets
build:
    cargo build --all-features --workspace

# Build release
build-release:
    cargo build --release --all-features --workspace

# Build single package
build-package package:
    cargo build --release -p {{package}}

# === Test Commands ===

# Run all tests
test:
    cargo test --all-features --workspace

# Run tests with ignored tests
test-all:
    cargo test --all-features --workspace -- --include-ignored

# Run tests with output
test-verbose:
    cargo test --all-features --workspace -- --nocapture --test-threads=1

# === Lint Commands ===

# Run clippy
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Check formatting
fmt:
    cargo fmt --all -- --check

# Format code
fmt-fix:
    cargo fmt --all

# === Doc Commands ===

# Build documentation
docs:
    cargo doc --no-deps --document-private-items --all-features --workspace

# === Development Commands ===

# Run harper binary
run *args:
    cargo run --all-features --bin harper -- {{args}}

# Run harper with TUI only
run-tui:
    cargo run --all-features --bin harper -- --no-server

# Watch and run on file changes
watch:
    cargo watch -x build -x test

# === Release Commands ===

# Generate changelog
changelog:
    git-cliff --config cliff.toml --output CHANGELOG.md

# Generate changelog for unreleased changes only
changelog-unreleased:
    git-cliff --config cliff.toml --output CHANGELOG.md --unreleased

# Bump version (patch by default)
bump +version="patch":
    cargo release bump {{version}} --execute --no-push --no-verify

# === CI Commands ===

# Run full validation
validate:
    bash scripts/validate.sh

# Run security audit
audit:
    cargo deny check

# Run benchmarks
bench:
    cargo bench --all --profile release

# === Bazel Commands ===

# Build with Bazel
bazel-build:
    bazel build :harper_bin

# Build with Bazel and repin
bazel-repin:
    CARGO_BAZEL_REPIN=true bazel build :harper_bin

# Test with Bazel
bazel-test:
    bazel test //...

# === Utility Commands ===

# Clean build artifacts
clean:
    cargo clean && bazel shutdown 2>/dev/null || true

# Update dependencies
update:
    cargo update

# Update lockfiles (cargo + bazel)
update-lockfiles:
    cargo update
    CARGO_BAZEL_REPIN=true bazel build :harper_bin 2>/dev/null || true

# Show dependency tree
deps:
    cargo tree

# Count lines of code
loc:
    find . -name '*.rs' -not -path '*/target/*' -exec wc -l {} + | tail -1

# === Docker Commands ===

# Build Docker image
docker-build:
    docker build -t harper:test .

# Run Docker image
docker-run:
    docker run --rm harper:test harper --version
