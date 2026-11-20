#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use polyfw::{Profile, Project};

struct Example {
    root: PathBuf,
}

impl Example {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("polyfw-library-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        copy_project(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples")
                .join(name),
            &root,
        );
        Self { root }
    }
}

impl Drop for Example {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_project(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if matches!(
            entry.file_name().to_str(),
            Some("target" | ".polyfw-state" | ".vscode")
        ) {
            continue;
        }
        let path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_project(&entry.path(), &path);
        } else {
            fs::copy(entry.path(), path).unwrap();
        }
    }
}

#[test]
fn a_project_reports_firmwares_selection_and_a_buildable_command() {
    let example = Example::new("multi-firmware");
    let project = Project::discover(example.root.join("firmwares/led-demo/src")).unwrap();
    assert_eq!(project.root(), example.root);

    let ids: Vec<String> = project.firmwares().map(|f| f.id().to_owned()).collect();
    assert_eq!(ids, ["buzzer-demo", "led-demo"]);
    let platforms: Vec<String> = project.platforms().map(|p| p.id().to_owned()).collect();
    assert_eq!(platforms, ["dm-mc02"]);

    let firmware = project.firmware("led-demo").unwrap();
    let platform = firmware.platform();
    assert_eq!(platform.chip(), "STM32H723VGTx");
    assert_eq!(platform.cpu(), "cortex-m7");
    assert_eq!(firmware.target(), "thumbv7em-none-eabihf");
    assert_eq!(firmware.package(), "led-demo");
    assert_eq!(firmware.bin(), "led-demo");
    assert_eq!(firmware.workspace(), example.root);

    let release: Profile = "release".parse().unwrap();
    let elf = firmware.elf_path(Some(&release));
    assert_eq!(
        elf,
        firmware
            .target_dir()
            .join("thumbv7em-none-eabihf/release/led-demo")
    );
    assert!(
        firmware.elf_path(None).ends_with("debug/led-demo"),
        "{}",
        firmware.elf_path(None).display()
    );

    let error = project.firmware("missing").unwrap_err();
    assert!(
        error.to_string().contains("unknown firmware: missing"),
        "{error}"
    );

    assert_eq!(project.saved_selection().unwrap(), None);
    project.save_selection("led-demo", Some(&release)).unwrap();
    let saved = project.saved_selection().unwrap().unwrap();
    assert_eq!(saved.firmware(), "led-demo");
    assert_eq!(saved.profile(), Some(&release));
    project.clear_selection().unwrap();
    assert_eq!(project.saved_selection().unwrap(), None);
    project.clear_selection().unwrap();

    firmware.check_platform_consistency().unwrap();
    let mut command = firmware.build_command(Some(&release));
    assert_eq!(command.get_current_dir(), Some(firmware.working_dir()));
    let args: Vec<&OsStr> = command.get_args().collect();
    for expected in [
        "build",
        "-p",
        "led-demo",
        "--locked",
        "--profile",
        "release",
    ] {
        assert!(args.contains(&OsStr::new(expected)), "{args:?}");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // ELF files start with 0x7f followed by "ELF".
    assert_eq!(&fs::read(&elf).unwrap()[..4], b"\x7fELF");
}

#[test]
fn opening_a_directory_without_a_configuration_fails() {
    let directory =
        std::env::temp_dir().join(format!("polyfw-library-empty-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let error = Project::open(&directory).unwrap_err();
    fs::remove_dir_all(&directory).unwrap();
    assert!(error.to_string().contains("polyfw.toml"), "{error}");
}
