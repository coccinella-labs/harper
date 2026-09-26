#!/bin/bash

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright 2026 coccinella-labs

set -euo pipefail

echo "Running comprehensive Harper tests..."

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    print_error "Cargo is not installed. Please install Rust first."
    exit 1
fi

print_status "Checking Rust toolchain..."
rustc --version
cargo --version

print_status "Updating dependencies..."
cargo update

print_status "Checking code formatting..."
if ! cargo fmt --all -- --check; then
    print_error "Code formatting check failed. Run 'cargo fmt' to fix."
    exit 1
fi
print_success "Code formatting is correct"

print_status "Running Clippy lints..."
if ! cargo clippy --all-targets --all-features --workspace -- -D warnings; then
    print_error "Clippy lints failed"
    exit 1
fi
print_success "Clippy lints passed"

print_status "Checking documentation..."
if ! cargo doc --no-deps --document-private-items --all-features --workspace --quiet; then
    print_error "Documentation check failed"
    exit 1
fi
print_success "Documentation check passed"

print_status "Running unit tests..."
if ! cargo test --lib --all-features --workspace --verbose; then
    print_error "Unit tests failed"
    exit 1
fi
print_success "Unit tests passed"

print_status "Running integration tests..."
if ! cargo test --tests --all-features --workspace --verbose; then
    print_error "Integration tests failed"
    exit 1
fi
print_success "Integration tests passed"

print_status "Building in debug mode..."
if ! cargo build --all-features --workspace; then
    print_error "Debug build failed"
    exit 1
fi
print_success "Debug build successful"

print_status "Building in release mode..."
if ! cargo build --release --all-features --workspace; then
    print_error "Release build failed"
    exit 1
fi
print_success "Release build successful"

print_status "Testing release build..."
if ! cargo test --release --all-features --workspace; then
    print_error "Release tests failed"
    exit 1
fi
print_success "Release tests passed"

# Security audit (optional - install if not present)
if command -v cargo-audit &> /dev/null; then
    print_status "Running security audit..."
    if ! cargo audit; then
        print_warning "Security audit found issues"
    else
        print_success "Security audit passed"
    fi
else
    print_warning "cargo-audit not installed. Run 'cargo install cargo-audit' to enable security checks."
fi

# Dependency check (optional - install if not present)
if command -v cargo-deny &> /dev/null; then
    print_status "Running dependency checks..."
    if ! cargo deny check; then
        print_warning "Dependency check found issues"
    else
        print_success "Dependency check passed"
    fi
else
    print_warning "cargo-deny not installed. Run 'cargo install cargo-deny' to enable dependency checks."
fi

print_success "All tests completed successfully!"
print_status "Harper is ready for deployment."
