use std::collections::BTreeMap;
use std::path::Path;

use polyfw::ProjectInfo;
use serde::Serialize;

#[derive(Serialize)]
struct InfoV2<'a> {
    format_version: u32,
    project_root: &'a Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_selection: Option<Selection<'a>>,
    platforms: BTreeMap<&'a str, Platform<'a>>,
    firmwares: BTreeMap<&'a str, FirmwareV2<'a>>,
}

#[derive(Serialize)]
struct Selection<'a> {
    firmware: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<&'a str>,
}

#[derive(Serialize)]
struct Platform<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    registered_firmwares: usize,
    target: &'a str,
    chip: &'a str,
}

#[derive(Serialize)]
struct FirmwareV2<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    platform: &'a str,
    package: &'a str,
    bin: &'a str,
    workspace: &'a Path,
    working_dir: &'a Path,
    target_dir: &'a Path,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_dir: Option<&'a Path>,
    manifest_path: &'a Path,
    target: &'a str,
    chip: &'a str,
    artifacts: BTreeMap<&'a str, ArtifactV2<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_dependencies: Option<Vec<DependencyV2<'a>>>,
}

#[derive(Serialize)]
struct ArtifactV2<'a> {
    path: &'a Path,
    exists: bool,
}

#[derive(Serialize)]
struct DependencyV2<'a> {
    package: &'a str,
    manifest_path: &'a Path,
    target_clippy: bool,
}

pub(crate) fn info_v2(report: &ProjectInfo) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&InfoV2 {
        format_version: 2,
        project_root: &report.root,
        saved_selection: saved_selection(report),
        platforms: platforms(report),
        firmwares: report
            .firmwares
            .iter()
            .map(|(id, firmware)| {
                let platform = &report.platforms[&firmware.platform];
                (
                    id.as_str(),
                    FirmwareV2 {
                        description: firmware.description.as_deref(),
                        platform: &firmware.platform,
                        package: &firmware.package,
                        bin: &firmware.bin,
                        workspace: &firmware.workspace,
                        working_dir: &firmware.working_dir,
                        target_dir: &firmware.target_dir,
                        build_dir: firmware.build_dir.as_deref(),
                        manifest_path: &firmware.manifest_path,
                        target: &platform.target,
                        chip: &platform.chip,
                        artifacts: firmware
                            .artifacts
                            .iter()
                            .map(|artifact| {
                                let profile = artifact
                                    .profile
                                    .as_ref()
                                    .map_or("dev", polyfw::Profile::name);
                                (
                                    profile,
                                    ArtifactV2 {
                                        path: &artifact.path,
                                        exists: artifact.exists,
                                    },
                                )
                            })
                            .collect(),
                        local_dependencies: firmware.local_dependencies.as_ref().map(|packages| {
                            let mut dependencies = packages
                                .iter()
                                .map(|package| DependencyV2 {
                                    package: &package.name,
                                    manifest_path: &package.absolute_manifest_path,
                                    target_clippy: package.embedded,
                                })
                                .collect::<Vec<_>>();
                            dependencies.sort_by_key(|dependency| {
                                (dependency.package, dependency.manifest_path)
                            });
                            dependencies
                        }),
                    },
                )
            })
            .collect(),
    })
}

fn saved_selection(report: &ProjectInfo) -> Option<Selection<'_>> {
    report.saved_selection.as_ref().map(|saved| Selection {
        firmware: &saved.firmware,
        profile: saved.profile.as_ref().map(polyfw::Profile::name),
    })
}

fn platforms(report: &ProjectInfo) -> BTreeMap<&str, Platform<'_>> {
    report
        .platforms
        .iter()
        .map(|(id, platform)| {
            (
                id.as_str(),
                Platform {
                    description: platform.description.as_deref(),
                    registered_firmwares: platform.registered_firmwares,
                    target: &platform.target,
                    chip: &platform.chip,
                },
            )
        })
        .collect()
}
