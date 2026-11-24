use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const REGISTRY_NAME_PATTERN: &str = "^[a-z][a-z0-9]*(-[a-z0-9]+)*$";

/// Project configuration for registered embedded firmware.
/// Cargo metadata additionally validates firmware workspace membership and binary targets at runtime.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "polyfw registry", extend(
    "$id" = concat!(
        "https://raw.githubusercontent.com/inaku-Gyan/polyfw/v",
        env!("CARGO_PKG_VERSION"),
        "/assets/polyfw.schema.json"
    )
))]
pub(crate) struct Registry {
    /// Full polyfw release version whose configuration contract this file follows.
    /// Includes any prerelease suffix. Does not change merely because a newer CLI runs.
    #[serde(rename = "config-version")]
    pub(crate) config_version: String,
    /// Oldest polyfw CLI release expected to interpret and execute this configuration
    /// correctly. Must not exceed config-version. The CLI also checks its own
    /// oldest supported configuration version before loading the registry.
    #[serde(rename = "min-polyfw-version")]
    pub(crate) min_polyfw_version: String,

    /// Optional tool settings; omitted values keep their defaults.
    #[serde(default)]
    pub(crate) config: Config,

    /// Reusable hardware/compiler settings, keyed by platform ID such as demo-board.
    #[schemars(extend("minProperties" = 1, "propertyNames" = {
        "type": "string",
        "description": "ID starting with a lowercase letter, followed by lowercase letters, digits or single hyphens.",
        "pattern": REGISTRY_NAME_PATTERN,
    }))]
    pub(crate) platforms: BTreeMap<String, Platform>,
    /// Selectable software entries, keyed by firmware ID such as `led-demo` or `buzzer-demo`.
    #[schemars(extend("minProperties" = 1, "propertyNames" = {
        "type": "string",
        "description": "ID starting with a lowercase letter, followed by lowercase letters, digits or single hyphens.",
        "pattern": REGISTRY_NAME_PATTERN,
    }))]
    pub(crate) firmwares: BTreeMap<String, Firmware>,
}

/// Project-wide checking and editor settings.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    /// Directory globs selecting reachable path packages for fmt, lint, test, verify
    /// and detailed info. Defaults to `["./**"]`; `[]` selects no dependency packages.
    /// The firmware itself is always included; registry and Git dependencies are excluded.
    ///
    /// Match each package's canonical Cargo.toml directory. Relative patterns start
    /// beside polyfw.toml; absolute paths and leading `..` can include external packages.
    /// `shared` matches only that directory, `shared/*` its immediate children, and
    /// `shared/**` the directory and all descendants. Literal prefixes are canonicalized;
    /// `..` must precede wildcards. Unmatched valid patterns select nothing; invalid
    /// patterns are errors. These globs select checks, not Cargo's dependency builds.
    #[serde(rename = "local-dependencies")]
    pub(crate) local_dependencies: Vec<String>,
    /// Editor-file management when saving or clearing firmware selection.
    pub(crate) vscode: VscodeConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            local_dependencies: vec!["./**".into()],
            vscode: VscodeConfig::default(),
        }
    }
}

/// VS Code launch-file generation settings.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct VscodeConfig {
    /// Replace the entire `.vscode/launch.json` on selection save and delete it on clear.
    /// Defaults to true. Set false to leave editor files untouched, including existing
    /// files generated while this option was enabled.
    pub(crate) enabled: bool,
}

impl Default for VscodeConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Reusable hardware and compilation settings, independent of Cargo workspace ownership.
#[derive(Debug, Default, Deserialize, Clone, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Platform {
    /// Optional human-readable label for platform information, independent of firmware descriptions.
    pub(crate) description: Option<String>,
    /// Explicit Cargo compilation target, including host-tuple or a custom JSON target.
    pub(crate) target: String,
    /// Declared target CPU. Cargo's native rustflags precedence determines its final use.
    pub(crate) cpu: String,
    /// probe-rs chip name for hardware commands and editor sessions, for example `STM32H723VGTx`.
    pub(crate) chip: String,
    /// Default Cargo working directory. Firmware may override it. Defaults to `"."` beside
    /// `polyfw.toml` when omitted at both levels; absolute paths and `..` are supported.
    #[serde(rename = "working-dir")]
    #[schemars(extend("default" = "."))]
    pub(crate) working_dir: Option<String>,
    /// Default intermediate/cache directory override (Cargo 1.91+). Omit at both levels
    /// to retain Cargo's configuration/environment defaults. Relative to `polyfw.toml`.
    #[serde(rename = "build-dir")]
    pub(crate) build_dir: Option<String>,
    /// Default final-output directory override. Omit at both levels for Cargo defaults.
    /// Relative to `polyfw.toml`, independently of `working-dir`.
    #[serde(rename = "target-dir")]
    pub(crate) target_dir: Option<String>,
}

/// A selectable software entry referring to one platform and Cargo package.
#[derive(Debug, Default, Deserialize, Clone, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Firmware {
    /// Optional human-readable label for firmware lists and the picker.
    /// Omission leaves the firmware without a description; it does not inherit the platform's.
    pub(crate) description: Option<String>,
    /// Platform ID declared in this registry's platforms table.
    pub(crate) platform: String,
    /// Cargo package name in the workspace discovered from the effective working directory. It must declare a host-testable library
    /// and at least one firmware binary.
    pub(crate) package: String,
    /// Firmware binary target name. Omit only if the package declares exactly one binary,
    /// counting feature-gated binaries independently of Cargo's default-run setting.
    pub(crate) bin: Option<String>,
    /// Cargo working directory overriding the platform; relative to `polyfw.toml`.
    #[serde(rename = "working-dir")]
    pub(crate) working_dir: Option<String>,
    /// Intermediate/cache directory overriding the platform; relative to `polyfw.toml`.
    #[serde(rename = "build-dir")]
    pub(crate) build_dir: Option<String>,
    /// Final-output directory overriding the platform; relative to `polyfw.toml`.
    #[serde(rename = "target-dir")]
    pub(crate) target_dir: Option<String>,
}
