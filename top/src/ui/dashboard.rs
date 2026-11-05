//! Sandboxes and the proxy's log

use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Cell, Paragraph, Row, Table, TableState};

use super::widgets::{
    ACCENT, RECEIVED, SENT, WORKDIR, egress, empty, home_relative, level, no_color, pane, percent,
};
use crate::app::{App, LogFilter, Pane, Region, Target};
use crate::model::{Sandbox, age, bytes};

/// The log's sandbox column
const CLIENT_WIDTH: usize = 12;
/// What the log keeps when many sandboxes take the rest
const LOG_MIN: u16 = 6;
/// The details' border, title and a line
const DETAILS_MIN: u16 = 3;
/// The details' field names
const FIELD_WIDTH: usize = 8;
/// The sandboxes' columns after the name and image: header, width, and whether right-aligned
const COLUMNS: [(&str, u16, bool); 8] = [
    ("STATE", 8, false),
    ("UP", 6, true),
    ("CPU", 6, true),
    ("MEM", 6, true),
    ("PIDS", 8, true),
    ("NET ↑", 5, true),
    ("NET ↓", 5, true),
    ("EGRESS", 6, false),
];
/// Longer names are cut
const NAME_MAX: usize = 24;
const HOST_MAX: usize = 30;
/// The hosts table's request, denied and received columns
const HOST_COLUMNS: [(&str, usize); 3] = [("REQ", 5), ("DENIED", 6), ("NET ↓", 6)];
/// Between the hosts table's columns, and between its side-by-side parts
const HOST_GAP: usize = 2;
const HOST_PART_GAP: usize = 4;

/// Returns where the panes are
pub fn draw(frame: &mut Frame, area: Rect, app: &App, now: Timestamp) -> Vec<Region> {
    if app.zoomed {
        return match app.focus {
            Pane::Sandboxes => zoomed_sandboxes(frame, area, app, now),
            Pane::Log => {
                log(frame, area, app);
                vec![pane_region(area, Pane::Log)]
            }
        };
    }
    let selected = app.selected_sandbox();
    let rows = u16::try_from(app.sandboxes().len().max(1)).unwrap_or(u16::MAX);
    let (table_height, details_height, log_height) = heights(area.height, rows, selected.is_some());
    let [table_area, details_area, log_area] = Layout::vertical([
        Constraint::Length(table_height),
        Constraint::Length(details_height),
        Constraint::Length(log_height),
    ])
    .areas(area);
    sandboxes(frame, table_area, app, now, details_height > 0);
    if let Some(sandbox) = selected.filter(|_| details_height > 0) {
        details(frame, details_area, app, sandbox);
    }
    log(frame, log_area, app);
    vec![
        table_region(table_area, details_height > 0),
        pane_region(details_area, Pane::Sandboxes),
        pane_region(log_area, Pane::Log),
    ]
}

fn pane_region(area: Rect, pane: Pane) -> Region {
    Region {
        area,
        target: Target::Pane(pane),
        rows: None,
    }
}

/// The sandboxes' rows are below the border and the column names, and above the bottom border unless the details
/// draw it
fn table_region(area: Rect, details_below: bool) -> Region {
    let bottom = u16::from(!details_below);
    Region {
        rows: Some((
            area.y + 2,
            usize::from(area.height.saturating_sub(2 + bottom)),
        )),
        ..pane_region(area, Pane::Sandboxes)
    }
}

/// The table, and the details below it with what's left
fn zoomed_sandboxes(frame: &mut Frame, area: Rect, app: &App, now: Timestamp) -> Vec<Region> {
    let rows = u16::try_from(app.sandboxes().len().max(1)).unwrap_or(u16::MAX);
    let table = rows + 2;
    match app.selected_sandbox() {
        Some(sandbox) if area.height >= table + DETAILS_MIN => {
            let [table_area, details_area] =
                Layout::vertical([Constraint::Length(table), Constraint::Fill(1)]).areas(area);
            sandboxes(frame, table_area, app, now, true);
            details(frame, details_area, app, sandbox);
            vec![
                table_region(table_area, true),
                pane_region(details_area, Pane::Sandboxes),
            ]
        }
        _ => {
            sandboxes(frame, area, app, now, false);
            vec![table_region(area, false)]
        }
    }
}

