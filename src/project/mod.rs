mod artifact;
pub(crate) mod context;
mod info;
mod init;
mod selection;
mod vscode;
mod workflow;

use std::io;
use std::path::Path;

use crate::cargo::model::Selection;
use crate::{OutputMode, Profile, ProgressEvent, Result, RunReport};

use context::{Context, discover_root, exact_root};

pub use info::{Artifact, DependencyInfo, FirmwareInfo, PlatformInfo, ProjectInfo};
pub use init::{InitReport, initialize};
pub use selection::{
    ClearReport, FirmwareChoice, FirmwareTarget, PickRequest, Removal, SavedSelection,
};

/// Firmware declarations available directly from project configuration.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct FirmwareSummary<'a> {
    pub id: &'a str,
    pub description: Option<&'a str>,
    pub platform: &'a str,
    pub package: &'a str,
    pub chip: &'a str,
}

/// Choose between the project overview and the detailed report for one firmware.
#[derive(Clone, Copy, Debug)]
pub enum InfoQuery<'a> {
    Overview,
    Firmware(&'a str),
    /// Ask the supplied picker to choose a firmware for a read-only query.
    Pick,
}

/// Artifact and child-process results from a successful build.
#[derive(Debug)]
#[non_exhaustive]
pub struct BuildReport {
    pub artifacts: Vec<Artifact>,
    pub execution: RunReport,
}

type Picker<'a> = dyn FnMut(PickRequest<'_>) -> io::Result<Option<String>> + 'a;

/// Callbacks and child output handling for a synchronous operation.
///
/// The default captures Cargo output. Operations that require a choice return
/// [`crate::ErrorKind::SelectionRequired`] when a picker is absent.
#[derive(Default)]
pub struct Options<'a> {
    output: OutputMode,
    progress: Option<&'a mut dyn FnMut(ProgressEvent<'_>)>,
    picker: Option<&'a mut Picker<'a>>,
    selection_saved: Option<&'a mut dyn FnMut(&SavedSelection)>,
}

impl<'a> Options<'a> {
    /// Chooses where child output goes.
    pub fn new(output: OutputMode) -> Self {
        Self {
            output,
            ..Self::default()
        }
    }

    /// Receives stage notifications on the calling thread.
    pub fn with_progress(mut self, progress: &'a mut impl FnMut(ProgressEvent<'_>)) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Supplies a firmware picker. Return `Ok(None)` to cancel.
    pub fn with_picker(
        mut self,
        picker: &'a mut impl FnMut(PickRequest<'_>) -> io::Result<Option<String>>,
    ) -> Self {
        self.picker = Some(picker);
        self
    }

    /// Receives the committed selection before any subsequent action starts.
    pub fn on_selection_saved(mut self, observer: &'a mut impl FnMut(&SavedSelection)) -> Self {
        self.selection_saved = Some(observer);
        self
    }
}

/// An opened host-side project and its validated configuration.
///
/// Configuration is read when opening. Cargo resolution and saved-state access
/// happen when an operation requests them. Open a new handle after editing the
/// project configuration.
#[derive(Debug)]
pub struct Project {
    context: Context,
}

