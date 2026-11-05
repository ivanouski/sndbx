//! What the keys do: in the terminal, with the dashboard suspended, or in the background

use std::io::Write;
use std::sync::mpsc::Sender;

use sndbx::builder::Builder;
use sndbx::docker;
use sndbx::removal;
use sndbx::settings::Settings;

use crate::app::Action;
use crate::sources::Event;

/// Bash if the image has it
const SHELL: &str = "command -v bash >/dev/null && exec bash; exec sh";

/// Runs an action that takes the terminal; the dashboard must be suspended
pub fn run_in_terminal(action: &Action) {
    match action {
        Action::Shell(name) => {
            println!("Opening a shell in {name}, exit returns to sndbx top");
            // Keeps an error, or what the shell's last command said, on the screen
            if !docker::run(&["exec", "-it", name, "sh", "-c", SHELL]) {
                wait_for_enter();
            }
        }
        Action::Resume(name) => {
            println!("Resuming {name}, it returns to sndbx top when it exits");
            if !docker::run(&["start", "-ai", name]) {
                wait_for_enter();
            }
        }
        Action::Build {
            names,
            rebuild_base,
        } => {
            let result = Settings::load_or_default()
                .and_then(|settings| Builder::new(&settings).build(names, *rebuild_base));
            report(result);
            wait_for_enter();
        }
        Action::RemoveImage(name) => {
            let result = Settings::load_or_default()
                .and_then(|settings| removal::remove(&settings, &[name.as_str()]));
            report(result);
            wait_for_enter();
        }
        _ => {}
    }
}

fn report(result: sndbx::Result<()>) {
    if let Err(e) = result {
        eprintln!("Error: {e:#}.");
    }
}

fn wait_for_enter() {
    print!("\nPress enter to return to sndbx top ");
    let _ = std::io::stdout().flush();
    let _ = std::io::stdin().read_line(&mut String::new());
}

/// What the footer says while a background action runs
pub fn progress(action: &Action) -> String {
    match action {
        Action::Stop(name) => format!("Stopping {name}…"),
        Action::Remove(name) => format!("Removing {name}…"),
        _ => String::new(),
    }
}

/// Runs an action in a thread that sends the outcome, then asks for a refresh
pub fn run_in_background(action: Action, events: Sender<Event>, refresh: Sender<()>) {
    std::thread::spawn(move || {
        sndbx::work_in_background();
        let result = match &action {
            // Docker's error comes with the kept output
            Action::Stop(name) => docker::run_muted(&["stop", name])
                .then(|| format!("Stopped {name}"))
                .ok_or_else(|| format!("can't stop {name}")),
            Action::Remove(name) => docker::run_muted(&["rm", "-f", name])
                .then(|| format!("Removed {name}"))
                .ok_or_else(|| format!("can't remove {name}")),
            _ => Ok(String::new()),
        };
        let kept = sndbx::take_kept_output();
        let result = result.map_err(|error| with_details(error, &kept));
        let _ = events.send(Event::Done(result));
        let _ = refresh.send(());
    });
}

/// The error on one line, with what docker printed meanwhile
fn with_details(error: String, kept: &str) -> String {
    let details: Vec<&str> = kept
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if details.is_empty() {
        error
    } else {
        format!("{error}: {}", details.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_details_on_one_line() {
        let kept = "Error response from daemon: cannot stop container: a\n\n  permission denied\n";
        assert_eq!(
            with_details("can't stop a".to_string(), kept),
            "can't stop a: Error response from daemon: cannot stop container: a · permission denied"
        );
        assert_eq!(with_details("failed".to_string(), "\n"), "failed");
    }
}