/// The table's, the details' and the log's heights in `height`. The log gets half at most, the table what its
/// `rows` need, up to all but `LOG_MIN`, and the details what's left, if that's `DETAILS_MIN` at least.
fn heights(height: u16, rows: u16, details: bool) -> (u16, u16, u16) {
    // The top border and the column names; the details' border is the table's bottom one
    let table = rows + 2;
    let top = (height - height / 2).max((table + 1).min(height.saturating_sub(LOG_MIN)));
    if details && top >= table + DETAILS_MIN {
        (table, top - table, height - top)
    } else {
        (top, 0, height - top)
    }
}

/// With the details below, which draw the bottom border
fn sandboxes(frame: &mut Frame, area: Rect, app: &App, now: Timestamp, details_below: bool) {
    let focused = app.focus == Pane::Sandboxes;
    let title = match &app.image_filter {
        Some(image) => format!("Sandboxes · {image}"),
        None => "Sandboxes".to_string(),
    };
    let mut block = pane(title, focused);
    if details_below {
        block = block.borders(Borders::TOP | Borders::LEFT | Borders::RIGHT);
    }
    let sandboxes = app.sandboxes();
    if sandboxes.is_empty() {
        let text = if !app.loaded {
            String::new()
        } else if let Some(error) = &app.snapshot.docker_error {
            error.clone()
        } else if let Some(image) = &app.image_filter {
            format!("No {image} sandboxes, esc shows all")
        } else {
            "No sandboxes, sndbx run starts one".to_string()
        };
        frame.render_widget(empty(text).block(block), area);
        return;
    }

    // The name as wide as the longest; the image takes the rest, which keeps the others at the right
    let longest = |texts: Vec<&str>, header: &str| {
        texts
            .into_iter()
            .chain([header])
            .map(|text| text.chars().count())
            .max()
            .unwrap_or_default()
    };
    let name_width = longest(
        sandboxes
            .iter()
            .map(|sandbox| sandbox.name.as_str())
            .collect(),
        "NAME",
    )
    .min(NAME_MAX);
    let image_width = longest(
        sandboxes
            .iter()
            .map(|sandbox| sandbox.image.as_str())
            .collect(),
        "IMAGE",
    );
    let to_u16 = |width: usize| u16::try_from(width).unwrap_or(u16::MAX);
    let mut widths = vec![
        Constraint::Length(to_u16(name_width)),
        Constraint::Min(to_u16(image_width)),
    ];
    widths.extend(
        COLUMNS
            .iter()
            .map(|(_, width, _)| Constraint::Length(*width)),
    );
    // Two spaces between the columns where they fit; the highlight symbol takes one
    let needed = 1
        + name_width
        + image_width
        + COLUMNS
            .iter()
            .map(|(_, width, _)| usize::from(*width))
            .sum::<usize>()
        + 2 * COLUMNS.len()
        + 2;
    let spacing = if usize::from(block.inner(area).width) >= needed {
        2
    } else {
        1
    };

    let mut header = vec![Cell::from("NAME"), Cell::from("IMAGE")];
    header.extend(
        COLUMNS
            .iter()
            .map(|(name, _, right)| aligned(*name, *right)),
    );
    let header = Row::new(header).style(Style::new().add_modifier(Modifier::DIM));
    let rows = sandboxes
        .iter()
        .map(|sandbox| sandbox_row(app, sandbox, now, name_width));
    let table = Table::new(rows, widths)
        .column_spacing(spacing)
        .header(header)
        .block(block)
        .highlight_symbol(marker(focused))
        .row_highlight_style(highlight(focused));
    let mut state = TableState::new().with_selected(Some(app.sandbox_index));
    frame.render_stateful_widget(table, area, &mut state);
}

fn aligned(text: impl Into<String>, right: bool) -> Cell<'static> {
    let line = Line::from(text.into());
    Cell::from(if right { line.right_aligned() } else { line })
}