impl Project {
    /// Opens the registry in an exact directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = exact_root(root.as_ref())?;
        Ok(Self {
            context: Context::load_config(root)?,
        })
    }

    /// Opens the nearest registry in the ancestors of an existing directory.
    pub fn discover(from_dir: impl AsRef<Path>) -> Result<Self> {
        let root = discover_root(from_dir.as_ref())?;
        Ok(Self {
            context: Context::load_config(root)?,
        })
    }

    /// Returns the canonical project root.
    pub fn root(&self) -> &Path {
        &self.context.root
    }

    /// Lists configured firmware in ID order, without running Cargo.
    pub fn firmwares(&self) -> impl ExactSizeIterator<Item = FirmwareSummary<'_>> {
        self.context.firmwares()
    }

    /// Looks up one firmware declaration without running Cargo.
    pub fn firmware(&self, id: &str) -> Option<FirmwareSummary<'_>> {
        self.context.firmware(id)
    }

    /// Lists configured platforms in ID order, without running Cargo.
    pub fn platforms(&self) -> impl ExactSizeIterator<Item = (&str, PlatformInfo)> {
        let registry = &self.context.registry;
        registry
            .platforms
            .keys()
            .map(move |id| (id.as_str(), PlatformInfo::load(registry, id)))
    }

    /// Resolves Cargo information and reports the saved selection, including stale firmware IDs.
    pub fn info(&self, query: InfoQuery<'_>, mut options: Options<'_>) -> Result<ProjectInfo> {
        let context = self.context.resolved()?;
        let picked;
        let firmware = match query {
            InfoQuery::Overview => None,
            InfoQuery::Firmware(id) => Some(id),
            InfoQuery::Pick => {
                let saved = context.read_saved_selection()?;
                picked = context.pick_firmware(saved.as_ref(), options.picker.as_deref_mut())?;
                Some(picked.as_str())
            }
        };
        ProjectInfo::load(&context, firmware)
    }

    /// Chooses a firmware from configuration. Saving may resolve Cargo to generate editor files.
    pub fn resolve_firmware(
        &self,
        choice: FirmwareChoice<'_>,
        mut options: Options<'_>,
    ) -> Result<FirmwareSummary<'_>> {
        let resolved;
        let context = if matches!(choice, FirmwareChoice::Save(_))
            && self.context.registry.config.vscode.enabled
        {
            resolved = self.context.resolved()?;
            &resolved
        } else {
            &self.context
        };
        let (id, _) = context.choose_firmware(choice, None, &mut options)?;
        self.context.require_firmware(&id)
    }

    /// Resolves an expected ELF path and checks whether a file is present there.
    pub fn artifact(
        &self,
        choice: FirmwareChoice<'_>,
        profile: Option<&Profile>,
        options: Options<'_>,
    ) -> Result<Artifact> {
        let (_, selection) = self.select_artifact(choice, profile, options)?;
        Ok(Artifact::from_selection(&selection, None))
    }

    /// Resolves an ELF for hardware use and rejects paths shared by different builds or chips.
    ///
    /// The returned artifact reports whether the file is present.
    pub fn artifact_for_hardware(
        &self,
        choice: FirmwareChoice<'_>,
        profile: Option<&Profile>,
        options: Options<'_>,
    ) -> Result<Artifact> {
        let (context, selection) = self.select_artifact(choice, profile, options)?;
        let artifact = Artifact::from_selection(&selection, None);
        context.validate_artifact(&selection, &artifact.path)?;
        Ok(artifact)
    }

    fn select_artifact(
        &self,
        choice: FirmwareChoice<'_>,
        profile: Option<&Profile>,
        mut options: Options<'_>,
    ) -> Result<(Context, Selection)> {
        let context = self.context.resolved()?;
        let (id, profile) = context.choose_firmware(choice, profile, &mut options)?;
        let mut selection = context.selection(&id)?;
        selection.profile = profile;
        Ok((context, selection))
    }

    /// Reads saved state, including a firmware ID that has since been removed.
    pub fn saved_selection(&self) -> Result<Option<SavedSelection>> {
        self.context.read_saved_selection()
    }

    /// Saves a named or picked firmware with an optional explicit profile.
    pub fn save_selection(
        &self,
        firmware: Option<&str>,
        profile: Option<&Profile>,
        mut options: Options<'_>,
    ) -> Result<SavedSelection> {
        let context = self.context.resolved()?;
        let (firmware, profile) =
            context.choose_firmware(FirmwareChoice::Save(firmware), profile, &mut options)?;
        Ok(SavedSelection { firmware, profile })
    }

    /// Clears managed selection files without resolving Cargo or reading old state.
    pub fn clear_selection(&self) -> Result<ClearReport> {
        self.context.clear_selection()
    }
}
