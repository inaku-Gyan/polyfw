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

# Run example integration tests; pass test-name filters and test-runner options directly.
[positional-arguments]
test-examples *args:
    cargo test --locked --test examples -- "$@"

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
