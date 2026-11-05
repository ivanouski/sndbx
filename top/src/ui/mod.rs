//! Draws the views

mod dashboard;
mod images;
mod widgets;

use jiff::Timestamp;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::{App, Region, View};

/// The smallest terminal the views fit
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 20;

/// Returns where things are, for the mouse
pub fn draw(frame: &mut Frame, app: &App, now: Timestamp) -> Vec<Region> {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        widgets::too_small(frame, area);
        return Vec::new();
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let mut regions = widgets::header(frame, header, app);
    regions.extend(match app.view {
        View::Dashboard => dashboard::draw(frame, body, app, now),
        View::Images => images::draw(frame, body, app, now),
    });
    widgets::footer(frame, footer, app);
    widgets::strip_colors(frame);
    regions
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    use super::*;
    use crate::model::{Sandbox, Snapshot, Stats};
    use crate::sources::Event;

    /// The color of the first cell of `text` in the buffer
    fn color_of(buffer: &Buffer, text: &str) -> Color {
        let width = usize::from(buffer.area.width);
        let symbols: Vec<&str> = buffer
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        let first = text.chars().next().unwrap().to_string();
        let start = (0..symbols.len())
            .find(|&index| {
                symbols[index] == first
                    && text.chars().enumerate().all(|(offset, c)| {
                        symbols.get(index + offset) == Some(&c.to_string().as_str())
                    })
                    && index % width + text.chars().count() <= width
            })
            .unwrap_or_else(|| panic!("{text} isn't drawn"));
        buffer.content[start].fg
    }

    #[test]
    fn colors_what_matters() {
        let mut app = App::new(View::Dashboard);
        let sandbox = |name: &str, state: &str, network: &str| Sandbox {
            name: name.to_string(),
            image: "claude".to_string(),
            state: state.to_string(),
            started: None,
            workdir: String::new(),
            mounts: Vec::new(),
            ports: Vec::new(),
            networks: vec![network.to_string()],
            proxy_ip: None,
            ip: None,
            pids_limit: Some(100),
        };
        app.apply(Event::Snapshot(Snapshot {
            sandboxes: vec![
                sandbox("busy", "running", sndbx::proxy::NETWORK),
                sandbox("wide", "restarting", "bridge"),
            ],
            ..Snapshot::default()
        }));
        let stats = Stats {
            cpu: "120.00%".to_string(),
            memory: "612M".to_string(),
            memory_percent: "75.00%".to_string(),
            pids: "95".to_string(),
            received: "48M".to_string(),
            sent: "1.2M".to_string(),
        };
        app.apply(Event::Stats([("busy".to_string(), stats)].into()));

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &app, Timestamp::now());
            })
            .unwrap();
        let mut buffer = terminal.backend().buffer().clone();

        // In the focused, selected row, whose background is dark gray
        assert_eq!(color_of(&buffer, "running"), Color::Green);
        let selected = buffer
            .content
            .iter()
            .position(|cell| cell.symbol() == "▶")
            .unwrap();
        assert_eq!(buffer.content[selected].fg, widgets::ACCENT);
        assert_eq!(buffer.content[selected + 1].bg, Color::Indexed(238));
        assert_eq!(color_of(&buffer, "restart…"), Color::Yellow);
        assert_eq!(color_of(&buffer, "120.0%"), Color::Red);
        assert_eq!(color_of(&buffer, "612M"), Color::Yellow);
        assert_eq!(color_of(&buffer, "95/100"), Color::Red);
        assert_eq!(color_of(&buffer, "1.2M"), widgets::SENT);
        assert_eq!(color_of(&buffer, "48M"), widgets::RECEIVED);
        assert_eq!(color_of(&buffer, "open"), Color::Yellow);
        assert_eq!(color_of(&buffer, "wide"), app.sandbox_color("wide"));
        assert_ne!(app.sandbox_color("wide"), app.sandbox_color("busy"));

        widgets::without_colors(&mut buffer);
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
    }

    /// The buffer's text, a line per row
    fn text(buffer: &Buffer) -> String {
        buffer
            .content
            .chunks(usize::from(buffer.area.width))
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    #[test]
    fn says_when_the_terminal_is_too_small() {
        let app = App::new(View::Dashboard);
        let mut terminal = Terminal::new(TestBackend::new(79, 30)).unwrap();
        let mut regions = None;
        terminal
            .draw(|frame| regions = Some(draw(frame, &app, Timestamp::now())))
            .unwrap();

        let text = text(terminal.backend().buffer());
        assert!(
            text.contains("terminal too small: 79x30, sndbx top needs 80x20"),
            "{text}"
        );
        assert_eq!(regions, Some(Vec::new()));
    }

    #[test]
    fn draws_the_log_within_its_lines() {
        let mut app = App::new(View::Dashboard);
        for (result, host) in [
            ("TCP_TUNNEL/200", "a.com"),
            ("TCP_DENIED/403", "b.com"),
            ("TCP_TUNNEL/200", "c.com"),
        ] {
            let line = format!(
                "2026-11-05T12:41:03+0000 1.1.1.1 {result} CONNECT {host}:443 dur=1ms in=1 out=1"
            );
            app.apply(Event::Log(sndbx::proxy::LogLine::parse(&line).unwrap()));
        }
        // Scrolled up in a shorter pane, before the app knows this one's height
        app.log_scroll = 2;

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &app, Timestamp::now());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = text(buffer);

        // All three fit, so it isn't scrolled
        assert!(text.contains("Proxy log · all · 1 denied"), "{text}");
        assert!(!text.contains(" up"), "{text}");
        for host in ["a.com", "b.com", "c.com"] {
            assert!(text.contains(host), "{host} isn't drawn");
        }
        assert_eq!(color_of(buffer, "1 denied"), Color::Red);
    }
}
