//! `logs proxy --pretty`: the access log with sandbox names for addresses, local times, and readable durations and
//! sizes, in color on a terminal

use std::ops::RangeInclusive;
use std::process::ExitStatus;

use anyhow::{Context, bail};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::{
    DENIED_RESULT, DURATION_FIELD, DURATION_UNIT, HTTPS_PORT, LOG_TIME_FORMAT, RECEIVED_FIELD,
    SENT_FIELD, STATUS_SEPARATOR, TUNNEL_METHOD,
};
use crate::Result;
use crate::docker::{self, NetworkContainer};
use crate::pretty::{self, COLUMN_GAP, Color, LINE_END, Painter, TIME_FORMAT};

// The widest values the columns fit, wider ones push the rest along
const STATUS_WIDTH: usize = "error 503".len();
const DURATION_WIDTH: usize = "999999s".len();
// The widest IPv4 address, so addresses don't widen the column
const MIN_CLIENT_WIDTH: usize = "255.255.255.255".len();
const SIZE_WIDTH: usize = "1023KB".len();

const OK: &str = "ok";
const DENIED: &str = "denied";
const ERROR: &str = "error";
const ABORTED: &str = "aborted";
// HTTP statuses
const SUCCESS_CODES: RangeInclusive<u16> = 100..=399;
const ERROR_CODES: RangeInclusive<u16> = 400..=599;
// Squid's status for a request that ended before a reply
const NO_REPLY_CODE: u16 = 0;

const PORT_SEPARATOR: char = ':';
const MILLIS_PER_SECOND: f64 = 1000.0;
const SECOND_UNIT: &str = "s";
const SIZE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
const SIZE_STEP: f64 = 1024.0;
const SENT_ARROW: char = '↑';
const RECEIVED_ARROW: char = '↓';
// Numbers below it get decimal places
const DECIMALS_BELOW: f64 = 10.0;
const DECIMAL_PLACES: usize = 1;

// Docker's -t ends lines with \r\n, and keeps the terminal from returning the cursor at a \n while it runs
const CARRIAGE_RETURN: char = '\r';
const TERMINAL_LINE_END: &str = "\r\n";
// docker exec's status when Ctrl-C stopped the command, 128 + SIGINT
const INTERRUPTED_STATUS: i32 = 130;

/// Follows the log `command` prints in the proxy `container`, naming the sandboxes on `network`
pub fn follow(container: &str, command: &[&str], network: &str) -> Result<()> {
    let child = docker::spawn_until_interrupted(container, command)?;
    let mut formatter = Formatter::new(
        || docker::network_containers(network),
        TimeZone::system(),
        Painter::for_stdout(),
    );
    let line_end = if docker::exec_uses_terminal() {
        TERMINAL_LINE_END
    } else {
        LINE_END
    };
    pretty::follow(
        child,
        line_end,
        |line| formatter.format(line.trim_end_matches(CARRIAGE_RETURN)),
        "can't read the proxy's log from docker",
        |status| check_exit(status.context("can't wait for docker")?),
    )
}

/// Docker printed its own error
fn check_exit(status: ExitStatus) -> Result<()> {
    if !status.success() && status.code() != Some(INTERRUPTED_STATUS) {
        bail!("can't follow the proxy's log");
    }
    Ok(())
}

/// An access log line's fields
struct Request<'a> {
    time: Timestamp,
    client: &'a str,
    /// Squid's result and the HTTP status
    status: &'a str,
    result: &'a str,
    code: u16,
    method: &'a str,
    url: &'a str,
    duration_ms: u64,
    sent: u64,
    received: u64,
}

impl<'a> Request<'a> {
    /// `None` for a line in another format
    fn parse(line: &'a str) -> Option<Self> {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [time, client, status, method, url, duration, sent, received] = fields[..] else {
            return None;
        };
        let (result, code) = status.split_once(STATUS_SEPARATOR)?;
        Some(Self {
            time: Timestamp::strptime(LOG_TIME_FORMAT, time).ok()?,
            client,
            status,
            result,
            code: code.parse().ok()?,
            method,
            url,
            duration_ms: duration
                .strip_prefix(DURATION_FIELD)?
                .strip_suffix(DURATION_UNIT)?
                .parse()
                .ok()?,
            sent: sent.strip_prefix(SENT_FIELD)?.parse().ok()?,
            received: received.strip_prefix(RECEIVED_FIELD)?.parse().ok()?,
        })
    }

    /// What came of it
    fn outcome(&self) -> (String, Option<Color>) {
        let code = self.code;
        if self.result.starts_with(DENIED_RESULT) {
            (DENIED.to_string(), Some(Color::BrightRed))
        } else if SUCCESS_CODES.contains(&code) {
            (OK.to_string(), Some(Color::Green))
        } else if ERROR_CODES.contains(&code) {
            (format!("{ERROR} {code}"), Some(Color::Yellow))
        } else if code == NO_REPLY_CODE {
            (ABORTED.to_string(), Some(Color::Yellow))
        } else {
            (self.status.to_string(), None)
        }
    }
}

struct Formatter<F> {
    sandboxes: Sandboxes<F>,
    time_zone: TimeZone,
    painter: Painter,
    /// The widest client so far, so the columns after it line up
    client_width: usize,
}

impl<F: FnMut() -> Vec<NetworkContainer>> Formatter<F> {
    fn new(list_running: F, time_zone: TimeZone, painter: Painter) -> Self {
        Self {
            sandboxes: Sandboxes::new(list_running),
            time_zone,
            painter,
            client_width: MIN_CLIENT_WIDTH,
        }
    }

