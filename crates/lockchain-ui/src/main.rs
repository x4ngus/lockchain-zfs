//! Desktop control deck built with Iced to steer Lockchain workflows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Local;
use iced::alignment::Vertical;
use iced::border::{Border, Radius};
use iced::widget::button;
use iced::widget::button::{Status as ButtonStatus, Style as ButtonStyle};
use iced::widget::{column, container, row, scrollable, text, text_input, toggler, Space};
use iced::{application, Font, Length, Size, Task, Theme};
use lockchain_core::config::LockchainConfig;
use lockchain_core::workflow::{
    self, ForgeMode, ProvisionOptions, WorkflowEvent, WorkflowLevel, WorkflowReport,
};
use lockchain_zfs::SystemZfsProvider;

/// Launch the Iced application with the Lockchain-specific theme and state.
pub fn main() -> iced::Result {
    lockchain_core::logging::init("info");
    application(
        "LockChain Control Deck",
        LockchainUi::update,
        LockchainUi::view,
    )
    .default_font(Font::with_name("JetBrains Mono"))
    .antialiasing(true)
    .window_size(Size::new(1280.0, 768.0))
    .theme(LockchainUi::theme)
    .run_with(LockchainUi::init)
}

/// Actions the operator can trigger from the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Directive {
    NewKey,
    NewKeySafe,
    SelfTest,
    RecoverKey,
    SelfHeal,
    Doctor,
}

/// Metadata used to render directive cards.
struct DirectiveEntry {
    directive: Directive,
    title: &'static str,
    subtitle: &'static str,
}

/// List of all available directives shown in the control deck.
const DIRECTIVES: &[DirectiveEntry] = &[
    DirectiveEntry {
        directive: Directive::NewKey,
        title: "New Key",
        subtitle: "Forge fresh USB key material",
    },
    DirectiveEntry {
        directive: Directive::NewKeySafe,
        title: "New Key (Safe mode)",
        subtitle: "Guided forge with confirmations",
    },
    DirectiveEntry {
        directive: Directive::SelfTest,
        title: "Self-test",
        subtitle: "Drill unlock + keystatus verification",
    },
    DirectiveEntry {
        directive: Directive::RecoverKey,
        title: "Recover Key",
        subtitle: "Derive fallback key from passphrase",
    },
    DirectiveEntry {
        directive: Directive::SelfHeal,
        title: "Self-heal Issues",
        subtitle: "Diagnose key material and dataset state",
    },
    DirectiveEntry {
        directive: Directive::Doctor,
        title: "Doctor",
        subtitle: "Full system audit & remediation tips",
    },
];

/// Visual severity mapping for workflow events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityLevel {
    Info,
    Success,
    Warn,
    Error,
    Security,
}

impl ActivityLevel {
    /// Short label used in status chips.
    fn label(self) -> &'static str {
        match self {
            ActivityLevel::Info => "INFO",
            ActivityLevel::Success => "SUCCESS",
            ActivityLevel::Warn => "WARN",
            ActivityLevel::Error => "ERROR",
            ActivityLevel::Security => "SECURE",
        }
    }

    /// Theme color associated with each activity level.
    fn color(self) -> iced::Color {
        match self {
            ActivityLevel::Info => iced::Color::from_rgb8(0x67, 0xd6, 0xff),
            ActivityLevel::Success => iced::Color::from_rgb8(0x8a, 0xff, 0x70),
            ActivityLevel::Warn => iced::Color::from_rgb8(0xff, 0xc1, 0x29),
            ActivityLevel::Error => iced::Color::from_rgb8(0xff, 0x47, 0x80),
            ActivityLevel::Security => iced::Color::from_rgb8(0xff, 0x73, 0xff),
        }
    }
}

impl From<WorkflowLevel> for ActivityLevel {
    fn from(level: WorkflowLevel) -> Self {
        match level {
            WorkflowLevel::Info => ActivityLevel::Info,
            WorkflowLevel::Success => ActivityLevel::Success,
            WorkflowLevel::Warn => ActivityLevel::Warn,
            WorkflowLevel::Error => ActivityLevel::Error,
            WorkflowLevel::Security => ActivityLevel::Security,
        }
    }
}

