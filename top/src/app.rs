//! The dashboard's state and what keys do to it

use std::collections::HashMap;

use ratatui::style::Color;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use sndbx::builder::BASE_NAME;
use sndbx::settings::Image;

use crate::model::{ProxyLog, Sandbox, Snapshot, Stats};
use crate::sources::Event;

/// What sandboxes are told apart by, in the table, the details and the log
pub const SANDBOX_COLORS: [Color; 10] = [
    Color::Cyan,
    Color::Green,
    Color::Blue,
    Color::Magenta,
    Color::Yellow,
    Color::LightCyan,
    Color::LightGreen,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightYellow,
];

/// How long a message stays in the footer
const MESSAGE_FOR: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, PartialEq, Debug, ValueEnum)]
pub enum View {
    Dashboard,
    Images,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pane {
    Sandboxes,
    Log,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LogFilter {
    All,
    Denied,
    /// The selected sandbox's
    Selected,
}

/// What's drawn where, for the mouse
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Tab(View),
    Pane(Pane),
    Images,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Region {
    pub area: Rect,
    pub target: Target,
    /// For a list, the line of its first row and how many rows show
    pub rows: Option<(u16, usize)>,
}

/// Lines a wheel step scrolls the log
const LOG_STEP: isize = 3;
/// What page up and down move
const PAGE: isize = 10;

/// What the main loop does for a key
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    Quit,
    /// With the dashboard suspended, as it takes the terminal
    Shell(String),
    Resume(String),
    Build {
        names: Vec<String>,
        rebuild_base: bool,
    },
    RemoveImage(String),
    /// In the background
    Stop(String),
    Remove(String),
}

impl Action {
    /// Whether it takes the terminal, so the dashboard is suspended meanwhile
    pub fn suspends(&self) -> bool {
        matches!(
            self,
            Self::Shell(_) | Self::Resume(_) | Self::Build { .. } | Self::RemoveImage(_)
        )
    }
}

/// A row of the Images view
#[derive(Clone, PartialEq, Debug)]
pub enum ImageRow {
    Configured(String),
    Base,
    /// In `images_dir` but not in the settings
    Buildable(String),
}

impl ImageRow {
    pub fn name(&self) -> &str {
        match self {
            Self::Configured(name) | Self::Buildable(name) => name,
            Self::Base => BASE_NAME,
        }
    }
}

pub struct Message {
    pub text: String,
    pub error: bool,
    at: Instant,
}

pub struct Confirm {
    pub prompt: String,
    action: Action,
}

pub struct App {
    pub view: View,
    pub focus: Pane,
    pub zoomed: bool,
    pub snapshot: Snapshot,
    /// Whether a snapshot came yet
    pub loaded: bool,
    pub stats: HashMap<String, Stats>,
    pub log: ProxyLog,
    pub log_filter: LogFilter,
    /// Lines up from the newest, 0 follows new ones
    pub log_scroll: usize,
    /// The log pane's rows in the last frame, which the oldest lines fill when scrolled up all the way
    log_rows: usize,
    /// Only this image's sandboxes
    pub image_filter: Option<String>,
    pub sandbox_index: usize,
    pub image_index: usize,
    pub message: Option<Message>,
    pub confirm: Option<Confirm>,
    /// Each sandbox's, kept while it's there; an index in `SANDBOX_COLORS`
    colors: HashMap<String, usize>,
}

impl App {
    pub fn new(view: View) -> Self {
        Self {
            view,
            focus: Pane::Sandboxes,
            zoomed: false,
            snapshot: Snapshot::default(),
            loaded: false,
            stats: HashMap::new(),
            log: ProxyLog::default(),
            log_filter: LogFilter::All,
            log_scroll: 0,
            log_rows: 1,
            image_filter: None,
            sandbox_index: 0,
            image_index: 0,
            message: None,
            confirm: None,
            colors: HashMap::new(),
        }
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Snapshot(snapshot) => {
                // Keep the selection on the same sandbox when others come or go
                let selected = self.selected_sandbox().map(|sandbox| sandbox.name.clone());
                self.snapshot = snapshot;
                self.loaded = true;
                self.assign_colors();
                if let Some(name) = selected
                    && let Some(index) = self
                        .sandboxes()
                        .iter()
                        .position(|sandbox| sandbox.name == name)
                {
                    self.sandbox_index = index;
                }
                self.clamp();
            }
            Event::Stats(stats) => self.stats = stats,
            Event::LogReset => {
                self.log.clear();
                self.log_scroll = 0;
            }
            Event::Log(line) => {
                if self.log_scroll > 0 && self.line_shown(&line, self.selected_sandbox()) {
                    // Keep the lines in view while scrolled up
                    self.log_scroll += 1;
                }
                let full = self.log.lines.len() == crate::model::LOG_LINES;
                self.log.push(line);
                // The oldest line went
                if full && self.log_scroll > 0 {
                    self.clamp_log_scroll();
                }
            }
            Event::Done(result) => match result {
                Ok(text) => self.say(text),
                Err(text) => self.warn(text),
            },
        }
    }

