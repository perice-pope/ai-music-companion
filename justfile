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
    cd apps/dashboard && pnpm test
    bash va-testing-kit/skills/test-app/scripts/run.test.sh

# Run Rust tests only
test-rust:
    cargo test --workspace

# Run frontend tests only
test-frontend:
    cd apps/desktop && pnpm test --passWithNoTests
    cd apps/dashboard && pnpm test

# Format all code
fmt:
    cargo fmt --all
    cd apps/desktop && pnpm format || true
    cd apps/dashboard && pnpm format || true

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check
    cd apps/desktop && pnpm format:check || true
    cd apps/dashboard && pnpm format:check || true

# Lint all code (same clippy form as CI — current clippy dropped the
# old `--deny warnings` wrapper flag)
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    cd apps/desktop && pnpm lint || true
    cd apps/dashboard && pnpm lint || true

# Run security audits (both lockfiles — the tauri shell is its own workspace, #525)
audit:
    cargo audit
    cargo audit --file apps/desktop/src-tauri/Cargo.lock
    cd apps/desktop && pnpm audit --audit-level=high || true
    cd apps/dashboard && pnpm audit --audit-level=high || true

# Build frontend
build-frontend:
    cd apps/desktop && pnpm build || true
    cd apps/dashboard && pnpm build || true

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
    cd apps/dashboard && rm -rf node_modules dist
