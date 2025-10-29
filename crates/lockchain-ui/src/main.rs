use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::Utc;
use iced::alignment::{Horizontal, Vertical};
use iced::theme;
use iced::widget::{
    self, button, column, container, image, row, scrollable, stack, text, text_input, toggler,
    Space,
};
use iced::{border, Background, Color, Element, Length, Size, Task, Theme};
use lockchain_core::config::LockchainConfig;
use lockchain_core::error::LockchainError;
use lockchain_core::provider::{DatasetKeyDescriptor, KeyState};
use lockchain_core::service::{LockchainService, UnlockOptions};
use lockchain_zfs::SystemZfsProvider;

const DEFAULT_CONFIG: &str = "/etc/lockchain-zfs.toml";
const LOGO_BYTES: &[u8] = include_bytes!("../../../assets/logos/lockchain-logo-square.png");

pub fn main() -> iced::Result {
    lockchain_core::logging::init("info");
    iced::application(
        "LockChain Control Deck",
        LockchainUi::update,
        LockchainUi::view,
    )
    .antialiasing(true)
    .window_size(Size::new(1120.0, 720.0))
    .theme(LockchainUi::theme)
    .run_with(LockchainUi::init)
}

#[derive(Debug, Clone)]
struct LogEntry {
    level: LogLevel,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogLevel {
    Info,
    Success,
    Warn,
    Error,
    Security,
}

impl LogLevel {
    fn color(self) -> Color {
        match self {
            LogLevel::Info => Color::from_rgb8(0x67, 0xd6, 0xff),
            LogLevel::Success => Color::from_rgb8(0x8a, 0xff, 0x70),
            LogLevel::Warn => Color::from_rgb8(0xff, 0xc1, 0x29),
            LogLevel::Error => Color::from_rgb8(0xff, 0x47, 0x80),
            LogLevel::Security => Color::from_rgb8(0xff, 0x73, 0xff),
        }
    }
}

#[derive(Debug, Clone)]
enum ModalState {
    None,
    Passphrase {
        dataset: String,
        value: String,
        error: Option<String>,
    },
    BreakglassConfirmName {
        dataset: String,
        input: String,
    },
    BreakglassConfirmPhrase {
        dataset: String,
        input: String,
    },
    BreakglassPassphrase {
        dataset: String,
        output_path: String,
        passphrase: String,
        error: Option<String>,
    },
    BreakglassComplete {
        dataset: String,
        output_path: PathBuf,
    },
}

#[derive(Debug, Clone)]
enum Message {
    StatusLoaded(Result<Vec<DatasetKeyDescriptor>, String>),
    Refresh,
    ToggleStrict(bool),
    Unlock(String),
    UnlockFinished {
        dataset: String,
        result: Result<Vec<String>, String>,
    },
    PassphraseInput(String),
    SubmitPassphrase,
    CancelModal,
    PassphraseUnlockFinished {
        dataset: String,
        result: Result<Vec<String>, String>,
    },
    ShowBreakglass(String),
    BreakglassNameInput(String),
    ConfirmBreakglassName,
    BreakglassPhraseInput(String),
    ConfirmBreakglassPhrase,
    BreakglassPassphraseInput(String),
    BreakglassOutputInput(String),
    ExecuteBreakglass,
    BreakglassFinished {
        dataset: String,
        result: Result<PathBuf, String>,
    },
}

struct LockchainUi {
    config_path: PathBuf,
    datasets: Vec<DatasetKeyDescriptor>,
    logs: Vec<LogEntry>,
    strict_usb: bool,
    modal: ModalState,
    loading: bool,
}

impl LockchainUi {
    fn init() -> (Self, Task<Message>) {
        let config_path = std::env::var("LOCKCHAIN_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_CONFIG));

        let state = Self {
            config_path: config_path.clone(),
            datasets: Vec::new(),
            logs: Vec::new(),
            strict_usb: false,
            modal: ModalState::None,
            loading: true,
        };

        let task = Task::perform(load_status(config_path), Message::StatusLoaded);

        (state, task)
    }

    fn update(state: &mut Self, message: Message) -> Task<Message> {
        state.handle(message)
    }

    fn view(state: &Self) -> Element<'_, Message> {
        state.render()
    }