fn sandbox_row<'a>(app: &App, sandbox: &Sandbox, now: Timestamp, name_width: usize) -> Row<'a> {
    let running = sandbox.running();
    let state = match sandbox.state.as_str() {
        "exited" => "stopped",
        state => state,
    };
    let up = sandbox
        .started
        .filter(|_| running)
        .map(|started| age(started, now))
        .unwrap_or_default();
    let usage = app.stats.get(&sandbox.name).filter(|_| running);
    let pids = usage.map(|usage| match sandbox.pids_limit {
        Some(limit) => format!("{}/{limit}", usage.pids),
        None => usage.pids.clone(),
    });
    let state_style = match state {
        "running" => Style::new().green(),
        "restarting" | "paused" | "removing" => Style::new().yellow(),
        "dead" => Style::new().red(),
        _ => Style::new(),
    };
    // Of the pids limit, which a runaway process count reaches
    let pids_used = usage.zip(sandbox.pids_limit).and_then(|(usage, limit)| {
        #[allow(clippy::cast_precision_loss)]
        usage
            .pids
            .parse::<f64>()
            .ok()
            .map(|pids| pids * 100.0 / limit as f64)
    });
    let text = |value: Option<&String>| value.cloned().unwrap_or_default();
    let cpu = usage.and_then(|usage| percent(&usage.cpu));
    let values = [
        (truncate(state, usize::from(COLUMNS[0].1)), state_style),
        (up, Style::new()),
        (
            // One decimal, which keeps 100% and more in the column
            cpu.map_or_else(
                || text(usage.map(|usage| &usage.cpu)),
                |cpu| format!("{cpu:.1}%"),
            ),
            level(cpu, 50.0, 100.0),
        ),
        (
            text(usage.map(|usage| &usage.memory)),
            level(
                usage.and_then(|usage| percent(&usage.memory_percent)),
                70.0,
                90.0,
            ),
        ),
        (pids.unwrap_or_default(), level(pids_used, 70.0, 90.0)),
        (text(usage.map(|usage| &usage.sent)), Style::new().fg(SENT)),
        (
            text(usage.map(|usage| &usage.received)),
            Style::new().fg(RECEIVED),
        ),
    ];
    let mut cells = vec![
        Cell::from(truncate(&sandbox.name, name_width))
            .style(Style::new().fg(app.sandbox_color(&sandbox.name))),
        Cell::from(sandbox.image.clone()),
    ];
    cells.extend(
        values
            .into_iter()
            .zip(COLUMNS)
            .map(|((value, style), (_, _, right))| aligned(value, right).style(style)),
    );
    // The last column, after the values
    cells.push(Cell::from(egress(sandbox.proxied())));
    let row = Row::new(cells);
    if running { row } else { row.dim() }
}

