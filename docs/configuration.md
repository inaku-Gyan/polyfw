# Project configuration

This guide connects an existing embedded Rust project to polyfw. The example project,
`firmware-demos`, has two firmware packages: `led-demo` and `buzzer-demo`. Each has a matching
firmware ID, and both run on the same physical board through the shared `demo-board` platform.
Replace the chip, target, CPU and package names with those of your project.

For everyday commands, see [Usage](usage.md). For package traversal and validation internals,
see [Design](design.md).

## Prepare the Cargo project

polyfw expects working firmware sources, runtime setup and linker scripts. Keep those in
the consuming project. Each firmware package must:

- Be a member of the Cargo workspace discovered from its effective working directory.
- Declare a library that can be checked on both the MCU and the development host.
- Declare at least one binary. A package with multiple binaries needs an explicit `bin`
  in its firmware registration.

A workspace containing both firmware packages can have this layout:

```text
firmware-demos/
├── .cargo/config.toml
├── .gitignore
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── polyfw.toml
└── firmwares/
    ├── led-demo/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       └── main.rs
    └── buzzer-demo/
        ├── Cargo.toml
        └── src/
            ├── lib.rs
            └── main.rs
```

The root `Cargo.toml` defines a virtual workspace:

```toml
[workspace]
members = ["firmwares/led-demo", "firmwares/buzzer-demo"]
resolver = "3"
```

Relevant parts of `firmwares/led-demo/Cargo.toml`:

```toml
[package]
name = "led-demo"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[[bin]]
name = "led-demo"
path = "src/main.rs"
test = false
bench = false
```

The second package, `firmwares/buzzer-demo/Cargo.toml`, declares its own library and binary:

```toml
[package]
name = "buzzer-demo"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[[bin]]
name = "buzzer-demo"
path = "src/main.rs"
test = false
bench = false
```

Keep host-testable logic in each package's library. For a hardware-only binary, `test = false` keeps
Cargo's default host test run from creating a binary test harness. The host Clippy stage
still checks enabled test targets; use conditional compilation where host and MCU code differ.
Run Cargo directly to confirm each package's host tests work before registering it.

From the consuming workspace, install its required components and target:

```sh
rustup component add rustfmt clippy
rustup target add thumbv7em-none-eabihf
```

Use the consuming project's Rust pin. polyfw's own development compiler is independent of it.
If a workspace has no lockfile, run `cargo generate-lockfile` there and commit the result.
Keep a lockfile for every workspace used by the checks; build, lint and test use `--locked`.

## Configure the firmware target

Declare the compilation target and CPU in a Platform in `polyfw.toml`. polyfw passes an
explicit `--target` for target-side build/check commands and appends the CPU to the target's
`CARGO_TARGET_<TARGET>_RUSTFLAGS` environment value. Cargo merges the flags with its configuration.
Keep runtime, linker, environment, feature and profile settings in the consuming Cargo project.
CPU selection alone does not provide a runtime or memory layout.

Cargo discovers configuration and rustup selects a toolchain from the effective `working-dir`.
Discovering a different workspace root does not change this directory. Workspace membership still
owns the lockfile and profiles. Packages outside their workspace directory are supported when
Cargo declares them members; use Cargo's `package.workspace` setting when needed. A separate
workspace registry or manifest field is unnecessary.