/// Normalised activity entry displayed in the timeline.
#[derive(Debug, Clone)]
struct ActivityItem {
    timestamp: String,
    level: ActivityLevel,
    message: String,
}

/// Application state backing the UI, including current directive and logs.
#[derive(Debug)]
struct LockchainUi {
    config_path: PathBuf,
    active_directive: Directive,
    secure_mode: bool,
    terminal_input: String,
    activity: Vec<ActivityItem>,
    executing: bool,
    pending_directive: Option<Directive>,
    status_line: String,
    total_events: usize,
    key_present: bool,
}

/// Messages produced by Iced interactions and background tasks.
#[derive(Debug, Clone)]
enum Message {
    DirectiveSelected(Directive),
    TerminalChanged(String),
    Execute,
    WorkflowFinished(Result<WorkflowReport, String>),
    ToggleSecure(bool),
    HelpPressed,
    KillSwitchPressed,
    Refresh,
}

impl LockchainUi {
    /// Construct initial UI state and schedule no async work.
    fn init() -> (Self, Task<Message>) {
        let config_path = std::env::var("LOCKCHAIN_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/etc/lockchain-zfs.toml"));

        let mut ui = Self {
            config_path,
            active_directive: Directive::NewKey,
            secure_mode: false,
            terminal_input: String::new(),
            activity: Vec::new(),
            executing: false,
            pending_directive: None,
            status_line: "Monitoring".into(),
            total_events: 0,
            key_present: false,
        };

