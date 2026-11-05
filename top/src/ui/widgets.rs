//! The header, footer and what the views share

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use sndbx::settings::Allow;

use super::{MIN_HEIGHT, MIN_WIDTH};
use crate::app::{App, Pane, Region, Target, View};

pub const ACCENT: Color = Color::Cyan;
/// Bytes sent and received
pub const SENT: Color = Color::Magenta;
pub const RECEIVED: Color = Color::Blue;
/// The workdir among the mounts
pub const WORKDIR: Color = Color::Magenta;

/// Yellow from `warn`, red from `alert`
pub fn level(value: Option<f64>, warn: f64, alert: f64) -> Style {
    match value {
        Some(value) if value >= alert => Style::new().red(),
        Some(value) if value >= warn => Style::new().yellow(),
        _ => Style::new(),
    }
}

/// Docker's `4.10%`
pub fn percent(text: &str) -> Option<f64> {
    text.trim().trim_end_matches('%').parse().ok()
}

/// Whether a sandbox goes through the proxy, which an open one doesn't
pub fn egress(proxied: bool) -> Span<'static> {
    if proxied {
        "proxy".green()
    } else {
        "open".yellow()
    }
}

/// Whether `NO_COLOR` asks for no colors
pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

/// Without colors when `NO_COLOR` is set
pub fn strip_colors(frame: &mut Frame) {
    if no_color() {
        without_colors(frame.buffer_mut());
    }
}

/// Keeps bold, dim and reversed, which still show the selection
pub fn without_colors(buffer: &mut ratatui::buffer::Buffer) {
    for cell in &mut buffer.content {
        cell.set_fg(Color::Reset).set_bg(Color::Reset);
    }
}

pub fn too_small(frame: &mut Frame, area: Rect) {
    let text = format!(
        "terminal too small: {}x{}, sndbx top needs {MIN_WIDTH}x{MIN_HEIGHT}",
        area.width, area.height
    );
    let [middle] = Layout::vertical([Constraint::Length(1)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), middle);
}

/// A pane's border and title, highlighted when it has the focus
pub fn pane<'a>(title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    let mut title: Line = title.into();
    title.spans.insert(0, Span::from(" "));
    title.spans.push(Span::from(" "));
    let block = Block::bordered().title(title);
    if focused {
        block
            .border_style(Style::new().fg(ACCENT))
            .title_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
    } else {
        block
    }
}

/// The views, then docker's, the proxy's and the settings' state; returns where the views' tabs are
pub fn header(frame: &mut Frame, area: Rect, app: &App) -> Vec<Region> {
    let tab = |key: &str, name: &str, view: View| {
        let label = format!("{key} {name}");
        if app.view == view {
            Span::from(format!("[{label}]")).bold()
        } else {
            Span::from(format!(" {label} ")).dim()
        }
    };
    let tabs = [
        (tab("1", "Dashboard", View::Dashboard), View::Dashboard),
        (tab("2", "Images", View::Images), View::Images),
    ];
    let mut left = Line::from(vec![" sndbx ".bold()]);
    let mut regions = Vec::new();
    for (span, view) in tabs {
        left.spans.push(" ".into());
        let x = area.x + u16::try_from(left.width()).unwrap_or(u16::MAX);
        let width = u16::try_from(span.width()).unwrap_or(u16::MAX);
        regions.push(Region {
            area: Rect::new(x, area.y, width, 1),
            target: Target::Tab(view),
            rows: None,
        });
        left.spans.push(span);
    }

    let snapshot = &app.snapshot;
    let mut right: Vec<Span> = Vec::new();
    let mut part = |spans: Vec<Span<'static>>| {
        if !right.is_empty() {
            right.push(" │ ".dim());
        }
        right.extend(spans);
    };
    if app.loaded {
        if snapshot.settings_error.is_some() {
            part(vec!["settings ".into(), "✗".red()]);
        }
        if snapshot.docker_error.is_some() {
            part(vec!["docker ".into(), "✗".red()]);
        } else {
            part(vec!["docker ".into(), "✓".green()]);
        }
        let allow = snapshot
            .settings
            .as_ref()
            .and_then(sndbx::settings::Settings::proxy)
            .map(|proxy| proxy.allow);
        match allow {
            None => part(vec!["proxy off".dim()]),
            Some(allow) => {
                let mode = match allow {
                    Allow::Listed => "allows listed",
                    Allow::All => "allows all",
                };
                let state = if snapshot.proxy_up {
                    "✓".green()
                } else {
                    "✗".red()
                };
                let mode = Span::from(format!(" {mode}"));
                let mode = if allow == Allow::All {
                    mode.yellow()
                } else {
                    mode
                };
                part(vec!["proxy ".into(), state, mode]);
            }
        }
    } else {
        part(vec!["reading docker…".dim()]);
    }
    right.push(" ".into());

    let [left_area, right_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(Line::from(right.clone()).width()).unwrap_or(u16::MAX)),
    ])
    .areas(area);
    frame.render_widget(left, left_area);
    frame.render_widget(Line::from(right), right_area);
    regions
}

/// A question, a message, or the keys
pub fn footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = if let Some(confirm) = &app.confirm {
        Line::from(vec![
            " ".into(),
            confirm.prompt.clone().yellow().bold(),
            " y/n".into(),
        ])
    } else if let Some(message) = &app.message {
        let text = Span::from(message.text.clone());
        Line::from(vec![
            " ".into(),
            if message.error {
                text.red()
            } else {
                text.green()
            },
        ])
    } else if let Some(error) = &app.snapshot.settings_error {
        Line::from(vec![" ".into(), error.clone().red()])
    } else {
        keys(&key_hints(app))
    };
    frame.render_widget(line, area);
}

fn key_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut hints = vec![("tab", "focus")];
    match (app.view, app.focus) {
        (View::Images, _) => {
            let mut hints = Vec::new();
            if app.selected_image().is_some_and(|row| app.buildable(&row)) {
                hints.extend([("b", "build"), ("B", "rebuild with base")]);
            }
            hints.extend([
                ("d", "remove"),
                ("enter", "show sandboxes"),
                ("1", "dashboard"),
                ("q", "quit"),
            ]);
            return hints;
        }
        (View::Dashboard, Pane::Sandboxes) => match app.selected_sandbox() {
            Some(sandbox) if sandbox.running() => {
                hints.extend([("enter", "shell"), ("s", "stop"), ("d", "remove")]);
            }
            Some(sandbox) if sandbox.state == "exited" => {
                hints.extend([("r", "resume"), ("d", "remove")]);
            }
            Some(_) => hints.push(("d", "remove")),
            None => {}
        },
        (View::Dashboard, Pane::Log) => hints.extend([("↑↓", "scroll"), ("G", "newest")]),
    }
    hints.extend([("f", "filter"), ("z", "zoom")]);
    if app.image_filter.is_some() {
        hints.push(("esc", "all"));
    }
    hints.push(("q", "quit"));
    hints
}

fn keys(hints: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut spans = vec![Span::from(" ")];
    for (key, action) in hints {
        spans.push(Span::from(*key).fg(ACCENT).bold());
        spans.push(Span::from(format!(" {action}  ")));
    }
    Line::from(spans)
}

/// With `~` for the home directory
pub fn home_relative(path: &Path) -> String {
    let home = sndbx::paths::home();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A placeholder for an empty pane
pub fn empty(text: impl Into<String>) -> Paragraph<'static> {
    Paragraph::new(Line::from(text.into()).dim()).wrap(ratatui::widgets::Wrap { trim: true })
}
