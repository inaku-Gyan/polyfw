# Design

polyfw provides a host Rust library and CLI for independently configured firmware projects.
[Configuration](configuration.md) owns the input contract and [Usage](usage.md) describes
observable command behavior.

## Ownership and boundaries

| Owner | Responsibility |
| --- | --- |
| Consuming project | Firmware packages, platform declarations, Cargo configuration, Rust pins, profiles and sources |
| polyfw | Registry validation, directory inheritance, firmware/profile selection, consistency checks, check planning, artifact lookup and process orchestration |
| Cargo and rustup | Workspace membership, configuration merging, fingerprints, compilation, tests and toolchain selection |
| probe-rs | Probe access, flashing, reset and RTT sessions |
| VS Code extension | Debug sessions using polyfw's generated launch configuration |

A Firmware is a selectable software entry bound to one Platform. A Platform declares target,
CPU and chip, and may provide defaults for the working/intermediate/final-output directories
and consistency checking. Workspace means Cargo workspace; there is no separately registered
workspace identity. A Board is physical hardware.

## Rust library

The `polyfw` Cargo package contains a library rooted at `src/lib.rs` and a binary rooted at
`src/cli/main.rs`. The binary uses the library's public interface. The default `cli` feature
enables the binary and its argument parsing, terminal UI and signal handling dependencies.
Rust host programs can disable default features when declaring the dependency.

The library owns configuration, project workflows, saved selection, editor writes, and Cargo
resolution and execution. The CLI owns argument conversion, firmware picker presentation,
terminal progress, JSON encoding, exit codes and the `probe-rs` subprocess lifecycle. It uses
library queries to obtain the firmware, chip and ELF for hardware actions.

`Project::open` reads and validates configuration at an exact root. `Project::discover` opens
the nearest registry from a given directory. The project keeps a configuration snapshot.
Firmware and platform lists are available from that snapshot, and operations that need Cargo
resolve it once per call. Reopen the project to read configuration edits.

Pass a firmware ID directly for an explicit build, or use `FirmwareChoice` to request saved-state
inheritance or persistence. When the choice requires input, the calling program can supply a
picker through `Options`. Use `saved_selection`, `save_selection` and `clear_selection` to work
with project-local selection state directly.

Cargo operations are synchronous. The default captures the child's stdout and stderr in the
stage report or failed-stage error. Inherited output lets the child use the caller's terminal.
Callbacks receive structured progress on the calling thread. An artifact reports the expected
ELF path and whether a file was present when checked.

## Load only the context a command needs

CLI parsing precedes project discovery. Help, version and schema work without a project or Rust
tools; schema is generated directly from the model. Probe listing only needs probe-rs and runs
from the caller's directory. Project discovery canonicalizes an explicit root or the nearest
ancestor containing polyfw.toml; an invalid selected project fails without fallback.

