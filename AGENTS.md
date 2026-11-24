# Repository instructions

## Load context for the task

- Before changing the Rust API, commands, selection, profiles, hardware behavior or JSON output,
  read the relevant sections of [Usage](docs/usage.md) and [Design](docs/design.md).
- Before changing configuration validation, Cargo resolution or dependency scopes, read
  [Project configuration](docs/configuration.md) and [Design](docs/design.md).
- Before changing test fixtures, generated documentation or verification commands, read
  [Development](docs/development.md).
- Before changing versions, packaging, release scripts/workflows or publishing, read
  [Releasing](docs/releasing.md).

## Common development commands

Run from the repository root. See [Development](docs/development.md#set-up-a-checkout) for
toolchain and fixture setup.

| Command | Purpose |
| --- | --- |
| `just check` | Run formatting checks, Clippy and tests |
| `just fix` | Rewrite sources with formatting and Clippy fixes; review the diff |
| `just schema` | Regenerate `assets/polyfw.schema.json` after model/schema or package-version changes; commit the result |
| `cargo test --locked --bin plfw args::tests` | Run focused parser tests |
| `cargo test --locked --test standalone` | Run project-free CLI and external-tool boundary tests |
| `just run help init` | Inspect a subcommand's help; replace `init` as needed |
| `just run --project-root /path/to/firmware-demos info` | Inspect a consuming project with the development binary |

## Preserve repository invariants

- Keep the bin crate in `src/cli/`. It calls the library's public interface in `src/lib.rs`.
- Construct Cargo children through `cargo::runner::workspace_cargo` / `workspace_command` and the
  context-aware `cargo_command` / `target_cargo` helpers; clear inherited `RUSTUP_TOOLCHAIN`.
  Preserve each firmware's resolved working/output directories. Other-workspace dependency
  checks use their owning workspace root. Apply the same process boundary to fixtures and probes.
- Use `cargo::runner::firmware_elf_path` for ELF lookup, reports and editor generation so every
  consumer shares the build/profile layout.
- Keep help, version and schema independent of projects and Rust tools. Probe listing needs
  only probe-rs. Preserve the lighter loading paths for selection clearing and reset.
- Keep selection/editor rollback and disabled-editor behavior consistent with the state-write
  contract in Design. A later action failure retains an already successful selection save.
- Edit configuration Schema descriptions in `src/config/model.rs` and CLI help in `src/cli/args.rs`.

## Complete the change

Use the change-specific checks in [Development](docs/development.md#validate-changes).
Update the owning English guide when its contract changes, and link to it instead of copying
its explanation into other pages. Keep this file focused on repository-specific instructions.
