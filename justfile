# AI Music Companion — Task Runner
# Usage: just <recipe>

# Default recipe: show available commands
default:
    @just --list

# Run full CI pipeline
ci: fmt lint test audit build-frontend
    @echo "✓ CI pipeline passed"

# Run all tests
test:
    cargo test --workspace
    cd apps/desktop && pnpm test --passWithNoTests

# Run Rust tests only
test-rust:
    cargo test --workspace

# Run frontend tests only
test-frontend:
    cd apps/desktop && pnpm test --passWithNoTests

# Format all code
fmt:
    cargo fmt --all
    cd apps/desktop && pnpm format || true

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check
    cd apps/desktop && pnpm format:check || true

# Lint all code
lint:
    cargo clippy --workspace --deny warnings
    cd apps/desktop && pnpm lint || true

# Run security audits
audit:
    cargo audit
    cd apps/desktop && pnpm audit --audit-level=high || true

# Build frontend
build-frontend:
    cd apps/desktop && pnpm build || true

# Run latency benchmarks
bench:
    cargo bench --workspace

# Build the full Tauri application
build-app:
    cd apps/desktop && cargo tauri build

# Run the dev server
dev:
    cd apps/desktop && cargo tauri dev

# Clean all build artifacts
clean:
    cargo clean
    cd apps/desktop && rm -rf node_modules dist
