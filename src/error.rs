use std::fmt;

use crate::BoxError;
use crate::process::StageOutput;

/// Result of a polyfw library operation.
pub type Result<T> = std::result::Result<T, Error>;

/// The operation or condition that caused a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    ProjectRoot,
    Configuration,
    Initialization,
    InvalidProfile,
    UnknownFirmware,
    SelectionRequired,
    SelectionCancelled,
    SelectionPicker,
    InvalidSelection,
    /// The project's saved-selection directory is inaccessible or has the wrong file type.
    InvalidStateDirectory,
    StaleSelection,
    SelectionWrite,
    Cargo,
    AmbiguousArtifact,
    ProcessLaunch,
    ProcessFailed,
}

/// A library failure with an error category and its underlying cause.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    context: Option<String>,
    source: BoxError,
    stage: Option<Box<StageOutput>>,
}

impl Error {
    /// Classifies the failure for callers that need to handle it programmatically.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the failed stage's Cargo exit status and any captured output.
    pub fn failed_stage(&self) -> Option<&StageOutput> {
        self.stage.as_deref()
    }

    pub(crate) fn new(kind: ErrorKind, source: impl Into<BoxError>) -> Self {
        Self {
            kind,
            context: None,
            source: source.into(),
            stage: None,
        }
    }

    pub(crate) fn context(kind: ErrorKind, context: String, source: impl Into<BoxError>) -> Self {
        Self {
            kind,
            context: Some(context),
            source: source.into(),
            stage: None,
        }
    }

    pub(crate) fn process_failed(stage: StageOutput) -> Self {
        Self::new(
            ErrorKind::ProcessFailed,
            format!("{} failed with {}", stage.command, stage.status),
        )
        .with_stage(stage)
    }

    pub(crate) fn with_stage(mut self, stage: StageOutput) -> Self {
        self.stage = Some(Box::new(stage));
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(context) = &self.context {
            write!(formatter, "{context}: ")?;
        }
        fmt::Display::fmt(&self.source, formatter)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
