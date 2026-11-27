# Design

polyfw provides a host Rust library and CLI for independently configured firmware projects.
[Configuration](configuration.md) owns the input contract and [Usage](usage.md) describes
observable command behavior.

## Ownership and boundaries

| Owner | Responsibility |
| --- | --- |
| Consuming project | Firmware packages, platform declarations, Cargo configuration, Rust pins, profiles and sources |
| polyfw | Registry validation, directory inheritance, firmware/profile selection, check planning, artifact lookup and process orchestration |
| Cargo and rustup | Workspace membership, configuration merging, fingerprints, compilation, tests and toolchain selection |
| probe-rs | Probe access, flashing, reset and RTT sessions |
| VS Code extension | Debug sessions using polyfw's generated launch configuration |

A Firmware is a selectable software entry bound to one Platform. A Platform declares a target
and chip and may supply default working, intermediate and final-output directories.
Workspace means Cargo workspace. A Board is physical hardware.

## Rust library

The `polyfw` Cargo package contains a library rooted at `src/lib.rs` and a `plfw` binary rooted at
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

Cargo operations are synchronous. The default captures output in the stage report or
failed-stage error. Inherited output lets the child use the caller's terminal. The build runner
consumes Cargo's structured messages from stdout and keeps the human-readable diagnostics
available in either mode. Callbacks receive structured progress on the calling thread. An
artifact reports its path and whether a file was present when checked.

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
an editor launch file. Dependency graphs are loaded for checks and detailed info.

Cargo validation requires a library and at least one binary, counting feature-gated binaries
independently of default-run. An omitted bin is resolved for artifact lookup and build verification
while remaining absent from build arguments. Same-named binaries follow Cargo's native output
behavior.

## Configuration compatibility

The [configuration guide](configuration.md#configuration-compatibility) owns version declarations,
compatibility rules, Schema policy and manual migration. `config::version` owns SemVer comparison
and the two release-maintained baselines. Version preflight runs before strict deserialization,
Cargo, state writes and hardware; malformed TOML still precedes the version gate.

The runtime uses one parser for supported configuration contracts. An incompatible contract change
advances the oldest supported version.

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

Target-side commands pass the Platform target. Cargo resolves
[the effective flags](configuration.md#configure-the-firmware-target) from the working directory
and environment. Host lint and tests explicitly select `host-tuple` and inherit Cargo's host flags.
Cargo resolves its host alias in the consuming toolchain. Custom target JSON filenames determine
artifact target names.

## Check planning

`scope` traverses selected firmware's full Cargo metadata rather than scanning directories.
Canonical manifests identify packages, keeping identical package names in different owners distinct.

1. Traverse normal, build and dev edges across Cargo's resolved platforms/features.
2. Select reachable local path packages matching config.local-dependencies; always select firmware,
   keep traversing excluded packages, and exclude registry/Git sources from explicit checks.
3. Seed local dependencies' host traversal in their own owning context to include test helpers.
4. Normal edges preserve compilation role; build/dev edges and proc macros switch to host.
   A package reached in both roles receives both checks.

The metadata cache retains the Cargo execution context. To discover an external
dependency's owner, run `cargo locate-project --workspace` from the working directory that supplied
its metadata, passing the absolute dependency manifest path. Record ownership from
`metadata.workspace_root`. MCU scopes preserve
compilation target and directory context. Host and formatting scopes
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

`cargo::runner::firmware_elf_path` resolves the public artifact path for reports, hardware and
editor consumers. Without a Cargo report it uses the effective target directory, resolved target
subdirectory, profile directory and binary name with its executable suffix. See
[target configuration](configuration.md#configure-the-firmware-target) for custom targets.
Built-in Windows targets use the `.exe` suffix. The `profile` module maps dev and test to
`debug`, release and bench to `release`, and custom profiles to their names.

Each build stage checks Cargo's `compiler-artifact` messages for an executable belonging to the
selected package and binary. The build report uses the path from that stage, including when Cargo
reuses a cached executable. Artifact verification completes before reporting stage success. See
[build usage](usage.md#build-check-and-clean) for the success contract.

Compatible intermediates can share build-dir while separate target-dir values preserve independent
final images. Cargo can replace a shared public filename in a later build. Cargo fingerprints
decide reuse.

Hardware actions that consume an ELF and managed editor generation check for ambiguous final paths
among registered firmwares at the effective profile. The comparison resolves the nearest existing
path ancestor so directory aliases are recognized before the first build. A shared path is valid
when the registrations resolve to the same Cargo workspace, working directory, package, binary,
compilation target, intermediate directory and chip. Custom target identity includes the resolved
JSON file. See [hardware usage](usage.md#flash-and-monitor) for the diagnostic and resolution.

Single-firmware clean delegates package cleanup with firmware and host targets using Cargo's
default profile rules. All-firmware clean deduplicates directory contexts and delegates full
cleanup; shared directories can include unregistered/manual outputs. No Platform isolation is
inferred.

## Selection and editor writes

Saved selection is project-local TOML containing a firmware and optional explicit profile. Absence,
malformed state and a stale firmware are distinct cases. Profile inheritance and one-time overrides
are resolved before orchestration; saving with no explicit profile clears the previous profile.
Initialization and selection operations share a directory check that inspects `.polyfw-state`
without following a link. Reads treat a missing directory as absent state. Writers create the
directory when needed and verify its type if another writer creates it first.

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
