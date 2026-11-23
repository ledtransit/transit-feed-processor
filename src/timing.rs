use std::future::Future;
use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use log::debug;

/// Logs `"<label>: <elapsed>"` at debug level when dropped
#[must_use = "bind the timer to a variable so it spans the measured action"]
pub struct Timer {
    label: String,
    started: Instant,
}

impl Timer {
    pub fn start(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            started: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        debug!("{}: {}", self.label, format_elapsed(self.started.elapsed()));
    }
}

const YELLOW_AT: Duration = Duration::from_secs(1);
const RED_AT: Duration = Duration::from_secs(10);

/// Formats an elapsed duration, colored green/yellow/red
pub fn format_elapsed(elapsed: Duration) -> String {
    let text = format!("{elapsed:.2?}");
    if !color_enabled() {
        return text;
    }
    let code = if elapsed >= RED_AT {
        "31" // red
    } else if elapsed >= YELLOW_AT {
        "33" // yellow
    } else {
        "32" // green
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn color_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal())
}

const DIVIDER: &str =
    "================================================================================";

pub async fn timed_phase<F: Future<Output = ()>>(label: &str, job: F) {
    println!("\n{DIVIDER}\n{label} — starting\n{DIVIDER}\n");
    let started = Instant::now();
    job.await;
    println!(
        "\n{DIVIDER}\n{label} — done in {}\n{DIVIDER}\n",
        format_elapsed(started.elapsed())
    );
}