        ui.push_activity(
            ActivityLevel::Info,
            "Control Deck online. Select a directive to begin.",
        );
        ui.key_present = ui.detect_key_presence();
        (ui, Task::none())
    }

    /// React to UI events and kick off any background tasks.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DirectiveSelected(directive) => {
                if !self.executing {
                    self.active_directive = directive;
                    self.status_line = directive_title(directive).into();
                }
                Task::none()
            }
            Message::TerminalChanged(value) => {
                self.terminal_input = value;
                Task::none()
            }
            Message::ToggleSecure(state) => {
                self.secure_mode = state;
                self.push_activity(
                    ActivityLevel::Info,
                    if state {
                        "Strict USB mode engaged."
                    } else {
                        "Strict USB mode disengaged."
                    },
                );
                Task::none()
            }
            Message::Execute => {
                if self.executing {
                    return Task::none();
                }
                if !self.directive_enabled(self.active_directive) {
                    self.push_activity(
                        ActivityLevel::Warn,
                        "Forge or insert a LockChain key before running this directive.",
                    );
                    return Task::none();
                }
                self.executing = true;
                self.pending_directive = Some(self.active_directive);
                self.push_activity(
                    ActivityLevel::Info,
                    format!("Executing {}", directive_title(self.active_directive)),
                );
                Task::perform(
                    run_directive(
                        self.config_path.clone(),
                        self.active_directive,
                        self.secure_mode,
                        self.terminal_input.clone(),
                    ),
                    Message::WorkflowFinished,
                )
            }
            Message::WorkflowFinished(result) => {
                self.executing = false;
                let directive = self
                    .pending_directive
                    .take()
                    .unwrap_or(self.active_directive);
                match result {
                    Ok(report) => {
                        self.push_activity(
                            ActivityLevel::Success,
                            format!("{} complete", report.title),
                        );
                        self.ingest_events(report.events);
                        if matches!(directive, Directive::NewKey | Directive::NewKeySafe) {
                            self.status_line = "Forge complete".into();
                            self.key_present = true;
                        } else {
                            self.status_line = "Monitoring".into();
                        }
                    }
                    Err(err) => {
                        self.push_activity(ActivityLevel::Error, err);
                        self.status_line = "Check diagnostics".into();
                    }
                }
                self.key_present = self.detect_key_presence();
                Task::none()
            }
            Message::HelpPressed => {
                self.push_activity(
                    ActivityLevel::Info,
                    help_text(self.active_directive).to_string(),
                );
                Task::none()
            }
            Message::KillSwitchPressed => {
                self.push_activity(
                    ActivityLevel::Warn,
                    "Killswitch placeholder: integrate ZFS unload-key workflow.",
                );
                Task::none()
            }
            Message::Refresh => {
                if self.executing {
                    return Task::none();
                }
                self.key_present = self.detect_key_presence();
                self.executing = true;
                self.pending_directive = Some(Directive::SelfHeal);
                self.push_activity(ActivityLevel::Info, "Running self-heal diagnostics…");
                Task::perform(
                    run_directive(
                        self.config_path.clone(),
                        Directive::SelfHeal,
                        self.secure_mode,
                        self.terminal_input.clone(),
                    ),
                    Message::WorkflowFinished,
                )
            }
        }
    }

    /// Produce the full view tree for the current state.
    fn view(&self) -> iced::Element<'_, Message> {
        let header = self.view_header();
        let main = self.view_body();
        let footer = self.view_footer();

        container(
            column![header, main, footer]
                .spacing(20)
                .width(Length::Fill),
        )
        .padding(24)
        .style(deck_background())
        .into()
    }

    /// Check whether the expected USB key location has raw material present.
    fn detect_key_presence(&self) -> bool {
        LockchainConfig::load(&self.config_path)
            .ok()
            .map(|cfg| cfg.key_hex_path().exists())
            .unwrap_or(false)
    }

    /// Determine if a directive should be interactable based on context.
    fn directive_enabled(&self, directive: Directive) -> bool {
        match directive {
            Directive::SelfTest => self.key_present,
            _ => true,
        }
    }

    /// Provide the application theme customisations for Iced.
    fn theme(&self) -> Theme {
        Theme::TokyoNight
    }

    /// Render the title bar and key state indicator.
    fn view_header(&self) -> iced::Element<'_, Message> {
        let title = text("Control Deck")
            .size(32)
            .style(text_color(iced::Color::from_rgb8(0x24, 0xd0, 0xff)));
        let subtitle = text("Cryptographic ZFS key management — powered by LockChain")
            .size(16)
            .style(text_color(iced::Color::from_rgb8(0xff, 0x73, 0xff)));

        let status_chip = container(
            text(if self.secure_mode {
                "SECURE"
            } else {
                "STANDARD"
            })
            .size(14)
            .style(text_color(if self.secure_mode {
                iced::Color::from_rgb8(0x8a, 0xff, 0x70)
            } else {
                iced::Color::from_rgb8(0xff, 0xc1, 0x29)
            })),
        )
        .padding([6, 12])
        .style(chip_style(self.secure_mode));

        let secure_toggle = toggler(self.secure_mode)
            .label("Secure")
            .size(22)
            .text_size(16)
            .on_toggle(Message::ToggleSecure);

        row![
            column![title, subtitle].spacing(4),
            Space::with_width(Length::Fill),
            status_chip,
            secure_toggle,
            button("Refresh")
                .padding([10, 18])
                .style(primary_button())
                .on_press(Message::Refresh)
        ]
        .align_y(Vertical::Center)
        .spacing(20)
        .into()
    }

    /// Assemble the three-column layout containing directives, terminal, and activity log.
    fn view_body(&self) -> iced::Element<'_, Message> {
        let directives: iced::Element<Message> =
            self.view_directive_panel().width(Length::Fill).into();
        let terminal: iced::Element<Message> =
            self.view_terminal_panel().width(Length::Fill).into();

        let left_column: iced::Element<Message> = column![directives, terminal]
            .spacing(16)
            .width(Length::FillPortion(5))
            .into();

        let activity: iced::Element<Message> = self
            .view_activity_panel()
            .width(Length::FillPortion(7))
            .into();

        row![left_column, activity]
            .spacing(24)
            .align_y(Vertical::Top)
            .into()
    }

    /// Build the directive selection list with cards and toggles.
    fn view_directive_panel(&self) -> iced::widget::Container<'_, Message> {
        let mut list = column![];
        for entry in DIRECTIVES {
            let active = entry.directive == self.active_directive;
            let enabled = self.directive_enabled(entry.directive);
            let mut button = button(
                column![
                    text(entry.title).size(20).style(text_color(if enabled {
                        iced::Color::from_rgb8(0xe7, 0xff, 0xff)
                    } else {
                        iced::Color::from_rgb8(0x55, 0x66, 0x88)
                    })),
                    text(entry.subtitle).size(14).style(text_color(if enabled {
                        iced::Color::from_rgb8(0x67, 0xd6, 0xff)
                    } else {
                        iced::Color::from_rgb8(0x44, 0x55, 0x8a)
                    }))
                ]
                .spacing(4),
            )
            .width(Length::Fill)
            .padding([12, 18])
            .style(directive_style(active, enabled));

            if enabled {
                button = button.on_press(Message::DirectiveSelected(entry.directive));
            }
            list = list.push(button);
        }

        container(
            column![
                text("Select Module Directive")
                    .size(18)
                    .style(text_color(iced::Color::from_rgb8(0xff, 0x51, 0xff))),
                list.spacing(10)
            ]
            .spacing(16),
        )
        .padding(20)
        .style(panel_style())
    }

    /// Show terminal-like inputs, status chip, and action buttons for the active directive.
    fn view_terminal_panel(&self) -> iced::widget::Container<'_, Message> {
        let input = text_input("Enter command or parameters…", &self.terminal_input)
            .on_input(Message::TerminalChanged)
            .size(18)
            .padding(12)
            .style(text_input_style());

        let execute_enabled = self.directive_enabled(self.active_directive);

        let mut execute = button(
            text("Execute")
                .size(18)
                .style(text_color(iced::Color::from_rgb8(0x05, 0x08, 0x1f))),
        )
        .width(Length::Fill)
        .padding([12, 18])
        .style(execute_button(execute_enabled));

        if execute_enabled {
            execute = execute.on_press(Message::Execute);
        }

        let status = column![
            text(format!(
                "System Status: {}",
                self.status_line.to_uppercase()
            ))
            .size(14)
            .style(text_color(iced::Color::from_rgb8(0x8a, 0xff, 0x70))),
            text(format!(
                "Active Module: {}",
                directive_title(self.active_directive)
            ))
            .size(14)
            .style(text_color(iced::Color::from_rgb8(0x67, 0xd6, 0xff)))
        ]
        .spacing(4);

        let notes: iced::Element<'_, Message> =
            if matches!(self.active_directive, Directive::SelfTest) && !execute_enabled {
                column![
                    text("Self-test unavailable until a LockChain key is forged or inserted.")
                        .size(14)
                        .style(text_color(iced::Color::from_rgb8(0xff, 0xc1, 0x29)))
                ]
                .spacing(4)
                .into()
            } else {
                column![].into()
            };

        container(
            column![
                text("> User Input Terminal")
                    .size(18)
                    .style(text_color(iced::Color::from_rgb8(0xff, 0x51, 0xff))),
                column![
                    text("Command Input:")
                        .size(14)
                        .style(text_color(iced::Color::from_rgb8(0x8a, 0xff, 0x70))),
                    input,
                    execute,
                    status,
                    notes,
                    row![
                        button("Help")
                            .padding([10, 16])
                            .style(help_button())
                            .on_press(Message::HelpPressed),
                        button("Killswitch")
                            .padding([10, 16])
                            .style(killswitch_button())
                            .on_press(Message::KillSwitchPressed)
                    ]
                    .spacing(12)
                ]
                .spacing(12)
            ]
            .spacing(16),
        )
        .padding(20)
        .style(panel_style())
    }

    /// Display the scrolling log of workflow events.
    fn view_activity_panel(&self) -> iced::widget::Container<'_, Message> {
        let mut column = column![];
        for item in self.activity.iter().rev() {
            let line = column![
                row![
                    text(format!("[{}]", item.timestamp))
                        .size(14)
                        .style(text_color(iced::Color::from_rgb8(0x67, 0xd6, 0xff))),
                    text(item.level.label())
                        .size(14)
                        .style(text_color(item.level.color()))
                ]
                .spacing(12),
                text(&item.message)
                    .size(14)
                    .style(text_color(iced::Color::from_rgb8(0xe7, 0xff, 0xff)))
            ]
            .spacing(6);
            column = column.push(container(line).padding([8, 12]).style(activity_entry()));
        }

        let scroll = scrollable(column.spacing(12)).height(Length::Fill);

        container(
            column![
                text("Runtime Activity Feed")
                    .size(18)
                    .style(text_color(iced::Color::from_rgb8(0xff, 0x51, 0xff))),
                scroll
            ]
            .spacing(16),
        )
        .padding(20)
        .style(panel_style())
    }

    /// Render the footer with a simple status line and dataset summary.
    fn view_footer(&self) -> iced::Element<'_, Message> {
        row![
            text(format!("Total Events: {}", self.total_events))
                .size(14)
                .style(text_color(iced::Color::from_rgb8(0x67, 0xd6, 0xff))),
            Space::with_width(Length::Fill),
            text(format!("Status: {}", self.status_line.to_uppercase()))
                .size(14)
                .style(text_color(iced::Color::from_rgb8(0x8a, 0xff, 0x70)))
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Convert workflow events into activity items and append them to the log.
    fn ingest_events(&mut self, events: Vec<WorkflowEvent>) {
        for event in events {
            self.push_activity(ActivityLevel::from(event.level), event.message);
        }
    }

    /// Push a single activity entry and prune the backlog when needed.
    fn push_activity(&mut self, level: ActivityLevel, message: impl Into<String>) {
        let ts = Local::now().format("%H:%M:%S").to_string();
        self.activity.push(ActivityItem {
            timestamp: ts,
            level,
            message: message.into(),
        });
        self.total_events += 1;
        if self.activity.len() > 400 {
            let excess = self.activity.len() - 400;
            self.activity.drain(0..excess);
        }
    }
}

