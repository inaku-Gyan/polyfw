# Usage

Run `plfw init` to create a `polyfw.toml` template, then follow
[Project configuration](configuration.md#initialize-a-configuration) to edit it for your project.
The commands below use its `led-demo` and `buzzer-demo` firmware targets, which share the
`demo-board` platform. Replace them with firmware IDs listed by your project's `plfw info`.

## Command overview

Run `plfw help COMMAND` for the installed version's arguments and examples.

| Command | Purpose | Aliases |
| --- | --- | --- |
| `init` | Create a configuration template for an existing firmware project | |
| `schema` | Print or write the project configuration's JSON Schema | |
| `info` | Inspect firmwares, saved selection and existing firmware images | `i` |
| `select` | Save or clear a firmware and optional profile | `s` |
| `build` | Build firmware | `b` |
| `clean` | Remove firmware artifacts | |
| `fmt` | Format selected source files in place | |
| `lint` | Run MCU and host Clippy, denying warnings | |
| `test` | Run host tests and doctests | `t` |
| `verify` | Check formatting, run Clippy, then run host tests | `v` |
| `probes` | List connected probes | |
| `run` | Flash and verify an existing image, then monitor | |
| `flash` | Flash and verify an existing image, reset and exit | `f`, `download`, `d` |
| `attach` | Monitor an existing image without flashing | |
| `reset` | Reset a board without an image | |

`plfw` does not forward arbitrary Cargo arguments. Define targets, features and profiles in the
consuming project, or invoke Cargo directly when an operation needs different flags.

## Choose a project

By default, `plfw` searches the working directory and its ancestors for the nearest
`polyfw.toml`. To operate elsewhere:

```sh
plfw --project-root /path/to/firmware-demos info
```

`--project-root` is global and can appear before or after the command. Relative values start
at the caller's working directory; the last explicit value wins. An invalid explicit root or
the nearest invalid configuration fails instead of falling back to another project.

`init` uses the exact current directory or explicit root without searching ancestors. See
[configuration initialization](configuration.md#initialize-a-configuration) for its creation rules.

The selected root owns state, editor output and hardware-command working directories. Cargo
working and output directories are resolved from each Firmware/Platform configuration. It need not be a Git repository. Help, version, `schema` and `probes` ignore project
discovery and `--project-root`. Help, version and schema need no external tools; `probes` needs
only probe-rs and runs from the caller's directory.

## Save a firmware and profile

```sh
plfw select led-demo --release
plfw build
plfw flash
```

To switch to the buzzer firmware on the same physical board:

```sh
plfw select buzzer-demo --release
plfw build
plfw flash
```

`select` saves the firmware and any explicit profile in `.polyfw-state/selected`. With editor
generation enabled, it also writes the debug configuration. Use `plfw select` to choose in a
searchable picker, or `plfw select --clear` to remove the state and managed launch file.

| Invocation | Firmware choice | Saved state |
| --- | --- | --- |
| `plfw build` | Saved firmware, or a picker if no firmware is saved | Unchanged |
| `plfw build buzzer-demo` | Named firmware for this invocation | Unchanged |
| `plfw build --select buzzer-demo` | Named firmware | Saved before building |
| `plfw build --select` | Always open the picker | Saved before building |
| `plfw build --all-firmwares` | Every registered firmware | Unchanged |

The same firmware rules apply to `clean`, `fmt`, `lint`, `test` and `verify`. Hardware commands
accept a firmware and `--select`, and operate on one firmware. `--all-firmwares` conflicts with a firmware
argument and `--select`. Pickers require a terminal; scripts should use a firmware ID or
`--all-firmwares`.

`build`, `run`, `flash` and `attach` inherit the saved profile, including with a one-time firmware
override. Use `--dev`, `-r`/`--release` or `--profile NAME` to override it. These forms are mutually
exclusive. Saving a selection retains only an explicitly supplied profile:
`plfw select led-demo` clears any previously saved profile. With no explicit or saved profile,
`plfw` omits the Cargo profile flag and Cargo's build default applies. An explicit `--dev` is saved.
`--all-firmwares` uses only the profile passed on that invocation. `clean`, `fmt`, `lint`, `test`,
`verify` and `reset` take no profile option.

A stale or malformed saved selection is an error. A one-time firmware override still reads that
state; replace it with `plfw select led-demo` or remove it with `plfw select --clear`.
Clearing works without Cargo and without parsing the old state. Concurrent saves and clears in the
same project wait for the current [selection write](design.md#selection-and-editor-writes) to finish.
A successful save remains even if the following build or hardware action fails.

## Inspect firmwares and images

```sh
plfw info
plfw info --firmware led-demo
plfw info --firmware buzzer-demo --json=2 > buzzer-demo.json
```

The overview lists all firmwares, their referenced platforms, the saved selection, and dev/release ELF
presence. A different saved profile adds another artifact column. Details show one firmware's
package, binary, workspace, target, chip, absolute ELF paths and check dependencies.
`plfw info --firmware` opens the picker for this query without saving the choice.

Platform descriptions appear in the platform overview and the firmware details. Firmware lists,
details and pickers use the firmware's own description. Missing descriptions leave list cells blank
and omit detail rows and picker description segments. See
[configuration](configuration.md#register-the-firmware-targets) for the optional description fields.

`info` does not build, connect to hardware, change selection or write editor files. A stale firmware
in otherwise valid saved state is reported without blocking the query. Malformed state still
fails. An ELF marked `present` may be older than the source or differ from the running image.

### JSON format versions

Bare `--json` uses format 2, including in releases that add newer formats. Pass `--json=2` to
select format 2 explicitly. A space between `--json` and the version is also accepted.
An unsupported value fails argument parsing before project discovery, and the error lists the
supported versions.

Format versions are independent of polyfw releases. Once released, each numbered version fixes the
JSON fields, types, omission rules and meanings for both views. Contract changes, including new
fields, use a new format version. New formats ship alongside existing numbered versions. Changing
the default format is a breaking change. Release notes announce planned removals and default
changes in advance and provide migration instructions. JSON object key order and whitespace may vary.

Format 2 includes these fields.

| Field | Meaning |
| --- | --- |
| `format_version` | Integer `2` |
| `project_root` | Absolute path to the directory containing `polyfw.toml` |
| `saved_selection` | Saved `firmware` and optional `profile`; omitted if no state exists |
| `platforms` | Map of referenced platforms with optional `description`, declared `target` and `chip`, and `registered_firmwares` counted across the registry |
| `firmwares` | Map of firmwares, limited to the requested firmware when using `--firmware` |
| `firmwares[ID].description`, `.platform`, `.package`, `.bin` | Optional description, registry values and the resolved binary name |
| `firmwares[ID].target`, `.chip` | Values declared by the firmware's platform in `polyfw.toml` |
| `firmwares[ID].manifest_path` | Absolute path to the firmware package's `Cargo.toml` |
| `firmwares[ID].workspace`, `.working_dir` | Discovered Cargo workspace and actual invocation directory |
| `firmwares[ID].target_dir`, `.build_dir` | Effective final/intermediate directories; `build_dir` is omitted if Cargo does not report it and no explicit override supplies it |
| `firmwares[ID].artifacts` | Map from profile name to `{path, exists}` |
| `firmwares[ID].local_dependencies` | Detail entries with `package`, absolute `manifest_path` and boolean `target_clippy` |

Directory and artifact paths are absolute. Artifacts report the expected path and whether a file
was present at that path when queried. Both views report dev, release and any different saved
profile. Only the detail view includes `local_dependencies`, excluding the firmware itself.
Dependencies are sorted by package name and manifest path. Missing saved selection, saved profile
and descriptions are omitted. `false`, empty arrays and explicitly supplied description strings,
including empty strings, are retained.

`target` preserves values such as `host-tuple` and relative custom target arguments. All reported
dependencies participate in host checks. `target_clippy` identifies those included in target-side
Clippy as well.

JSON goes to stdout; interactive prompts go to stderr. Name the firmware in scripts so terminal
availability and picker cancellation cannot change the result.

#### Migrate JSON readers from 0.2.0

Use `plfw info --json=2` in scripts that previously called `polyfw info --json`.

| 0.2.0 field | Format 2 migration |
| --- | --- |
| `repository_root` | Read `project_root` |
| `platforms[ID].cpu`, `firmwares[ID].check_platform` | These fields were removed with their [configuration settings](configuration.md#configuration-compatibility) |
| `firmwares[ID].target` | Now the declared target argument. Use the artifact's `path` to locate the compiled binary |
| `firmwares[ID].artifacts` | Look up the profile directly as a key, such as `artifacts.release.path` |
| `firmwares[ID].local_dependencies` | Read absolute manifest paths and use `target_clippy: true` in place of `target: "embedded"` |

## Build, check and clean

```sh
plfw build led-demo --release
plfw build buzzer-demo --release
plfw build --all-firmwares --release
plfw fmt led-demo
plfw lint led-demo
plfw test led-demo
plfw verify --all-firmwares
plfw clean led-demo
```

Builds run from the effective `working-dir` with explicit Platform `--target` and `--locked`. An explicit registry `bin` adds
`--bin`; otherwise Cargo builds the package's default targets. Every firmware is built separately,
including with `--all-firmwares`.

A build succeeds when Cargo exits successfully and reports the registered binary's executable
for that invocation. Cargo can report a cached executable. If Cargo skips the binary because its
required features are inactive, `plfw build` reports the firmware and its feature requirements,
even if an executable from an earlier build still exists.

Checks cover the firmware package and its configured, reachable local dependencies:

- `fmt` runs `cargo fmt` and **rewrites source files**.
- `lint` checks runtime libraries and binaries for the MCU, then selected libraries and tests
  for the host. Both stages deny warnings.
- `test` runs the selected packages' default host test targets and library doctests.
- `verify` checks formatting without rewriting files, then runs all lint stages, then all test
  stages. It stops at the first failure. It does not build a release image.

Checks operate on whole packages, so other binaries can be included. MCU lint is grouped by compilation context; host work and formatting retain distinct working/output
directory contexts even when their packages share a workspace. Host work explicitly uses
`--target host-tuple`. Other-workspace dependencies use
their own workspace-root execution environment and native output directories. Grouped commands use workspace
defaults and unify package features. The scope is not an exact reproduction of one image's
compilation graph. See [configuration](configuration.md#select-local-dependencies-for-checks)
to control dependency selection.

Cargo chooses output directories unless Firmware or Platform overrides them. The configuration
example shares intermediate artifacts through `build-dir` and keeps final images separate:

```text
target/firmware/led-demo/thumbv7em-none-eabihf/release/led-demo
target/firmware/buzzer-demo/thumbv7em-none-eabihf/release/buzzer-demo
```

Builds and target-side Clippy use the [firmware target and Cargo flag precedence](configuration.md#configure-the-firmware-target).

Absent, dev and test profiles use `debug`; release and bench use `release`; custom profiles use
their own names. Define custom profiles in the consuming Cargo workspace.

`clean led-demo` delegates package cleaning in that firmware's context, selecting its Platform
target and the host target. It follows Cargo's default package-clean profile behavior (dev/debug);
release and custom-profile artifacts can remain. `clean --all-firmwares` performs full Cargo
cleaning for each distinct directory context, across targets and profiles. Shared intermediate
and final directories can include other packages, platforms and manual Cargo builds. The command
shows registered firmware sharing the directories; it does not establish an isolated cleanup scope.

## Flash and monitor

Install [probe-rs](https://probe.rs/docs/getting-started/installation/) on `PATH`, configure host
probe access, then connect the board:

```sh
plfw probes
plfw build led-demo --release
plfw flash led-demo --release
plfw attach led-demo --release
# Or flash and start monitoring in one command:
plfw run led-demo --release
```

To build and run the buzzer firmware instead:

```sh
plfw build buzzer-demo --release
plfw run buzzer-demo --release
```

`run`, `flash` and `attach` use an existing ELF and **never build**. `attach` needs the ELF that
matches the running firmware. `reset` requires no ELF. The registry supplies the chip.

Before starting probe-rs, `run`, `flash` and `attach` check the expected image path at the effective
profile against the other registered firmwares. They report the path and the conflicting firmware
IDs if different Cargo builds or chips share it. Set distinct `target-dir` values for those
firmwares. Compatible intermediates can share a `build-dir`.

The four firmware hardware commands accept transient `--probe VID:PID[:SERIAL]`, `--speed KHZ` and
`--connect-under-reset` options. Omitted probe/speed options are delegated to probe-rs. For example:

```sh
plfw run led-demo --release --probe VID:PID:SERIAL --speed 4000
```

`run` delegates to `probe-rs run --verify`; `flash` uses `probe-rs download --verify --reset`.
`run` and `attach` default to RTT `no-block-skip`, which drops whole messages when the buffer
is full. Choose `--rtt-channel-mode no-block-trim` or `block-if-full` if needed. RTT/defmt output
requires matching support in the firmware. `--no-catch-reset` and `--no-catch-hardfault` disable
probe-rs capture for those events.

Ctrl-C ends the probe-rs session; firmware may continue running. Attaching may briefly halt the
core, and connecting under reset drives the reset line. Firmware hardware commands run from the
operating project root, so project-relative probe-rs settings resolve there.

The integration targets the probe-rs 0.32.0 CLI contract. Automated tests check command
construction and subprocess behavior; they do not establish physical flashing or RTT behavior.
Hardware sessions for this standalone tool remain unverified.

## Debug with VS Code

Install `probe-rs.probe-rs-debugger` and a matching probe-rs CLI on the editor host. Generated
settings follow the extension's
[0.32.0 schema](https://github.com/probe-rs/vscode/blob/v0.32.0/package.json).
Open the operating project root as the VS Code folder, then:

```sh
plfw select led-demo --release
plfw build
```

The saved selection generates `Launch led-demo (release)` and `Attach led-demo (release)`
in `.vscode/launch.json`. Running `plfw select buzzer-demo --release` replaces them with
`Launch buzzer-demo (release)` and `Attach buzzer-demo (release)`. A selection without an explicit
profile omits the name suffix and uses the debug ELF.

- **Launch** flashes and verifies the existing image, resets and halts before startup. Set a
  breakpoint and continue to reach the application.
- **Attach** uses the existing image without flashing or resetting; connecting may pause the core.

Both use SWD, core 0 and RTT channel 0 with `Defmt`/`NoBlockSkip`. Neither builds the image.
Generating the launch file applies the [shared image check](#flash-and-monitor), including before
the first build. A conflict fails the selection save before either file is written. The generated
entries reflect the registry and paths at the time of the last successful save.

One-time CLI overrides leave the launch file unchanged; every selection save replaces the
entire file, and clearing selection deletes it. CLI probe flags are not saved into debugger
settings. Multiple connected probes need an explicit DAP probe selector; use a manually managed
configuration when that is required.

Set [`config.vscode.enabled = false`](configuration.md#choose-who-manages-vs-code-configuration)
to retain a manually maintained launch file. Physical VS Code sessions and RTT delivery remain
unverified; binary platform smoke tests do not verify these sessions.

## Use plfw in scripts and CI

Install a pinned published polyfw version and the consuming project's toolchains, targets,
rustfmt and Clippy. From the project root, use explicit firmware scope:

```sh
plfw verify --all-firmwares
plfw build --all-firmwares --release
```

Use `plfw info --firmware led-demo --json=2` when a script needs resolved paths. See
[JSON format versions](#json-format-versions) for the compatibility contract.

Build and check output includes numbered stages, status and elapsed time. Redirected output omits
colors and Unicode marks. `--color always` and `--color never` override automatic color selection.
`NO_COLOR=1` disables automatic colors. Cargo may need network access to resolve dependencies.

## Troubleshoot

| Symptom | Action |
| --- | --- |
| No `polyfw.toml` found | Run from the consuming project or pass `--project-root DIR` |
| Configuration version missing or incompatible | Follow [configuration compatibility](configuration.md#configuration-compatibility); choose a compatible CLI or manually migrate the file |
| A command fails on an unrelated platform | Fix each registered firmware's working directory and Cargo configuration; most project commands validate the whole registry |
| Unexpected CPU or missing linker flags | Review [Cargo flag precedence](configuration.md#configure-the-firmware-target) for the firmware's working directory |
| Package has no library target | Add a host-testable library to the firmware package |
| Multiple binary targets | Set the firmware's `bin` to a declared binary; `default-run` does not select it for `plfw` |
| Saved firmware removed or state malformed | Run `plfw select led-demo` to replace it, or `plfw select --clear` |
| Unexpected release/dev image | Inspect saved profile with `info`; use an explicit profile or save a new selection |
| ELF missing | Run the reported build command for that firmware and profile |
| Host lint/test tries to compile hardware code | Separate host-testable library code and configure target-specific binary/test compilation |
| A local dependency is missing from checks | Inspect `info --firmware led-demo` and canonical directory-glob matching |
| `--locked` fails | Update and commit the owning workspace's lockfile with Cargo |
| Probe missing or ambiguous | Check `plfw probes`, host permissions and the explicit `--probe` selector |
| VS Code changes disappear | Disable launch generation before maintaining that file yourself |
