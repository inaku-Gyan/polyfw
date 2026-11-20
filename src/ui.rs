use std::fmt::Display;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use clap::ColorChoice;
use comfy_table::{CellAlignment, ContentArrangement, Table, presets};
use dialoguer::FuzzySelect;
use dialoguer::console::{self, Term, style, truncate_str};
use dialoguer::theme::ColorfulTheme;

use crate::Result;
use crate::info::{Firmware, Report};
use crate::model::Registry;
use crate::profile::Profile;

pub(crate) struct Ui {
    unicode: bool,
    width: usize,
}

pub enum Target {
    Host,
    Embedded,
    None,
}

pub(crate) fn color_enabled(color: ColorChoice) -> bool {
    match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            io::stdout().is_terminal()
                && io::stderr().is_terminal()
                && std::env::var_os("NO_COLOR").is_none()
        }
    }
}

impl Ui {
    pub(crate) fn new(color: ColorChoice) -> Self {
        let stdout_is_terminal = io::stdout().is_terminal();
        let stderr_is_terminal = io::stderr().is_terminal();
        let enabled = color_enabled(color);
        console::set_colors_enabled(enabled);
        console::set_colors_enabled_stderr(enabled);

        Self {
            unicode: stdout_is_terminal && stderr_is_terminal,
            width: usize::from(Term::stdout().size().1).max(20),
        }
    }

    pub(crate) fn error(&self, error: &dyn Display) {
        eprintln!("{} {error}", style("error:").red().bold());
    }

    pub(crate) fn initialized(&self, path: &Path) {
        println!("{} {}", style("created").green().bold(), path.display());
        println!("Edit the TODO values in polyfw.toml to match your Cargo firmware project.");
        println!("Add /.polyfw-state/, /target/ and /.vscode/launch.json to its .gitignore.");
        println!("Then run polyfw info from that directory, or pass --project-root <DIR>.");
    }

    pub(crate) fn selected(&self, firmware: &str, profile: Option<&Profile>) {
        let profile = profile
            .map(|profile| format!(" ({})", profile.name()))
            .unwrap_or_default();
        println!(
            "{} {}{profile}",
            style("selected").green().bold(),
            style(firmware).bold()
        );
    }

    pub(crate) fn removal(&self, path: &str, result: &io::Result<()>) {
        match result {
            Ok(()) => println!("{} {path}", style("removed").green().bold()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                println!("{} {path}", style("not found").yellow());
            }
            Err(error) => eprintln!("{} {path}: {error}", style("failed").red().bold()),
        }
    }

    pub(crate) fn cleaning_directories(&self, platform: &str, firmwares: &[&str]) {
        println!(
            "{} {} (firmwares: {})",
            style("cleaning Cargo directories").cyan().bold(),
            style(platform).yellow().bold(),
            firmwares.join(", ")
        );
    }

    pub(crate) fn cleaning_package(&self, package: &str, platform: &str, firmwares: &[&str]) {
        println!(
            "{} {} (target-dir: {}; firmwares: {})",
            style("cleaning package").cyan().bold(),
            style(package).yellow().bold(),
            style(platform).dim(),
            firmwares.join(", ")
        );
    }

    pub(crate) fn label(&self, action: &str, packages: &[&str], target: Target) -> String {
        let target = match target {
            Target::Host => "(host)",
            Target::Embedded => "(emb)",
            Target::None => "",
        };
        format!("{} {}: {}", action, target, packages.join(", "))
    }

