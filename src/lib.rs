//! Load a polyfw project and resolve its firmware without the polyfw executable.
//!
//! [`Project`] reads `polyfw.toml`, resolves each firmware through Cargo, and reports the
//! values the CLI uses: probe-rs chip, compilation target, Cargo directories and ELF paths.
//! It also builds the `cargo build` command for a firmware, and reads or saves the selection.
//! Callers run the returned commands and own the terminal, hardware and editor sessions.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let project = polyfw::Project::discover(".")?;
//! let firmware = project.firmware("led-demo")?;
//! let release = "release".parse::<polyfw::Profile>()?;
//! let status = firmware.build_command(Some(&release)).status()?;
//! assert!(status.success());
//! println!("{} on {}", firmware.elf_path(Some(&release)).display(), firmware.platform().chip());
//! # Ok(())
//! # }
//! ```
//!
//! The default `cli` feature builds the polyfw executable. Disable default features to depend
//! on the library alone.

#![forbid(unsafe_code)]
#![cfg_attr(
    not(feature = "cli"),
    expect(
        dead_code,
        reason = "check planning, reports and schema output serve only the CLI"
    )
)]

mod api;
mod cargo_config;
mod config_version;
mod info;
mod local_dependencies;
mod model;
mod profile;
mod registry;
mod runner;
mod scope;
mod vscode;

#[cfg(feature = "cli")]
#[doc(hidden)]
pub mod app;
#[cfg(feature = "cli")]
mod cli;
#[cfg(feature = "cli")]
mod commands;
#[cfg(feature = "cli")]
mod init;
#[cfg(feature = "cli")]
mod probe;
#[cfg(feature = "cli")]
mod ui;

pub use api::{Error, Firmware, Platform, Project, SavedSelection};
pub use profile::{ParseProfileError, Profile};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
