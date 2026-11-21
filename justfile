# List repository recipes (default).
[default]
help:
    just --list

# Run the development CLI; pass arguments directly, or omit them for CLI help.
[positional-arguments]
run *args:
    cargo run --locked -- "$@"

# Regenerate assets/polyfw.schema.json from the current Rust model.
schema:
    just run schema --output assets/polyfw.schema.json

# Check formatting, Clippy and tests.
check:
    cargo fmt --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    cargo test --locked --no-default-features --test library

# Run library unit tests, API integration tests and doctests with default features disabled.
test-lib:
    cargo test --locked --no-default-features --lib --test library
    cargo test --locked --no-default-features --doc

# Run binary unit tests and CLI integration tests.
test-cli:
    cargo test --locked --bin polyfw --test standalone --test project_root --test platforms

# Run the demo project tests with optional test harness arguments.
[positional-arguments]
test-demos *args:
    cargo test --locked --test demos -- "$@"

# Format Rust sources and apply Clippy fixes; review the resulting diff.
fix:
    cargo fmt
    cargo clippy --locked --all-targets --fix --allow-dirty --allow-staged

# Clean build artifacts and remove example build directories.
clean:
    cargo clean
    rm -rf dist
    rm -rf examples/*/target
    rm -rf examples/*/.polyfw-state