/// The selected sandbox's mounts, ports and the hosts it reached through the proxy
fn details(frame: &mut Frame, area: Rect, app: &App, sandbox: &Sandbox) {
    let mut title = vec![Span::from(sandbox.name.clone()).fg(app.sandbox_color(&sandbox.name))];
    if let Some(ip) = &sandbox.ip {
        title.push(Span::from(format!(" · {ip}")));
    }
    let title = Line::from(title);
    let block = pane(title, app.focus == Pane::Sandboxes).border_set(border::Set {
        top_left: "├",
        top_right: "┤",
        ..border::PLAIN
    });
    let width = usize::from(block.inner(area).width);

    let config_dir = app
        .configured_image(&sandbox.image)
        .map(|image| image.config_dir);
    let is_config = |source: &str| {
        config_dir
            .as_ref()
            .is_some_and(|dir| std::path::Path::new(source) == dir)
    };
    let mount = |mount: &crate::model::Mount| {
        let mut text = format!(
            "{} → {}",
            home_relative(std::path::Path::new(&mount.source)),
            mount.destination
        );
        if mount.destination == sandbox.workdir {
            text.push_str(" (workdir)");
            vec![Span::from(text).fg(WORKDIR)]
        } else {
            vec![Span::from(text)]
        }
    };

    let mounts: Vec<Vec<Span>> = sandbox
        .mounts
        .iter()
        .filter(|item| !is_config(&item.source))
        .map(mount)
        .collect();
    let mut lines = packed("mounts", mounts, ", ", width);
    if let Some(config) = sandbox.mounts.iter().find(|item| is_config(&item.source)) {
        lines.extend(packed("config", vec![mount(config)], "", width));
    }
    if !sandbox.ports.is_empty() {
        let ports = sandbox
            .ports
            .iter()
            .map(|port| vec![Span::from(port.clone()).yellow()])
            .collect();
        lines.extend(packed("ports", ports, ", ", width));
    }
    let room = usize::from(block.inner(area).height).saturating_sub(lines.len());
    match &sandbox.proxy_ip {
        _ if !sandbox.proxied() => {
            lines.extend(packed(
                "hosts",
                vec![vec!["not behind the proxy".dim()]],
                "",
                width,
            ));
        }
        None => lines.extend(packed("hosts", Vec::new(), "", width)),
        Some(_) => {
            let hosts = app.log.hosts_of(sandbox);
            if hosts.is_empty() {
                lines.extend(packed("hosts", vec![vec!["none yet".dim()]], "", width));
            } else {
                lines.extend(host_table(&hosts, width, room, app.zoomed));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The most asked for hosts on a line, as many as fit
fn host_summary(hosts: &[(&str, crate::model::Traffic)], width: usize) -> Vec<Line<'static>> {
    let items = hosts
        .iter()
        .map(|(host, traffic)| {
            let host = Span::from((*host).to_string());
            vec![
                if traffic.only_denied() {
                    host.red()
                } else {
                    host
                },
                format!(" {}", traffic.requests).dim(),
            ]
        })
        .collect();
    packed("hosts", items, " · ", width)
        .into_iter()
        .take(1)
        .collect()
}

/// A host's row in the hosts table
fn host_cells(host: &str, traffic: crate::model::Traffic, host_width: usize) -> Vec<Span<'static>> {
    let gap = " ".repeat(HOST_GAP);
    let [requests, denied, received] = HOST_COLUMNS.map(|(_, width)| width);
    let name = Span::from(format!("{:<host_width$}", truncate(host, host_width)));
    let denials = if traffic.denied > 0 {
        traffic.denied.to_string()
    } else {
        String::new()
    };
    vec![
        if traffic.only_denied() {
            name.red()
        } else {
            name
        },
        Span::from(format!("{gap}{:>requests$}", traffic.requests)),
        format!("{gap}{denials:>denied$}").red(),
        Span::from(format!("{gap}{:>received$}", bytes(traffic.bytes_out))).fg(RECEIVED),
    ]
}

/// The hosts with their requests, denials and bytes received, in as many parts side by side as `width` fits and
/// down `height` lines, the first the column names
fn host_table(
    hosts: &[(&str, crate::model::Traffic)],
    width: usize,
    height: usize,
    zoomed: bool,
) -> Vec<Line<'static>> {
    let label = || Span::from(format!(" {:<FIELD_WIDTH$} ", "hosts")).dim();
    let indent = FIELD_WIDTH + 2;
    let rows = height.saturating_sub(1);
    if rows == 0 {
        return host_summary(hosts, width);
    }

    let host_width = hosts
        .iter()
        .map(|(host, _)| host.chars().count())
        .max()
        .unwrap_or_default()
        .clamp(4, HOST_MAX);
    let part_width = host_width
        + HOST_COLUMNS
            .iter()
            .map(|(_, width)| HOST_GAP + width)
            .sum::<usize>();
    let fit =
        ((width.saturating_sub(indent) + HOST_PART_GAP) / (part_width + HOST_PART_GAP)).max(1);
    let capacity = rows * fit;
    // The last cell says how many more there are when they don't fit
    let (shown, more) = if hosts.len() > capacity {
        (capacity - 1, hosts.len() - capacity + 1)
    } else {
        (hosts.len(), 0)
    };
    let cells = shown + usize::from(more > 0);
    let parts = cells.div_ceil(rows).min(fit);
    let rows = cells.div_ceil(parts);

    let mut header = vec![label()];
    for part in 0..parts {
        if part > 0 {
            header.push(Span::from(" ".repeat(HOST_PART_GAP)));
        }
        let gap = " ".repeat(HOST_GAP);
        let text: String = std::iter::once(format!("{:<host_width$}", "HOST"))
            .chain(
                HOST_COLUMNS
                    .iter()
                    .map(|(name, column)| format!("{gap}{name:>column$}")),
            )
            .collect();
        header.push(text.dim());
    }
    let mut lines = vec![Line::from(header)];
    for row in 0..rows {
        let mut spans = vec![Span::from(" ".repeat(indent))];
        for part in 0..parts {
            let index = part * rows + row;
            if index >= cells {
                break;
            }
            if part > 0 {
                spans.push(Span::from(" ".repeat(HOST_PART_GAP)));
            }
            if index == shown {
                let hint = if zoomed { "" } else { ", z shows all" };
                spans.push(format!("… {more} more{hint}").dim());
                continue;
            }
            let (host, traffic) = hosts[index];
            spans.extend(host_cells(host, traffic, host_width));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// A field's items on as many lines of `width` as they take, the name on the first one only
fn packed(
    name: &str,
    items: Vec<Vec<Span<'static>>>,
    separator: &'static str,
    width: usize,
) -> Vec<Line<'static>> {
    let indent = FIELD_WIDTH + 2;
    let mut lines = Vec::new();
    let mut line = vec![Span::from(format!(" {name:<FIELD_WIDTH$} ")).dim()];
    let mut used = indent;
    let mut empty = true;
    let items = if items.is_empty() {
        vec![vec![Span::from("—")]]
    } else {
        items
    };
    for item in items {
        let item_width: usize = item.iter().map(Span::width).sum();
        if !empty && used + separator.chars().count() + item_width > width {
            lines.push(Line::from(std::mem::take(&mut line)));
            line.push(Span::from(" ".repeat(indent)));
            used = indent;
            empty = true;
        }
        if !empty {
            line.push(Span::from(separator).dim());
            used += separator.chars().count();
        }
        used += item_width;
        line.extend(item);
        empty = false;
    }
    lines.push(Line::from(line));
    lines
}

fn log(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Pane::Log;
    let filter = match app.log_filter {
        LogFilter::All => "all".to_string(),
        LogFilter::Denied => "denied".to_string(),
        LogFilter::Selected => app
            .selected_sandbox()
            .map_or_else(|| "selected".to_string(), |sandbox| sandbox.name.clone()),
    };
    // Inside the borders; as the app's limit, for a frame whose height changed since
    let lines = app.shown_lines();
    let height = usize::from(area.height.saturating_sub(2));
    let scroll = app.log_scroll.min(lines.len().saturating_sub(height));
    let scrolled = if scroll > 0 {
        format!(" · {scroll} up")
    } else {
        String::new()
    };
    let mut title = Line::from(format!("Proxy log · {filter}{scrolled}"));
    let denied = app.log.denied();
    if denied > 0 {
        title.push_span(" · ");
        title.push_span(Span::from(format!("{denied} denied")).red());
    }
    let block = pane(title, focused);

    if lines.is_empty() {
        let proxy_on = app
            .snapshot
            .settings
            .as_ref()
            .is_some_and(|settings| settings.proxy().is_some());
        let text = if !app.loaded {
            String::new()
        } else if !proxy_on && !app.snapshot.proxy_up {
            format!(
                "The proxy is off, set proxy in {} to turn it on",
                home_relative(&sndbx::settings::path())
            )
        } else if !app.snapshot.proxy_up {
            format!(
                "{} isn't running, sndbx run starts it",
                sndbx::proxy::CONTAINER
            )
        } else if app.log_filter == LogFilter::All {
            "No requests yet".to_string()
        } else {
            "No requests match, f changes the filter".to_string()
        };
        frame.render_widget(empty(text).block(block), area);
        return;
    }

    let end = lines.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    let zone = TimeZone::system();
    let items: Vec<Line> = lines[start..end]
        .iter()
        .map(|line| {
            let sandbox = app.snapshot.sandbox_for(line);
            let client = format!(
                "{:<CLIENT_WIDTH$}",
                truncate(
                    sandbox.map_or(line.client.as_str(), |sandbox| sandbox.name.as_str()),
                    CLIENT_WIDTH
                )
            );
            let time = line.time.to_zoned(zone.clone()).strftime("%T").to_string();
            let port = line.port.map(|port| format!(":{port}")).unwrap_or_default();
            let status = format!("{:03}", line.status);
            let host = if line.host.is_empty() {
                Span::from("—").dim()
            } else {
                Span::from(line.host.clone())
            };
            if line.denied() {
                return Line::from(format!("{time} {client} {status} {host}{port}")).red();
            }
            Line::from(vec![
                Span::from(time).dim(),
                " ".into(),
                match sandbox {
                    Some(sandbox) => Span::from(client).fg(app.sandbox_color(&sandbox.name)),
                    None => Span::from(client).dim(),
                },
                " ".into(),
                Span::from(status).style(status_style(line.status)),
                " ".into(),
                host,
                Span::from(port).dim(),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(items).block(block), area);
}

fn status_style(status: u16) -> Style {
    match status {
        200..=299 => Style::new().green(),
        300..=399 => Style::new().cyan(),
        400..=499 => Style::new().yellow(),
        500.. => Style::new().magenta(),
        _ => Style::new().dim(),
    }
}

/// A dark gray from the 256 colors, darker than the 16's bright black in most themes
const SELECTED: Color = Color::Indexed(238);

/// The selected row: in the focused pane on a dark gray background, which keeps the cells' colors, or reversed
/// without colors, where the background would be gone
pub fn highlight(focused: bool) -> Style {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    match (focused, no_color()) {
        (false, _) => bold,
        (true, false) => bold.bg(SELECTED),
        (true, true) => bold.add_modifier(Modifier::REVERSED),
    }
}

/// Before the selected row, in the accent color when the pane has the focus
pub fn marker(focused: bool) -> Span<'static> {
    let marker = Span::from("▶");
    if focused { marker.fg(ACCENT) } else { marker }
}

/// At most `width` characters, with `…` when cut
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_log_to_half() {
        // A few sandboxes: the details take what the log doesn't
        assert_eq!(heights(22, 2, true), (4, 7, 11));
        // None selected, so no details
        assert_eq!(heights(22, 1, false), (11, 0, 11));
        // Too many for details as well
        assert_eq!(heights(22, 7, true), (11, 0, 11));
        assert_eq!(heights(22, 6, true), (8, 3, 11));
        // The table takes all but the log's least
        assert_eq!(heights(22, 10, true), (13, 0, 9));
        assert_eq!(heights(22, 30, true), (16, 0, 6));
        assert_eq!(heights(23, 2, true), (4, 8, 11));
    }

    #[test]
    fn packs_items_on_lines() {
        let item = |text: &str| vec![Span::from(text.to_string())];
        let text =
            |lines: Vec<Line>| -> Vec<String> { lines.iter().map(ToString::to_string).collect() };
        assert_eq!(
            text(packed(
                "hosts",
                vec![item("a.com 1"), item("b.com 2"), item("c.com 3")],
                " · ",
                30
            )),
            [" hosts    a.com 1 · b.com 2", "          c.com 3"]
        );
        assert_eq!(text(packed("ports", Vec::new(), ", ", 30)), [" ports    —"]);
    }

    #[test]
    fn tables_hosts() {
        use crate::model::Traffic;
        let traffic = |requests, denied| Traffic {
            requests,
            denied,
            bytes_in: 0,
            bytes_out: requests * 1000,
        };
        let hosts = [
            ("a.com", traffic(30, 0)),
            ("b.com", traffic(20, 20)),
            ("c.com", traffic(10, 2)),
            ("d.com", traffic(5, 0)),
        ];
        let text =
            |lines: Vec<Line>| -> Vec<String> { lines.iter().map(ToString::to_string).collect() };

        assert_eq!(
            text(host_table(&hosts, 80, 5, false)),
            [
                " hosts    HOST     REQ  DENIED   NET ↓",
                "          a.com     30             30K",
                "          b.com     20      20     20K",
                "          c.com     10       2     10K",
                "          d.com      5            5.0K",
            ]
        );
        // Side by side where they fit, down first
        assert_eq!(
            text(host_table(&hosts, 80, 3, false)),
            [
                " hosts    HOST     REQ  DENIED   NET ↓    HOST     REQ  DENIED   NET ↓",
                "          a.com     30             30K    c.com     10       2     10K",
                "          b.com     20      20     20K    d.com      5            5.0K",
            ]
        );
        assert_eq!(
            text(host_table(&hosts, 40, 3, false)),
            [
                " hosts    HOST     REQ  DENIED   NET ↓",
                "          a.com     30             30K",
                "          … 3 more, z shows all",
            ]
        );
        assert_eq!(
            text(host_table(&hosts, 40, 1, false)),
            [" hosts    a.com 30 · b.com 20 · c.com 10"]
        );
    }
}