    /// Gives each new sandbox the first color no other has, in name order, and drops the gone ones
    fn assign_colors(&mut self) {
        let mut names: Vec<&str> = self
            .snapshot
            .sandboxes
            .iter()
            .map(|sandbox| sandbox.name.as_str())
            .collect();
        names.sort_unstable();
        self.colors.retain(|name, _| names.contains(&name.as_str()));
        for name in names {
            if self.colors.contains_key(name) {
                continue;
            }
            let free = (0..SANDBOX_COLORS.len())
                .find(|index| !self.colors.values().any(|used| used == index))
                .unwrap_or(self.colors.len() % SANDBOX_COLORS.len());
            self.colors.insert(name.to_string(), free);
        }
    }

    pub fn sandbox_color(&self, name: &str) -> Color {
        self.colors
            .get(name)
            .map_or(Color::Reset, |index| SANDBOX_COLORS[*index])
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    /// Drops an old message
    pub fn tick(&mut self) {
        if self
            .message
            .as_ref()
            .is_some_and(|message| message.at.elapsed() > MESSAGE_FOR)
        {
            self.message = None;
        }
    }

    /// The sandboxes shown, the image filter's
    pub fn sandboxes(&self) -> Vec<&Sandbox> {
        self.snapshot
            .sandboxes
            .iter()
            .filter(|sandbox| {
                self.image_filter
                    .as_ref()
                    .is_none_or(|image| sandbox.image == *image)
            })
            .collect()
    }

    pub fn selected_sandbox(&self) -> Option<&Sandbox> {
        self.sandboxes().get(self.sandbox_index).copied()
    }

    fn line_shown(&self, line: &sndbx::proxy::LogLine, selected: Option<&Sandbox>) -> bool {
        match self.log_filter {
            LogFilter::All => true,
            LogFilter::Denied => line.denied(),
            LogFilter::Selected => selected.is_some_and(|sandbox| sandbox.sent(line)),
        }
    }

    pub fn shown_lines(&self) -> Vec<&sndbx::proxy::LogLine> {
        let selected = self.selected_sandbox();
        self.log
            .lines
            .iter()
            .filter(|line| self.line_shown(line, selected))
            .collect()
    }

    /// Configured images in the settings' order, base, then the buildable ones not in the settings
    pub fn image_rows(&self) -> Vec<ImageRow> {
        let configured = self
            .snapshot
            .settings
            .as_ref()
            .map(sndbx::settings::Settings::image_names)
            .unwrap_or_default();
        let mut rows: Vec<ImageRow> = configured
            .iter()
            .cloned()
            .map(ImageRow::Configured)
            .collect();
        rows.push(ImageRow::Base);
        rows.extend(
            self.snapshot
                .buildable
                .iter()
                .filter(|name| !configured.contains(name))
                .cloned()
                .map(ImageRow::Buildable),
        );
        rows
    }

    pub fn selected_image(&self) -> Option<ImageRow> {
        self.image_rows().get(self.image_index).cloned()
    }

    /// The settings' entry for a configured image
    pub fn configured_image(&self, name: &str) -> Option<Image> {
        self.snapshot.settings.as_ref()?.image(name).ok()
    }

    fn clamp(&mut self) {
        let clamp =
            |index: &mut usize, count: usize| *index = (*index).min(count.saturating_sub(1));
        let (sandboxes, images) = (self.sandboxes().len(), self.image_rows().len());
        clamp(&mut self.sandbox_index, sandboxes);
        clamp(&mut self.image_index, images);
        self.clamp_log_scroll();
    }

    fn clamp_log_scroll(&mut self) {
        // Up to the oldest lines filling the pane
        self.log_scroll = self
            .log_scroll
            .min(self.shown_lines().len().saturating_sub(self.log_rows));
    }

    /// What the last frame drew the log's lines in, if it drew the log
    pub fn set_log_rows(&mut self, rows: usize) {
        self.log_rows = rows.max(1);
        self.clamp_log_scroll();
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<Action> {
        self.message = None;
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Some(Action::Quit);
        }
        if let Some(confirm) = self.confirm.take() {
            return matches!(key.code, KeyCode::Char('y' | 'Y')).then_some(confirm.action);
        }
        match key.code {
            KeyCode::Char('q') => return Some(Action::Quit),
            KeyCode::Char('1') => self.view = View::Dashboard,
            KeyCode::Char('2') => self.view = View::Images,
            // In either view
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-PAGE),
            KeyCode::PageDown => self.move_by(PAGE),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_by(isize::MAX / 2),
            _ => {
                let action = match self.view {
                    View::Dashboard => self.dashboard_key(key),
                    View::Images => self.images_key(key),
                };
                self.clamp();
                return action;
            }
        }
        self.clamp();
        None
    }