/// Human-friendly label for directives when status lines need text.
fn directive_title(directive: Directive) -> &'static str {
    match directive {
        Directive::NewKey => "New Key",
        Directive::NewKeySafe => "New Key (Safe mode)",
        Directive::SelfTest => "Self-test",
        Directive::RecoverKey => "Recover Key",
        Directive::SelfHeal => "Self-heal Issues",
        Directive::Doctor => "Doctor",
    }
}

/// Contextual help string shown in the terminal panel.
fn help_text(directive: Directive) -> &'static str {
    match directive {
        Directive::NewKey => "Forge a new 32-byte USB key. Provide dataset=<name> to target a specific encryption root.",
        Directive::NewKeySafe => "Safe forge prompts for review. Supply dataset=<name> as needed.",
        Directive::SelfTest => "Provision a scratch encrypted pool, unlock it with the current key, then tear it down. Supports dataset=<name>, device=/dev/sdX, mount=/run/lockchain, filename=lockchain.key, rebuild=false, passphrase=<secret>.",
        Directive::RecoverKey => "Derive fallback key using passphrase. Provide dataset=<name> passphrase=<secret> [output=/path].",
        Directive::SelfHeal => "Runs diagnostics against key file, checksum, and dataset keystatus.",
        Directive::Doctor => "Runs self-heal plus systemd/journal/initramfs audits. Provide no args; review warnings for remediation guidance.",
    }
}

