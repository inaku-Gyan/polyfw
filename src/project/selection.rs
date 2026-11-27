use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cargo::model::Selection;
use crate::cargo::runner::firmware_elf_path;
use crate::{Error, ErrorKind, InternalResult, Profile, Result, files};

use super::{FirmwareSummary, context::Context, vscode};

pub(super) const STATE_DIRECTORY: &str = ".polyfw-state";
const SELECTION_FILE: &str = ".polyfw-state/selected";

fn state_directory_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(metadata) if metadata.is_symlink() => Err(Error::new(
            ErrorKind::InvalidStateDirectory,
            format!(
                "{} is a symbolic link. Use a directory in the project root.",
                path.display()
            ),
        )),
        Ok(_) => Err(Error::new(
            ErrorKind::InvalidStateDirectory,
            format!("{} must be a directory", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::context(
            ErrorKind::InvalidStateDirectory,
            format!("could not inspect {}", path.display()),
            error,
        )),
    }
}

pub(super) fn create_state_directory(path: &Path) -> Result<bool> {
    if state_directory_exists(path)? {
        return Ok(false);
    }
    match fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error)
            if error.kind() == io::ErrorKind::AlreadyExists && state_directory_exists(path)? =>
        {
            Ok(false)
        }
        Err(error) => Err(Error::context(
            ErrorKind::SelectionWrite,
            format!("could not create {}", path.display()),
            error,
        )),
    }
}

/// How to choose a single firmware for an operation.
#[derive(Clone, Copy, Debug)]
pub enum FirmwareChoice<'a> {
    /// Use this firmware and the supplied profile. Access only configuration and Cargo data.
    Explicit(&'a str),
    /// Validate saved state and inherit its profile if none is supplied. Use the given
    /// firmware ID when present. Otherwise use the saved firmware or call the picker.
    WithSaved(Option<&'a str>),
    /// Save before executing. With no ID, call the picker even when a selection exists.
    /// Save only the supplied profile, clearing any previously saved profile if omitted.
    Save(Option<&'a str>),
}

pub(super) struct ChosenFirmware {
    pub(super) id: String,
    pub(super) profile: Option<Profile>,
    pub(super) save: bool,
}

/// A single firmware or every registered firmware.
#[derive(Clone, Copy, Debug)]
pub enum FirmwareTarget<'a> {
    One(FirmwareChoice<'a>),
    /// Use the supplied profile and each firmware in registry order.
    All,
}

impl<'a> From<&'a str> for FirmwareTarget<'a> {
    fn from(firmware: &'a str) -> Self {
        Self::One(FirmwareChoice::Explicit(firmware))
    }
}

/// Valid candidates and the optional default presented to a firmware picker.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PickRequest<'a> {
    pub firmwares: &'a [FirmwareSummary<'a>],
    pub default: Option<&'a str>,
}

/// The firmware and optional profile stored in project-local selection state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SavedSelection {
    pub firmware: String,
    pub profile: Option<Profile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SelectionFile {
    firmware: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<Profile>,
}

/// The result of attempting to remove one project-local file.
#[derive(Debug)]
#[non_exhaustive]
pub struct Removal {
    /// Path relative to the project root.
    pub path: &'static str,
    pub result: io::Result<()>,
}

/// Results of clearing the editor file and saved selection, in that order when both are managed.
#[derive(Debug)]
#[non_exhaustive]
pub struct ClearReport {
    pub files: Vec<Removal>,
}

impl ClearReport {
    /// Returns true when each file was removed or was already absent.
    pub fn success(&self) -> bool {
        self.files.iter().all(|file| {
            file.result
                .as_ref()
                .map_or_else(|error| error.kind() == io::ErrorKind::NotFound, |()| true)
        })
    }
}

impl Context {
    pub(crate) fn pick_firmware(
        &self,
        saved: Option<&SavedSelection>,
        picker: Option<&mut super::Picker<'_>>,
    ) -> Result<String> {
        let Some(picker) = picker else {
            return Err(Error::new(
                ErrorKind::SelectionRequired,
                "no firmware selected",
            ));
        };
        let firmwares = self.firmwares().collect::<Vec<_>>();
        let request = PickRequest {
            firmwares: &firmwares,
            default: saved.map(|saved| saved.firmware.as_str()),
        };
        picker(request)
            .map_err(|error| Error::new(ErrorKind::SelectionPicker, error))?
            .ok_or_else(|| Error::new(ErrorKind::SelectionCancelled, "selection cancelled"))
    }

    pub(super) fn choose_firmware(
        &self,
        choice: FirmwareChoice<'_>,
        profile: Option<&Profile>,
        picker: Option<&mut super::Picker<'_>>,
    ) -> Result<ChosenFirmware> {
        let (id, profile) = match choice {
            FirmwareChoice::Explicit(firmware) => (firmware.to_owned(), profile.cloned()),
            FirmwareChoice::WithSaved(firmware) => {
                let saved = self.read_selection()?;
                let id = match (firmware, saved.as_ref()) {
                    (Some(id), _) => id.to_owned(),
                    (None, Some(saved)) => saved.firmware.clone(),
                    (None, None) => self.pick_firmware(None, picker)?,
                };
                (
                    id,
                    profile
                        .cloned()
                        .or_else(|| saved.and_then(|saved| saved.profile)),
                )
            }
            FirmwareChoice::Save(firmware) => {
                let id = match firmware {
                    Some(id) => id.to_owned(),
                    None => {
                        let saved = self.read_selection()?;
                        self.pick_firmware(saved.as_ref(), picker)?
                    }
                };
                (id, profile.cloned())
            }
        };
        self.require_firmware(&id)?;
        Ok(ChosenFirmware {
            id,
            profile,
            save: matches!(choice, FirmwareChoice::Save(_)),
        })
    }

