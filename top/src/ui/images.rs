//! The configured and buildable images, and the selected one's settings

use jiff::Timestamp;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState, Wrap};
use sndbx::builder::BASE_NAME;
use sndbx::settings::Settings;

use super::dashboard::{highlight, marker};
use super::widgets::{egress, empty, home_relative, pane};
use crate::app::{App, ImageRow, Region, Target};
use crate::model::{Sandbox, age};

/// Returns where the table is
pub fn draw(frame: &mut Frame, area: Rect, app: &App, now: Timestamp) -> Vec<Region> {
    let Some(settings) = &app.snapshot.settings else {
        let text = app.snapshot.settings_error.clone().unwrap_or_default();
        frame.render_widget(empty(text).block(pane("Images", true)), area);
        return Vec::new();
    };
    let rows = app.image_rows();
    let height = u16::try_from(rows.len() + 3)
        .unwrap_or(u16::MAX)
        .min(area.height / 2);
    let [top, bottom] =
        Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(area);
    table(frame, top, app, settings, &rows, now);
    if let Some(row) = rows.get(app.image_index) {
        details(frame, bottom, app, settings, row, now);
    }
    // Below the border and the column names
    vec![
        Region {
            area: top,
            target: Target::Images,
            rows: Some((top.y + 2, usize::from(top.height.saturating_sub(3)))),
        },
        Region {
            area: bottom,
            target: Target::Images,
            rows: None,
        },
    ]
}

fn tag(settings: &Settings, app: &App, row: &ImageRow) -> String {
    match row {
        ImageRow::Configured(name) => app
            .configured_image(name)
            .map_or_else(|| settings.default_tag(name), |image| image.tag),
        row => settings.default_tag(row.name()),
    }
}

fn table(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    settings: &Settings,
    rows: &[ImageRow],
    now: Timestamp,
) {
    let base_created = app
        .snapshot
        .image(&settings.default_tag(BASE_NAME))
        .and_then(|info| info.created);
    let tags: Vec<String> = rows.iter().map(|row| tag(settings, app, row)).collect();
    let tag_width = tags
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(0)
        .clamp(12, 32);
    let table_rows = rows.iter().zip(&tags).map(|(row, tag)| {
        let info = app.snapshot.image(tag);
        let (size, built) = match info {
            Some(info) => {
                let stale = base_created
                    .zip(info.created)
                    .is_some_and(|(base, created)| created < base)
                    && *row != ImageRow::Base
                    && app.snapshot.buildable.iter().any(|name| name == row.name());
                let built = info
                    .created
                    .map(|created| age(created, now))
                    .unwrap_or_default();
                let built = if stale {
                    Line::from(vec![Span::from(built), " ⚠".yellow()])
                } else {
                    Line::from(built)
                };
                (info.size.clone(), built)
            }
            None => (String::new(), Line::from("not built".yellow())),
        };
        let (sandboxes, net, ports) = match row {
            ImageRow::Configured(name) => {
                let image = app.configured_image(name);
                let proxied =
                    settings.proxy().is_some() && image.as_ref().is_some_and(|image| image.proxy);
                let ports = image
                    .map(|image| image.ports.join(", "))
                    .filter(|ports| !ports.is_empty())
                    .unwrap_or_else(|| "—".to_string());
                (
                    sandbox_counts(&app.snapshot.sandboxes, name),
                    Line::from(egress(proxied)),
                    ports,
                )
            }
            ImageRow::Base => (String::new(), Line::default(), String::new()),
            ImageRow::Buildable(_) => (
                "not in settings".to_string(),
                Line::default(),
                String::new(),
            ),
        };
        let cells = Row::new([
            Cell::from(row.name().to_string()),
            Cell::from(tag.clone().dim()),
            Cell::from(size),
            Cell::from(built),
            Cell::from(sandboxes),
            Cell::from(net),
            Cell::from(ports),
        ]);
        if matches!(row, ImageRow::Buildable(_)) {
            cells.dim()
        } else {
            cells
        }
    });
    let widths = [
        Constraint::Length(9),
        Constraint::Length(u16::try_from(tag_width).unwrap_or(32)),
        Constraint::Length(6),
        Constraint::Length(9),
        Constraint::Length(16),
        Constraint::Length(6),
        Constraint::Fill(1),
    ];
    let header = Row::new(["NAME", "TAG", "SIZE", "BUILT", "SANDBOXES", "NET", "PORTS"])
        .style(Style::new().add_modifier(Modifier::DIM));
    let table = Table::new(table_rows, widths)
        .header(header)
        .block(pane("Images", true))
        .highlight_symbol(marker(true))
        .row_highlight_style(highlight(true));
    let mut state = TableState::new().with_selected(Some(app.image_index));
    frame.render_stateful_widget(table, area, &mut state);
}

fn sandbox_counts(sandboxes: &[Sandbox], image: &str) -> String {
    let (up, stopped) = sandboxes
        .iter()
        .filter(|sandbox| sandbox.image == image)
        .fold((0, 0), |(up, stopped), sandbox| {
            if sandbox.running() {
                (up + 1, stopped)
            } else {
                (up, stopped + 1)
            }
        });
    match (up, stopped) {
        (0, 0) => "—".to_string(),
        (up, 0) => format!("{up} up"),
        (0, stopped) => format!("{stopped} stopped"),
        (up, stopped) => format!("{up} up, {stopped} stopped"),
    }
}

fn details(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    settings: &Settings,
    row: &ImageRow,
    now: Timestamp,
) {
    let dockerfile = settings.images_dir().join(row.name()).join("Dockerfile");
    let source = if dockerfile.is_file() {
        home_relative(&dockerfile)
    } else {
        "—".to_string()
    };
    let mut lines = vec![field("source", source)];
    match row {
        ImageRow::Configured(name) => {
            if let Some(image) = app.configured_image(name) {
                lines.push(field(
                    "config",
                    format!(
                        "{} → ~/{}",
                        home_relative(&image.config_dir),
                        image.config_target
                    ),
                ));
                lines.push(field(
                    "user",
                    image
                        .user
                        .clone()
                        .unwrap_or_else(|| "the image's".to_string()),
                ));
                let args = if image.docker_args.is_empty() {
                    "—".to_string()
                } else {
                    image.docker_args.join(" ")
                };
                lines.push(field("docker_args", args));
            }
            let sandboxes: Vec<String> = app
                .snapshot
                .sandboxes
                .iter()
                .filter(|sandbox| sandbox.image == *name)
                .map(
                    |sandbox| match sandbox.started.filter(|_| sandbox.running()) {
                        Some(started) => {
                            format!("{} (running {})", sandbox.name, age(started, now))
                        }
                        None if sandbox.running() => format!("{} (running)", sandbox.name),
                        None => format!("{} (stopped)", sandbox.name),
                    },
                )
                .collect();
            let sandboxes = if sandboxes.is_empty() {
                "none".to_string()
            } else {
                sandboxes.join(", ")
            };
            lines.push(field("sandboxes", sandboxes));
        }
        ImageRow::Base => lines.push(field(
            "",
            "The images in images_dir are built on it; B rebuilds it before an image".to_string(),
        )),
        ImageRow::Buildable(_) => lines.push(field(
            "",
            "Not in the settings yet; b builds it and adds it".to_string(),
        )),
    }
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(pane(row.name(), false));
    frame.render_widget(paragraph, area);
}

fn field(name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::from(format!(" {name:<12} ")).dim(),
        Span::from(value),
    ])
}