Native flag precedence applies. `CARGO_ENCODED_RUSTFLAGS` and `RUSTFLAGS` can replace target flags,
including the injected CPU; matching conditional target flags can also change the CPU. Introducing
target flags suppresses lower-priority `build.rustflags`, as it does in Cargo. Keep required linker
flags at the appropriate target level. `info` reports the declared Platform CPU; consistency
checking is described [below](#check-platform-consistency).

`host-tuple` resolves to Cargo's concrete host target. A custom JSON target uses its file stem for
artifact lookup; a relative target-file argument follows Cargo's working-directory rules.
Every Cargo child clears inherited `RUSTUP_TOOLCHAIN`, including metadata and configuration probes.
An outer `cargo +VERSION` therefore does not override the consuming project's toolchain.
polyfw does not generate rust-analyzer settings; configure the editor through its native facilities.

## Initialize a configuration

From your existing Cargo firmware project, create a configuration template:

```sh
polyfw init
```

To initialize an existing directory from elsewhere:

```sh
polyfw --project-root /path/to/firmware-demos init
```

The destination is exactly the current directory or the explicit `--project-root`, with relative
paths starting at the caller's directory. It must already exist. Initialization does not search
ancestors, so it can create a separate configuration inside an existing project's subdirectory.
Any existing `polyfw.toml` entry, including a directory or dangling symlink, is an error; move or
remove it yourself before generating a replacement. There is no overwrite or merge mode.

The template contains one platform and one firmware, TODO values for target, CPU, chip
and firmware package, and the default checking/editor settings. The description examples may be
edited or omitted, and the platform working directory defaults to `.` beside `polyfw.toml`.
Edit the TODO values to match your project; set `bin` when the package has multiple binaries.
The schema link and `config-version` use the running CLI's release version;
`min-polyfw-version` uses its conservative
[authoring minimum](#configuration-compatibility).

Alongside the template, `init` creates `.polyfw-state/.gitignore` with a single `*` pattern.
An existing regular file at that path supplies the project's custom rules. A path conflict, such
as a symbolic link at `.polyfw-state` or its `.gitignore`, fails initialization. If initialization
fails, `init` reports the affected path and rolls back the paths it created so you can fix the
issue and retry. On success, it lists the files it created.

The command is non-interactive and works without Cargo, Rust, Git or network access. After editing
the template, add the [ignore entries below](#register-the-firmware-targets) and run `polyfw info`
to validate the configuration and Cargo setup.

## Register the firmware targets

Keep `polyfw.toml` at the `firmware-demos` root, which owns selection state and generated editor
files. Cargo output directories are configured independently. Customize the generated template to register both firmware targets
against the same platform:

```toml
#:schema https://raw.githubusercontent.com/inaku-Gyan/polyfw/v0.2.0/assets/polyfw.schema.json

config-version = "0.2.0"
min-polyfw-version = "0.2.0"

[config]
local-dependencies = ["./**"]

[config.vscode]
enabled = true

[platforms.demo-board]
description = "Shared demo board"
target = "thumbv7em-none-eabihf"
cpu = "cortex-m7"
chip = "STM32H723VGTx"
working-dir = "."
build-dir = "target/build/demo-board"
# check-platform defaults to true.

[firmwares.led-demo]
description = "LED demo firmware"
platform = "demo-board"
package = "led-demo"
target-dir = "target/firmware/led-demo"

[firmwares.buzzer-demo]
description = "Buzzer demo firmware"
platform = "demo-board"
package = "buzzer-demo"
target-dir = "target/firmware/buzzer-demo"
```

The firmware IDs select the two firmware packages for the same hardware. Each package has one
binary, so both registrations omit `bin`. Both inherit the same working and intermediate directories, while retaining separate final images.

Both `config` tables are optional; omitting them preserves the defaults shown here.
Unknown fields are errors at every level. Firmware and platform IDs start with a lowercase letter and
contain lowercase letters, digits and single hyphens, for example `demo-board`.
At least one platform and one firmware are required.

| Field | Required | Meaning |
| --- | --- | --- |
| `config-version` | Yes | Full polyfw release version whose configuration contract the file follows |
| `min-polyfw-version` | Yes | Minimum CLI release expected to interpret and execute this configuration correctly |
| `platforms.ID.description` | No | Label shown in platform information; independent of firmware descriptions |
| `platforms.ID.target` | Yes | Explicit compilation target, `host-tuple` or custom JSON target argument |
| `platforms.ID.cpu` | Yes | Declared CPU supplied through target-specific rustflags |
| `platforms.ID.chip` | Yes | probe-rs chip name used by hardware commands and VS Code |
| `firmwares.ID.description` | No | Label shown in firmware lists and the selector |
| `firmwares.ID.platform` | Yes | ID of a platform in the same registry |
| `firmwares.ID.package` | Yes | Firmware package name in the discovered Cargo workspace |
| `firmwares.ID.bin` | No | Binary target name; omit only if the package declares exactly one binary |
| `platforms.ID.working-dir`, `firmwares.ID.working-dir` | No | Cargo invocation directory; firmware overrides platform, default `"."` beside `polyfw.toml` |
| `platforms.ID.build-dir`, `firmwares.ID.build-dir` | No | Intermediate/cache directory override; requires Cargo 1.91+ when explicitly supplied |
| `platforms.ID.target-dir`, `firmwares.ID.target-dir` | No | Final-output directory override |
| `platforms.ID.check-platform`, `firmwares.ID.check-platform` | No | Effective target/CPU consistency check; firmware overrides platform, default `true` |
| `config.local-dependencies` | No | Directory globs selecting reachable path dependencies for checks; default `["./**"]` |
| `config.vscode.enabled` | No | Manage the entire `.vscode/launch.json` on selection save/clear; default `true` |

For an explicit binary, add `bin = "led-demo"` to `[firmwares.led-demo]`. Without `bin`, polyfw
infers the sole binary for ELF lookup but leaves Cargo's default build targets intact.
It counts all declared binaries, including feature-gated targets, independently of `default-run`.

Descriptions are independent: omitting a firmware description does not inherit its platform's.
Omitted descriptions stay absent; supplied strings, including empty or whitespace-only strings,
are preserved.

Directory fields inherit independently. Firmware values override Platform values. Omitting
`working-dir` at both levels uses `"."` beside `polyfw.toml`; an explicit empty or whitespace-only
directory is an error. Omitting `build-dir` and `target-dir` at both levels preserves Cargo's native
directory configuration, environment and defaults. All three relative directory values start
beside `polyfw.toml`, even when inherited; `working-dir` does not rebase either output directory.
Absolute paths and `..` can refer outside the polyfw project. The working directory must exist;
Cargo manages creation of output directories.

A Platform is independent of workspace ownership. Its required target and CPU do not guarantee
complete cache equivalence: Cargo fingerprints also account for toolchains, flags, features and
dependencies. Share `build-dir` to reuse compatible intermediates and choose distinct `target-dir`
values when final images must coexist. Same-named binaries follow native Cargo behavior; no extra
polyfw collision error or ELF copying is added. A shared final filename can be replaced by a later
build. polyfw never inspects Cargo's private intermediate layout.

The `#:schema` URL pins the release named by `config-version`, including patch and prerelease
suffixes: for example, `0.2.0-rc.1` uses `v0.2.0-rc.1`. A newer compatible CLI can operate on
the project without changing the declarations or Schema URL. See
[configuration compatibility](#configuration-compatibility) for the runtime check.
Keep the fixed tag instead of `main` or `latest` so later releases cannot change its editor rules.

For offline use or a development build, generate a local schema using the executable
corresponding to the declared configuration contract:

```sh
polyfw schema --output polyfw.schema.json
```

Then replace the remote directive in `polyfw.toml` with:

```toml
#:schema ./polyfw.schema.json
```

Regenerate the local file when changing the configuration contract. `schema` emits the running
executable's own model, even if a project declares an older contract; it does not select a
historical schema. The command works without a project,
Rust installation or network access. Its `$id` is the same versioned URL as the released file;
`$schema` identifies the JSON Schema Draft 7 dialect, independently of the software version.

Source builds use the version declared in `Cargo.toml`, without a Git or development suffix.
A future tag's URL is unavailable until the tag is pushed. While developing changes under an
unchanged version, its existing tag still serves the released schema; use the generated local
file to match the development executable.

The schema checks configuration structure; polyfw additionally validates Cargo workspace
membership and targets at runtime.
Most project commands resolve every registered firmware before acting, so a broken registration can
prevent a command for another firmware from starting. Consistency checking only applies to the
selected target-side build/check operations.

The consuming project's root `.gitignore` should cover Cargo's default build output and the
generated VS Code launch configuration.

```gitignore
/target/
/.vscode/launch.json
```

The `*` pattern also ignores `.polyfw-state/.gitignore` itself. To share the state ignore rule with
every clone, add `/.polyfw-state/` to the project's root `.gitignore` and commit that entry.

Then inspect the registry and run the first checks:

```sh
polyfw info
polyfw info --firmware led-demo
polyfw info --firmware buzzer-demo
polyfw select led-demo --release
polyfw verify --all-firmwares
polyfw build --all-firmwares --release
```

Connect the firmware before running `polyfw flash`. See [hardware commands](usage.md#flash-and-monitor)
for probe selection and image freshness.

## Check platform consistency

`check-platform` defaults to `true`. Before `build` and the target-side Clippy stages of `lint` or
`verify`, polyfw compares the selected firmware's effective Cargo target/CPU with its Platform.
The comparison includes polyfw's explicit target and injected CPU, plus Cargo environment flag
precedence. A different Cargo `build.target` default is harmless when the explicit target wins.
The CPU comparison uses the last explicit CPU option, including a literal `native` value.

A mismatch or inability to verify reliably is an error. The current checker supports ordinary
target-specific string/array flags and environment overrides. It reports configuration includes,
conditional target sections, profile/unstable configuration sources and rustflags response files
as unverifiable, rather than approximating their effect. Missing explicit CPU flags are also
unverifiable; compiler-default CPU equivalence is not inferred. Set `check-platform = false` on a
Platform or Firmware to skip this analysis silently and let Cargo handle those configurations.
A Firmware's explicit `false` overrides a Platform's `true`.

Checking covers target-side compilation and its dependencies, including cached builds. It does
not inspect custom compiler/wrapper internals or identify the contents of an existing ELF. Host
tests, build scripts and proc macros are excluded. `info`, `fmt`, `test`, `clean` and hardware
commands do not trigger it. `info` continues to show the declared CPU even when checking is off.

## Configuration compatibility

Both top-level version declarations are required, even when the optional `config` tables are
omitted. Use complete SemVer release versions, including any prerelease suffix, rather than
version ranges. `config-version` names the CLI release whose configuration contract the file
follows; it is neither the firmware package version nor the last CLI that ran on the project.
`min-polyfw-version` is the oldest CLI expected to interpret and execute the configuration
correctly. Neither declaration changes merely because the operating CLI is upgraded.

The current configuration floor and conservative minimum CLI recommendation are both `0.2.0`.
Each CLI compares version precedence, ignoring build metadata, and requires all three conditions:

```text
min-polyfw-version <= config-version
CLI.oldest-supported-config-version <= config-version
min-polyfw-version <= CLI.version
```

A newer CLI can read an older configuration while that contract remains supported. An older CLI
can also read a newer contract when the file's minimum CLI allows it and the remaining content
passes validation. For example, a hypothetical `config-version = "0.3.0"` with
`min-polyfw-version = "0.2.0"` allows CLI `0.2.0` when the file uses only behavior that CLI
understands. A version declaration does not make unknown fields or invalid values acceptable.

Missing declarations, invalid versions, inconsistent declarations and incompatible versions
stop every command that loads project configuration, before Cargo, state writes or hardware
actions. Errors identify the file and relevant versions. Select a compatible CLI or manually
migrate the configuration; changing a version string alone does not migrate its contents.
There is no automatic migration, bypass or inferred version for existing files. Help, version,
`schema` and `probes` keep their [project-free behavior](usage.md#choose-a-project).

For an existing file without declarations, review its fields and their meanings against the
chosen release's guide, then add both declarations at the file's top level and align the editor
Schema URL. Earlier CLI implementations that reject these fields cannot operate on the new
configuration.

Compatibility is maintained according to the change:

| Change | Required action |
| --- | --- |
| No configuration contract change | Keep the compatibility baselines and existing declarations unchanged |
| Adopt a new field or capability | Update the file's contract version and minimum CLI to releases that support it; newer CLIs retain their configuration floor while old files still work |
| Old fields, meanings or default behavior cannot be preserved | The CLI raises its configuration floor; the release also raises the recommended minimum CLI if older CLIs cannot correctly execute the new contract |

Each release provides conservative authoring guidance. polyfw does not infer minimum versions
per field. Older contracts are retained only while the current implementation preserves their
meaning; support is dropped when it would require legacy aliases, old-default branches or
historical parsers. No release promises permanent forward or backward compatibility.

## Select local dependencies for checks

`config.local-dependencies` filters reachable Cargo path packages for `fmt`, `lint`, `test`,
`verify` and detailed `info` output. It matches the canonical directory containing each package's
`Cargo.toml`. Patterns are a union; relative patterns start at the directory holding `polyfw.toml`.

```toml
[config]
local-dependencies = ["./**", "../common/**"]
```

| Pattern | Matching package directories |
| --- | --- |
| `"shared"` | Only `shared` itself |
| `"shared/*"` | Immediate children, such as `shared/control` |
| `"shared/**"` | `shared` and descendants at any depth |
| `"../common/**"` | A sibling directory and its descendants |

`*` and `?` stay within one directory component; `**` spans zero or more complete components.
Character classes such as `[ab]` and `[!ab]` are supported. Matching is case-sensitive and includes
hidden directories. `.` and trailing separators are normalized; `..` must precede the first
wildcard. Existing literal prefixes are canonicalized, so a symlink outside the project is not
covered by the default pattern. Include its physical directory explicitly when needed.

The selected firmware is always checked. An empty array selects no dependency packages; valid
unmatched patterns select nothing, while malformed patterns fail before Cargo runs. Registry and
Git dependencies are excluded. The globs do not scan directories, and traversal continues through
excluded packages. They select polyfw's checks; Cargo still builds whatever dependencies those
packages require. See [check planning](design.md#check-planning) for host and MCU classification.

In this example, `polyfw verify led-demo` checks the LED package; sharing a workspace does not
make `buzzer-demo` its dependency. Use `polyfw verify --all-firmwares` to check both firmware packages.

## Add firmware and platforms

Add a `firmwares.ID` table for each selectable program/platform binding. Different firmware IDs
may refer to the same package and binary; a firmware always references exactly one Platform.
A Platform can describe a board model or MCU type and serve firmware in different workspaces.
Override `working-dir` on a firmware to select another Cargo configuration/toolchain context.
Cargo supplies workspace ownership; membership does not require physical containment.

Run `polyfw info`, `polyfw verify --all-firmwares` and `polyfw build --all-firmwares --release`
after changing the registry. The polyfw project root need not be a Cargo workspace or Git repository.

Migrating from 0.1 requires replacing `bsps`/`boards` with `platforms`/`firmwares`, moving workspace
selection to `working-dir`, and declaring target/CPU. Review directory defaults and update both
version declarations. Replace saved selection with `polyfw select FIRMWARE` or clear it using
`polyfw select --clear`. Old configuration, CLI, JSON and saved-state aliases are not retained.

## Choose who manages VS Code configuration

By default, saving a selection replaces the entire `.vscode/launch.json`, and clearing the
selection deletes it. For a project with a manually maintained launch file, opt out before saving:

```toml
[config.vscode]
enabled = false
```

With generation disabled, saving and clearing touch only selection state. Existing editor files
remain as they are, including a launch file generated before the option was disabled.
See [VS Code debugging](usage.md#debug-with-vs-code) for the generated sessions.

## Optional just wrappers

Projects can expose familiar recipes while keeping polyfw installed separately:

```just
[private]
_polyfw *args:
    polyfw --project-root "{{justfile_directory()}}" {{args}}

build *args:
    just _polyfw build {{args}}
```

The absolute project root makes the wrapper independent of the caller's working directory.
Forward other recipes in the same way. A later explicit `--project-root` overrides an earlier
value if the caller needs to operate on another project.
