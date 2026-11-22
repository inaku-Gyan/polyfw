use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use polyfw::{
    Check, FirmwareChoice, FirmwareTarget, InfoQuery, Options, OutputMode, PickRequest,
    ProgressEvent, Project, SavedSelection,
};

use crate::Result;
use crate::args::{FirmwareArgs, InfoJsonVersion, Task};
use crate::probe::{self, Action};
use crate::ui::Ui;

const ACTION_PICKER_HINT: &str = "pass a firmware name or run polyfw select in a terminal";

pub(crate) fn run(task: Task, root: Option<&Path>, ui: &Ui) -> Result<ExitCode> {
    if let Task::Schema { output } = task {
        match output {
            Some(path) => polyfw::write_configuration_schema(path)?,
            None => io::stdout()
                .lock()
                .write_all(polyfw::configuration_schema()?.as_bytes())?,
        }
        return Ok(ExitCode::SUCCESS);
    }
    let directory = std::env::current_dir()?;
    if let Task::Probes = task {
        probe::list(&directory, ui)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Task::Init = task {
        let root = directory.join(root.unwrap_or(Path::new(".")));
        let report = polyfw::initialize(root)?;
        ui.initialized(
            &report.configuration,
            &report.state_ignore,
            report.state_ignore_created,
        );
        return Ok(ExitCode::SUCCESS);
    }
    let project = match root {
        Some(root) => Project::open(directory.join(root))?,
        None => Project::discover(directory)?,
    };
    run_project(&project, task, ui)
}

fn run_project(project: &Project, task: Task, ui: &Ui) -> Result<ExitCode> {
    let picker_hint = if matches!(
        &task,
        Task::Info {
            firmware: Some(None),
            ..
        }
    ) {
        "pass a firmware name with `polyfw info --firmware <FIRMWARE>`"
    } else {
        ACTION_PICKER_HINT
    };
    let mut picker = |request: PickRequest<'_>| ui.prompt_firmware(request, picker_hint);
    let mut progress = |event: ProgressEvent<'_>| ui.progress_event(event);
    let mut saved = |selection: &SavedSelection| {
        ui.selected(&selection.firmware, selection.profile.as_ref());
    };
    let options = Options::new(OutputMode::Inherit)
        .with_picker(&mut picker)
        .with_progress(&mut progress)
        .on_selection_saved(&mut saved);
    match task {
        Task::Info { firmware, json } => {
            let query = match &firmware {
                None => InfoQuery::Overview,
                Some(Some(id)) => InfoQuery::Firmware(id),
                Some(None) => InfoQuery::Pick,
            };
            let report = project.info(query, options)?;
            match json {
                Some(InfoJsonVersion::V1 | InfoJsonVersion::Latest) => {
                    println!("{}", crate::json::info_v1(&report)?);
                }
                None => ui.print_info(&report, !matches!(query, InfoQuery::Pick)),
            }
        }
        Task::Select { clear: true, .. } => {
            let report = project.clear_selection()?;
            for removal in &report.files {
                ui.removal(removal.path, &removal.result);
            }
            return Ok(if report.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        Task::Select {
            firmware, profile, ..
        } => {
            project.save_selection(firmware.as_deref(), profile.explicit().as_ref(), options)?;
        }
        Task::Build { firmwares, profile } => {
            project.build(target(&firmwares), profile.explicit().as_ref(), options)?;
        }
        Task::Clean(firmwares) => {
            project.clean(target(&firmwares), options)?;
        }
        Task::Fmt(firmwares) => {
            project.check(Check::Format, target(&firmwares), options)?;
        }
        Task::Lint(firmwares) => {
            project.check(Check::Lint, target(&firmwares), options)?;
        }
        Task::Test(firmwares) => {
            project.check(Check::Test, target(&firmwares), options)?;
        }
        Task::Verify(firmwares) => {
            project.check(Check::Verify, target(&firmwares), options)?;
        }
        Task::Run {
            target,
            profile,
            monitor,
        } => {
            probe::run(
                project,
                ui,
                Action::Run { monitor: &monitor },
                &target,
                profile.explicit().as_ref(),
                options,
            )?;
        }
        Task::Flash { target, profile } => {
            probe::run(
                project,
                ui,
                Action::Flash,
                &target,
                profile.explicit().as_ref(),
                options,
            )?;
        }
        Task::Attach {
            target,
            profile,
            monitor,
        } => {
            probe::run(
                project,
                ui,
                Action::Attach { monitor: &monitor },
                &target,
                profile.explicit().as_ref(),
                options,
            )?;
        }
        Task::Reset(target) => {
            probe::run(project, ui, Action::Reset, &target, None, options)?;
        }
        Task::Init | Task::Schema { .. } | Task::Probes => {
            unreachable!("handled before opening the project")
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn target(args: &FirmwareArgs) -> FirmwareTarget<'_> {
    if args.all_firmwares {
        FirmwareTarget::All
    } else {
        FirmwareTarget::One(choice(args.firmware.as_deref(), args.select))
    }
}

pub(crate) fn choice(firmware: Option<&str>, save: bool) -> FirmwareChoice<'_> {
    if save {
        FirmwareChoice::Save(firmware)
    } else {
        FirmwareChoice::WithSaved(firmware)
    }
}
