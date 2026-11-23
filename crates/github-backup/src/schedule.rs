// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `--every`: run a backup now and then again at a fixed interval, for
//! containers and services that should not depend on cron.

use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use crate::run::{EXIT_INCOMPLETE, EXIT_OK};
use crate::shutdown::Shutdown;
use crate::ui::format_duration;

/// The shortest interval accepted: anything shorter is a typo, and would
/// hammer the API.
const MIN_INTERVAL: Duration = Duration::from_secs(60);

/// Parses an interval such as `90s`, `30m`, `6h`, `24h` or `7d`.
///
/// # Errors
///
/// A message naming the accepted forms.
pub(crate) fn parse_interval(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let usage =
        || format!("'{text}' is not an interval; use a whole number with a unit: 30m, 6h, 24h, 7d");
    let split = text.find(|c: char| !c.is_ascii_digit()).ok_or_else(usage)?;
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().map_err(|_| usage())?;
    let unit_secs = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(usage()),
    };
    let interval = Duration::from_secs(number.checked_mul(unit_secs).ok_or_else(usage)?);
    if interval < MIN_INTERVAL {
        return Err(format!(
            "'{text}' is too short; the interval must be at least 1m"
        ));
    }
    Ok(interval)
}

/// Something that ends the loop: a shutdown signal, in production.
pub(crate) trait Stop {
    /// Waits for the stop request; returns its exit status.
    async fn recv(&mut self) -> u8;
}

impl Stop for Shutdown {
    async fn recv(&mut self) -> u8 {
        Shutdown::recv(self).await
    }
}

/// Runs `run_once` now and then every `interval` (measured from the start of
/// each run) until a stop request arrives while waiting, which returns `0`,
/// or a run is interrupted (its status `130`/`143` is returned).
///
/// A run that fails or is incomplete is logged and the loop carries on: the
/// next run retries it.  `run_once` gets the stop source so that it can
/// interrupt a run in progress.
pub(crate) async fn run_every<S: Stop>(
    interval: Duration,
    stop: &mut S,
    mut run_once: impl AsyncFnMut(&mut S) -> u8,
) -> u8 {
    info!(
        interval = %format_duration(interval.as_secs()),
        "scheduled mode: backing up now, then every {}",
        format_duration(interval.as_secs())
    );
    loop {
        let started = Instant::now();
        let code = run_once(stop).await;
        match code {
            EXIT_OK => info!("scheduled run finished successfully"),
            EXIT_INCOMPLETE => warn!("scheduled run finished incomplete; the next run retries"),
            130 | 143 => return code,
            _ => error!(
                exit_status = code,
                "scheduled run failed; the next run retries"
            ),
        }

        let wait = interval.saturating_sub(started.elapsed());
        let next = chrono::Local::now()
            + chrono::Duration::from_std(wait).unwrap_or_else(|_| chrono::Duration::zero());
        info!(
            next_run = %next.format("%Y-%m-%d %H:%M:%S %:z"),
            "next backup in {}",
            format_duration(wait.as_secs())
        );
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            _ = stop.recv() => {
                info!("stop requested while waiting for the next run; exiting");
                return EXIT_OK;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_with_units_parse() {
        assert_eq!(parse_interval("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_interval("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_interval("24h").unwrap(), Duration::from_secs(86_400));
        assert_eq!(
            parse_interval(" 7d ").unwrap(),
            Duration::from_secs(604_800)
        );
    }

    #[test]
    fn malformed_and_too_short_intervals_are_rejected() {
        for bad in [
            "",
            "24",
            "h",
            "1.5h",
            "-1h",
            "6 h",
            "6hours",
            "10x",
            "99999999999999999d",
        ] {
            assert!(parse_interval(bad).is_err(), "{bad:?}");
        }
        assert!(parse_interval("30s").unwrap_err().contains("at least 1m"));
    }

    /// A stop source fed by a channel; `recv` pends while nothing was sent.
    struct FakeStop(tokio::sync::mpsc::UnboundedReceiver<u8>);

    impl Stop for FakeStop {
        async fn recv(&mut self) -> u8 {
            match self.0.recv().await {
                Some(code) => code,
                None => std::future::pending().await,
            }
        }
    }

    #[tokio::test]
    async fn runs_repeat_and_a_stop_while_waiting_exits_0() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stop = FakeStop(rx);
        let mut runs = 0;
        let code = run_every(Duration::from_millis(20), &mut stop, async |_| {
            runs += 1;
            if runs == 3 {
                tx.send(143).unwrap();
            }
            // Failed and incomplete runs do not end the loop.
            [EXIT_OK, 1, EXIT_INCOMPLETE][runs % 3]
        })
        .await;
        assert_eq!(code, EXIT_OK);
        assert_eq!(runs, 3);
    }

    #[tokio::test]
    async fn an_interrupted_run_ends_the_loop_with_its_status() {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stop = FakeStop(rx);
        let mut runs = 0;
        let code = run_every(Duration::from_secs(3600), &mut stop, async |_| {
            runs += 1;
            130
        })
        .await;
        assert_eq!((code, runs), (130, 1));
    }

    #[tokio::test]
    async fn the_next_run_is_due_one_interval_after_the_previous_started() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut stop = FakeStop(rx);
        let mut starts = Vec::new();
        run_every(Duration::from_millis(200), &mut stop, async |_| {
            starts.push(Instant::now());
            // A run taking most of the interval must not push the next one
            // a whole interval further out.
            tokio::time::sleep(Duration::from_millis(150)).await;
            if starts.len() == 2 {
                tx.send(130).unwrap();
            }
            EXIT_OK
        })
        .await;
        let gap = starts[1] - starts[0];
        assert!(
            gap >= Duration::from_millis(190) && gap < Duration::from_millis(320),
            "{gap:?}"
        );
    }
}