    fn theme(state: &Self) -> Theme {
        state.palette()
    }

    fn palette(&self) -> Theme {
        Theme::custom(
            String::from("lockchain-neon"),
            theme::Palette {
                background: Color::from_rgb8(0x05, 0x08, 0x1f),
                text: Color::from_rgb8(0xe7, 0xff, 0xff),
                primary: Color::from_rgb8(0x24, 0xd0, 0xff),
                success: Color::from_rgb8(0x8a, 0xff, 0x70),
                danger: Color::from_rgb8(0xff, 0x47, 0x80),
            },
        )
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StatusLoaded(result) => {
                self.loading = false;
                match result {
                    Ok(list) => {
                        self.datasets = list;
                        self.push_log(LogLevel::Info, "Dataset status updated");
                    }
                    Err(err) => {
                        self.push_log(LogLevel::Error, format!("Failed to load datasets: {err}"));
                    }
                }
                Task::none()
            }
            Message::Refresh => {
                self.loading = true;
                Task::perform(load_status(self.config_path.clone()), Message::StatusLoaded)
            }
            Message::ToggleStrict(value) => {
                self.strict_usb = value;
                self.push_log(
                    LogLevel::Info,
                    if value {
                        "Strict USB mode engaged"
                    } else {
                        "Strict USB mode disengaged"
                    }
                    .to_string(),
                );
                Task::none()
            }
            Message::Unlock(dataset) => {
                self.loading = true;
                self.push_log(LogLevel::Info, format!("Unlock requested for {dataset}"));
                let config_path = self.config_path.clone();
                let strict_usb = self.strict_usb;
                let dataset_for_future = dataset.clone();
                Task::perform(
                    unlock_dataset(config_path, dataset_for_future, strict_usb, None),
                    move |result| Message::UnlockFinished {
                        dataset: dataset.clone(),
                        result,
                    },
                )
            }
            Message::UnlockFinished { dataset, result } => {
                self.loading = false;
                match result {
                    Ok(unlocked) => {
                        if unlocked.is_empty() {
                            self.push_log(
                                LogLevel::Success,
                                format!("Dataset {dataset} already unlocked"),
                            );
                        } else {
                            self.push_log(
                                LogLevel::Success,
                                format!("Unlocked {} ({} datasets)", dataset, unlocked.len()),
                            );
                        }
                        return Task::perform(
                            load_status(self.config_path.clone()),
                            Message::StatusLoaded,
                        );
                    }
                    Err(err) => {
                        if err.contains("MissingKeySource") {
                            self.modal = ModalState::Passphrase {
                                dataset,
                                value: String::new(),
                                error: None,
                            };
                            self.push_log(
                                LogLevel::Warn,
                                "USB key missing; passphrase required".to_string(),
                            );
                        } else {
                            self.push_log(LogLevel::Error, format!("Unlock failed: {err}"));
                        }
                    }
                }
                Task::none()
            }
            Message::PassphraseInput(value) => {
                if let ModalState::Passphrase { dataset, error, .. } = &mut self.modal {
                    self.modal = ModalState::Passphrase {
                        dataset: dataset.clone(),
                        value,
                        error: error.clone(),
                    };
                }
                Task::none()
            }
            Message::SubmitPassphrase => {
                if let ModalState::Passphrase { dataset, value, .. } = &self.modal {
                    let dataset_name = dataset.clone();
                    let pass = value.clone();
                    if pass.is_empty() {
                        return Task::none();
                    }
                    self.loading = true;
                    self.modal = ModalState::None;
                    return Task::perform(
                        unlock_dataset(
                            self.config_path.clone(),
                            dataset_name.clone(),
                            self.strict_usb,
                            Some(pass),
                        ),
                        move |result| Message::PassphraseUnlockFinished {
                            dataset: dataset_name.clone(),
                            result,
                        },
                    );
                }
                Task::none()
            }
            Message::PassphraseUnlockFinished { dataset, result } => {
                self.loading = false;
                match result {
                    Ok(_) => {
                        self.push_log(
                            LogLevel::Success,
                            format!("Unlock successful with passphrase for {dataset}"),
                        );
                        return Task::perform(
                            load_status(self.config_path.clone()),
                            Message::StatusLoaded,
                        );
                    }
                    Err(err) => {
                        self.push_log(LogLevel::Error, format!("Passphrase unlock failed: {err}"));
                    }
                }
                Task::none()
            }
            Message::CancelModal => {
                self.modal = ModalState::None;
                Task::none()
            }
            Message::ShowBreakglass(dataset) => {
                self.modal = ModalState::BreakglassConfirmName {
                    dataset,
                    input: String::new(),
                };
                Task::none()
            }
            Message::BreakglassNameInput(value) => {
                if let ModalState::BreakglassConfirmName { dataset, .. } = &self.modal {
                    self.modal = ModalState::BreakglassConfirmName {
                        dataset: dataset.clone(),
                        input: value,
                    };
                }
                Task::none()
            }
            Message::ConfirmBreakglassName => {
                if let ModalState::BreakglassConfirmName { dataset, input } = &self.modal {
                    if &input.trim().to_string() == dataset {
                        self.modal = ModalState::BreakglassConfirmPhrase {
                            dataset: dataset.clone(),
                            input: String::new(),
                        };
                    }
                }
                Task::none()
            }
            Message::BreakglassPhraseInput(value) => {
                if let ModalState::BreakglassConfirmPhrase { dataset, .. } = &self.modal {
                    self.modal = ModalState::BreakglassConfirmPhrase {
                        dataset: dataset.clone(),
                        input: value,
                    };
                }
                Task::none()
            }
            Message::ConfirmBreakglassPhrase => {
                if let ModalState::BreakglassConfirmPhrase { dataset, input } = &self.modal {
                    if input.trim() == "BREAKGLASS" {
                        let default_path = format!(
                            "/var/lib/lockchain/{}_{}.key",
                            dataset.replace('/', "-"),
                            Utc::now().format("%Y%m%d%H%M%S")
                        );
                        self.modal = ModalState::BreakglassPassphrase {
                            dataset: dataset.clone(),
                            output_path: default_path,
                            passphrase: String::new(),
                            error: None,
                        };
                    }
                }
                Task::none()
            }
            Message::BreakglassPassphraseInput(value) => {
                if let ModalState::BreakglassPassphrase {
                    dataset,
                    output_path,
                    error,
                    ..
                } = &self.modal
                {
                    self.modal = ModalState::BreakglassPassphrase {
                        dataset: dataset.clone(),
                        output_path: output_path.clone(),
                        passphrase: value,
                        error: error.clone(),
                    };
                }
                Task::none()
            }
            Message::BreakglassOutputInput(value) => {
                if let ModalState::BreakglassPassphrase {
                    dataset,
                    passphrase,
                    error,
                    ..
                } = &self.modal
                {
                    self.modal = ModalState::BreakglassPassphrase {
                        dataset: dataset.clone(),
                        output_path: value,
                        passphrase: passphrase.clone(),
                        error: error.clone(),
                    };
                }
                Task::none()
            }
            Message::ExecuteBreakglass => {
                if let ModalState::BreakglassPassphrase {
                    dataset,
                    output_path,
                    passphrase,
                    ..
                } = &self.modal
                {
                    if passphrase.is_empty() {
                        self.push_log(LogLevel::Warn, "Passphrase required".to_string());
                        return Task::none();
                    }
                    let dataset_name = dataset.clone();
                    let pass = passphrase.clone();
                    let output = PathBuf::from(output_path);
                    self.loading = true;
                    self.modal = ModalState::None;
                    return Task::perform(
                        breakglass(self.config_path.clone(), dataset_name.clone(), pass, output),
                        move |result| Message::BreakglassFinished {
                            dataset: dataset_name.clone(),
                            result,
                        },
                    );
                }
                Task::none()
            }
            Message::BreakglassFinished { dataset, result } => {
                self.loading = false;
                match result {
                    Ok(path) => {
                        log::warn!(
                            "[LC4000] break-glass recovery invoked, output {}",
                            path.display()
                        );
                        self.push_log(
                            LogLevel::Security,
                            format!("Break-glass key written to {}", path.display()),
                        );
                        self.modal = ModalState::BreakglassComplete {
                            dataset,
                            output_path: path,
                        };
                    }
                    Err(err) => {
                        self.push_log(LogLevel::Error, format!("Break-glass failed: {err}"));
                    }
                }
                Task::none()
            }
        }
    }

