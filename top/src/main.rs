//! sndbx top: a terminal dashboard of the sandboxes, the proxy's requests and the images

mod actions;
mod app;
mod model;
mod sources;
mod ui;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Parser;
use jiff::Timestamp;
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::Cell;
use ratatui::crossterm::event::{self, Event as TerminalEvent, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::{DefaultTerminal, Terminal};
use sndbx::Result;

use crate::app::{Action, App, Pane, Region, Target, View};
use crate::sources::{Event, Sources};
use crate::ui::{MIN_HEIGHT, MIN_WIDTH};

/// How long to wait for a key before drawing again, and at most before quitting on a signal
const FRAME: Duration = Duration::from_millis(250);

/// The signal that asked the dashboard to quit, 0 before one did
static QUIT_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[derive(Parser)]
#[command(
    name = "sndbx top",
    bin_name = "sndbx top",
    version,
    about = "Shows the sandboxes, the proxy's requests and the images, and acts on them.",
    override_usage = "sndbx top [options]"
)]
struct Cli {
    /// The view to start in
    #[arg(long, value_enum, default_value = "dashboard", value_name = "VIEW")]
    view: View,

    /// Print a frame as text and exit, for scripts and tests
    #[arg(long)]
    once: bool,

    /// The frame's size with --once
    #[arg(
        long,
        value_name = "WIDTHxHEIGHT",
        default_value = "88x24",
        value_parser = size,
        requires = "once"
    )]
    size: Size,
}

#[derive(Clone, Copy)]
struct Size {
    width: u16,
    height: u16,
}

/// At least the smallest the views fit, as a smaller frame only says the terminal is too small
fn size(value: &str) -> Result<Size, String> {
    let invalid = || format!("expected a size like 88x24, of {MIN_WIDTH}x{MIN_HEIGHT} at least");
    let (width, height) = value.split_once('x').ok_or_else(invalid)?;
    let number = |text: &str, min: u16| text.parse().ok().filter(|number| *number >= min);
    Ok(Size {
        width: number(width, MIN_WIDTH).ok_or_else(invalid)?,
        height: number(height, MIN_HEIGHT).ok_or_else(invalid)?,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    sndbx::exit_code(sndbx::paths::check_home().and_then(|()| {
        if cli.once {
            once(cli.view, cli.size)
        } else {
            interactive(cli.view)
        }
    }))
}

/// Reads everything once and prints the frame without styles
fn once(view: View, size: Size) -> Result<()> {
    let mut app = App::new(view);
    app.apply(Event::Snapshot(sources::snapshot()));
    let running = running(&app.snapshot);
    if view == View::Dashboard && !running.is_empty() {
        app.apply(Event::Stats(sources::stats(&running)));
    }
    for line in sources::log_lines(app.snapshot.proxy_up) {
        app.apply(Event::Log(line));
    }

    let mut terminal = Terminal::new(TestBackend::new(size.width, size.height))?;
    terminal.draw(|frame| {
        ui::draw(frame, &app, Timestamp::now());
    })?;
    for row in terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(size.width))
    {
        let line: String = row.iter().map(Cell::symbol).collect();
        sndbx::print_line(line.trim_end());
    }
    Ok(())
}

fn running(snapshot: &model::Snapshot) -> Vec<String> {
    snapshot
        .sandboxes
        .iter()
        .filter(|sandbox| sandbox.running())
        .map(|sandbox| sandbox.name.clone())
        .collect()
}

fn interactive(view: View) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("sndbx top needs a terminal, --once prints a frame instead");
    }
    handle_signals();
    let sources = sources::start();
    let mut terminal = ratatui::try_init().context("can't set up the terminal")?;
    // After ratatui's hook, so the mouse goes back first
    let restore = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        capture_mouse(false);
        restore(info);
    }));
    capture_mouse(true);
    let result = Dashboard {
        terminal: &mut terminal,
        sources: &sources,
        app: App::new(view),
        regions: Vec::new(),
    }
    .run();
    if quit_signal() == Some(libc::SIGHUP) {
        // The terminal is gone: there's nothing to restore, and writing to it or to stderr fails, which panics in
        // eprintln, as in the terminal's drop
        sources.stop();
        std::process::exit(0);
    }
    capture_mouse(false);
    let _ = ratatui::try_restore();
    sources.stop();
    // What failed once a signal came, like reading the terminal, is the quitting
    if quit_signal().is_some() {
        return Ok(());
    }
    result
}

/// The app on the terminal, with what the sources send
struct Dashboard<'a> {
    terminal: &'a mut DefaultTerminal,
    sources: &'a Sources,
    app: App,
    /// What the last frame drew where, for the mouse
    regions: Vec<Region>,
}

