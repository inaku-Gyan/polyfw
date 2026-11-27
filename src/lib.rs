//! Inspect, build and check registered embedded Rust firmware from a host program.
//!
//! Use `polyfw = { version = "0.4", default-features = false }` for a library-only
//! dependency. The default `cli` feature enables the executable and its terminal dependencies.
//!
//! ```no_run
//! use polyfw::{Options, Profile, Project};
//!
//! fn main() -> polyfw::Result<()> {
//!     let project = Project::open("/path/to/firmware-project")?;
//!     for firmware in project.firmwares() {
//!         println!("{} ({})", firmware.id, firmware.platform);
//!     }
//!     let profile: Profile = "release".parse()?;
//!     let report = project.build("led-demo".into(), Some(&profile), Options::default())?;
//!     for artifact in report.artifacts {
//!         println!("{}", artifact.path.display());
//!     }
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]

mod cargo;
mod config;
mod error;
mod files;
mod process;
mod profile;
mod project;
#[cfg(test)]
#[path = "../tests/support/cargo_env.rs"]
mod test_cargo_env;

pub use cargo::workflow::Check;
pub use config::{configuration_schema, write_configuration_schema};
pub use error::{Error, ErrorKind, Result};
pub use process::{
    CleanScope, OutputMode, ProgressEvent, RunReport, Stage, StageKind, StageOutput, StageTarget,
};
pub use profile::Profile;
pub use project::{
    Artifact, BuildReport, ClearReport, DependencyInfo, FirmwareChoice, FirmwareInfo,
    FirmwareSummary, FirmwareTarget, InfoQuery, InitReport, Options, PickRequest, PlatformInfo,
    Project, ProjectInfo, Removal, SavedSelection, initialize,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type InternalResult<T> = std::result::Result<T, BoxError>;
