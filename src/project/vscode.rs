use serde_json::json;

use crate::InternalResult as Result;
use crate::cargo::model::Selection;
use crate::cargo::runner::firmware_elf_path;

pub(crate) const LAUNCH_FILE: &str = ".vscode/launch.json";

pub(crate) fn configuration(selection: &Selection) -> Result<String> {
    let elf = firmware_elf_path(selection)
        .to_string_lossy()
        .replace('\\', "/");
    // Configuration fields follow the probe-rs extension's 0.32.0 schema.
    // https://github.com/probe-rs/vscode/blob/v0.32.0/package.json
    let configurations = [("launch", "Launch"), ("attach", "Attach")].map(|(request, label)| {
        let name = match &selection.profile {
            Some(profile) => format!("{label} {} ({})", selection.id, profile.name()),
            None => format!("{label} {}", selection.id),
        };
        let mut configuration = json!({
            "name": name,
            "type": "probe-rs-debug",
            "request": request,
            "cwd": "${workspaceFolder}",
            "chip": selection.platform.chip,
            "wireProtocol": "Swd",
            "coreConfigs": [{
                "coreIndex": 0,
                "programBinary": elf,
                "rttEnabled": true,
                "rttChannelFormats": [{
                    "channelNumber": 0,
                    "dataFormat": "Defmt",
                    "mode": "NoBlockSkip",
                }],
            }],
        });
        if request == "launch" {
            configuration["flashingConfig"] = json!({
                "flashingEnabled": true,
                "verifyAfterFlashing": true,
                "haltAfterReset": true,
            });
        }
        configuration
    });
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({
            "version": "0.2.0",
            "configurations": configurations,
        }))?
    ))
}
