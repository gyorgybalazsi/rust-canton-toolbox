//! A small desktop window for running the syncer: pick a profile, choose how
//! to start, press Start, watch the log, press Stop.
//!
//! It is optional. The syncer does not depend on it, and everything here can
//! still be done with the `syncer` command line or the `just sync-*` recipes.

mod runner;
mod setup;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::{
    Column, button, checkbox, column, container, pick_list, radio, row, rule, scrollable, space,
    text, text_input,
};
use iced::{Element, Fill, Font, Subscription, Task, window};

use runner::{Event, StopHandle};
use setup::Ready;

/// Lines kept in memory. The full log is in `sync.log`.
const LOG_CAPACITY: usize = 5_000;
/// Lines drawn in the window.
const LOG_SHOWN: usize = 400;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Canton event graph syncer")
        .subscription(App::subscription)
        .exit_on_close_request(false)
        .window_size((1280.0, 800.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Resume,
    Fresh,
    FromDaysAgo,
}

enum Run {
    Idle,
    Starting,
    Running {
        pid: Option<u32>,
        since: Instant,
        stop: StopHandle,
    },
    Stopping {
        since: Instant,
    },
    Finished {
        ok: bool,
        summary: String,
    },
}

impl Run {
    fn is_active(&self) -> bool {
        matches!(
            self,
            Run::Starting | Run::Running { .. } | Run::Stopping { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Warn,
    Error,
}

struct LogLine {
    level: Level,
    text: String,
}

struct App {
    root: PathBuf,
    dotenv: setup::DotEnv,
    env_note: String,
    config_path: String,
    profiles: Vec<String>,
    profile: Option<String>,
    config_error: Option<String>,
    check: Option<Result<Ready, String>>,
    mode: Mode,
    days: String,
    confirming: bool,
    run: Run,
    log: VecDeque<LogLine>,
    only_problems: bool,
    last_progress: Option<String>,
    /// The latest error line, shown next to a failed run.
    last_error: Option<String>,
    closing: bool,
}

#[derive(Debug, Clone)]
enum Message {
    ConfigPathChanged(String),
    ReloadConfig,
    CreateConfig,
    ProfileSelected(String),
    ModeSelected(Mode),
    DaysChanged(String),
    Start,
    ConfirmStart,
    CancelStart,
    Stop,
    Runner(Event),
    Tick,
    OnlyProblems(bool),
    CopyLog,
    ClearLog,
    CloseRequested,
}

impl App {
    fn new() -> Self {
        let root = setup::workspace_root();
        let dotenv = setup::DotEnv::new(&root);
        let config_path = setup::default_config_path(&root).display().to_string();
        let mut app = App {
            root,
            dotenv,
            env_note: String::new(),
            config_path,
            profiles: Vec::new(),
            profile: None,
            config_error: None,
            check: None,
            mode: Mode::Resume,
            days: "7".into(),
            confirming: false,
            run: Run::Idle,
            log: VecDeque::new(),
            only_problems: false,
            last_progress: None,
            last_error: None,
            closing: false,
        };
        app.reload_config();
        app
    }

    fn config_path(&self) -> PathBuf {
        PathBuf::from(self.config_path.trim())
    }

    fn log_path(&self) -> PathBuf {
        self.root.join("sync.log")
    }

    fn reload_config(&mut self) {
        self.env_note = self.dotenv.load();
        match setup::read_profiles(&self.config_path()) {
            Ok(found) => {
                let keep = self.profile.clone().filter(|p| found.profiles.contains(p));
                self.profile = keep.or_else(|| {
                    found
                        .profiles
                        .contains(&found.active)
                        .then_some(found.active.clone())
                        .or_else(|| found.profiles.first().cloned())
                });
                self.profiles = found.profiles;
                self.config_error = None;
            }
            Err(e) => {
                self.profiles.clear();
                self.profile = None;
                self.config_error = Some(e);
            }
        }
        self.recheck();
    }

    fn recheck(&mut self) {
        self.check = self
            .profile
            .as_deref()
            .map(|p| setup::check_profile(&self.config_path(), p));
    }

    fn days(&self) -> Option<u32> {
        self.days
            .trim()
            .parse()
            .ok()
            .filter(|d| (1..=3650).contains(d))
    }

    /// Why Start is disabled, if it is.
    fn blocker(&self) -> Option<String> {
        if self.run.is_active() {
            return Some("The syncer is running.".into());
        }
        match &self.check {
            None => return Some("Choose a config file with at least one profile.".into()),
            Some(Err(_)) => return Some("Fix the setup problem shown above.".into()),
            Some(Ok(_)) => {}
        }
        if self.mode == Mode::FromDaysAgo && self.days().is_none() {
            return Some("Enter a number of days between 1 and 3650.".into());
        }
        None
    }

    fn sync_args(&self, ready: &Ready) -> Vec<String> {
        let mut args = vec![
            "sync".to_string(),
            "--config-file".into(),
            self.config_path().display().to_string(),
        ];
        if let Some(p) = &self.profile {
            args.extend(["--profile".into(), p.clone()]);
        }
        if ready.use_keycloak {
            args.push("--use-keycloak".into());
        }
        match self.mode {
            Mode::Resume => {}
            Mode::Fresh => args.push("--fresh".into()),
            Mode::FromDaysAgo => {
                let days = self.days().unwrap_or(1);
                args.extend(["--from-days-ago".into(), days.to_string()]);
            }
        }
        args
    }

    fn launch(&mut self) -> Task<Message> {
        self.confirming = false;
        // `.env` may have changed since the last check.
        self.env_note = self.dotenv.load();
        self.recheck();
        let Some(Ok(ready)) = self.check.clone() else {
            return Task::none();
        };
        let (program, prefix_args) = runner::locate_syncer();
        if program.file_name().is_some_and(|n| n == "cargo") {
            self.push_line(
                "No syncer program next to this window; building it with cargo first (the first build takes a few minutes).".into(),
            );
        }
        let launch = runner::Launch {
            program,
            prefix_args,
            args: self.sync_args(&ready),
            cwd: self.root.clone(),
            log_file: self.log_path(),
        };
        self.run = Run::Starting;
        self.last_progress = None;
        self.last_error = None;
        Task::stream(runner::run(launch)).map(Message::Runner)
    }

    fn push_line(&mut self, text: String) {
        let level = if text.contains(" ERROR ")
            || text.starts_with("Error:")
            || text.contains("panicked at")
        {
            self.last_error = Some(text.clone());
            Level::Error
        } else if text.contains(" WARN ") || text.starts_with("WARN") {
            Level::Warn
        } else {
            Level::Info
        };
        if text.contains("[Progress]") || text.contains("ACS loading complete") {
            self.last_progress = text
                .split_once("[Progress] ")
                .map(|(_, rest)| rest.to_string())
                .or_else(|| Some(text.clone()));
        }
        if self.log.len() == LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(LogLine { level, text });
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConfigPathChanged(path) => {
                self.config_path = path;
                self.reload_config();
            }
            Message::ReloadConfig => self.reload_config(),
            Message::CreateConfig => {
                let target = self.config_path();
                let example = setup::example_config_path(&self.root);
                if !target.exists()
                    && let Err(e) = std::fs::copy(&example, &target)
                {
                    self.config_error = Some(format!(
                        "Could not copy {} to {}: {e}",
                        example.display(),
                        target.display()
                    ));
                    return Task::none();
                }
                self.reload_config();
            }
            Message::ProfileSelected(p) => {
                self.profile = Some(p);
                self.recheck();
            }
            Message::ModeSelected(mode) => {
                if self.run.is_active() {
                    return Task::none();
                }
                self.mode = mode;
                self.confirming = false;
            }
            Message::DaysChanged(days) => {
                self.days = days.chars().filter(char::is_ascii_digit).collect();
            }
            Message::Start => {
                if self.blocker().is_none() {
                    if self.mode == Mode::Resume {
                        return self.launch();
                    }
                    self.confirming = true;
                }
            }
            Message::ConfirmStart => {
                if self.blocker().is_none() {
                    return self.launch();
                }
            }
            Message::CancelStart => self.confirming = false,
            Message::Stop => {
                if let Run::Running { stop, .. } = &self.run {
                    stop.stop();
                    self.run = Run::Stopping {
                        since: Instant::now(),
                    };
                }
            }
            Message::Runner(event) => {
                match event {
                    Event::Started { pid, stop } => {
                        if self.closing {
                            stop.stop();
                            self.run = Run::Stopping {
                                since: Instant::now(),
                            };
                        } else if matches!(self.run, Run::Starting) {
                            self.run = Run::Running {
                                pid,
                                since: Instant::now(),
                                stop,
                            };
                        }
                    }
                    Event::Line(line) => self.push_line(line),
                    Event::Exited { ok, summary } => {
                        self.push_line(format!("(syncer {summary})"));
                        if self.mode != Mode::Resume {
                            // Never wipe the database twice by accident: the next
                            // start continues unless a fresh start is chosen again.
                            self.mode = Mode::Resume;
                            self.push_line("(next start will resume; choose a fresh start again if you need one)".into());
                        }
                        self.run = Run::Finished { ok, summary };
                        if self.closing {
                            return iced::exit();
                        }
                    }
                }
            }
            Message::Tick => {}
            Message::OnlyProblems(on) => self.only_problems = on,
            Message::CopyLog => {
                let all = self
                    .log
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                return iced::clipboard::write(all);
            }
            Message::ClearLog => self.log.clear(),
            Message::CloseRequested => {
                // Stop the syncer first so it is not left running unseen.
                self.closing = true;
                match &self.run {
                    Run::Running { stop, .. } => {
                        stop.stop();
                        self.run = Run::Stopping {
                            since: Instant::now(),
                        };
                    }
                    Run::Starting | Run::Stopping { .. } => {}
                    Run::Idle | Run::Finished { .. } => return iced::exit(),
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let close = window::close_requests().map(|_| Message::CloseRequested);
        if self.run.is_active() {
            Subscription::batch([
                close,
                iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            ])
        } else {
            close
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let controls = column![
            text("Canton event graph syncer").size(24),
            text("Copies a Canton ledger's transactions and contracts into SurrealDB, then keeps following it.")
                .style(text::secondary),
            self.view_setup(),
            rule::horizontal(1),
            self.view_mode(),
        ]
        .spacing(16)
        .padding(iced::Padding::new(20.0).right(28.0));

        row![
            scrollable(controls).width(500).height(Fill),
            rule::vertical(1),
            column![self.view_controls(), rule::horizontal(1), self.view_log()]
                .spacing(16)
                .padding(20)
                .width(Fill)
                .height(Fill),
        ]
        .height(Fill)
        .into()
    }

    fn view_setup(&self) -> Element<'_, Message> {
        let editable = !self.run.is_active();
        let path_input = text_input("path to config.toml", &self.config_path)
            .on_input_maybe(editable.then_some(Message::ConfigPathChanged))
            .width(Fill);
        let config_row = row![
            path_input,
            button("Reload").on_press_maybe(editable.then_some(Message::ReloadConfig)),
        ]
        .spacing(10)
        .align_y(iced::Center);

        let mut col = Column::new()
            .spacing(8)
            .push(text("Config file"))
            .push(config_row);
        if let Some(err) = &self.config_error {
            col = col.push(text(err.clone()).style(text::danger));
            if !self.config_path().exists() {
                col = col.push(
                    text("The example has a 'local' profile for a Daml sandbox on localhost:6865. Copy it, then edit it for your ledger and SurrealDB server.")
                        .size(13)
                        .style(text::secondary),
                );
                col = col.push(
                    button("Create from example")
                        .style(button::secondary)
                        .on_press_maybe(editable.then_some(Message::CreateConfig)),
                );
            }
            return section("Setup", col.into());
        }

        col = col.push(
            row![
                label("Profile"),
                pick_list(
                    self.profiles.as_slice(),
                    self.profile.clone(),
                    Message::ProfileSelected
                )
                .placeholder("choose a profile"),
            ]
            .spacing(10)
            .align_y(iced::Center),
        );

        match &self.check {
            Some(Ok(ready)) => {
                col = col
                    .push(fact("Ledger", &ready.ledger_url))
                    .push(fact("Sign-in", &ready.sign_in))
                    .push(fact("Parties", &ready.parties))
                    .push(fact("SurrealDB", &ready.database));
            }
            Some(Err(e)) => {
                col = col.push(text(e.clone()).style(text::danger));
                col = col.push(
                    text("Add missing secrets to a .env file at the repository root and press Reload, or set them in the environment before opening this window. The window never stores them.")
                        .size(13)
                        .style(text::secondary),
                );
            }
            None => {}
        }
        col = col.push(text(&self.env_note).size(13).style(text::secondary));
        section("Setup", col.into())
    }

    fn view_mode(&self) -> Element<'_, Message> {
        let editable = !self.run.is_active();
        let selected = Some(self.mode);
        // Radios have no disabled state; `update` ignores them while running.
        let choice = |title: &'static str,
                      mode: Mode,
                      extra: Option<Element<'static, Message>>,
                      help: &'static str|
         -> Element<'_, Message> {
            let mut head = row![radio(title, mode, selected, Message::ModeSelected)]
                .spacing(8)
                .align_y(iced::Center);
            if let Some(extra) = extra {
                head = head.push(extra);
            }
            column![head, text(help).size(13).style(text::secondary)]
                .spacing(2)
                .into()
        };

        let days: Element<'static, Message> = row![
            text_input("7", &self.days)
                .on_input_maybe(editable.then_some(Message::DaysChanged))
                .width(60),
            text("days ago"),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into();

        let col = column![
            choice(
                "Resume",
                Mode::Resume,
                None,
                "Continue from the last offset stored in SurrealDB. On an empty database it starts at the profile's starting offset."
            ),
            choice(
                "Fresh start",
                Mode::Fresh,
                None,
                "Delete the synced data, load the contracts that are active now, then follow new transactions."
            ),
            choice(
                "Fresh start from",
                Mode::FromDaysAgo,
                Some(days),
                "Delete the synced data and replay the ledger from that point. Older history may have been pruned by the participant."
            ),
        ]
        .spacing(10);
        section("How to start", col.into())
    }

    fn view_controls(&self) -> Element<'_, Message> {
        let blocker = self.blocker();
        let start_label = match self.mode {
            Mode::Resume => "Start",
            Mode::Fresh | Mode::FromDaysAgo => "Start fresh…",
        };
        let start = button(start_label)
            .style(button::success)
            .on_press_maybe((blocker.is_none() && !self.confirming).then_some(Message::Start));
        let stop = button("Stop")
            .style(button::danger)
            .on_press_maybe(matches!(self.run, Run::Running { .. }).then_some(Message::Stop));

        let status: Element<'_, Message> = match &self.run {
            Run::Idle => text("Not running").into(),
            Run::Starting => text("Starting…").style(text::primary).into(),
            Run::Running { pid, since, .. } => {
                let pid = pid.map(|p| format!(", process {p}")).unwrap_or_default();
                text(format!("Running for {}{pid}", elapsed(*since)))
                    .style(text::success)
                    .into()
            }
            Run::Stopping { since } => text(format!("Stopping… ({})", elapsed(*since)))
                .style(text::warning)
                .into(),
            Run::Finished { ok: true, summary } => {
                text(format!("Not running (last run {summary})")).into()
            }
            Run::Finished { ok: false, summary } => text(match &self.last_error {
                Some(e) => format!("The last run failed ({summary}): {e}"),
                None => format!("The last run failed ({summary}). See the log."),
            })
            .style(text::danger)
            .into(),
        };

        let mut col =
            column![row![start, stop, status].spacing(12).align_y(iced::Center)].spacing(8);

        if self.confirming {
            let db = match &self.check {
                Some(Ok(r)) => r.fresh_target.clone(),
                _ => "the configured database".into(),
            };
            col = col.push(
                container(
                    column![
                        text(format!(
                            "This deletes the synced data in {db} before starting."
                        )),
                        row![
                            button("Delete and start")
                                .style(button::danger)
                                .on_press(Message::ConfirmStart),
                            button("Cancel")
                                .style(button::secondary)
                                .on_press(Message::CancelStart),
                        ]
                        .spacing(10),
                    ]
                    .spacing(8),
                )
                .padding(10)
                .style(container::bordered_box),
            );
        } else if let Some(reason) = blocker.filter(|_| !self.run.is_active()) {
            col = col.push(text(reason).size(13).style(text::secondary));
        }

        if let Some(progress) = &self.last_progress {
            col = col.push(fact("Last progress", progress));
        }
        col.into()
    }

    fn view_log(&self) -> Element<'_, Message> {
        let problems = self.log.iter().filter(|l| l.level != Level::Info).count();
        let header = row![
            text("Log").size(18),
            space::horizontal(),
            checkbox(self.only_problems)
                .label(format!("Warnings and errors only ({problems})"))
                .on_toggle(Message::OnlyProblems),
            button("Copy")
                .style(button::secondary)
                .on_press(Message::CopyLog),
            button("Clear")
                .style(button::secondary)
                .on_press(Message::ClearLog),
        ]
        .spacing(10)
        .align_y(iced::Center);

        let lines: Vec<&LogLine> = self
            .log
            .iter()
            .filter(|l| !self.only_problems || l.level != Level::Info)
            .collect();
        let start = lines.len().saturating_sub(LOG_SHOWN);
        let body = Column::with_children(lines[start..].iter().map(|l| {
            let t = text(l.text.as_str()).font(Font::MONOSPACE).size(12);
            match l.level {
                Level::Info => t.into(),
                Level::Warn => t.style(text::warning).into(),
                Level::Error => t.style(text::danger).into(),
            }
        }))
        .spacing(1)
        .padding(8)
        .width(Fill);

        let body: Element<'_, Message> = if self.log.is_empty() {
            container(text("Nothing yet. Press Start to run the syncer.").style(text::secondary))
                .padding(8)
                .into()
        } else {
            body.into()
        };

        column![
            header,
            container(scrollable(body).anchor_bottom().height(Fill).width(Fill))
                .style(container::bordered_box)
                .height(Fill)
                .width(Fill),
            text(format!(
                "The full log is also written to {}",
                self.log_path().display()
            ))
            .size(13)
            .style(text::secondary),
        ]
        .spacing(8)
        .height(Fill)
        .into()
    }
}

fn section<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    column![text(title).size(18), body].spacing(8).into()
}

fn label(s: &str) -> Element<'_, Message> {
    container(text(s)).width(105).into()
}

fn fact<'a>(name: &'a str, value: &'a str) -> Element<'a, Message> {
    row![label(name), text(value)].spacing(10).into()
}

fn elapsed(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    match (s / 3600, (s / 60) % 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s:02}s"),
        (h, m, _) => format!("{h}h {m:02}m"),
    }
}
