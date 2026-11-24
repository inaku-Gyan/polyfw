# Changelog

## 0.3.0 (unreleased)

### Breaking changes

- Renamed the installed command from `polyfw` to `plfw` (`plfw.exe` on Windows). Update scripts
  and CI to invoke `plfw`. For a Cargo or Binstall installation, run `cargo uninstall polyfw`
  before reinstalling the `polyfw` package. For an archive installation, remove the old executable
  from `PATH` when adding `plfw`. Check the installed version with `plfw --version`.
- `info --json` uses format 2. It reports `project_root`, declared platform targets, absolute
  manifest paths and artifacts keyed by profile. The 0.2.0 layout is retired. See the
  [JSON migration](docs/usage.md#migrate-json-readers-from-020) to update scripts.
- The 0.3.0 configuration contract removes `check-platform` and `cpu`. Configure `target-cpu`
  and other compiler flags in Cargo. See the
  [configuration migration](docs/configuration.md#configuration-compatibility).

### Added

- Exposed the Rust library for project discovery, inspection, saved selection, builds and checks.
  Depend on `polyfw` with default features disabled for library-only use.

### Fixed

- Serialized concurrent selection saves and clears within a project.
- `init` creates the state directory's ignore file and rolls back newly created files if
  initialization fails.
- Passed compatible output paths to Cargo on Windows while preserving paths that require
  extended path syntax.

## [0.2.0]

### Changed

- Replaced boards and BSPs with firmwares and platforms throughout the configuration,
  CLI, JSON output and saved selection. Use `--all-firmwares` and `info --firmware`.
  Projects upgrading from 0.1 must replace the old registry tables, update both version
  declarations and replace or clear their saved selection. [Configuration migration](https://github.com/inaku-Gyan/polyfw/blob/v0.2.0/docs/configuration.md#add-firmware-and-platforms)
- Platforms declare a target, CPU and probe chip. Each firmware resolves its owning workspace
  through Cargo from its effective `working-dir`. Firmware settings can override a platform's
  `working-dir`, `target-dir` and `build-dir`, so firmwares can share intermediate artifacts
  and keep final images in separate directories.
- Aligned builds and checks with each firmware's resolved Cargo context. Platform consistency
  checks compare the effective Cargo target and CPU with the platform declaration.
- Bundled the user guides in the crate and release archives for offline use.

### Added

- Support for external Cargo workspace members.
- Optional firmware and platform descriptions and resolved Cargo directories in `info`.
- Three runnable Cortex-M example projects covering a single firmware, a shared workspace
  and multiple workspaces.

## [0.1.0]

- First standalone release. Register boards and BSPs in `polyfw.toml`, save a board and
  profile, and inspect firmware images with `info`.
- Build, clean, format, lint and test firmware and configured local dependencies through Cargo.
  Flash or monitor existing images through probe-rs and optionally generate VS Code debug
  configuration.
- Create a versioned configuration template with `init` and export its release-linked JSON
  Schema with `schema`.
- Install from crates.io or use prebuilt Linux, macOS and Windows archives from GitHub Releases.

[0.2.0]: https://github.com/inaku-Gyan/polyfw/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/inaku-Gyan/polyfw/releases/tag/v0.1.0
