use std::path::Path;
use std::process::ExitCode;

use crate::cli::{Cli, Task};
use crate::registry::Context;
use crate::ui::Ui;
use crate::{Result, commands, init, probe, registry};

/// Run the polyfw executable.
pub fn main() -> ExitCode {
    let (cli, mut command) = Cli::parse_with_color();
    let ui = Ui::new(cli.color);

    let result = match cli.command {
        None => command
            .print_help()
            .map(|()| ExitCode::SUCCESS)
            .map_err(Into::into),
        Some(task) => run(task, cli.project_root.as_deref(), &ui),
    };

    match result {
        Ok(exit) => exit,
        Err(error) => {
            ui.error(&error);
            ExitCode::FAILURE
        }
    }
}

fn run(task: Task, project_root: Option<&Path>, ui: &Ui) -> Result<ExitCode> {
    if let Task::Schema { output } = task {
        registry::write_schema(output.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Task::Probes = task {
        probe::list(&std::env::current_dir()?, ui)?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Task::Init = task {
        let root = registry::project_root(
            Some(project_root.unwrap_or(Path::new("."))),
            &std::env::current_dir()?,
        )?;
        init::run(&root, ui)?;
        return Ok(ExitCode::SUCCESS);
    }
    let root = registry::project_root(project_root, &std::env::current_dir()?)?;
    let context = match &task {
        Task::Select { clear: true, .. } => Context::load_config(root)?,
        Task::Reset(target) => {
            let mut context = Context::load_config(root)?;
            if target.select && context.registry.config.vscode.enabled {
                context.resolve_cargo()?;
            }
            context
        }
        _ => Context::load(root)?,
    };
    commands::run(context, task, ui)
}
