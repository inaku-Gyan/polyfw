use std::error::Error;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::Path;

use crate::Result;
use crate::config_version::{CLI_VERSION, MIN_REQUIRED_CLI_VERSION};
use crate::registry::{REGISTRY_FILE, STATE_DIRECTORY};
use crate::ui::Ui;

pub(crate) fn run(root: &Path, ui: &Ui) -> Result<()> {
    let path = root.join(REGISTRY_FILE);
    // Reserve the configuration path before touching state to exclude concurrent initializers.
    let file = File::create_new(&path).map_err(|error| {
        if error.kind() == ErrorKind::AlreadyExists {
            format!(
                "{} already exists; move or remove it before running polyfw init",
                path.display()
            )
        } else {
            format!("could not create {}: {error}", path.display())
        }
    })?;
    let state_directory = root.join(STATE_DIRECTORY);
    let ignore_path = state_directory.join(".gitignore");
    let mut directory_created = false;
    let mut ignore_created = false;
    let result = (|| {
        directory_created = create_state_directory(&state_directory)?;
        if let Some(ignore) = create_state_ignore(&ignore_path)? {
            ignore_created = true;
            write_created_file(ignore, &ignore_path, b"*\n")?;
        }
        write_created_file(file, &path, template().as_bytes())
    })();
    if let Err(mut error) = result {
        if ignore_created {
            error = rollback(error, &ignore_path, fs::remove_file(&ignore_path));
        }
        if directory_created {
            error = rollback(error, &state_directory, fs::remove_dir(&state_directory));
        }
        return Err(rollback(error, &path, fs::remove_file(&path)));
    }
    ui.initialized(&path, &ignore_path, ignore_created);
    Ok(())
}

fn create_state_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(false),
        Ok(_) => Err(format!("{} must be a directory", path.display()).into()),
        Err(error) if error.kind() == ErrorKind::NotFound => fs::create_dir(path)
            .map(|()| true)
            .map_err(|error| format!("could not create {}: {error}", path.display()).into()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display()).into()),
    }
}

fn create_state_ignore(path: &Path) -> Result<Option<File>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(None),
        Ok(_) => Err(format!("{} must be a regular file", path.display()).into()),
        Err(error) if error.kind() == ErrorKind::NotFound => File::create_new(path)
            .map(Some)
            .map_err(|error| format!("could not create {}: {error}", path.display()).into()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display()).into()),
    }
}

fn write_created_file(mut file: File, path: &Path, contents: &[u8]) -> Result<()> {
    file.write_all(contents)
        .map_err(|error| format!("could not write {}: {error}", path.display()).into())
}

fn rollback(error: Box<dyn Error>, path: &Path, result: io::Result<()>) -> Box<dyn Error> {
    match result {
        Err(rollback) if rollback.kind() != ErrorKind::NotFound => format!(
            "{error}\nCould not remove {} during rollback: {rollback}",
            path.display()
        )
        .into(),
        _ => error,
    }
}

fn template() -> String {
    let source = format!(
        include_str!("../assets/polyfw.toml"),
        CLI_VERSION = CLI_VERSION,
        MIN_REQUIRED_CLI_VERSION = MIN_REQUIRED_CLI_VERSION,
    );
    let (header, body) = source
        .split_once('\n')
        .expect("initialization template must contain a schema header and a body");
    assert!(
        header.starts_with("#:schema "),
        "initialization template must start with a schema directive"
    );
    format!(
        "#:schema https://raw.githubusercontent.com/inaku-Gyan/polyfw/v{CLI_VERSION}/assets/polyfw.schema.json\n{body}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_version;
    use crate::model::Registry;

    #[test]
    fn the_template_uses_the_release_contract_and_authoring_minimum() {
        let source = template();
        config_version::preflight(&source, Path::new("polyfw.toml")).unwrap();
        let registry: Registry = toml::from_str(&source).unwrap();
        assert_eq!(registry.config_version, CLI_VERSION);
        assert_eq!(registry.min_polyfw_version, MIN_REQUIRED_CLI_VERSION);

        let schema: serde_json::Value =
            serde_json::from_str(&crate::registry::schema_source().unwrap()).unwrap();
        assert_eq!(
            source.lines().next().unwrap().strip_prefix("#:schema "),
            schema["$id"].as_str()
        );
    }
}