    pub(super) fn select_firmware(
        &self,
        chosen: ChosenFirmware,
        options: &mut super::Options<'_>,
    ) -> Result<Selection> {
        let mut selection = self.selection(&chosen.id)?;
        selection.profile = chosen.profile;
        if chosen.save {
            self.save_with_observer(&chosen.id, selection.profile.as_ref(), options)?;
        }
        Ok(selection)
    }

    pub(super) fn save_with_observer(
        &self,
        id: &str,
        profile: Option<&Profile>,
        options: &mut super::Options<'_>,
    ) -> Result<SavedSelection> {
        let saved = self.save_selection(id, profile)?;
        if let Some(observer) = options.selection_saved.as_deref_mut() {
            observer(&saved);
        }
        Ok(saved)
    }

    pub(crate) fn read_selection(&self) -> Result<Option<SavedSelection>> {
        let saved = self.read_saved_selection()?;
        if let Some(saved) = &saved
            && self.firmware(&saved.firmware).is_none()
        {
            return Err(Error::new(
                ErrorKind::StaleSelection,
                format!(
                    "saved firmware {} at {} is no longer registered",
                    saved.firmware,
                    self.selection_path().display()
                ),
            ));
        }
        Ok(saved)
    }

    pub(crate) fn read_saved_selection(&self) -> Result<Option<SavedSelection>> {
        if !state_directory_exists(&self.root.join(STATE_DIRECTORY))? {
            return Ok(None);
        }
        let path = self.selection_path();
        let value = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(Error::context(
                    ErrorKind::InvalidSelection,
                    path.display().to_string(),
                    error,
                ));
            }
        };
        let saved: SelectionFile = toml::from_str(&value).map_err(|error| {
            Error::context(
                ErrorKind::InvalidSelection,
                format!("invalid saved selection at {}", path.display()),
                error,
            )
        })?;
        Ok(Some(SavedSelection {
            firmware: saved.firmware,
            profile: saved.profile,
        }))
    }

    pub(crate) fn selection_path(&self) -> PathBuf {
        self.root.join(SELECTION_FILE)
    }

    pub(crate) fn save_selection(
        &self,
        id: &str,
        profile: Option<&Profile>,
    ) -> Result<SavedSelection> {
        self.require_firmware(id)?;
        let saved = SelectionFile {
            firmware: id.to_owned(),
            profile: profile.cloned(),
        };
        let launch = if self.registry.config.vscode.enabled {
            let mut selection = self.selection(id)?;
            selection.profile = profile.cloned();
            let path = firmware_elf_path(&selection, None);
            self.validate_artifact(&selection, &path)?;
            Some(
                vscode::configuration(&selection, &path)
                    .map_err(|error| Error::new(ErrorKind::SelectionWrite, error))?,
            )
        } else {
            None
        };
        let _lock = self.lock_selection()?;
        self.write_selection(&saved, launch.as_deref())
            .map_err(|error| Error::new(ErrorKind::SelectionWrite, error))?;
        Ok(SavedSelection {
            firmware: saved.firmware,
            profile: saved.profile,
        })
    }

    fn write_selection(&self, saved: &SelectionFile, launch: Option<&str>) -> InternalResult<()> {
        let value = toml::to_string(saved)?;
        let path = self.selection_path();
        let Some(launch) = launch else {
            return files::replace(&path, value.as_bytes());
        };
        let launch_path = self.root.join(vscode::LAUNCH_FILE);
        let previous_launch = match fs::read(&launch_path) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("{}: {error}", launch_path.display()).into()),
        };
        files::replace(&launch_path, launch.as_bytes())?;
        if let Err(error) = files::replace(&path, value.as_bytes()) {
            let rollback = match previous_launch {
                Some(previous) => files::replace(&launch_path, &previous),
                None => fs::remove_file(&launch_path).map_err(Into::into),
            };
            rollback.map_err(|rollback| {
                format!(
                    "{error}; restoring {} also failed: {rollback}",
                    launch_path.display()
                )
            })?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn clear_selection(&self) -> Result<ClearReport> {
        let managed_launch = self.registry.config.vscode.enabled;
        let mut paths = Vec::with_capacity(2);
        if managed_launch {
            paths.push(vscode::LAUNCH_FILE);
        }
        paths.push(SELECTION_FILE);

        let launch_missing = !managed_launch
            || fs::symlink_metadata(self.root.join(vscode::LAUNCH_FILE))
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound);
        // Check the state directory last. Every writer creates it before modifying files.
        let report = if launch_missing && !state_directory_exists(&self.root.join(STATE_DIRECTORY))?
        {
            paths
                .into_iter()
                .map(|path| Removal {
                    path,
                    result: Err(io::ErrorKind::NotFound.into()),
                })
                .collect()
        } else {
            let _lock = self.lock_selection()?;
            paths
                .into_iter()
                .map(|path| Removal {
                    path,
                    result: fs::remove_file(self.root.join(path)),
                })
                .collect()
        };
        Ok(ClearReport { files: report })
    }

    fn lock_selection(&self) -> Result<fs::File> {
        let directory = self.root.join(STATE_DIRECTORY);
        create_state_directory(&directory)?;
        let path = directory.join("selection.lock");
        let lock = (|| -> io::Result<_> {
            let file = fs::File::options()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?;
            file.lock()?;
            Ok(file)
        })();
        lock.map_err(|error| {
            Error::context(
                ErrorKind::SelectionWrite,
                format!("could not lock selection at {}", path.display()),
                error,
            )
        })
    }
}
