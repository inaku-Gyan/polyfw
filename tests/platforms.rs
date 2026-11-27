#![cfg(feature = "cli")]
#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod support;

struct Project {
    root: PathBuf,
    project: PathBuf,
    workspace: PathBuf,
}

#[test]
fn info_requires_a_terminal_to_pick_even_when_a_firmware_is_saved() {
    let fixture = Project::new("info-picker");
    fixture.config("", "target-dir = \"../images/led\"", "");
    let state = fixture.project.join(".polyfw-state/selected");
    let launch = fixture.project.join(".vscode/launch.json");
    for saved in [false, true] {
        if saved {
            succeed(
                fixture
                    .command(&["select", "led-demo", "--release"])
                    .output()
                    .unwrap(),
            );
        }
        let expected_state = fs::read(&state).ok();
        let expected_launch = fs::read(&launch).ok();
        for json in [false, true] {
            let mut args = vec!["info", "--firmware"];
            if json {
                args.push("--json");
            }
            let output = fixture.command(&args).output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("plfw info --firmware <FIRMWARE>")
            );
            assert_eq!(fs::read(&state).ok(), expected_state);
            assert_eq!(fs::read(&launch).ok(), expected_launch);
        }
    }
}

#[cfg(unix)]
#[test]
fn selection_operations_reject_a_state_directory_link_to_another_project() {
    use std::os::unix::fs::symlink;

    let source = Project::new("linked-state");
    let owner = Project::new("state-owner");
    for (fixture, firmware, profile) in [
        (&source, "led-demo", "--dev"),
        (&owner, "buzzer-demo", "--release"),
    ] {
        fixture.config(
            "",
            "target-dir = \"../images/led\"",
            "target-dir = \"../images/buzzer\"",
        );
        succeed(
            fixture
                .command(&["select", firmware, profile])
                .output()
                .unwrap(),
        );
    }

    let state = source.project.join(".polyfw-state");
    let launch = source.project.join(".vscode/launch.json");
    let owner_state = owner.project.join(".polyfw-state");
    let owner_selection = owner_state.join("selected");
    let owner_launch = owner.project.join(".vscode/launch.json");
    let previous_launch = fs::read(&launch).unwrap();
    let previous_owner_selection = fs::read(&owner_selection).unwrap();
    let previous_owner_launch = fs::read(&owner_launch).unwrap();
    fs::remove_dir_all(&state).unwrap();
    symlink(&owner_state, &state).unwrap();

    for args in [
        ["info", "--json"].as_slice(),
        &["select", "led-demo", "--dev"],
        &["select", "--clear"],
    ] {
        let output = source.command(args).output().unwrap();
        assert!(!output.status.success(), "{args:?} accepted a link");
        assert!(output.stdout.is_empty(), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(&state.display().to_string()), "{error}");
        assert!(error.contains("symbolic link"), "{error}");
        assert_eq!(fs::read(&launch).unwrap(), previous_launch);
        assert_eq!(
            fs::read(&owner_selection).unwrap(),
            previous_owner_selection
        );
        assert_eq!(fs::read(&owner_launch).unwrap(), previous_owner_launch);
    }
    assert_eq!(fs::read_link(state).unwrap(), owner_state);
}

