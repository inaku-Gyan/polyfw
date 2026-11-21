use std::path::Path;
use std::process::Command;

mod cargo_env;

pub(crate) use cargo_env::isolate_cargo_environment;

#[cfg(feature = "cli")]
pub fn polyfw() -> Command {
    isolate_cargo_environment(Command::new(env!("CARGO_BIN_EXE_polyfw")))
}

pub fn workspace_cargo(directory: &Path) -> Command {
    let mut command = isolate_cargo_environment(Command::new("cargo"));
    command
        .current_dir(directory)
        .env_remove("RUSTUP_TOOLCHAIN");
    command
}