/// Parse key=value arguments from the terminal input field.
fn parse_kv(input: &str) -> (HashMap<String, String>, Vec<String>) {
    let mut map = HashMap::new();
    let mut free = Vec::new();

    for token in input.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            map.insert(key.to_lowercase(), value.to_string());
        } else if let Some((key, value)) = token.split_once(':') {
            map.insert(key.to_lowercase(), value.to_string());
        } else {
            free.push(token.to_string());
        }
    }

    (map, free)
}

/// Accept several truthy strings when toggling options via text.
fn parse_bool(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Kick off the selected workflow and return a `Message` when finished.
async fn run_directive(
    config_path: PathBuf,
    directive: Directive,
    secure_mode: bool,
    raw_input: String,
) -> Result<WorkflowReport, String> {
    let mut config = LockchainConfig::load(&config_path).map_err(|e| e.to_string())?;
    let provider = SystemZfsProvider::from_config(&config).map_err(|err| format!("{err}"))?;

    let (kv, free) = parse_kv(&raw_input);

    match directive {
        Directive::NewKey | Directive::NewKeySafe => {
            let dataset = resolve_dataset(&config, &kv, &free)?;
            let mode = if matches!(directive, Directive::NewKeySafe) {
                ForgeMode::Safe
            } else {
                ForgeMode::Standard
            };

            let mut options = ProvisionOptions::default();
            if let Some(device) = kv.get("device").map(|s| s.to_string()) {
                options.usb_device = Some(device);
            }
            if let Some(mount) = kv.get("mount").map(|s| PathBuf::from(s)) {
                options.mountpoint = Some(mount);
            }
            if let Some(file) = kv
                .get("filename")
                .or_else(|| kv.get("file"))
                .map(|s| s.to_string())
            {
                options.key_filename = Some(file);
            }
            if let Some(pass) = kv.get("passphrase").map(|s| s.to_string()) {
                options.passphrase = Some(pass);
            }
            if let Some(force) = kv.get("force").map(|v| parse_bool(v)) {
                options.force_wipe = force;
            } else if matches!(mode, ForgeMode::Standard) {
                options.force_wipe = true;
            }
            if let Some(rebuild) = kv.get("rebuild").map(|v| parse_bool(v)) {
                options.rebuild_initramfs = rebuild;
            }

            workflow::forge_key(&mut config, &provider, &dataset, mode, options)
                .map_err(|e| e.to_string())
        }
        Directive::SelfTest => {
            let dataset = resolve_dataset(&config, &kv, &free)?;
            workflow::self_test(&config, provider, &dataset, secure_mode).map_err(|e| e.to_string())
        }
        Directive::RecoverKey => {
            let dataset = resolve_dataset(&config, &kv, &free)?;
            let passphrase = kv
                .get("passphrase")
                .map(|s| s.to_string())
                .or_else(|| {
                    if !free.is_empty() {
                        Some(free.join(" "))
                    } else {
                        None
                    }
                })
                .ok_or_else(|| "passphrase=<secret> required for recovery".to_string())?;

            let output = kv
                .get("output")
                .map(PathBuf::from)
                .unwrap_or_else(|| default_recovery_path(&dataset));

            workflow::recover_key(&config, provider, &dataset, passphrase.as_bytes(), &output)
                .map_err(|e| e.to_string())
        }
        Directive::SelfHeal => workflow::self_heal(&config, provider).map_err(|e| e.to_string()),
        Directive::Doctor => workflow::doctor(&config, provider).map_err(|e| e.to_string()),
    }
}

/// Reuse CLI dataset resolution semantics inside the UI.
fn resolve_dataset(
    config: &LockchainConfig,
    kv: &HashMap<String, String>,
    free: &[String],
) -> Result<String, String> {
    if let Some(ds) = kv.get("dataset") {
        return Ok(ds.clone());
    }
    if let Some(first) = free.first() {
        if first.contains('/') {
            return Ok(first.clone());
        }
    }
    config
        .policy
        .datasets
        .first()
        .cloned()
        .ok_or_else(|| "No dataset configured; add one to policy.datasets".to_string())
}

/// Derive a sensible filename for fallback key recovery output.
fn default_recovery_path(dataset: &str) -> PathBuf {
    let sanitized = dataset.replace('/', "-");
    let timestamp = Local::now().format("%Y%m%d%H%M%S");
    Path::new("/var/lib/lockchain").join(format!("{}_{}.key", sanitized, timestamp))
}

/// Base background styling for the entire control deck.
fn deck_background() -> impl Fn(&Theme) -> iced::widget::container::Style + Copy {
    |_| iced::widget::container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgb8(
            0x05, 0x08, 0x1f,
        ))),
        ..Default::default()
    }
}

