<h1 align="center">polyfw</h1>
<p align="center">One CLI to build, check, flash, and monitor embedded Rust firmware.</p>
<p align="center">
  <a href="https://crates.io/crates/polyfw"><img alt="crates.io version" src="https://img.shields.io/crates/v/polyfw"></a>
  <a href="https://github.com/inaku-Gyan/polyfw/actions/workflows/ci.yml"><img alt="CI status" src="https://github.com/inaku-Gyan/polyfw/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-blue"></a>
</p>

- [Features](#features)
- [Quick start](#quick-start)
- [Install](#install)
- [Use as a Rust dependency](#use-as-a-rust-dependency)
- [Documentation](#documentation)

## Features

- **Firmware registry:** map selectable firmwares to platforms declaring the target, CPU and
  probe chip, with configurable Cargo working and output directories.
- **Checks across workspaces:** run formatting checks, MCU and host Clippy, and host tests for
  firmware and reachable local dependencies.
- **Saved selections:** choose a firmware and profile, check target/CPU consistency, and inspect
  resolved ELF paths.
- **Flash and debug:** flash and monitor through probe-rs; generate optional VS Code debug configurations.

Cargo owns workspace membership, configuration merging, dependencies and profiles; polyfw
coordinates the firmware workflow.

## Quick start

Initialize a configuration in your existing firmware project:

```sh
polyfw init
```

[Edit the generated configuration](docs/configuration.md#initialize-a-configuration) to match
its workspace, chip and firmware packages. The `firmware-demos` example registers `led-demo`
and `buzzer-demo` for the same physical board through the `demo-board` platform. From that
project or any subdirectory:

```sh
polyfw info
polyfw select led-demo --release
polyfw verify --all-firmwares
polyfw build --all-firmwares --release
polyfw flash
polyfw select buzzer-demo --release
polyfw flash
```

`verify --all-firmwares` checks formatting, runs MCU and host Clippy, and runs host tests for
both packages. `build --all-firmwares --release` creates both release images; the saved selection
chooses which image to flash.
`run`, `flash` and `attach` use an existing image, so rebuild after changing source. Use
`polyfw run` to flash and monitor, or `polyfw attach` to monitor firmware already on the board.

To operate on a project from elsewhere:

```sh
polyfw --project-root /path/to/firmware-demos info
```

Use `polyfw help build` for command help, or see [Usage](docs/usage.md) for the full workflow.

## Install

**Prebuilt binary** with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall polyfw
```

**Build from source** with Cargo:

```sh
cargo install polyfw --locked
```

[Release archives](https://github.com/inaku-Gyan/polyfw/releases) are available for Linux GNU
and macOS on x86_64/aarch64, and Windows x86_64 MSVC. Each includes the executable, README, MIT
license and guides. Verify the archive against `SHA256SUMS`, extract it and put the executable
on `PATH`. See [release assets and build targets](docs/releasing.md#assets-and-build-targets)
for the build and test environments.

<details>
<summary>Install a prerelease</summary>

Specify a published prerelease explicitly:

```sh
cargo binstall polyfw --version 0.2.0-rc.1
# Alternatively:
cargo install polyfw --locked --version 0.2.0-rc.1
```

</details>

<details>
<summary>Install a development checkout</summary>

Run from the polyfw repository:

```sh
cargo install --path . --locked
polyfw --version
```

</details>

Firmware operations need Cargo and rustup, plus the toolchain, targets, rustfmt and Clippy
required by the consuming project. Flashing and monitoring also need
[probe-rs](https://probe.rs/docs/getting-started/installation/) on `PATH`.

## Use as a Rust dependency

Host programs can inspect a project, manage its saved selection, and run Cargo builds and checks.
Use the library with the CLI feature disabled in the dependency declaration.

```toml
[dependencies]
polyfw = { version = "0.3", default-features = false }
```

Here is an example building a firmware named `hello` with `release` profile:

```rust
use polyfw::{Check, Options, OutputMode, Profile, Project};

fn main() -> polyfw::Result<()> {
    let project = match std::env::args_os().nth(1) {
        Some(root) => Project::open(root)?,
        None => Project::discover(".")?,
    };
    println!("Project: {}", project.root().display());

    for (id, platform) in project.platforms() {
        println!("Platform {id}: {}", platform.target);
    }
    for firmware in project.firmwares() {
        println!("Firmware {} on {}", firmware.id, firmware.platform);
    }
    if let Some(saved) = project.saved_selection()? {
        let profile = saved.profile.as_ref().map_or("default", Profile::name);
        println!("Saved selection: {} ({profile})", saved.firmware);
    }

    let firmware = "hello";
    let profile: Profile = "release".parse()?;
    let output = OutputMode::Inherit;
    project.check(Check::Test, firmware.into(), Options::new(output))?;
    let report = project.build(firmware.into(), Some(&profile), Options::new(output))?;
    for artifact in report.artifacts {
        println!("ELF {} (present: {})", artifact.path.display(), artifact.exists);
    }
    Ok(())
}
```

For project loading, selection and process behavior, see [Rust library](docs/design.md#rust-library).

## Documentation

| Task | Guide |
| --- | --- |
| Configure firmware, platforms, checks and editor generation | [Project configuration](docs/configuration.md) |
| Select, build, test, flash, debug and troubleshoot | [Usage](docs/usage.md) |
| Explore the ARM integration-test projects | [Demos](https://github.com/inaku-Gyan/polyfw/tree/main/demos) |
| Understand validation, dependency scopes and state management | [Design](docs/design.md) |
| Develop polyfw and run checks | [Development](docs/development.md) |
| Prepare and publish a release | [Releasing](docs/releasing.md) |

The guides ship with the crate and binary archives; relative links work offline with that version.
The CLI's `--help` and `schema` output also describe the installed version.

Licensed under the [MIT license](LICENSE).
