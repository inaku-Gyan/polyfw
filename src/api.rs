use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::Selection;
use crate::profile::Profile;
use crate::registry::{self, Context};
use crate::{cargo_config, runner};

/// An error from loading a project, resolving a firmware or writing selection state.
///
/// Messages match the polyfw CLI and can name CLI commands as the remedy.
pub struct Error(Box<dyn std::error::Error + Send + Sync>);

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

#[doc(hidden)]
impl From<Box<dyn std::error::Error + Send + Sync>> for Error {
    fn from(error: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self(error)
    }
}

/// A loaded polyfw project whose firmwares are resolved through Cargo.
///
/// Loading validates `polyfw.toml` and runs `cargo metadata` once per firmware, as every
/// firmware command of the CLI does. The values reflect the files at load time; load the
/// project again after `polyfw.toml` or Cargo manifests change.
#[derive(Debug)]
pub struct Project {
    context: Context,
}

impl Project {
    /// Loads the project in the nearest directory at or above `start` that has a `polyfw.toml`.
    ///
    /// A relative `start` resolves against the current directory.
    ///
    /// # Errors
    ///
    /// Fails if no ancestor has a `polyfw.toml`, or if loading fails as in [`Project::open`].
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, Error> {
        let start = std::path::absolute(start.as_ref()).map_err(boxed)?;
        Self::load(registry::project_root(None, &start)?)
    }