/// Shared styling for the directive/terminal/activity panels.
fn panel_style() -> impl Fn(&Theme) -> iced::widget::container::Style + Copy {
    |_| iced::widget::container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgba(
            0.05, 0.08, 0.2, 0.7,
        ))),
        border: Border {
            radius: Radius::from(12.0),
            width: 1.5,
            color: iced::Color::from_rgb8(0x24, 0xd0, 0xff),
        },
        ..Default::default()
    }
}

/// Container styling for individual activity log entries.
fn activity_entry() -> impl Fn(&Theme) -> iced::widget::container::Style + Copy {
    |_| iced::widget::container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgba(
            0.03, 0.05, 0.18, 0.8,
        ))),
        border: Border {
            radius: Radius::from(8.0),
            width: 1.0,
            color: iced::Color::from_rgb8(0xff, 0x73, 0xff),
        },
        ..Default::default()
    }
}

/// Styling for directive cards, adapting to focus and disabled states.
fn directive_style(
    active: bool,
    enabled: bool,
) -> impl Fn(&Theme, ButtonStatus) -> ButtonStyle + Copy {
    move |_theme, _status| {
        if !enabled {
            ButtonStyle {
                background: Some(iced::Background::Color(iced::Color::from_rgb8(
                    0x12, 0x15, 0x29,
                ))),
                text_color: iced::Color::from_rgb8(0x55, 0x66, 0x88),
                border: Border {
                    color: iced::Color::from_rgb8(0x25, 0x28, 0x40),
                    width: 1.0,
                    radius: Radius::from(10.0),
                },
                ..ButtonStyle::default()
            }
        } else if active {
            ButtonStyle {
                background: Some(iced::Background::Color(iced::Color::from_rgb8(
                    0x1a, 0x2b, 0x66,
                ))),
                text_color: iced::Color::from_rgb8(0xe7, 0xff, 0xff),
                border: Border {
                    color: iced::Color::from_rgb8(0xff, 0x73, 0xff),
                    width: 2.0,
                    radius: Radius::from(10.0),
                },
                ..ButtonStyle::default()
            }
        } else {
            ButtonStyle {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.07, 0.10, 0.24, 0.8,
                ))),
                text_color: iced::Color::from_rgb8(0xe7, 0xff, 0xff),
                border: Border {
                    color: iced::Color::from_rgb8(0x24, 0xd0, 0xff),
                    width: 1.0,
                    radius: Radius::from(10.0),
                },
                ..ButtonStyle::default()
            }
        }
    }
}