    fn render(&self) -> Element<'_, Message> {
        let header = self.view_header();
        let body = self.view_body();
        let base = container(column![header, body].spacing(16).padding(20))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(cyber_container())
            .into();

        match self.modal_view_option() {
            Some(modal) => stack![base, modal].into(),
            None => base,
        }
    }

    fn view_header(&self) -> Element<'_, Message> {
        let logo = image(image::Handle::from_bytes(LOGO_BYTES.to_vec()))
            .width(Length::Fixed(90.0))
            .height(Length::Fixed(90.0));

        let title = text("LockChain Control Deck")
            .size(36)
            .style(text_color(Color::from_rgb8(0x24, 0xd0, 0xff)));

        let subtitle = text("Cybernetic ZFS key management – stay encrypted, stay neon")
            .size(16)
            .style(text_color(Color::from_rgb8(0xff, 0x73, 0xff)));

        row![
            logo,
            column![title, subtitle]
                .spacing(4)
                .align_x(Horizontal::Left),
            Space::with_width(Length::Fill),
            toggler(self.strict_usb)
                .label("Strict USB")
                .size(22)
                .text_alignment(Horizontal::Center)
                .on_toggle(Message::ToggleStrict),
            button("Refresh").on_press(Message::Refresh)
        ]
        .align_y(Vertical::Center)
        .spacing(20)
        .into()
    }

    fn view_body(&self) -> Element<'_, Message> {
        let dataset_panel = self.dataset_list();
        let log_panel = self.log_panel();
        row![dataset_panel, log_panel].spacing(20).into()
    }

    fn dataset_list(&self) -> Element<'_, Message> {
        let mut list = column![];
        if self.datasets.is_empty() {
            list = list.push(
                text("No datasets discovered")
                    .style(text_color(Color::from_rgb8(0xff, 0x73, 0xff))),
            );
        } else {
            for entry in &self.datasets {
                let status_text = match entry.state {
                    KeyState::Available => {
                        text("available").style(text_color(Color::from_rgb8(0x8a, 0xff, 0x70)))
                    }
                    KeyState::Unavailable => {
                        text("locked").style(text_color(Color::from_rgb8(0xff, 0x47, 0x80)))
                    }
                    KeyState::Unknown(_) => {
                        text("unknown").style(text_color(Color::from_rgb8(0xff, 0xc1, 0x29)))
                    }
                };

                let row = row![
                    column![
                        text(&entry.dataset)
                            .size(20)
                            .style(text_color(Color::from_rgb8(0xe7, 0xff, 0xff))),
                        text(&entry.encryption_root)
                            .size(14)
                            .style(text_color(Color::from_rgb8(0x67, 0xd6, 0xff))),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                    status_text,
                    button("Unlock")
                        .on_press(Message::Unlock(entry.dataset.clone()))
                        .style(|theme, status| button::primary(theme, status)),
                    button("Break-glass")
                        .on_press(Message::ShowBreakglass(entry.dataset.clone()))
                        .style(|theme, status| button::danger(theme, status)),
                ]
                .spacing(12)
                .align_y(Vertical::Center);

                list = list.push(container(row).style(cyber_row()).padding(12));
            }
        }

        let scroll = scrollable(list.spacing(12))
            .width(Length::FillPortion(2))
            .height(Length::Fill);
        container(scroll).style(cyber_panel()).padding(16).into()
    }

    fn log_panel(&self) -> Element<'_, Message> {
        let mut column = column![];
        for entry in self.logs.iter().rev() {
            let color = entry.level.color();
            let text = text(&entry.message).style(text_color(color)).size(14);
            column = column.push(container(text));
        }
        let scroll = scrollable(column.spacing(6))
            .width(Length::Fill)
            .height(Length::Fill);
        let header = text("Activity Feed")
            .size(20)
            .style(text_color(Color::from_rgb8(0xff, 0x73, 0xff)));
        column![header, scroll]
            .spacing(12)
            .width(Length::FillPortion(1))
            .into()
    }

    fn modal_view_option(&self) -> Option<Element<'_, Message>> {
        match &self.modal {
            ModalState::None => None,
            ModalState::Passphrase {
                dataset,
                value,
                error,
            } => {
                let title = text(format!("Passphrase required for {dataset}")).size(22);
                let input = text_input("enter passphrase", value)
                    .secure(true)
                    .on_input(Message::PassphraseInput)
                    .on_submit(Message::SubmitPassphrase);
                let mut column = column![title, input];
                if let Some(err) = error {
                    column = column
                        .push(text(err).style(text_color(Color::from_rgb8(0xff, 0x47, 0x80))));
                }
                column = column.push(
                    row![
                        button("Cancel").on_press(Message::CancelModal),
                        button("Unlock").on_press(Message::SubmitPassphrase)
                    ]
                    .spacing(12),
                );
                Some(modal_container(column))
            }
            ModalState::BreakglassConfirmName { dataset, input } => {
                let column = column![
                    text("Break-glass recovery :: confirm dataset").size(22),
                    text("Type the dataset name exactly to continue:"),
                    text_input(dataset, input).on_input(Message::BreakglassNameInput),
                    row![
                        button("Cancel").on_press(Message::CancelModal),
                        button("Continue").on_press(Message::ConfirmBreakglassName)
                    ]
                    .spacing(12),
                ]
                .spacing(12);
                Some(modal_container(column))
            }
            ModalState::BreakglassConfirmPhrase { dataset, input } => {
                let column = column![
                    text(format!(
                        "Dataset {dataset} confirmed. Type BREAKGLASS to proceed."
                    )),
                    text_input("BREAKGLASS", input).on_input(Message::BreakglassPhraseInput),
                    row![
                        button("Cancel").on_press(Message::CancelModal),
                        button("Continue").on_press(Message::ConfirmBreakglassPhrase)
                    ]
                    .spacing(12),
                ]
                .spacing(12);
                Some(modal_container(column))
            }
            ModalState::BreakglassPassphrase {
                dataset,
                output_path,
                passphrase,
                error,
            } => {
                let mut column = column![
                    text(format!("Emergency key derivation for {dataset}")),
                    text("Specify output path and passphrase."),
                    text_input("/var/lib/lockchain/<file>.key", output_path)
                        .on_input(Message::BreakglassOutputInput),
                    text_input("passphrase", passphrase)
                        .secure(true)
                        .on_input(Message::BreakglassPassphraseInput),
                    row![
                        button("Cancel").on_press(Message::CancelModal),
                        button("Derive key").on_press(Message::ExecuteBreakglass)
                    ]
                    .spacing(12),
                ];
                if let Some(err) = error {
                    column = column
                        .push(text(err).style(text_color(Color::from_rgb8(0xff, 0x47, 0x80))));
                }
                Some(modal_container(column.spacing(12)))
            }
            ModalState::BreakglassComplete {
                dataset,
                output_path,
            } => {
                let column = column![
                    text("Break-glass complete").size(22),
                    text(format!("Dataset unlocked: {dataset}")),
                    text(format!("Key written to {}", output_path.display())),
                    text("Remember to securely delete this file once the incident concludes."),
                    button("Close").on_press(Message::CancelModal),
                ]
                .spacing(12);
                Some(modal_container(column))
            }
        }
    }

    fn push_log(&mut self, level: LogLevel, msg: impl Into<String>) {
        self.logs.push(LogEntry {
            level,
            message: msg.into(),
        });
        if self.logs.len() > 200 {
            self.logs.drain(0..self.logs.len() - 200);
        }
    }
}

