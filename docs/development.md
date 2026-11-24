# Development

This guide is for changes to polyfw itself. To integrate an existing firmware project, use
[Project configuration](configuration.md); to understand implementation constraints, read
[Design](design.md).

## Set up a checkout

Use rustup with the repository's `rust-toolchain.toml`, plus `just`. The pin includes rustfmt,
Clippy and the LLVM tools for coverage. Install the embedded target used by Cargo-running test
fixtures and run the checks.

```sh
rustup target add thumbv7em-none-eabihf
just check
```

`just check` checks Rust formatting, runs Clippy for all targets with warnings denied, and runs
the Rust tests. It also runs the library consumer test with default features disabled.

Use `just --list` for repository recipes. `just fix` formats source and applies Clippy fixes;
review those changes before committing. The Rust minimum in `Cargo.toml` is the supported compiler
floor; `rust-toolchain.toml` selects the development compiler. Assess both when introducing Rust
features or updating dependencies.

## Run against a consuming project

`just run` runs the development CLI with locked dependencies. Pass `plfw` arguments directly
after the recipe name; their boundaries are preserved, including quoted paths with spaces.
With no arguments, it prints CLI help.

The repository root has no firmware registry of its own. The three small
[ARM demo projects](https://github.com/inaku-Gyan/polyfw/tree/main/demos) are consuming projects
and integration-test inputs. Run one with the development binary.

```sh
just run --project-root demos/minimal info
just run --project-root demos/minimal verify --all-firmwares
just run --project-root demos/minimal build --all-firmwares --release
```

Or point it at an existing consumer:

```sh
just run --project-root /path/to/firmware-demos info
just run --project-root /path/to/firmware-demos verify --all-firmwares
```

Or install this checkout with `cargo install --path . --locked`. Cargo children launched by `plfw`
use the consuming workspaces' pins rather than inheriting the compiler used to run `plfw`.

## Find the relevant code

| Files | Responsibility |
| --- | --- |
| `src/lib.rs` | Public Rust types and entry points |
| `src/cli/` | Arguments, dispatch, terminal output, JSON encoding and hardware subprocesses |
| `src/config/` | Configuration model, Schema, version preflight and dependency path patterns |
| `src/project/` | Project discovery, queries, workflows, selection state and editor files |
| `src/project/init.rs`, `assets/polyfw.toml` | Configuration template and exclusive initialization |
| `src/cargo/` | Cargo context and metadata, dependency scopes, stage planning and ELF paths |
| `src/process.rs` | Stage execution, progress and child output |
| `tests/library.rs` | Public Rust API exercised as a downstream caller |
| `tests/project_root.rs`, `tests/standalone.rs` | Executable-level project isolation and external-tool boundaries |
| `demos/`, `tests/demos.rs` | Self-contained Cortex-M projects and their end-to-end tests |
| `.github/workflows/release.yml` | Tag validation, native binary archives and publication |

Edit the initialization template in `assets/polyfw.toml`. Its schema directive points to
`polyfw.schema.json` in the same directory; `init` replaces it with the matching release URL
when generating a consumer configuration.

Unit tests live beside their implementation. CLI parser tests check argument combinations;
subprocess fixtures check commands without accessing hardware.

## Maintain generated and public documentation

Each description has one source:

| Surface | Source to edit | Follow-up |
| --- | --- | --- |
| CLI help | Documentation comments and help strings in `src/cli/args.rs` | Inspect `just run help COMMAND` |
| Configuration Schema | Model comments and schema annotations in `src/config/model.rs` | Run `just schema` and commit `assets/polyfw.schema.json` |
| Rust API | Rustdoc on the public items exported by `src/lib.rs` | Run `cargo doc --locked --no-deps --no-default-features` |
| Installation and navigation | `README.md` | Keep relative guide links usable in source, crate and binary archives |
| Configuration rules | `docs/configuration.md` | Match model defaults, Cargo requirements and validation |
| User operations and JSON output | `docs/usage.md` | Match CLI parsing, command behavior and serialization |
| Architecture and invariants | `docs/design.md` | Update alongside changes to ownership, algorithms or boundaries |
| Contributor workflow | This document | Match recipes and automation |
| Demo projects | `demos/README.md` and each project's README | Keep shared commands in the index and project structure in each README |
| Publication procedure | `docs/releasing.md` | Match the tagged workflow |
| Agent instructions | `AGENTS.md` | Keep repository-specific rules and conditional document pointers concise |

The schema equality test rejects output that differs from the Rust generator. Editing the JSON
alone is not sufficient. Its `$id` uses the compile-time `CARGO_PKG_VERSION` and the corresponding
`v<VERSION>` source tag; `$schema` remains the Draft 7 dialect URI. Regenerate it after package
version changes as well as model changes. Follow the
[version preparation steps](releasing.md#prepare-a-version) to keep the committed schema and
configuration guide's pinned link aligned. The release workflow checks that link, and every
release binary must emit the committed schema.

The schema describes configuration structure; Cargo-dependent constraints still need runtime
validation and prose explanation. Use a locally generated schema for an untagged development
build, as described in [Configuration](configuration.md#register-the-firmware-targets).

Use English throughout, with the `firmware-demos` project for user-facing examples: `led-demo`
and `buzzer-demo` share the `demo-board` Platform. Put complete explanations in their owning guide
and link from other pages when further reading is needed. Keep simple explanations in prose
and omit unnecessary background references. Use relative links between bundled guides and
README so they work offline with the matching release. Keep external links only when the task
requires outside documentation.

## Keep test fixtures isolated

Cargo-running fixtures must carry a copy of the repository Rust pin in their temporary project
root. A dependency workspace outside that root needs its own pin. Set up their Cargo commands
through the workspace launcher as well. The ancestor-toolchain regression checks that an unusable
outer pin cannot be hidden by a developer's default installation.

Fixture commands clear inherited Cargo settings for the compilation target, output directories,
and `rustc` and `rustdoc` flags. Apply environment overrides under test after creating the fixture
command.

Canonicalize temporary roots before writing relative or absolute path dependencies. On macOS,
`/var` and `/private/var` can name the same directory; mixing them can produce lockfile package
collisions. Fixtures do not need a `TMPDIR` override. Tests expecting a compiler failure must check
the intended diagnostic so an unrelated setup failure cannot count as success.

Ordinary fixture builds may emit unused-dependency warnings. CI explicitly allows those build
warnings; the repository's Clippy command still uses `-D warnings`. Hardware command and debugger
fixtures establish generated arguments/settings, not physical device behavior.

## Run focused tests

| Command | Coverage |
| --- | --- |
| `just test-lib` | Library unit tests, the public Rust API integration test and doctests with default features disabled |
| `just test-cli` | Binary unit tests and the `standalone`, `project_root` and `platforms` integration targets |
| `just test-demos` | End-to-end tests against copies of the three demo projects |

## Measure test coverage

After setting up a checkout, install the coverage subcommand.

```sh
cargo install cargo-llvm-cov --version 0.9.2 --locked
just coverage
```

`just coverage` runs the default-feature unit and integration tests, including the demo builds,
and measures line coverage in `src/` on the host. Open `coverage/html/index.html` to inspect
individual files. The same run writes `coverage/lcov.info` for Codecov.

The reports remain available after `cargo clean`. Use `just clean` to remove them along with build
outputs. `just check` also checks the Rustdoc example and verifies the library with default
features disabled.

## Maintain the examples

Each example directory is a self-contained consuming project. Copy the whole project directory
when trying it outside this checkout. Every Cargo workspace includes a committed lockfile. Keep
each example's root Rust pin aligned with the repository pin. Nested workspaces may inherit this
pin through directory ancestry; add a local pin when a workspace needs different toolchain
requirements. See [compiler isolation](design.md#workspace-and-compiler-isolation) for how the
working directory determines toolchain selection.

Run the demo project tests from the repository root after setting up the checkout.
[Checkout setup](#set-up-a-checkout)

```sh
just test-demos
```

Use `just test-demos minimal --nocapture` to run the minimal project's tests with live output.
The recipe forwards test-name filters and harness options after Cargo's `--` separator.

`tests/demos.rs` copies the checked-in projects into temporary directories and invokes the
built `plfw` executable. It exercises real MCU builds, host library tests, selection, external
workspace members and dependency scopes. The tests check reported artifact paths, platform
targets and chips, and linked 32-bit ARM ELF executables with Thumb entry points in flash, including
independent builds of a package's two binaries. Failing host tests introduced in the copies check
that polyfw includes reachable shared dependencies and excludes unrelated packages, with separate
cases for each shared library. The multi-workspace example also contains passing host tests for
pulse timing and LED rate selection; its integration test checks that these tests actually run.

Failure fixtures check that `verify` runs formatting, MCU Clippy, host Clippy and host tests in
order, stops at the failing stage, and leaves sources unchanged during format checking. A failed
build after `--select` must retain the newly saved firmware and profile. Cleanup tests start with
real dev/release images, check package-wide deletion and preservation, then exercise full cleanup
and rebuilding. See [Usage](usage.md#build-check-and-clean) for the cleanup contract.

Cargo fetches locked dependencies as needed using each command's effective Rust toolchain and
inherits the caller's network settings. With dependencies cached and the target installed, set
`CARGO_NET_OFFLINE=true` for an offline run. These tests are included in `just check`.

## Validate changes

- For Rust, CLI help or model documentation changes, run `just check`. Regenerate the schema when
  its model descriptions, fields or package version change, and inspect the affected command help.
- For workflow or packaging changes, run `just check` and the package/dry-run checks in
  [Releasing](releasing.md#prepare-a-version).
- For Markdown-only changes, verify links, heading anchors, configuration snippets and command
  claims against their sources. Run affected examples where practical.
- Before committing, run `git diff --check` and review the complete diff, including generated files.

## Understand repository automation

CI runs `just check` followed by `cargo build --locked --release` on pushes to `main` and non-draft
pull requests, skipping Markdown-only changes. Tagged releases invoke the same workflow before
building the native release binaries.

Coverage runs on the same `main` pushes and non-draft pull requests. The run summary shows line
coverage, and the `coverage-html` artifact contains the full HTML report. The workflow uploads LCOV
to Codecov using GitHub OIDC, with tokenless uploads for fork pull requests. Codecov's project
and patch statuses are informational. The run summary records Codecov upload failures.

Enable the Codecov GitHub App for this repository to publish coverage. The README badge shows
`main` coverage after the first successful upload from that branch. [Codecov setup](https://docs.codecov.com/docs/quick-start)

For publishing credentials, stable/prerelease tags, binary targets and retry rules, follow
[Releasing](releasing.md).