impl Dashboard<'_> {
    /// Until a key or a signal quits
    fn run(&mut self) -> Result<()> {
        while quit_signal().is_none() {
            self.apply_events();
            self.draw()?;
            match self.next_action()? {
                Some(Action::Quit) => return Ok(()),
                Some(action) => self.perform(action)?,
                None => {}
            }
        }
        Ok(())
    }

    /// What the sources sent since the last frame
    fn apply_events(&mut self) {
        while let Ok(event) = self.sources.events.try_recv() {
            let snapshot = matches!(event, Event::Snapshot(_));
            self.app.apply(event);
            if snapshot {
                self.sources.set_running(running(&self.app.snapshot));
            }
        }
        self.sources.show_stats(self.app.view == View::Dashboard);
    }

    fn draw(&mut self) -> Result<()> {
        self.app.tick();
        self.terminal
            .draw(|frame| self.regions = ui::draw(frame, &self.app, Timestamp::now()))?;
        // Inside the borders
        if let Some(log) = self
            .regions
            .iter()
            .find(|region| region.target == Target::Pane(Pane::Log))
        {
            self.app
                .set_log_rows(usize::from(log.area.height.saturating_sub(2)));
        }
        Ok(())
    }

    /// What a key pressed before the next frame asks for; clicks and the wheel act right away
    fn next_action(&mut self) -> Result<Option<Action>> {
        if !event::poll(FRAME)? {
            return Ok(None);
        }
        Ok(match event::read()? {
            TerminalEvent::Key(key) if key.kind == KeyEventKind::Press => self.app.key(key),
            TerminalEvent::Mouse(mouse) => {
                self.app.mouse(mouse, &self.regions);
                None
            }
            _ => None,
        })
    }

    fn perform(&mut self, action: Action) -> Result<()> {
        if action.suspends() {
            self.suspend(|| actions::run_in_terminal(&action))?;
            let _ = self.sources.refresh.send(());
        } else {
            self.app.say(actions::progress(&action));
            let sources = self.sources;
            actions::run_in_background(action, sources.sender.clone(), sources.refresh.clone());
        }
        Ok(())
    }

    /// Gives the terminal to `run`, then takes it back, unless a signal came meanwhile to quit
    fn suspend(&mut self, run: impl FnOnce()) -> Result<()> {
        capture_mouse(false);
        // Drawing hides it, and leaving the alternate screen doesn't show it
        self.terminal.show_cursor()?;
        ratatui::restore();
        run();
        if quit_signal().is_some() {
            return Ok(());
        }
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        capture_mouse(true);
        redraw(self.terminal)
    }
}

/// Has the terminal report clicks and the wheel, in SGR's format, which crossterm reads; not every move, which
/// crossterm's `EnableMouseCapture` asks for too
fn capture_mouse(capture: bool) {
    use std::io::Write;
    let sequence = if capture {
        "\x1b[?1000h\x1b[?1006h"
    } else {
        "\x1b[?1006l\x1b[?1000l"
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(sequence.as_bytes());
    let _ = stdout.flush();
}

/// Clears the screen and has the next frame draw every cell; unlike `Terminal::clear`, without asking the terminal
/// where the cursor is, which a terminal can be slow to answer
fn redraw(terminal: &mut DefaultTerminal) -> Result<()> {
    terminal.backend_mut().clear()?;
    // Resets the buffer with what's on the screen, the other one is empty between frames
    terminal.swap_buffers();
    Ok(())
}

/// Ctrl-C is a key while the dashboard runs; while a command has the terminal, it stops the command but not the
/// dashboard. SIGTERM and SIGHUP quit, cleaning up like q: the main loop sees them within a frame, or once a
/// command that has the terminal ends. Handlers rather than ignoring the signals, as commands get the default
/// actions back.
fn handle_signals() {
    extern "C" fn ignore(_signal: libc::c_int) {}
    extern "C" fn quit(signal: libc::c_int) {
        QUIT_SIGNAL.store(signal, Ordering::Relaxed);
    }
    // SAFETY: the handlers do nothing or store to an atomic, which is async-signal-safe
    unsafe {
        libc::signal(libc::SIGINT, ignore as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, quit as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, quit as *const () as libc::sighandler_t);
    }
}

/// The signal that asked to quit, if one did
fn quit_signal() -> Option<libc::c_int> {
    Some(QUIT_SIGNAL.load(Ordering::Relaxed)).filter(|signal| *signal != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        let parsed = size("88x24").unwrap();
        assert_eq!((parsed.width, parsed.height), (88, 24));
        let smallest = size("80x20").unwrap();
        assert_eq!((smallest.width, smallest.height), (80, 20));
        for invalid in [
            "0x24", "88x0", "79x24", "88x19", "88", "x24", "88x-1", "axb",
        ] {
            assert!(size(invalid).is_err(), "{invalid}");
        }
    }
}