fn modal_container(content: widget::Column<'_, Message>) -> Element<'_, Message> {
    container(
        container(content.padding(20).spacing(12))
            .width(Length::Fixed(420.0))
            .style(modal_panel()),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

fn cyber_panel() -> impl Fn(&Theme) -> container::Style + Copy {
    |_| {
        container::Style::default()
            .background(Background::Color(Color::from_rgba(0.05, 0.08, 0.2, 0.6)))
            .border(border::Border {
                color: Color::from_rgb8(0x24, 0xd0, 0xff),
                width: 1.0,
                radius: border::Radius::new(12.0),
            })
    }
}

fn cyber_row() -> impl Fn(&Theme) -> container::Style + Copy {
    |_| {
        container::Style::default()
            .background(Background::Color(Color::from_rgba(0.07, 0.1, 0.24, 0.8)))
            .border(border::Border {
                color: Color::from_rgb8(0xff, 0x73, 0xff),
                width: 1.0,
                radius: border::Radius::new(8.0),
            })
    }
}

fn cyber_container() -> impl Fn(&Theme) -> container::Style + Copy {
    |_| {
        container::Style::default()
            .background(Background::Color(Color::from_rgb8(0x02, 0x05, 0x18)))
    }
}

fn modal_panel() -> impl Fn(&Theme) -> container::Style + Copy {
    |_| {
        container::Style::default()
            .background(Background::Color(Color::from_rgba(0.03, 0.06, 0.16, 0.95)))
            .border(border::Border {
                color: Color::from_rgb8(0xff, 0x47, 0x80),
                width: 1.5,
                radius: border::Radius::new(12.0),
            })
    }
}

fn text_color(color: Color) -> impl Fn(&Theme) -> text::Style + Copy {
    move |_| text::Style { color: Some(color) }
}

async fn load_status(config_path: PathBuf) -> Result<Vec<DatasetKeyDescriptor>, String> {
    let service = build_service(&config_path).map_err(|e| e.to_string())?;
    service.list_keys().map_err(|e| e.to_string())
}

async fn unlock_dataset(
    config_path: PathBuf,
    dataset: String,
    strict_usb: bool,
    passphrase: Option<String>,
) -> Result<Vec<String>, String> {
    let service = build_service(&config_path).map_err(|e| e.to_string())?;
    let mut options = UnlockOptions::default();
    options.strict_usb = strict_usb;
    if let Some(pass) = passphrase {
        options.fallback_passphrase = Some(pass);
    }
    service
        .unlock_with_retry(&dataset, options)
        .map(|r| r.unlocked)
        .map_err(|e| e.to_string())
}

async fn breakglass(
    config_path: PathBuf,
    _dataset: String,
    passphrase: String,
    output: PathBuf,
) -> Result<PathBuf, String> {
    let service = build_service(&config_path).map_err(|e| e.to_string())?;
    let key = service
        .derive_fallback_key(passphrase.as_bytes())
        .map_err(|e| e.to_string())?;
    lockchain_core::keyfile::write_raw_key_file(&output, &key)
        .map_err(|e| LockchainError::from(e).to_string())?;
    Ok(output)
}

fn build_service(
    config_path: &Path,
) -> Result<LockchainService<SystemZfsProvider>, LockchainError> {
    let config = Arc::new(LockchainConfig::load(config_path)?);
    let provider = SystemZfsProvider::from_config(&config)?;
    Ok(LockchainService::new(config, provider))
}
