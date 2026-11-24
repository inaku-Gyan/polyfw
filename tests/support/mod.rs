use std::path::Path;
use std::process::Command;

mod cargo_env;

pub(crate) use cargo_env::isolate_cargo_environment;

#[cfg(feature = "cli")]
pub fn plfw() -> Command {
    isolate_cargo_environment(Command::new(env!("CARGO_BIN_EXE_plfw")))
}

pub fn workspace_cargo(directory: &Path) -> Command {
    workspace_command(directory, "cargo")
}

pub fn workspace_command(directory: &Path, program: &str) -> Command {
    let mut command = isolate_cargo_environment(Command::new(program));
    command
        .current_dir(directory)
        .env_remove("RUSTUP_TOOLCHAIN");
    command
}
