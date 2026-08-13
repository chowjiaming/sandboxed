# justfile — task runner for sandboxed. Run `just` to list recipes.

# List all available recipes
default:
    @just --list

# Run the pure-Rust unit tests
test:
    cargo test

# Lint with warnings as errors
lint:
    cargo fmt --check
    cargo clippy -- -D warnings

# Auto-fix formatting, then lint
fix:
    cargo fmt
    cargo clippy --fix --allow-dirty

# Build the WASM package into pkg/
build:
    wasm-pack build --target web

# Build + serve locally at http://localhost:8080
serve: build
    miniserve . --index index.html

# Run the full verification suite (what CI runs)
verify: lint test build
    @echo "All checks passed."

# Remove build artifacts (target/ and pkg/)
clean:
    cargo clean
    rm -rf pkg