impl Project {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("polyfw-platforms-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        let workspace = root.join("workspace");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(workspace.join(".cargo")).unwrap();
        for directory in [&root, &workspace] {
            fs::write(
                directory.join("rust-toolchain.toml"),
                include_str!("../rust-toolchain.toml"),
            )
            .unwrap();
        }
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"led-demo\", \"buzzer-demo\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        for name in ["led-demo", "buzzer-demo"] {
            let directory = workspace.join(name);
            fs::create_dir_all(directory.join("src")).unwrap();
            fs::write(directory.join("Cargo.toml"), format!("[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n[[bin]]\nname = \"demo-image\"\npath = \"src/main.rs\"\n")).unwrap();
            fs::write(
                directory.join("src/lib.rs"),
                "#![forbid(unsafe_code)]\npub fn value() -> bool { true }\n",
            )
            .unwrap();
            fs::write(
                directory.join("src/main.rs"),
                format!("fn main() {{ println!({name:?}); }}\n"),
            )
            .unwrap();
        }
        succeed(
            support::workspace_cargo(&workspace)
                .args(["generate-lockfile", "--offline"])
                .output()
                .unwrap(),
        );
        let fixture = Self {
            root,
            project,
            workspace,
        };
        fixture.config("", "", "");
        fixture
    }

    fn config(&self, platform: &str, led: &str, buzzer: &str) {
        fs::write(self.project.join("polyfw.toml"), format!(
            "config-version = \"0.3.0\"\nmin-polyfw-version = \"0.3.0\"\n\
             [platforms.demo-board]\ntarget = \"host-tuple\"\nchip = \"test-chip\"\nworking-dir = \"../workspace\"\n{platform}\n\
             [firmwares.led-demo]\ndescription = \"LED\"\nplatform = \"demo-board\"\npackage = \"led-demo\"\n{led}\n\
             [firmwares.buzzer-demo]\ndescription = \"Buzzer\"\nplatform = \"demo-board\"\npackage = \"buzzer-demo\"\n{buzzer}\n"
        )).unwrap();
    }

    fn split_workspaces(&self) {
        let buzzer_workspace = self.root.join("buzzer-workspace");
        fs::rename(self.workspace.join("buzzer-demo"), &buzzer_workspace).unwrap();
        fs::write(
            self.workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"led-demo\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        for workspace in [&self.workspace, &buzzer_workspace] {
            succeed(
                support::workspace_cargo(workspace)
                    .args(["generate-lockfile", "--offline"])
                    .output()
                    .unwrap(),
            );
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = support::plfw();
        command
            .current_dir(&self.project)
            .args(args)
            .env("CARGO_NET_OFFLINE", "true")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("NO_COLOR", "1");
        command
    }

    fn info(&self) -> serde_json::Value {
        serde_json::from_slice(&succeed(self.command(&["info", "--json"]).output().unwrap()).stdout)
            .unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn succeed(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn assert_ambiguous_image(output: Output, firmwares: [&str; 2], image: &str) {
    assert!(
        !output.status.success(),
        "command accepted the shared image"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    for expected in [
        "ambiguous firmware image",
        firmwares[0],
        firmwares[1],
        image,
        "target-dir",
    ] {
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[cfg(unix)]
#[test]
fn probe_actions_reject_shared_images_from_different_workspaces() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Project::new("ambiguous-probe-image");
    fixture.split_workspaces();
    fixture.config(
        "target-dir = \"../images\"",
        "",
        "working-dir = \"../buzzer-workspace\"",
    );
    succeed(
        fixture
            .command(&["build", "--all-firmwares", "--release"])
            .output()
            .unwrap(),
    );
    let info = fixture.info();
    let led = &info["firmwares"]["led-demo"]["artifacts"]["release"];
    let buzzer = &info["firmwares"]["buzzer-demo"]["artifacts"]["release"];
    let image = led["path"].as_str().unwrap();
    assert_eq!(buzzer["path"], image);
    assert_eq!(led["exists"], true);
    assert_eq!(buzzer["exists"], true);

    let tools = fixture.root.join("tools");
    fs::create_dir(&tools).unwrap();
    let probe = tools.join("probe-rs");
    fs::write(
        &probe,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$POLYFW_TEST_PROBE_LOG\"\n",
    )
    .unwrap();
    fs::set_permissions(probe, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let path =
        std::env::join_paths(std::iter::once(tools).chain(std::env::split_paths(&path))).unwrap();
    let log = fixture.root.join("probe-args");
    for action in ["flash", "run", "attach"] {
        let output = fixture
            .command(&[action, "led-demo", "--release"])
            .env("PATH", &path)
            .env("POLYFW_TEST_PROBE_LOG", &log)
            .output()
            .unwrap();
        assert!(!log.exists(), "{action} launched probe-rs");
        assert_ambiguous_image(output, ["led-demo", "buzzer-demo"], image);
    }
}

#[test]
fn an_ambiguous_debug_image_leaves_the_existing_selection_and_launch_file_intact() {
    let fixture = Project::new("ambiguous-debug-image");
    fixture.config(
        "",
        "target-dir = \"images/led\"",
        "target-dir = \"images/buzzer\"",
    );
    succeed(
        fixture
            .command(&["select", "buzzer-demo", "--dev"])
            .output()
            .unwrap(),
    );
    let state = fixture.project.join(".polyfw-state/selected");
    let launch = fixture.project.join(".vscode/launch.json");
    let previous_state = fs::read(&state).unwrap();
    let previous_launch = fs::read(&launch).unwrap();
    let config = fixture.project.join("polyfw.toml");
    let source = fs::read_to_string(&config).unwrap();
    let conflict = format!(
        "{source}\n[platforms.other-board]\ntarget = \"host-tuple\"\nchip = \"another-chip\"\n\
         working-dir = \"../workspace\"\ntarget-dir = \"images/led\"\n\
         [firmwares.led-other-chip]\nplatform = \"other-board\"\npackage = \"led-demo\"\n"
    );
    fs::write(&config, &conflict).unwrap();
    let info = fixture.info();
    let artifact = &info["firmwares"]["led-other-chip"]["artifacts"]["release"];
    assert_eq!(artifact["exists"], false);
    assert_ambiguous_image(
        fixture
            .command(&["select", "led-other-chip", "--release"])
            .output()
            .unwrap(),
        ["led-demo", "led-other-chip"],
        artifact["path"].as_str().unwrap(),
    );
    assert_eq!(fs::read(&state).unwrap(), previous_state);
    assert_eq!(fs::read(&launch).unwrap(), previous_launch);

    fs::write(
        &config,
        format!("{conflict}\n[config.vscode]\nenabled = false\n"),
    )
    .unwrap();
    succeed(
        fixture
            .command(&["select", "led-other-chip", "--release"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        fixture.info()["saved_selection"]["firmware"],
        "led-other-chip"
    );
    assert_eq!(fs::read(&launch).unwrap(), previous_launch);
}

#[cfg(unix)]
#[test]
fn debug_setup_detects_shared_output_through_a_symlink_before_building() {
    use std::os::unix::fs::symlink;

    let fixture = Project::new("symlinked-image");
    let images = fixture.root.join("images");
    fs::create_dir(&images).unwrap();
    symlink(&images, fixture.root.join("alias")).unwrap();
    fixture.config(
        "",
        "target-dir = \"../images/new\"",
        "target-dir = \"../alias/new\"",
    );
    let info = fixture.info();
    let image = info["firmwares"]["led-demo"]["artifacts"]["dev"]["path"]
        .as_str()
        .unwrap();
    assert!(!images.join("new").exists());
    assert_ambiguous_image(
        fixture.command(&["select", "led-demo"]).output().unwrap(),
        ["led-demo", "buzzer-demo"],
        image,
    );
    assert!(!fixture.project.join(".vscode/launch.json").exists());
    assert!(!fixture.project.join(".polyfw-state/selected").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn debug_setup_distinguishes_custom_targets_with_the_same_filename() {
    let fixture = Project::new("custom-target-image");
    let output = succeed(
        support::workspace_command(&fixture.workspace, "rustc")
            .env("RUSTC_BOOTSTRAP", "1")
            .args(["-Z", "unstable-options", "--print", "target-spec-json"])
            .output()
            .unwrap(),
    );
    let mut spec: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    spec["exe-suffix"] = ".elf".into();
    for (directory, panic_strategy) in [("target-a", "abort"), ("target-b", "unwind")] {
        let directory = fixture.workspace.join(directory);
        fs::create_dir(&directory).unwrap();
        spec["panic-strategy"] = panic_strategy.into();
        fs::write(
            directory.join("board.json"),
            serde_json::to_vec(&spec).unwrap(),
        )
        .unwrap();
    }
    std::os::unix::fs::symlink(
        "../target-a/board.json",
        fixture.workspace.join("target-b/alias.json"),
    )
    .unwrap();
    let cache = fixture.root.join("cache");
    fs::create_dir(&cache).unwrap();
    std::os::unix::fs::symlink(cache, fixture.root.join("cache-alias")).unwrap();

    let config = fixture.project.join("polyfw.toml");
    let mut source = fs::read_to_string(&config).unwrap();
    for (id, target, cache) in [
        ("target-a", "target-a/board.json", "../cache/new"),
        ("target-b", "target-b/alias.json", "../cache-alias/new"),
    ] {
        source.push_str(&format!(
            "\n[platforms.{id}]\ntarget = {target:?}\nchip = \"test-chip\"\n\
             working-dir = \"../workspace\"\ntarget-dir = \"images\"\nbuild-dir = {cache:?}\n\
             [firmwares.{id}]\nplatform = {id:?}\npackage = \"led-demo\"\n"
        ));
    }
    fs::write(&config, &source).unwrap();
    succeed(fixture.command(&["select", "target-a"]).output().unwrap());

    fs::write(
        &config,
        source.replace("target-b/alias.json", "target-b/board.json"),
    )
    .unwrap();
    let info = fixture.info();
    let image = info["firmwares"]["target-a"]["artifacts"]["dev"]["path"]
        .as_str()
        .unwrap();
    assert!(
        image.ends_with("images/board/debug/demo-image.elf"),
        "{image}"
    );
    assert_ambiguous_image(
        fixture.command(&["select", "target-a"]).output().unwrap(),
        ["target-a", "target-b"],
        image,
    );
}

#[test]
fn build_rejects_a_skipped_binary_even_when_an_old_executable_exists() {
    let fixture = Project::new("required-features");
    let manifest = fixture.workspace.join("buzzer-demo/Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap();
    let gated = format!("{source}required-features = [\"device\"]\n[features]\ndevice = []\n");
    let rejected_build = |arguments: &[&str]| {
        let output = fixture.command(arguments).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        for expected in [
            "binary `demo-image` of firmware `buzzer-demo`",
            "required Cargo features (device)",
            "[fail] failed",
        ] {
            assert!(stderr.contains(expected), "{stderr}");
        }
        stderr
    };

    fs::write(&manifest, &gated).unwrap();
    rejected_build(&["build", "buzzer-demo"]);
    assert_eq!(
        fixture.info()["firmwares"]["buzzer-demo"]["artifacts"]["dev"]["exists"],
        false
    );

    fs::write(&manifest, format!("{gated}default = [\"device\"]\n")).unwrap();
    for _ in 0..2 {
        let output = succeed(fixture.command(&["build", "buzzer-demo"]).output().unwrap());
        assert!(output.stdout.is_empty());
    }
    let info = fixture.info();
    let artifact = Path::new(
        info["firmwares"]["buzzer-demo"]["artifacts"]["dev"]["path"]
            .as_str()
            .unwrap(),
    );
    let previous = fs::read(artifact).unwrap();

    fs::write(&manifest, gated).unwrap();
    let stderr = rejected_build(&["build", "--all-firmwares"]);
    assert!(!stderr.contains("[2/2]"), "{stderr}");
    assert_eq!(fs::read(artifact).unwrap(), previous);
}

#[cfg(target_os = "linux")]
#[test]
fn launch_paths_round_trip_literal_backslashes_in_linux_directories() {
    let fixture = Project::new("backslash-output");
    fixture.config("", r#"target-dir = 'out\logs'"#, "");
    let output_dir = fixture.project.join(r"out\logs");
    let output = succeed(
        support::workspace_cargo(&fixture.workspace)
            .args([
                "build",
                "--locked",
                "--offline",
                "--target",
                "host-tuple",
                "-p",
                "led-demo",
                "--message-format=json",
            ])
            .env("CARGO_TARGET_DIR", &output_dir)
            .output()
            .unwrap(),
    );
    let executable = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .expect("Cargo reported a built executable");
    assert!(executable.starts_with(output_dir));
    assert!(executable.is_file());

    succeed(fixture.command(&["select", "led-demo"]).output().unwrap());
    let info = fixture.info();
    let artifact = &info["firmwares"]["led-demo"]["artifacts"]["dev"];
    assert_eq!(artifact["exists"], true);
    assert_eq!(Path::new(artifact["path"].as_str().unwrap()), executable);

    let launch: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.project.join(".vscode/launch.json")).unwrap())
            .unwrap();
    for entry in launch["configurations"].as_array().unwrap() {
        let path = entry["coreConfigs"][0]["programBinary"].as_str().unwrap();
        assert_eq!(Path::new(path), executable, "{}", entry["request"]);
    }
}

#[test]
fn omitted_working_directories_use_the_configuration_directory_and_allow_firmware_overrides() {
    let fixture = Project::new("working-defaults");
    let source = fs::read_to_string(fixture.project.join("polyfw.toml")).unwrap();
    let mut config: toml::Value = toml::from_str(&source).unwrap();
    config["platforms"]["demo-board"]
        .as_table_mut()
        .unwrap()
        .remove("working-dir");
    let path = fixture.workspace.join("polyfw.toml");
    let info = || {
        let output = succeed(
            fixture
                .command(&["info", "--json", "--project-root"])
                .arg(&fixture.workspace)
                .output()
                .unwrap(),
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    for firmware in info()["firmwares"].as_object().unwrap().values() {
        assert_eq!(
            Path::new(firmware["working_dir"].as_str().unwrap()),
            fixture.workspace
        );
    }

    config["firmwares"]["led-demo"]
        .as_table_mut()
        .unwrap()
        .insert("working-dir".into(), "led-demo".into());
    fs::write(path, toml::to_string(&config).unwrap()).unwrap();
    let info = info();
    for (id, directory) in [
        ("led-demo", fixture.workspace.join("led-demo")),
        ("buzzer-demo", fixture.workspace.clone()),
    ] {
        assert_eq!(
            Path::new(info["firmwares"][id]["working_dir"].as_str().unwrap()),
            directory
        );
        assert_eq!(
            Path::new(info["firmwares"][id]["workspace"].as_str().unwrap()),
            fixture.workspace
        );
    }
}

#[test]
fn info_displays_platform_descriptions_and_omits_missing_firmware_description_rows() {
    let fixture = Project::new("labels");
    fixture.config("description = \"Shared demo board\"", "", "");
    let path = fixture.project.join("polyfw.toml");
    let source = fs::read_to_string(&path).unwrap();
    let mut config: toml::Value = toml::from_str(&source).unwrap();
    for id in ["led-demo", "buzzer-demo"] {
        config["firmwares"][id]
            .as_table_mut()
            .unwrap()
            .remove("description");
    }
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let info = fixture.info();
    assert_eq!(
        info["platforms"]["demo-board"]["description"],
        "Shared demo board"
    );
    for firmware in info["firmwares"].as_object().unwrap().values() {
        assert!(firmware.get("description").is_none());
    }
    for args in [vec!["info"], vec!["info", "--firmware", "led-demo"]] {
        let output = succeed(fixture.command(&args).output().unwrap());
        let text = String::from_utf8(output.stdout).unwrap();
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("Shareddemoboard"), "{text}");
        if args.len() > 1 {
            assert!(
                !text
                    .lines()
                    .any(|line| line.split_whitespace().next() == Some("description")),
                "{text}"
            );
        }
    }

    config["platforms"]["demo-board"]
        .as_table_mut()
        .unwrap()
        .remove("description");
    fs::write(path, toml::to_string(&config).unwrap()).unwrap();
    let output = succeed(
        fixture
            .command(&["info", "--firmware", "led-demo"])
            .output()
            .unwrap(),
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("description"), "{text}");
}

#[test]
fn cargo_fixtures_use_their_own_output_directories() {
    let output = succeed(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "directory_inheritance_and_native_defaults_use_cargos_actual_workspace_and_outputs",
            ])
            .env("CARGO_BUILD_TARGET_DIR", "ambient-target")
            .env("CARGO_BUILD_BUILD_DIR", "ambient-build")
            .output()
            .unwrap(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed"), "{stdout}");
}

#[test]
fn directory_inheritance_and_native_defaults_use_cargos_actual_workspace_and_outputs() {
    let fixture = Project::new("directories");
    let buzzer = fixture.root.join("images/buzzer");
    fixture.config(
        "build-dir = \"../cache\"\ntarget-dir = \"../images/default\"",
        "target-dir = \"../images/led\"",
        &format!(
            "target-dir = {}",
            toml::Value::String(buzzer.display().to_string())
        ),
    );
    for name in ["cache", "images/led", "images/buzzer"] {
        fs::create_dir_all(fixture.root.join(name)).unwrap();
    }
    let info = fixture.info();
    for (id, output) in [
        ("led-demo", fixture.root.join("images/led")),
        ("buzzer-demo", buzzer),
    ] {
        let firmware = &info["firmwares"][id];
        assert_eq!(
            Path::new(firmware["working_dir"].as_str().unwrap()),
            fixture.workspace
        );
        assert_eq!(
            Path::new(firmware["workspace"].as_str().unwrap()),
            fixture.workspace
        );
        assert_eq!(
            Path::new(firmware["build_dir"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            fixture.root.join("cache")
        );
        assert_eq!(
            Path::new(firmware["target_dir"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            output
        );
    }
    fixture.config("", "", "");
    fs::write(
        fixture.workspace.join(".cargo/config.toml"),
        "[build]\ntarget-dir = \"../cargo-output\"\nbuild-dir = \"../cargo-cache\"\n",
    )
    .unwrap();
    for name in ["cargo-output", "cargo-cache"] {
        fs::create_dir_all(fixture.root.join(name)).unwrap();
    }
    let info = fixture.info();
    assert_eq!(
        Path::new(
            info["firmwares"]["led-demo"]["target_dir"]
                .as_str()
                .unwrap()
        )
        .canonicalize()
        .unwrap(),
        fixture.root.join("cargo-output")
    );
    assert_eq!(
        Path::new(info["firmwares"]["led-demo"]["build_dir"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        fixture.root.join("cargo-cache")
    );

    let override_dir = fixture.root.join("output-override");
    fs::create_dir_all(&override_dir).unwrap();
    let output = succeed(
        fixture
            .command(&["info", "--json"])
            .env("CARGO_BUILD_TARGET_DIR", &override_dir)
            .output()
            .unwrap(),
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let target_dir = info["firmwares"]["led-demo"]["target_dir"]
        .as_str()
        .unwrap();
    assert_eq!(Path::new(target_dir).canonicalize().unwrap(), override_dir);
}

#[test]
fn cargo_selects_the_native_flag_source_for_the_platform_target() {
    let fixture = Project::new("flag-precedence");
    let version = succeed(
        support::workspace_cargo(&fixture.workspace)
            .arg("-vV")
            .output()
            .unwrap(),
    );
    let version = String::from_utf8(version.stdout).unwrap();
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    fs::write(
        fixture.workspace.join(".cargo/config.toml"),
        format!(
            "[build]\ntarget = \"polyfw-file-target\"\nrustflags = \"--cfg from_build\"\n\
             [target.{host:?}]\nrustflags = [\"--cfg\", \"from_file\"]\n\
             [target.'cfg(all())']\nrustflags = \"--cfg from_cfg\"\n"
        ),
    )
    .unwrap();
    fs::write(
        fixture.workspace.join("led-demo/build.rs"),
        r#"fn main() {
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let expected = std::env::var("POLYFW_EXPECTED_RUSTFLAGS").unwrap();
    assert_eq!(flags, expected);
    println!("cargo::rerun-if-env-changed=POLYFW_EXPECTED_RUSTFLAGS");
}
"#,
    )
    .unwrap();
    let flag_key = format!(
        "CARGO_TARGET_{}_RUSTFLAGS",
        host.replace(['-', '.'], "_").to_uppercase()
    );
    let build = |environment: &[(&str, &str)], expected: &[&str]| {
        succeed(
            fixture
                .command(&["build", "led-demo"])
                .env("CARGO_BUILD_TARGET", "polyfw-env-target")
                .env(&flag_key, "--cfg from_env")
                .envs(environment.iter().copied())
                .env("POLYFW_EXPECTED_RUSTFLAGS", expected.join("\x1f"))
                .output()
                .unwrap(),
        );
    };
    build(
        &[],
        &[
            "--cfg",
            "from_file",
            "--cfg",
            "from_env",
            "--cfg",
            "from_cfg",
        ],
    );
    build(
        &[("RUSTFLAGS", "--cfg global_flags")],
        &["--cfg", "global_flags"],
    );
    build(
        &[
            ("RUSTFLAGS", "--cfg global_flags"),
            ("CARGO_ENCODED_RUSTFLAGS", ""),
        ],
        &[],
    );
}

#[test]
fn cargo_applies_includes_and_conditional_flags_to_builds_and_target_checks() {
    let fixture = Project::new("native-config");
    fs::write(
        fixture.workspace.join(".cargo/config.toml"),
        "include = [\"flags.toml\"]\n",
    )
    .unwrap();
    fs::write(
        fixture.workspace.join(".cargo/flags.toml"),
        "[target.'cfg(all())']\nrustflags = [\"--cfg\", \"conditional_flags\"]\n",
    )
    .unwrap();
    fs::write(fixture.workspace.join("led-demo/src/lib.rs"), "#![allow(unexpected_cfgs)]\n#[cfg(not(conditional_flags))]\ncompile_error!(\"native conditional flags were lost\");\npub fn value() -> bool { true }\n").unwrap();
    for command in ["build", "lint"] {
        succeed(fixture.command(&[command, "led-demo"]).output().unwrap());
    }
}

#[test]
fn cargo_uses_build_flags_when_target_flags_are_absent() {
    let fixture = Project::new("build-flags");
    fs::write(
        fixture.workspace.join(".cargo/config.toml"),
        "[build]\nrustflags = \"--cfg build_flags\"\n",
    )
    .unwrap();
    fs::write(fixture.workspace.join("led-demo/src/lib.rs"), "#![allow(unexpected_cfgs)]\n#[cfg(not(build_flags))]\ncompile_error!(\"native build flags were lost\");\npub fn value() -> bool { true }\n").unwrap();
    succeed(fixture.command(&["build", "led-demo"]).output().unwrap());
    assert_eq!(
        fixture.info()["firmwares"]["led-demo"]["artifacts"]["dev"]["exists"],
        true
    );
}

#[test]
fn removed_compiler_fields_are_rejected_in_configuration() {
    let fixture = Project::new("removed-field");
    for (platform, led, field) in [
        ("cpu = \"native\"", "", "cpu"),
        ("check-platform = true", "", "check-platform"),
        ("", "check-platform = false", "check-platform"),
    ] {
        fixture.config(platform, led, "");
        let output = fixture.command(&["info"]).output().unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains(&format!("unknown field `{field}`")),
            "{error}"
        );
    }
}

#[test]
fn a_firmware_with_an_explicit_rlib_can_build() {
    let fixture = Project::new("rlib-firmware");
    let package = fixture.workspace.join("led-demo");
    let manifest = package.join("Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!("{source}\n[lib]\ncrate-type = [\"rlib\"]\n"),
    )
    .unwrap();
    fs::write(
        package.join("src/main.rs"),
        "fn main() { assert!(led_demo::value()); }\n",
    )
    .unwrap();

    succeed(fixture.command(&["build", "led-demo"]).output().unwrap());
}

#[test]
fn an_rlib_example_does_not_count_as_a_firmware_library() {
    let fixture = Project::new("rlib-example");
    let package = fixture.workspace.join("led-demo");
    fs::rename(package.join("src/lib.rs"), package.join("src/example.rs")).unwrap();
    let manifest = package.join("Cargo.toml");
    let source = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!(
            "{source}\n[[example]]\nname = \"helper\"\npath = \"src/example.rs\"\ncrate-type = [\"rlib\"]\n"
        ),
    )
    .unwrap();

    let output = fixture.command(&["build", "led-demo"]).output().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("package led-demo has no library target"),
        "{error}"
    );
}

#[test]
fn shared_intermediates_keep_same_named_final_images_independent() {
    let fixture = Project::new("split-outputs");
    fixture.split_workspaces();
    fixture.config(
        "build-dir = \"../cache\"",
        "target-dir = \"../images/led\"",
        "target-dir = \"../images/buzzer\"\nworking-dir = \"../buzzer-workspace\"",
    );
    for id in ["led-demo", "buzzer-demo", "led-demo"] {
        succeed(fixture.command(&["build", id]).output().unwrap());
    }
    let info = fixture.info();
    for id in ["led-demo", "buzzer-demo"] {
        let artifact = &info["firmwares"][id]["artifacts"]["dev"];
        let image = artifact["path"].as_str().unwrap();
        let output = succeed(Command::new(image).output().unwrap());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), id);
        assert_eq!(artifact["exists"], true);
    }
}

#[test]
fn external_members_can_supply_the_working_directory_without_changing_workspace_ownership() {
    let fixture = Project::new("external-member");
    let member = fixture.root.join("external/led-demo");
    fs::create_dir_all(member.parent().unwrap()).unwrap();
    fs::rename(fixture.workspace.join("led-demo"), &member).unwrap();
    let manifest = member.join("Cargo.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("[package]", "[package]\nworkspace = \"../../workspace\""),
    )
    .unwrap();
    fs::write(
        fixture.workspace.join("Cargo.toml"),
        "[workspace]\nmembers = [\"../external/led-demo\", \"buzzer-demo\"]\nresolver = \"3\"\n",
    )
    .unwrap();
    succeed(
        support::workspace_cargo(&fixture.workspace)
            .args(["generate-lockfile", "--offline"])
            .output()
            .unwrap(),
    );
    fixture.config(
        "",
        &format!(
            "working-dir = {}",
            toml::Value::String(member.display().to_string())
        ),
        "",
    );
    let info = fixture.info();
    let firmware = &info["firmwares"]["led-demo"];
    assert_eq!(firmware["target"], "host-tuple");
    assert_eq!(firmware["chip"], "test-chip");
    assert_eq!(
        Path::new(firmware["manifest_path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        manifest
    );
    assert_eq!(
        Path::new(firmware["workspace"].as_str().unwrap()),
        fixture.workspace
    );
    assert_eq!(Path::new(firmware["working_dir"].as_str().unwrap()), member);
    succeed(fixture.command(&["build", "led-demo"]).output().unwrap());
}