    pub(crate) fn print_info(&self, report: &Report, show_saved_selection: bool) {
        if let Some(saved) = report
            .saved_selection
            .as_ref()
            .filter(|_| show_saved_selection)
        {
            let profile = saved
                .profile
                .as_ref()
                .map(|profile| format!(" ({})", profile.name()))
                .unwrap_or_default();
            println!("saved selection: {}{profile}\n", saved.firmware);
        }
        if let Some((id, firmware)) = report
            .firmwares
            .iter()
            .find(|(_, firmware)| firmware.local_dependencies.is_some())
        {
            self.print_firmware_info(report, id, firmware);
            return;
        }
        let platform_rows = report
            .platforms
            .iter()
            .map(|(name, platform)| {
                vec![
                    name.clone(),
                    platform.registered_firmwares.to_string(),
                    platform.target.clone(),
                    platform.cpu.clone(),
                    platform.chip.clone(),
                    platform.description.clone().unwrap_or_default(),
                ]
            })
            .collect::<Vec<_>>();
        self.section("platforms", Some(platform_rows.len()), false);
        self.print_table(
            &["name", "firmwares", "target", "CPU", "chip", "description"],
            &platform_rows,
        );

        let mut headers = vec![
            "status".to_owned(),
            "name".to_owned(),
            "platform".to_owned(),
        ];
        if let Some(firmware) = report.firmwares.values().next() {
            headers.extend(
                firmware
                    .artifacts
                    .iter()
                    .map(|artifact| format!("{} ELF", artifact.profile.name())),
            );
        }
        headers.push("description".to_owned());
        let firmware_rows = report
            .firmwares
            .iter()
            .map(|(name, firmware)| {
                let saved = report
                    .saved_selection
                    .as_ref()
                    .filter(|saved| saved.firmware == *name);
                let mut row = if let Some(saved) = saved {
                    let profile = saved
                        .profile
                        .as_ref()
                        .map(|profile| format!(" ({})", profile.name()))
                        .unwrap_or_default();
                    vec![
                        style(format!("[saved]{profile}"))
                            .green()
                            .bold()
                            .to_string(),
                        style(name).yellow().to_string(),
                        style(&firmware.platform).yellow().to_string(),
                    ]
                } else {
                    vec![String::new(), name.clone(), firmware.platform.clone()]
                };
                row.extend(
                    firmware
                        .artifacts
                        .iter()
                        .map(|artifact| artifact_status(artifact.exists)),
                );
                let description = firmware.description.as_deref().unwrap_or_default();
                row.push(if saved.is_some() {
                    style(description).yellow().to_string()
                } else {
                    description.to_owned()
                });
                row
            })
            .collect::<Vec<_>>();
        self.section("firmwares", Some(firmware_rows.len()), true);
        self.print_table(
            &headers.iter().map(String::as_str).collect::<Vec<_>>(),
            &firmware_rows,
        );
    }

