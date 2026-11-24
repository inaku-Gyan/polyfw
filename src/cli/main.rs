#![forbid(unsafe_code)]

mod args;
mod dispatch;
mod json;
mod probe;
mod ui;

use std::process::ExitCode;

use polyfw::ErrorKind;

use crate::args::Cli;
use crate::ui::Ui;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> ExitCode {
    let (cli, mut command) = Cli::parse_with_color();
    let ui = Ui::new(cli.color);
    let result = match cli.command {
        None => command
            .print_help()
            .map(|()| ExitCode::SUCCESS)
            .map_err(Into::into),
        Some(task) => dispatch::run(task, cli.project_root.as_deref(), &ui),
    };
    match result {
        Ok(exit) => exit,
        Err(error) => {
            let advice = error.downcast_ref::<polyfw::Error>().map(|error| match error.kind() {
                ErrorKind::ProjectRoot => "; use --project-root <DIR>",
                ErrorKind::UnknownFirmware => "; run plfw info for valid names",
                ErrorKind::StaleSelection => ";\n    run `plfw select [FIRMWARE]` to replace it or `plfw select --clear` to reset selection",
                ErrorKind::InvalidSelection => "; run plfw select FIRMWARE or plfw select --clear",
                _ => "",
            }).unwrap_or_default();
            ui.error(&format!("{error}{advice}"));
            ExitCode::FAILURE
        }
    }
}