/// Style sheet for the main Execute button depending on availability.
fn execute_button(enabled: bool) -> impl Fn(&Theme, ButtonStatus) -> ButtonStyle + Copy {
    move |_theme, status| {
        if !enabled {
            ButtonStyle {
                background: Some(iced::Background::Color(iced::Color::from_rgb8(
                    0x12, 0x15, 0x29,
                ))),
                text_color: iced::Color::from_rgb8(0x55, 0x66, 0x88),
                border: Border {
                    color: iced::Color::from_rgb8(0x25, 0x28, 0x40),
                    width: 1.0,
                    radius: Radius::from(8.0),
                },
                ..ButtonStyle::default()
            }
        } else {
            let base = iced::Color::from_rgb8(0x24, 0xd0, 0xff);
            let background = match status {
                ButtonStatus::Pressed => iced::Color::from_rgb8(0x1a, 0xa0, 0xc8),
                _ => base,
            };
            ButtonStyle {
                background: Some(iced::Background::Color(background)),
                text_color: iced::Color::from_rgb8(0x05, 0x08, 0x1f),
                border: Border {
                    color: iced::Color::from_rgb8(0x24, 0xd0, 0xff),
                    width: 1.0,
                    radius: Radius::from(8.0),
                },
                ..ButtonStyle::default()
            }
        }
    }
}