    /// The wheel scrolls what's under the pointer, a click focuses it and selects the row
    pub fn mouse(&mut self, mouse: MouseEvent, regions: &[Region]) {
        let position = Position::new(mouse.column, mouse.row);
        let Some(region) = regions.iter().find(|region| region.area.contains(position)) else {
            return;
        };
        // A question waits for its key
        if self.confirm.is_some() {
            return;
        }
        let step = match mouse.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::Down(MouseButton::Left) => 0,
            _ => return,
        };
        self.message = None;
        match region.target {
            Target::Tab(view) => {
                if step == 0 {
                    self.view = view;
                }
                return;
            }
            Target::Pane(pane) => self.focus = pane,
            Target::Images => {}
        }
        if step != 0 {
            let lines = if self.focus == Pane::Log && region.target != Target::Images {
                LOG_STEP
            } else {
                1
            };
            self.move_by(step * lines);
        } else if let Some((first, shown)) = region.rows
            && mouse.row >= first
        {
            self.select_at(region.target, usize::from(mouse.row - first), shown);
        }
        self.clamp();
    }

    /// Selects the row at `line` of the `shown` ones, which scroll to keep the selected one in view
    fn select_at(&mut self, target: Target, line: usize, shown: usize) {
        let (images, sandboxes) = (self.image_rows().len(), self.sandboxes().len());
        let (index, count) = match target {
            Target::Images => (&mut self.image_index, images),
            Target::Pane(Pane::Sandboxes) => (&mut self.sandbox_index, sandboxes),
            _ => return,
        };
        let offset = (*index + 1).saturating_sub(shown);
        if offset + line < count {
            *index = offset + line;
        }
    }

    fn dashboard_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => self.switch_focus(),
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            KeyCode::Char('f') => {
                self.log_filter = match self.log_filter {
                    LogFilter::All => LogFilter::Denied,
                    LogFilter::Denied => LogFilter::Selected,
                    LogFilter::Selected => LogFilter::All,
                };
                self.log_scroll = 0;
            }
            KeyCode::Esc => {
                if self.zoomed {
                    self.zoomed = false;
                } else {
                    self.image_filter = None;
                }
            }
            _ if self.focus == Pane::Sandboxes => return self.sandbox_key(key),
            _ => {}
        }
        None
    }

    fn switch_focus(&mut self) {
        self.focus = match self.focus {
            Pane::Sandboxes => Pane::Log,
            Pane::Log => Pane::Sandboxes,
        };
    }

    /// Down is positive; in the log, down is towards the newest line
    fn move_by(&mut self, delta: isize) {
        let moved = |index: usize| index.saturating_add_signed(delta);
        match (self.view, self.focus) {
            (View::Images, _) => self.image_index = moved(self.image_index),
            (View::Dashboard, Pane::Sandboxes) => self.sandbox_index = moved(self.sandbox_index),
            (View::Dashboard, Pane::Log) => {
                self.log_scroll = self.log_scroll.saturating_add_signed(-delta);
            }
        }
    }

    fn sandbox_key(&mut self, key: KeyEvent) -> Option<Action> {
        let sandbox = self.selected_sandbox()?;
        let name = sandbox.name.clone();
        let running = sandbox.running();
        // As sndbx resume takes them
        let stopped = sandbox.state == "exited";
        let state = sandbox.state.clone();
        match key.code {
            KeyCode::Enter if running => Some(Action::Shell(name)),
            KeyCode::Char('r') if stopped => Some(Action::Resume(name)),
            KeyCode::Char('s') if running => {
                self.ask(format!("Stop {name}?"), Action::Stop(name));
                None
            }
            KeyCode::Char('d') => {
                let state = if running { "running " } else { "" };
                self.ask(
                    format!("Remove the {state}sandbox {name}?"),
                    Action::Remove(name),
                );
                None
            }
            KeyCode::Enter if stopped => {
                self.warn(format!("{name} isn't running, r resumes it"));
                None
            }
            KeyCode::Char('r') if running => {
                self.warn(format!("{name} is running, enter opens a shell"));
                None
            }
            KeyCode::Enter | KeyCode::Char('r') => {
                self.warn(format!("{name} is {state}"));
                None
            }
            _ => None,
        }
    }

    fn images_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char(key @ ('b' | 'B')) => {
                let row = self.selected_image()?;
                if !self.buildable(&row) {
                    self.warn(format!(
                        "{} has no Dockerfile in images_dir, sndbx build can't build it",
                        row.name()
                    ));
                    return None;
                }
                let rebuild_base = key == 'B' || row == ImageRow::Base;
                let names = match row {
                    ImageRow::Base => Vec::new(),
                    row => vec![row.name().to_string()],
                };
                return Some(Action::Build {
                    names,
                    rebuild_base,
                });
            }
            KeyCode::Char('d') => match self.selected_image()? {
                ImageRow::Configured(name) => {
                    self.ask(
                        format!("Remove the image {name} and comment out its entry?"),
                        Action::RemoveImage(name),
                    );
                }
                ImageRow::Base => {
                    self.warn(format!(
                        "{BASE_NAME} can't be removed, the other images are built on it"
                    ));
                }
                ImageRow::Buildable(name) if self.built(&name) => {
                    self.ask(
                        format!("Remove the image {name}?"),
                        Action::RemoveImage(name),
                    );
                }
                ImageRow::Buildable(name) => self.warn(format!("{name} isn't built")),
            },
            KeyCode::Enter => {
                if let Some(ImageRow::Configured(name)) = self.selected_image() {
                    self.image_filter = Some(name);
                    self.sandbox_index = 0;
                    self.focus = Pane::Sandboxes;
                    self.view = View::Dashboard;
                }
            }
            _ => {}
        }
        None
    }

    /// Whether `b` can build it: base, or an image with a directory in `images_dir`
    pub fn buildable(&self, row: &ImageRow) -> bool {
        match row {
            ImageRow::Base | ImageRow::Buildable(_) => true,
            ImageRow::Configured(name) => self.snapshot.buildable.contains(name),
        }
    }

    /// Whether an image not in the settings is built, under its default tag
    fn built(&self, name: &str) -> bool {
        self.snapshot
            .settings
            .as_ref()
            .is_some_and(|settings| self.snapshot.image(&settings.default_tag(name)).is_some())
    }

    fn ask(&mut self, prompt: String, action: Action) {
        self.confirm = Some(Confirm { prompt, action });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut App, code: KeyCode) -> Option<Action> {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn sandbox(name: &str, image: &str, state: &str, ip: Option<&str>) -> Sandbox {
        Sandbox {
            name: name.to_string(),
            image: image.to_string(),
            state: state.to_string(),
            started: "2026-11-05T12:00:00Z".parse().ok(),
            workdir: String::new(),
            mounts: Vec::new(),
            ports: Vec::new(),
            networks: Vec::new(),
            proxy_ip: ip.map(ToString::to_string),
            ip: ip.map(ToString::to_string),
            pids_limit: None,
        }
    }

    fn app() -> App {
        let mut app = App::new(View::Dashboard);
        app.apply(Event::Snapshot(Snapshot {
            sandboxes: vec![
                sandbox("a", "claude", "running", Some("1.1.1.1")),
                sandbox("b", "codex", "running", Some("2.2.2.2")),
                sandbox("c", "claude", "exited", None),
            ],
            ..Snapshot::default()
        }));
        for (client, result, host) in [
            ("1.1.1.1", "TCP_TUNNEL/200", "a.com"),
            ("2.2.2.2", "TCP_DENIED/403", "b.com"),
            ("2.2.2.2", "TCP_DENIED/403", "b.com"),
            ("1.1.1.1", "TCP_DENIED/403", "c.com"),
        ] {
            let line = format!(
                "2026-11-05T12:41:03+0000 {client} {result} CONNECT {host}:443 dur=1ms in=1 out=1"
            );
            app.apply(Event::Log(sndbx::proxy::LogLine::parse(&line).unwrap()));
        }
        app
    }

    #[test]
    fn acts_on_the_selected_sandbox() {
        let mut app = app();
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Some(Action::Shell("a".to_string()))
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_sandbox().unwrap().name, "c");
        assert_eq!(press(&mut app, KeyCode::Enter), None);
        assert!(app.message.as_ref().unwrap().text.contains("r resumes it"));
        assert!(app.message.as_ref().unwrap().error);
        assert_eq!(
            press(&mut app, KeyCode::Char('r')),
            Some(Action::Resume("c".to_string()))
        );

        // Only a stopped one, as sndbx resume takes
        app.apply(Event::Snapshot(Snapshot {
            sandboxes: vec![sandbox("p", "claude", "paused", None)],
            ..Snapshot::default()
        }));
        assert_eq!(press(&mut app, KeyCode::Char('r')), None);
        assert_eq!(app.message.as_ref().unwrap().text, "p is paused");
    }

    #[test]
    fn confirms_stopping_and_removing() {
        let mut app = app();
        assert_eq!(press(&mut app, KeyCode::Char('s')), None);
        assert_eq!(app.confirm.as_ref().unwrap().prompt, "Stop a?");
        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            Some(Action::Stop("a".to_string()))
        );
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(press(&mut app, KeyCode::Char('n')), None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn switches_panes() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Pane::Log);
        // Keys for the sandboxes do nothing in the log
        assert_eq!(press(&mut app, KeyCode::Enter), None);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Pane::Sandboxes);
    }

    #[test]
    fn starts_over_when_the_log_is_read_again() {
        let mut app = app();
        app.log_scroll = 2;
        app.apply(Event::LogReset);
        assert_eq!((app.log_scroll, app.log.lines.len()), (0, 0));
    }

    #[test]
    fn filters_the_log() {
        let mut app = app();
        assert_eq!(app.shown_lines().len(), 4);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.shown_lines().len(), 3);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.log_filter, LogFilter::Selected);
        assert_eq!(app.shown_lines().len(), 2);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.shown_lines().len(), 2);
    }

    #[test]
    fn gives_sandboxes_their_own_colors() {
        let mut app = app();
        let colors = |app: &App| -> Vec<Color> {
            ["a", "b", "c"]
                .iter()
                .map(|name| app.sandbox_color(name))
                .collect()
        };
        assert_eq!(colors(&app), &SANDBOX_COLORS[..3]);

        // A gone sandbox's color goes to the next new one; the others keep theirs
        app.apply(Event::Snapshot(Snapshot {
            sandboxes: vec![
                sandbox("a", "claude", "running", None),
                sandbox("c", "claude", "running", None),
                sandbox("d", "claude", "running", None),
            ],
            ..Snapshot::default()
        }));
        assert_eq!(app.sandbox_color("a"), SANDBOX_COLORS[0]);
        assert_eq!(app.sandbox_color("c"), SANDBOX_COLORS[2]);
        assert_eq!(app.sandbox_color("d"), SANDBOX_COLORS[1]);
        assert_eq!(app.sandbox_color("b"), Color::Reset);
    }

    fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16, regions: &[Region]) {
        app.mouse(
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            regions,
        );
    }

    #[test]
    fn follows_the_mouse() {
        let mut app = app();
        let regions = [
            Region {
                area: Rect::new(8, 0, 13, 1),
                target: Target::Tab(View::Images),
                rows: None,
            },
            Region {
                area: Rect::new(0, 1, 80, 6),
                target: Target::Pane(Pane::Sandboxes),
                // Rows from line 3, two of them show
                rows: Some((3, 2)),
            },
            Region {
                area: Rect::new(0, 7, 60, 10),
                target: Target::Pane(Pane::Log),
                rows: None,
            },
        ];
        let click = MouseEventKind::Down(MouseButton::Left);

        // The second row shown
        mouse(&mut app, click, 5, 4, &regions);
        assert_eq!(app.sandbox_index, 1);
        // Scrolled to keep the third in view, the first line shows the second
        mouse(&mut app, MouseEventKind::ScrollDown, 5, 4, &regions);
        assert_eq!(app.sandbox_index, 2);
        mouse(&mut app, click, 5, 3, &regions);
        assert_eq!(app.sandbox_index, 1);
        // Below the rows
        mouse(&mut app, click, 5, 6, &regions);
        assert_eq!(app.sandbox_index, 1);

        mouse(&mut app, MouseEventKind::ScrollUp, 5, 10, &regions);
        assert_eq!((app.focus, app.log_scroll), (Pane::Log, 3));
        mouse(&mut app, MouseEventKind::ScrollDown, 5, 10, &regions);
        assert_eq!(app.log_scroll, 0);

        // Waits for the answer
        app.focus = Pane::Sandboxes;
        press(&mut app, KeyCode::Char('s'));
        mouse(&mut app, click, 10, 0, &regions);
        assert_eq!(app.view, View::Dashboard);
        press(&mut app, KeyCode::Char('n'));
        mouse(&mut app, click, 10, 0, &regions);
        assert_eq!(app.view, View::Images);
    }

    #[test]
    fn keeps_the_selected_sandbox() {
        let mut app = app();
        press(&mut app, KeyCode::Down);
        app.apply(Event::Snapshot(Snapshot {
            sandboxes: vec![
                sandbox("0", "claude", "running", None),
                sandbox("b", "codex", "running", None),
            ],
            ..Snapshot::default()
        }));
        assert_eq!(app.selected_sandbox().unwrap().name, "b");
    }

    #[test]
    fn shows_an_images_sandboxes() {
        let mut app = app();
        app.image_filter = Some("claude".to_string());
        assert_eq!(app.sandboxes().len(), 2);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.sandboxes().len(), 3);
    }

    #[test]
    fn removes_built_images_the_settings_dont_have() {
        let mut app = App::new(View::Images);
        app.apply(Event::Snapshot(Snapshot {
            settings: Some(sndbx::settings::Settings::default()),
            buildable: vec!["built".to_string(), "unbuilt".to_string()],
            images: [(
                "sndbx:built".to_string(),
                crate::model::ImageInfo {
                    size: "1G".to_string(),
                    created: None,
                },
            )]
            .into(),
            ..Snapshot::default()
        }));
        assert_eq!(
            app.image_rows(),
            [
                ImageRow::Base,
                ImageRow::Buildable("built".to_string()),
                ImageRow::Buildable("unbuilt".to_string())
            ]
        );

        press(&mut app, KeyCode::Char('d'));
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .text
                .starts_with("base can't be removed")
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            Some(Action::RemoveImage("built".to_string()))
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.message.as_ref().unwrap().text, "unbuilt isn't built");
    }

    #[test]
    fn builds_only_images_in_images_dir() {
        let app = App {
            snapshot: Snapshot {
                buildable: vec!["mine".to_string()],
                ..Snapshot::default()
            },
            ..App::new(View::Images)
        };
        assert!(app.buildable(&ImageRow::Base));
        assert!(app.buildable(&ImageRow::Buildable("new".to_string())));
        assert!(app.buildable(&ImageRow::Configured("mine".to_string())));
        assert!(!app.buildable(&ImageRow::Configured("pulled".to_string())));
    }

    #[test]
    fn pages_through_images() {
        let mut app = App::new(View::Images);
        app.apply(Event::Snapshot(Snapshot {
            buildable: (0..15).map(|i| format!("image-{i:02}")).collect(),
            ..Snapshot::default()
        }));
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.image_index, 10);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.image_index, 15);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.image_index, 5);
    }

    #[test]
    fn keeps_the_scroll_within_a_full_log() {
        let mut app = app();
        let line = |host: &str| {
            let line = format!(
                "2026-11-05T12:42:00+0000 1.1.1.1 TCP_TUNNEL/200 CONNECT {host}:443 dur=1ms in=1 out=1"
            );
            sndbx::proxy::LogLine::parse(&line).unwrap()
        };
        for _ in 0..crate::model::LOG_LINES {
            app.apply(Event::Log(line("a.com")));
        }
        app.set_log_rows(10);
        // At the oldest lines
        app.focus = Pane::Log;
        press(&mut app, KeyCode::Home);
        assert_eq!(app.log_scroll, crate::model::LOG_LINES - 10);
        app.apply(Event::Log(line("b.com")));
        assert_eq!(app.log_scroll, crate::model::LOG_LINES - 10);
    }

    #[test]
    fn stops_scrolling_at_the_oldest_lines() {
        let mut app = app();
        app.focus = Pane::Log;
        // The 4 lines fit
        app.set_log_rows(10);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.log_scroll, 0);

        // 20 in 10 rows
        app.set_log_rows(10);
        for _ in 0..16 {
            let line = "2026-11-05T12:42:00+0000 1.1.1.1 TCP_TUNNEL/200 CONNECT a.com:443 dur=1ms in=1 out=1";
            app.apply(Event::Log(sndbx::proxy::LogLine::parse(line).unwrap()));
        }
        press(&mut app, KeyCode::Home);
        assert_eq!(app.log_scroll, 10);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.log_scroll, 10);
        // Moves right away
        press(&mut app, KeyCode::Down);
        assert_eq!(app.log_scroll, 9);

        // A taller pane, as when zoomed
        app.set_log_rows(15);
        assert_eq!(app.log_scroll, 5);
    }

    #[test]
    fn switches_views() {
        let mut app = app();
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.view, View::Images);
        assert_eq!(app.image_rows(), [ImageRow::Base]);
        assert_eq!(
            press(&mut app, KeyCode::Char('b')),
            Some(Action::Build {
                names: Vec::new(),
                rebuild_base: true
            })
        );
        assert_eq!(press(&mut app, KeyCode::Char('q')), Some(Action::Quit));
    }
}