`init` runs before loading configuration and uses the exact current/explicit existing directory.
It reserves `polyfw.toml` by exclusive creation before preparing the
[local state ignore file](configuration.md#initialize-a-configuration) and writing the template
with release constants. On failure it rolls back newly created paths. It needs no Cargo or
hardware context.

Loading has two stages:

1. Configuration preflight and strict model parsing validate versions, IDs, platform references,
   directory strings and dependency patterns.
2. Cargo resolution independently resolves each Firmware's inherited directories, invokes metadata
   there, discovers workspace membership/binaries, and records Cargo's effective output paths.

Most commands resolve every registered firmware. Clearing selection only loads configuration and
does not parse old state. Reset also uses configuration alone unless saving selection must generate
an editor launch file. Dependency graphs are loaded for checks and detailed info. Platform
consistency checking is a separate selected-build/target-lint preflight, not registry loading.

Cargo validation requires a library and at least one binary, counting feature-gated binaries
independently of default-run. An omitted bin is resolved for artifact lookup while remaining absent
from build arguments. Same-named binaries follow Cargo's native output behavior.

## Configuration compatibility

The [configuration guide](configuration.md#configuration-compatibility) owns version declarations,
compatibility rules, Schema policy and manual migration. `config::version` owns SemVer comparison
and the two release-maintained baselines. Version preflight runs before strict deserialization,
Cargo, state writes and hardware; malformed TOML still precedes the version gate.

The 0.2.0 contract raises both baselines to 0.2.0. Older contracts are retained only while their
meaning remains intact, without aliases or historical parsers. Firmware/Platform renaming applies
to configuration, CLI, JSON and saved state; the saved-state location remains the same.

## Workspace and compiler isolation

All Cargo children use `cargo::runner` workspace or context-aware command
constructors, including metadata and fixtures. They clear inherited RUSTUP_TOOLCHAIN. Firmware
commands preserve the resolved working-dir; a discovered workspace root does not change cwd.
Cargo/rustup search that directory's physical ancestry. Workspace membership still owns the
lockfile and manifest profiles, including for members located outside the workspace tree.

Raw Platform/Firmware declarations remain separate from each Firmware's resolved CargoContext.
The latter records invocation cwd, discovered owner, resolved artifact target, effective final
and optional intermediate directories, and explicit directory overrides. Omitted output overrides
leave inherited Cargo environment/configuration/defaults untouched. Relative directory inputs
start beside polyfw.toml, independently of each other; absolute and parent paths are supported.
An omitted working-dir at both levels defaults to that project directory.

On Windows, Cargo children receive explicit output overrides in ordinary drive-path syntax
when it addresses the same directory. MinGW can then link host build scripts and proc macros
using object files in those directories. Paths that require extended syntax keep their original
form. See [Windows path naming](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).

Target-side commands explicitly pass the Platform target. CPU injection appends to the inherited
target-specific environment flags; Cargo merges string/array settings and applies its native
source precedence, including global flag suppression and matching cfg flags. Host commands do
not inject Platform CPU and explicitly select host-tuple. Cargo's host alias resolves through
that consuming Cargo's host identity; custom target JSON filenames determine artifact stems.

## Platform consistency checking

`cargo::platform` runs the enabled check-platform preflight before selected builds and target
Clippy. Firmware overrides Platform, including false; the final default is true. It analyzes the
same child environment after polyfw's CPU/directory overrides, with the explicit compilation
target. Last explicit CPU mismatch, absent determinable CPU, response files or unsupported
configuration sources produce actionable errors. See the [supported boundary](configuration.md#check-platform-consistency).

The checker guards unsupported sources before invoking the pinned reader, preventing its hidden
cfg subprocesses from bypassing the runner boundary. Disabled checking performs no configuration
analysis or comparison. Native Cargo always executes with its own settings. Reporting uses declared
Platform CPU and actual Cargo directory/artifact paths, without claiming an effective CPU.
This contract concerns Cargo configuration; custom compiler/wrapper internals and existing ELF
contents are outside it. Cached builds still receive the preflight without forcing compilation.

## Check planning

`scope` traverses selected firmware's full Cargo metadata rather than scanning directories.
Canonical manifests identify packages, keeping identical package names in different owners distinct.

1. Traverse normal, build and dev edges across Cargo's resolved platforms/features.
2. Select reachable local path packages matching config.local-dependencies; always select firmware,
   keep traversing excluded packages, and exclude registry/Git sources from explicit checks.
3. Seed local dependencies' host traversal in their own owning context to include test helpers.
4. Normal edges preserve compilation role; build/dev edges and proc macros switch to host.
   A package reached in both roles receives both checks.

Metadata caches retain execution contexts rather than only workspace roots. Ownership mappings
come from metadata.workspace_root, never from the invocation directory. MCU scopes preserve
compilation target/CPU, directory context and consistency policy. Host and formatting scopes
deduplicate by their actual execution/directory context. Other-workspace dependencies use their
own workspace root and native directories, even if another firmware registers that workspace.

| Stage | Cargo selection |
| --- | --- |
| Formatting | Package names, once per host execution context |
| MCU Clippy | Runtime package IDs, --lib --bins, explicit Platform target |
| Host Clippy | Host package IDs, --lib --tests --target host-tuple |
| Host tests | Default package tests/doctests, --target host-tuple |

Grouped commands retain Cargo defaults and feature unification. Different toolchain/cwd contexts
can format/check the same source more than once. This review scope is not the exact graph of a
single image. verify shares the scope, executes format-check, lint, then tests, and stops on failure.

## Artifact identity

`cargo::runner::firmware_elf_path` is the single public-layout resolver for info, hardware and editor
consumers: effective target-dir / resolved-target / profile-directory / binary. Windows host
executables have the native .exe suffix. profile owns dev/test => debug, release/bench => release,
and custom-profile names. Cargo's private build-dir layout is never inspected.

Builds operate once per selected firmware. Compatible intermediates can share build-dir while
separate target-dir values preserve independent final images. Shared public filenames can be
replaced; no extra collision rejection or ELF-copy mechanism is introduced. Cargo fingerprints,
not Platform identity alone, decide reuse.

Single-firmware clean delegates package cleanup with firmware and host targets using Cargo's
default profile rules. All-firmware clean deduplicates directory contexts and delegates full
cleanup; shared directories can include unregistered/manual outputs. No Platform isolation is
inferred. Neither operation performs platform consistency checking.

## Selection and editor writes

Saved selection is project-local TOML containing a firmware and optional explicit profile. Absence,
malformed state and a stale firmware are distinct cases. Profile inheritance and one-time overrides
are resolved before orchestration; saving with no explicit profile clears the previous profile.

Selection writers for each project take a blocking exclusive lock on `.polyfw-state/selection.lock`.
The lock covers launch and selection file changes, including rollback, and is released when the
operation ends. Once created, the lock file stays in place through clears so concurrent writers
always lock the same file.

A save may precede a build or hardware action. Its success is retained if the later action fails.
With editor generation enabled, the save first replaces the launch file, then writes selection
state. If the state write fails, it restores the previous launch bytes or removes the newly created
file. Each replacement uses a temporary file, rename and cleanup guard. This is rollback between
files, not a filesystem transaction covering both files.

Clearing removes the managed launch file before the state, including a launch file left without
state. Disabled generation leaves editor files untouched during both save and clear. polyfw owns
the whole managed launch file; its entries have no build task and share the ELF resolver.

## Hardware subprocesses

Hardware actions validate their selection and required existing ELF before starting probe-rs.
They do not build or determine source/image freshness. Arguments are explicit and the child's
terminal input/output are inherited.

polyfw installs a Ctrl-C handler that keeps the parent waiting while probe-rs receives the original
terminal signal and cleans up. Process-launch errors retain their OS error; only a missing
executable adds the installation hint. Automated tests use subprocess fixtures, so physical
flashing, RTT and debugger behavior need separate hardware validation.