    /// Loads the project whose `polyfw.toml` is in `root`.
    ///
    /// A relative `root` resolves against the current directory.
    ///
    /// # Errors
    ///
    /// Fails if `root` is not a directory, if `polyfw.toml` is missing, invalid or requires
    /// a newer polyfw, or if Cargo cannot resolve a registered firmware.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, Error> {
        let root = std::path::absolute(root.as_ref()).map_err(boxed)?;
        Self::load(registry::project_root(Some(&root), &root)?)
    }

    fn load(root: PathBuf) -> Result<Self, Error> {
        Ok(Self {
            context: Context::load(root)?,
        })
    }

    /// The canonical directory that contains `polyfw.toml`.
    pub fn root(&self) -> &Path {
        &self.context.root
    }

    /// Every registered platform, sorted by ID.
    pub fn platforms(&self) -> impl Iterator<Item = Platform> + '_ {
        self.context
            .registry
            .platforms
            .iter()
            .map(|(id, platform)| Platform {
                id: id.clone(),
                platform: platform.clone(),
            })
    }

    /// Every registered firmware, sorted by ID.
    pub fn firmwares(&self) -> impl Iterator<Item = Firmware> + '_ {
        self.context.registry.firmwares.keys().map(|id| {
            let selection = self
                .context
                .selection(id)
                .expect("loading validates every firmware's platform reference");
            Firmware { selection }
        })
    }

    /// The firmware registered as `id`.
    ///
    /// # Errors
    ///
    /// Fails if no firmware has that ID.
    pub fn firmware(&self, id: &str) -> Result<Firmware, Error> {
        Ok(Firmware {
            selection: self.context.selection(id)?,
        })
    }

    /// The selection saved by `polyfw select`, or `None` if nothing is saved.
    ///
    /// The saved firmware may no longer be registered; [`Project::firmware`] reports that.
    ///
    /// # Errors
    ///
    /// Fails if the state file cannot be read or is malformed.
    pub fn saved_selection(&self) -> Result<Option<SavedSelection>, Error> {
        Ok(self
            .context
            .read_saved_selection()?
            .map(|saved| SavedSelection {
                firmware: saved.firmware,
                profile: saved.profile,
            }))
    }

    /// Saves `firmware` and an optional profile as the selection, like `polyfw select`.
    ///
    /// When the project enables VS Code generation, this also replaces `.vscode/launch.json`,
    /// and restores the previous file if writing the selection fails. It does not build.
    ///
    /// # Errors
    ///
    /// Fails if the firmware is unknown or a file cannot be written.
    pub fn save_selection(&self, firmware: &str, profile: Option<&Profile>) -> Result<(), Error> {
        Ok(self.context.save_selection(firmware, profile)?)
    }

    /// Removes the saved selection, and the generated launch file when VS Code generation is
    /// enabled, like `polyfw select --clear`. Missing files are not an error.
    ///
    /// # Errors
    ///
    /// Fails if an existing file cannot be removed. Every file is attempted first.
    pub fn clear_selection(&self) -> Result<(), Error> {
        let mut first_error = None;
        for (path, result) in self.context.clear_selection() {
            if let Err(error) = result
                && error.kind() != ErrorKind::NotFound
            {
                first_error.get_or_insert_with(|| {
                    Error(format!("could not remove {path}: {error}").into())
                });
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// A saved firmware and optional profile from `polyfw select`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSelection {
    firmware: String,
    profile: Option<Profile>,
}

impl SavedSelection {
    /// The saved firmware ID.
    pub fn firmware(&self) -> &str {
        &self.firmware
    }

    /// The saved profile, or `None` for Cargo's default `dev` profile.
    pub fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }
}

/// Hardware and compilation settings that firmwares share.
#[derive(Clone, Debug)]
pub struct Platform {
    id: String,
    platform: crate::model::Platform,
}

impl Platform {
    /// The platform ID from `polyfw.toml`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The optional human-readable label.
    pub fn description(&self) -> Option<&str> {
        self.platform.description.as_deref()
    }

    /// The declared Cargo target: a triple, `host-tuple` or a custom target JSON path.
    ///
    /// [`Firmware::target`] gives the resolved name used in output paths.
    pub fn target(&self) -> &str {
        &self.platform.target
    }

    /// The declared target CPU, passed to target builds as `-Ctarget-cpu`.
    pub fn cpu(&self) -> &str {
        &self.platform.cpu
    }

    /// The probe-rs chip name, for example `STM32H723VGTx`.
    pub fn chip(&self) -> &str {
        &self.platform.chip
    }
}

/// A registered firmware with its platform and resolved Cargo context.
#[derive(Clone, Debug)]
pub struct Firmware {
    selection: Selection,
}

impl Firmware {
    /// The firmware ID from `polyfw.toml`.
    pub fn id(&self) -> &str {
        &self.selection.id
    }

    /// The optional human-readable label. Firmware does not inherit the platform's.
    pub fn description(&self) -> Option<&str> {
        self.selection.firmware.description.as_deref()
    }

    /// The platform this firmware runs on.
    pub fn platform(&self) -> Platform {
        Platform {
            id: self.selection.firmware.platform.clone(),
            platform: self.selection.platform.clone(),
        }
    }

    /// The Cargo package name.
    pub fn package(&self) -> &str {
        &self.selection.firmware.package
    }

    /// The binary target name, resolved from the package when `polyfw.toml` omits it.
    pub fn bin(&self) -> &str {
        &self.selection.firmware.resolved_bin
    }

    /// The directory Cargo commands for this firmware run in.
    pub fn working_dir(&self) -> &Path {
        &self.selection.firmware.cargo.working_dir
    }

    /// The root of the Cargo workspace that owns the package.
    pub fn workspace(&self) -> &Path {
        &self.selection.firmware.cargo.workspace
    }

    /// The effective Cargo final-output directory.
    pub fn target_dir(&self) -> &Path {
        &self.selection.firmware.cargo.target_dir
    }

    /// The effective Cargo intermediate directory, if Cargo or `polyfw.toml` reports one.
    pub fn build_dir(&self) -> Option<&Path> {
        self.selection.firmware.cargo.build_dir.as_deref()
    }

    /// The resolved target name in output paths: a triple, or a custom target's file stem.
    pub fn target(&self) -> &str {
        &self.selection.firmware.cargo.target
    }

    /// The ELF path that a build with `profile` writes. `None` means Cargo's `dev` profile.
    ///
    /// The file may not exist, or may be older than the sources.
    pub fn elf_path(&self, profile: Option<&Profile>) -> PathBuf {
        runner::firmware_elf_path(&self.with_profile(profile))
    }

    /// The `cargo build` command that writes [`Firmware::elf_path`] for `profile`.
    ///
    /// It runs in [`Firmware::working_dir`] with the platform target, CPU flags, output
    /// directories and `--locked`, as `polyfw build` does. The caller runs it and may add
    /// arguments such as `--message-format=json`. Call
    /// [`Firmware::check_platform_consistency`] first to match the CLI's checks.
    pub fn build_command(&self, profile: Option<&Profile>) -> Command {
        runner::build_command(&self.with_profile(profile))
    }

    /// Checks that the effective Cargo configuration builds for the platform's target and
    /// CPU, as `polyfw build` does before building. Does nothing when `check-platform` is
    /// false for this firmware.
    ///
    /// # Errors
    ///
    /// Fails if Cargo configuration selects another target CPU, does not determine one, or
    /// uses a configuration source the check cannot read reliably.
    pub fn check_platform_consistency(&self) -> Result<(), Error> {
        Ok(cargo_config::check(&self.selection)?)
    }

    fn with_profile(&self, profile: Option<&Profile>) -> Selection {
        Selection {
            profile: profile.cloned(),
            ..self.selection.clone()
        }
    }
}

fn boxed(error: std::io::Error) -> Error {
    Error(Box::new(error))
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Project>();
    assert_send_sync::<Firmware>();
    assert_send_sync::<Error>();
};