    /// The line as is if it's in another format
    fn format(&mut self, line: &str) -> String {
        match Request::parse(line) {
            Some(request) => self.columns(&request),
            None => line.to_string(),
        }
    }

    fn columns(&mut self, request: &Request) -> String {
        let (outcome, outcome_color) = request.outcome();
        let time = request.time.to_zoned(self.time_zone.clone());
        let duration = duration(request.duration_ms);
        [
            self.painter
                .paint(&time.strftime(TIME_FORMAT).to_string(), Some(Color::Dim)),
            self.client(request),
            self.painter
                .paint(&format!("{outcome:<STATUS_WIDTH$}"), outcome_color),
            self.painter
                .paint(&format!("{duration:>DURATION_WIDTH$}"), Some(Color::Dim)),
            self.painter.paint(
                &size_column(RECEIVED_ARROW, request.received),
                Some(Color::Cyan),
            ),
            self.painter
                .paint(&size_column(SENT_ARROW, request.sent), Some(Color::Magenta)),
            self.target(request),
        ]
        .join(COLUMN_GAP)
    }

    /// The host for HTTPS, with the port in color only if it isn't HTTPS's; the method and URL for HTTP
    fn target(&self, request: &Request) -> String {
        if request.method != TUNNEL_METHOD {
            return format!("{} {}", request.method, request.url);
        }
        match request.url.rsplit_once(PORT_SEPARATOR) {
            Some((host, port)) if port.parse() == Ok(HTTPS_PORT) => host.to_string(),
            Some((host, port)) => {
                let port = self
                    .painter
                    .paint(&format!("{PORT_SEPARATOR}{port}"), Some(Color::BrightRed));
                format!("{host}{port}")
            }
            None => request.url.to_string(),
        }
    }

    /// The sandbox's name, or the address if none had it then
    fn client(&mut self, request: &Request) -> String {
        let name = self.sandboxes.name(request.client, request.time);
        let (client, color) = match &name {
            Some(name) => (name.as_str(), Some(Color::BrightCyan)),
            None => (request.client, None),
        };
        self.client_width = self.client_width.max(client.chars().count());
        let width = self.client_width;
        self.painter.paint(&format!("{client:<width$}"), color)
    }
}

/// Seconds, with a decimal below 10
fn duration(ms: u64) -> String {
    format!("{}{SECOND_UNIT}", number(to_float(ms) / MILLIS_PER_SECOND))
}

fn size_column(arrow: char, bytes: u64) -> String {
    format!("{arrow}{:>SIZE_WIDTH$}", size(bytes))
}

/// In the largest unit that keeps it below 1024, with a decimal below 10
fn size(bytes: u64) -> String {
    let mut value = to_float(bytes);
    let mut unit = 0;
    // Rounded, as 1023.7KB would show as 1024KB
    while value.round() >= SIZE_STEP && unit + 1 < SIZE_UNITS.len() {
        value /= SIZE_STEP;
        unit += 1;
    }
    let number = if unit == 0 {
        bytes.to_string()
    } else {
        number(value)
    };
    format!("{number}{}", SIZE_UNITS[unit])
}

#[expect(
    clippy::cast_precision_loss,
    reason = "exact up to 2^53, far beyond the durations and sizes in the log"
)]
fn to_float(value: u64) -> f64 {
    value as f64
}

/// With decimal places below `DECIMALS_BELOW`, else whole
fn number(value: f64) -> String {
    let decimal = format!("{value:.DECIMAL_PLACES$}");
    // Rounding may reach it, like 9.96 to 10.0
    if decimal
        .parse::<f64>()
        .is_ok_and(|rounded| rounded < DECIMALS_BELOW)
    {
        decimal
    } else {
        format!("{value:.0}")
    }
}

/// The sandboxes seen on the proxy's network, to name a client by the one that had its address at the time, as
/// docker gives a stopped container's address to the next one. Times are in whole seconds, as the log's are.
struct Sandboxes<F> {
    list_running: F,
    seen: Vec<NetworkContainer>,
    /// The second the last listing covers
    listed: Option<i64>,
    /// The last second listed again for an unknown address
    relisted: Option<i64>,
}

impl<F: FnMut() -> Vec<NetworkContainer>> Sandboxes<F> {
    fn new(list_running: F) -> Self {
        Self {
            list_running,
            seen: Vec::new(),
            listed: None,
            relisted: None,
        }
    }

    /// Lists the running ones again for a request after the last listing
    fn name(&mut self, ip: &str, time: Timestamp) -> Option<String> {
        let second = time.as_second();
        if self.listed.is_none_or(|listed| second > listed) {
            self.refresh(second);
        }
        self.find(ip, second)
            .or_else(|| self.find_listing_again(ip, second))
    }

    /// Once a second, for a request in the second of the last listing, which a sandbox may have started in after it
    fn find_listing_again(&mut self, ip: &str, second: i64) -> Option<String> {
        if self.listed != Some(second) || self.relisted == Some(second) {
            return None;
        }
        self.relisted = Some(second);
        self.refresh(second);
        self.find(ip, second)
    }

    /// Covers the request's second too, as the proxy's clock may run ahead of the host's
    fn refresh(&mut self, second: i64) {
        self.listed = Some(second.max(Timestamp::now().as_second()));
        for container in (self.list_running)() {
            if !self.seen.contains(&container) {
                self.seen.push(container);
            }
        }
    }

    /// The latest started at or before the second
    fn find(&self, ip: &str, second: i64) -> Option<String> {
        self.seen
            .iter()
            .filter(|container| container.ip == ip && container.started.as_second() <= second)
            .max_by_key(|container| container.started)
            .map(|container| container.name.clone())
    }
}

#[cfg(test)]
mod tests;