    fn print_firmware_info(&self, report: &Report, id: &str, firmware: &Firmware) {
        let platform = &report.platforms[&firmware.platform];
        self.section("firmware", None, false);
        let mut fields = vec![("name", id.to_owned())];
        if let Some(description) = &firmware.description {
            fields.push(("description", description.clone()));
        }
        fields.push(("platform", firmware.platform.clone()));
        if let Some(description) = &platform.description {
            fields.push(("platform description", description.clone()));
        }
        fields.extend([
            ("workspace", firmware.workspace.display().to_string()),
            ("working-dir", firmware.working_dir.display().to_string()),
            ("target-dir", firmware.target_dir.display().to_string()),
            (
                "build-dir",
                firmware
                    .build_dir
                    .as_ref()
                    .map(|directory| directory.display().to_string())
                    .unwrap_or_else(|| "not reported by Cargo".into()),
            ),
            ("check-platform", firmware.check_platform.to_string()),
            ("target", platform.target.clone()),
            ("cpu", platform.cpu.clone()),
            ("chip", platform.chip.clone()),
            ("package", firmware.package.clone()),
            ("bin", firmware.bin.clone()),
        ]);
        let rows = fields
            .into_iter()
            .map(|(key, value)| vec![key.to_owned(), value])
            .collect::<Vec<_>>();
        self.print_table(&["field", "value"], &rows);

        self.section("artifacts", Some(firmware.artifacts.len()), true);
        let rows = firmware
            .artifacts
            .iter()
            .map(|artifact| {
                vec![
                    artifact.profile.name().to_owned(),
                    artifact_status(artifact.exists),
                    artifact.path.display().to_string(),
                ]
            })
            .collect::<Vec<_>>();
        self.print_table(&["profile", "ELF", "path"], &rows);

        if let Some(packages) = &firmware.local_dependencies {
            self.section("local dependencies", Some(packages.len()), true);
            let rows = packages
                .iter()
                .map(|package| {
                    vec![
                        package.name.clone(),
                        if package.embedded { "embedded" } else { "host" }.to_owned(),
                        package.manifest_path.display().to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            self.print_table(&["package", "target", "manifest"], &rows);
        }
    }

    pub(crate) fn prompt_firmware(
        &self,
        registry: &Registry,
        saved: Option<&str>,
        no_terminal_hint: &str,
    ) -> Result<String> {
        if !Term::stderr().is_term() {
            return Err(format!("no firmware selected; {no_terminal_hint}").into());
        }
        let firmware_width = registry
            .firmwares
            .keys()
            .map(|name| name.chars().count())
            .max()
            .unwrap_or(5)
            .min(24);
        let platform_width = registry
            .firmwares
            .values()
            .map(|firmware| firmware.platform.chars().count())
            .max()
            .unwrap_or(3)
            .min(12);
        let description_width = self
            .width
            .saturating_sub(firmware_width + platform_width + 8) // 8 is the width of the marker
            .min(30);
        let names: Vec<String> = registry
            .firmwares
            .iter()
            .map(|(id, firmware)| {
                let marker = if saved == Some(id.as_str()) {
                    " [saved]"
                } else {
                    ""
                };
                let description = firmware
                    .description
                    .as_deref()
                    .map(|description| {
                        format!(
                            "  {:<description_width$}",
                            truncate_str(
                                description,
                                description_width,
                                if self.unicode { "…" } else { "..." },
                            )
                        )
                    })
                    .unwrap_or_default();
                format!(
                    "{id:<firmware_width$}  {:<platform_width$}{description}{marker}",
                    firmware.platform,
                )
            })
            .collect();
        let initial = saved
            .and_then(|id| {
                registry
                    .firmwares
                    .keys()
                    .position(|candidate| candidate == id)
            })
            .unwrap_or(0);
        let theme = ColorfulTheme {
            prompt_prefix: style("?".to_owned()).cyan().bold(),
            success_prefix: style("ok".to_owned()).green(),
            error_prefix: style("!".to_owned()).red(),
            active_item_prefix: style(">".to_owned()).cyan().bold(),
            ..ColorfulTheme::default()
        };
        eprintln!(
            "{}",
            style("type to filter; arrows to move; Enter to choose; Esc to cancel").dim()
        );
        let Some(index) = FuzzySelect::with_theme(&theme)
            .with_prompt("firmware")
            .items(&names)
            .default(initial)
            .max_length(14)
            .highlight_matches(true)
            .report(false)
            .interact_opt()?
        else {
            return Err("selection cancelled".into());
        };
        registry
            .firmwares
            .keys()
            .nth(index)
            .cloned()
            .ok_or_else(|| "selection index out of range".into())
    }

    fn section(&self, title: &str, count: Option<usize>, blank_before: bool) {
        if blank_before {
            println!();
        }
        let title = style(title).cyan().bold();
        match count {
            Some(count) => println!("{title} {}", style(format!("({count})")).dim()),
            None => println!("{title}"),
        }
    }

    pub(crate) fn command(&self, command: &str) {
        eprintln!("  {} {}", style("$").yellow().bold(), style(command).cyan());
    }

    fn print_table(&self, headers: &[&str], rows: &[Vec<String>]) {
        for line in render_table(headers, rows, self.width).lines() {
            println!("  {line}");
        }
    }
}

pub(crate) struct Progress<'a> {
    ui: &'a Ui,
    total: usize,
    current: usize,
}

pub(crate) struct Step {
    index: usize,
    total: usize,
    started: Instant,
}

impl<'a> Progress<'a> {
    pub(crate) fn new(ui: &'a Ui, total: usize) -> Self {
        Self {
            ui,
            total,
            current: 0,
        }
    }

    pub(crate) fn start(&mut self, label: &str) -> Step {
        self.current += 1;
        let step = Step {
            index: self.current,
            total: self.total,
            started: Instant::now(),
        };
        let arrow = if self.ui.unicode { "▶" } else { ">" };
        eprintln!(
            "{} {} {}",
            style(format!("[{}/{}]", step.index, step.total))
                .cyan()
                .bold(),
            style(arrow).cyan().bold(),
            style(label).bold()
        );
        step
    }

    pub(crate) fn finish(&self, step: Step, success: bool) {
        let (marker, word) = if success {
            (self.success_marker(), "done")
        } else {
            (self.failure_marker(), "failed")
        };
        eprintln!(
            "  {} {} ({})",
            marker,
            style(word).bold(),
            style(format_duration(step.started.elapsed())).dim()
        );
    }

    fn success_marker(&self) -> String {
        if self.ui.unicode {
            style("✓").green().bold().to_string()
        } else {
            style("[ok]").green().bold().to_string()
        }
    }

    fn failure_marker(&self) -> String {
        if self.ui.unicode {
            style("✗").red().bold().to_string()
        } else {
            style("[fail]").red().bold().to_string()
        }
    }
}

fn artifact_status(exists: bool) -> String {
    if exists {
        style("present").green().to_string()
    } else {
        style("unbuilt").dim().to_string()
    }
}

fn render_table(headers: &[&str], rows: &[Vec<String>], width: usize) -> String {
    let mut table = Table::new();
    table
        .load_style(presets::NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(width.saturating_sub(2).min(u16::MAX as usize) as u16)
        .set_header(
            headers
                .iter()
                .map(|header| style(*header).dim().bold().to_string()),
        )
        .add_rows(rows.iter().cloned());
    for (header, column) in headers.iter().zip(table.column_iter_mut()) {
        column.set_padding((0, 2));
        if header.ends_with(" ELF") {
            column.set_cell_alignment(CellAlignment::Center);
        }
    }
    if let Some(column) = table.column_iter_mut().last() {
        column.set_padding((0, 0));
    }
    table.trim_fmt()
}

fn format_duration(duration: Duration) -> String {
    let milliseconds = duration.as_millis();
    if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else if milliseconds < 60_000 {
        format!("{:.1}s", milliseconds as f64 / 1_000.0)
    } else {
        format!(
            "{}m {}s",
            milliseconds / 60_000,
            (milliseconds % 60_000) / 1_000
        )
    }
}

fn render_command(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn run_command(ui: &Ui, mut command: Command) -> io::Result<()> {
    let rendered = render_command(&command);
    ui.command(&rendered);
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{rendered} failed with {status}")))
    }
}

pub(crate) fn run_command_with_progress(
    ui: &Ui,
    progress: &mut Progress<'_>,
    label: &str,
    command: Command,
) -> io::Result<()> {
    let step = progress.start(label);
    let result = run_command(ui, command);
    progress.finish(step, result.is_ok());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_fit_narrow_terminals_without_losing_cell_contents() {
        let headers = ["name", "firmwares", "workspace", "target", "cpu", "chip"];
        let rows = vec![vec![
            "dm-mc02".into(),
            "12".into(),
            "platform/dm-mc02".into(),
            "thumbv7em-none-eabihf".into(),
            "cortex-m7".into(),
            "STM32H723VGTx".into(),
        ]];
        for width in [20, 48, 80] {
            let rendered = render_table(&headers, &rows, width);
            assert!(
                rendered
                    .lines()
                    .all(|line| console::measure_text_width(line) + 2 <= width),
                "table exceeded {width} columns: {rendered}"
            );
            for cell in &rows[0] {
                let rendered = render_table(&["value"], &[vec![cell.clone()]], width);
                let compact: String = console::strip_ansi_codes(&rendered)
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                assert!(compact.contains(cell), "lost {cell}: {rendered}");
            }
        }
    }

    #[test]
    fn styled_unicode_cells_wrap_using_their_terminal_display_width() {
        let styled = style("界界界界界界界界界界界界")
            .green()
            .force_styling(true)
            .to_string();
        let rendered = render_table(&["x"], &[vec![styled]], 20);
        assert!(
            rendered
                .lines()
                .all(|line| console::measure_text_width(line) + 2 <= 20)
        );
        assert_eq!(
            console::strip_ansi_codes(&rendered).matches('界').count(),
            12
        );
        assert!(rendered.contains("\x1b["));
    }

    #[test]
    fn durations_use_milliseconds_below_one_second() {
        assert_eq!(format_duration(Duration::from_millis(240)), "240ms");
        assert_eq!(format_duration(Duration::from_millis(1_240)), "1.2s");
    }

    #[test]
    fn command_runners_preserve_launch_errors_and_failed_exit_statuses() {
        let ui = Ui::new(ColorChoice::Never);
        for with_progress in [false, true] {
            for missing in [false, true] {
                let mut command = if missing {
                    Command::new(
                        std::env::temp_dir()
                            .join(format!("polyfw-missing-executable-{}", std::process::id())),
                    )
                } else {
                    Command::new("rustc")
                };
                command.arg("--polyfw-invalid-argument");
                let rendered = render_command(&command);
                let result = if with_progress {
                    run_command_with_progress(
                        &ui,
                        &mut Progress::new(&ui, 1),
                        "test command",
                        command,
                    )
                } else {
                    run_command(&ui, command)
                };
                let error = result.unwrap_err();
                if missing {
                    assert_eq!(error.kind(), io::ErrorKind::NotFound);
                    assert!(error.raw_os_error().is_some());
                } else {
                    assert_eq!(error.kind(), io::ErrorKind::Other);
                    assert!(error.to_string().contains("failed with"), "{error}");
                    assert!(error.to_string().starts_with(&rendered), "{error}");
                }
            }
        }
    }
}