/// Reusable primary button style for positive actions.
fn primary_button() -> impl Fn(&Theme, ButtonStatus) -> ButtonStyle + Copy {
    move |_theme, status| {
        let base = iced::Color::from_rgb8(0x24, 0xd0, 0xff);
        let background = match status {
            ButtonStatus::Pressed => iced::Color::from_rgb8(0x1a, 0xa0, 0xc8),
            _ => base,
        };
        ButtonStyle {
            background: Some(iced::Background::Color(background)),
            text_color: iced::Color::from_rgb8(0x05, 0x08, 0x1f),
            border: Border {
                color: iced::Color::from_rgb8(0x24, 0xd0, 0xff),
                width: 1.0,
                radius: Radius::from(8.0),
            },
            ..ButtonStyle::default()
        }
    }
}

/// Button styling for the inline help toggle.
fn help_button() -> impl Fn(&Theme, ButtonStatus) -> ButtonStyle + Copy {
    move |_theme, _status| ButtonStyle {
        background: Some(iced::Background::Color(iced::Color::from_rgb8(
            0x12, 0x66, 0x4f,
        ))),
        text_color: iced::Color::from_rgb8(0xe7, 0xff, 0xff),
        border: Border {
            color: iced::Color::from_rgb8(0x12, 0x66, 0x4f),
            width: 1.0,
            radius: Radius::from(6.0),
        },
        ..ButtonStyle::default()
    }
}

/// Styling for the kill-switch button that stands out from primary actions.
fn killswitch_button() -> impl Fn(&Theme, ButtonStatus) -> ButtonStyle + Copy {
    move |_theme, _status| ButtonStyle {
        background: Some(iced::Background::Color(iced::Color::from_rgb8(
            0x70, 0x13, 0x39,
        ))),
        text_color: iced::Color::from_rgb8(0xff, 0x73, 0xff),
        border: Border {
            color: iced::Color::from_rgb8(0x70, 0x13, 0x39),
            width: 1.0,
            radius: Radius::from(6.0),
        },
        ..ButtonStyle::default()
    }
}

/// The pill-style indicator that shows whether secure mode is enabled.
fn chip_style(secure: bool) -> impl Fn(&Theme) -> iced::widget::container::Style + Copy {
    move |_| iced::widget::container::Style {
        background: Some(iced::Background::Color(if secure {
            iced::Color::from_rgba(0.08, 0.20, 0.14, 0.9)
        } else {
            iced::Color::from_rgba(0.20, 0.12, 0.24, 0.9)
        })),
        border: Border {
            radius: Radius::from(999.0),
            width: 1.0,
            color: if secure {
                iced::Color::from_rgb8(0x8a, 0xff, 0x70)
            } else {
                iced::Color::from_rgb8(0xff, 0xc1, 0x29)
            },
        },
        ..Default::default()
    }
}
/// Text input styling that keeps the neon look while typing.
fn text_input_style(
) -> impl Fn(&Theme, iced::widget::text_input::Status) -> iced::widget::text_input::Style + Copy {
    move |_theme, status| {
        let border = match status {
            iced::widget::text_input::Status::Focused => iced::Color::from_rgb8(0x24, 0xd0, 0xff),
            _ => iced::Color::from_rgb8(0x3a, 0x45, 0x7d),
        };
        iced::widget::text_input::Style {
            background: iced::Background::Color(iced::Color::from_rgba(0.04, 0.07, 0.20, 0.9)),
            border: Border {
                radius: Radius::from(8.0),
                width: 1.0,
                color: border,
            },
            icon: iced::Color::WHITE,
            placeholder: iced::Color::from_rgb8(0x67, 0xd6, 0xff),
            value: iced::Color::from_rgb8(0xe7, 0xff, 0xff),
            selection: iced::Color::from_rgb8(0x24, 0xd0, 0xff),
        }
    }
}

/// Helper to override text color based on the theme palette.
fn text_color(color: iced::Color) -> impl Fn(&Theme) -> iced::widget::text::Style + Copy {
    move |_| iced::widget::text::Style {
        color: Some(color),
        ..Default::default()
    }
}
