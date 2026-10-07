//! Downloading with a progress bar, resuming where an earlier try stopped.

use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::text::Line;

use super::screen::{Part, Wizard};
use crate::dashboard::format;
use crate::platform;

/// A bar for a fraction done: `██████████░░░░░░`.
pub fn bar(fraction: f64, width: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Bytes as `36 MB` or `8.97 GB`.
pub fn size(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= 1e9 { format!("{:.2} GB", bytes / 1e9) } else { format!("{:.0} MB", bytes / 1e6) }
}

/// How a download ended.
pub enum Fetched {
    /// The whole file is there.
    Done,
    /// The user quit; what arrived so far is kept for next time.
    Stopped,
}

/// Downloads `url` to `destination`, which should end up `expected` bytes.
///
/// `intro` is shown above the progress bar. A file already complete is not
/// fetched again.
pub fn fetch(
    wizard: &mut Wizard,
    intro: &[Part],
    url: &str,
    destination: &Path,
    expected: u64,
) -> Result<Fetched, String> {
    let have = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
    if have(destination) == expected {
        return Ok(Fetched::Done);
    }
    let mut curl = platform::start_download(url, destination)
        .map_err(|error| format!("cannot start downloading: {error}"))?;

    let started = Instant::now();
    let at_start = have(destination);
    let mut finished = None;
    let carry_on = wizard.watch("q stop (the download picks up where it left off next time)", || {
        if let Some(status) = curl.try_wait().map_err(|error| error.to_string())? {
            finished = Some(status);
            return Ok(None);
        }
        let now = have(destination);
        let elapsed = started.elapsed().as_secs_f64().max(0.5);
        let speed = now.saturating_sub(at_start) as f64 / elapsed;
        let left = if speed > 0.0 {
            format!("about {} left", format::duration(expected.saturating_sub(now) as f64 / speed))
        } else {
            "starting…".to_owned()
        };
        let mut body = intro.to_vec();
        body.push(Part::Gap);
        body.push(Part::Line(Line::raw(format!(
            "    {}  {:>3}%   {} of {}",
            bar(now as f64 / expected as f64, 28),
            (now * 100 / expected.max(1)).min(100),
            size(now),
            size(expected),
        ))));
        body.push(Part::Line(Line::raw(format!("    {:.1} MB/s · {left}", speed / 1e6))));
        Ok(Some(body))
    })?;

    if !carry_on {
        let _ = curl.kill();
        let _ = curl.wait();
        return Ok(Fetched::Stopped);
    }
    let status = finished.ok_or("the download ended without a status")?;
    if !status.success() {
        let mut said = String::new();
        if let Some(mut stderr) = curl.stderr.take() {
            let _ = stderr.read_to_string(&mut said);
        }
        return Err(format!("the download failed: {}", said.trim()));
    }
    if have(destination) != expected {
        return Err(format!(
            "the download is {} but should be {} — run again to resume it",
            size(have(destination)),
            size(expected)
        ));
    }
    // Give the bar a moment at 100%, so finishing is visible rather than abrupt.
    std::thread::sleep(Duration::from_millis(300));
    Ok(Fetched::Done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_and_sizes() {
        assert_eq!(bar(0.5, 8), "████░░░░");
        assert_eq!(bar(1.5, 4), "████");
        assert_eq!(size(37_285_825), "37 MB");
        assert_eq!(size(9_631_000_000), "9.63 GB");
    }
}
