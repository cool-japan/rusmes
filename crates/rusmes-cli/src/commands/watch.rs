//! `--watch` mode: continuously redraw terminal output at a fixed interval.
//!
//! Callers pass in an async closure that renders one "frame" of output as a
//! `String`, a refresh interval in milliseconds, and an optional cancellation
//! receiver.  The runner clears the screen, prints the rendered frame, waits
//! for the interval, then repeats.  It exits cleanly on SIGINT (Ctrl-C) or
//! when the cancellation receiver fires.

use anyhow::Result;
use crossterm::{
    cursor,
    terminal::{self, ClearType},
    ExecutableCommand,
};
use std::io::Write;
use std::time::Duration;
use tokio::signal::ctrl_c;
use tokio::sync::oneshot;
use tokio::time::sleep;

/// Run `render_fn` in a loop, refreshing the terminal every `interval_ms`
/// milliseconds.  Returns as soon as Ctrl-C is received or the optional
/// `cancel` channel fires.
///
/// # Arguments
/// * `interval_ms` — refresh interval in milliseconds (clamped to ≥1 ms)
/// * `render_fn`   — async closure that returns a frame string or an error
/// * `cancel`      — optional oneshot receiver; when it fires the loop exits
pub async fn run_watch<F, Fut>(
    interval_ms: u64,
    mut render_fn: F,
    cancel: Option<oneshot::Receiver<()>>,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let interval = Duration::from_millis(interval_ms.max(1));
    let mut stdout = std::io::stdout();

    // Convert the optional cancel receiver into a fused future so we can
    // poll it repeatedly across iterations.  When `cancel` is `None` we use
    // a channel whose sender is immediately dropped, which makes the receiver
    // return `Err(RecvError)` immediately — we special-case that path below.
    enum CancelFut {
        Active(oneshot::Receiver<()>),
        /// No cancel channel was supplied; never resolves.
        Absent,
        /// The cancel signal has already been received.
        Fired,
    }

    let mut cancel_fut = match cancel {
        Some(rx) => CancelFut::Active(rx),
        None => CancelFut::Absent,
    };

    loop {
        // If cancellation already fired on a previous iteration, exit.
        if matches!(cancel_fut, CancelFut::Fired) {
            writeln!(stdout)?;
            break;
        }

        // Render the frame.
        let frame = render_fn().await?;

        // Clear terminal and move cursor to top-left.
        stdout.execute(terminal::Clear(ClearType::All))?;
        stdout.execute(cursor::MoveTo(0, 0))?;
        write!(stdout, "{frame}")?;
        stdout.flush()?;

        // Wait for: interval expiry, SIGINT, or cancel channel — whichever
        // comes first.
        match cancel_fut {
            CancelFut::Active(rx) => {
                // We need to be able to re-use `rx` if the sleep wins, so we
                // use a mutable reference inside the select.
                let mut rx = rx;
                tokio::select! {
                    _ = sleep(interval) => {
                        // Sleep won: put the receiver back and keep looping.
                        cancel_fut = CancelFut::Active(rx);
                    }
                    _ = ctrl_c() => {
                        writeln!(stdout)?;
                        // `rx` is dropped here — that's fine.
                        break;
                    }
                    result = &mut rx => {
                        // Cancel channel fired (or sender was dropped).
                        let _ = result;
                        cancel_fut = CancelFut::Fired;
                        continue;
                    }
                }
            }
            CancelFut::Absent => {
                tokio::select! {
                    _ = sleep(interval) => {
                        cancel_fut = CancelFut::Absent;
                    }
                    _ = ctrl_c() => {
                        writeln!(stdout)?;
                        break;
                    }
                }
            }
            CancelFut::Fired => {
                // Handled at the top of the loop; shouldn't reach here.
                writeln!(stdout)?;
                break;
            }
        }
    }

    Ok(())
}

/// Convenience wrapper for production use (no cancel channel, interval in
/// seconds).
///
/// # Arguments
/// * `interval_secs` — refresh interval in seconds (clamped to ≥1 s)
/// * `render_fn`     — async closure that returns a frame string or an error
pub async fn run_watch_secs<F, Fut>(interval_secs: u64, render_fn: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let ms = interval_secs.max(1).saturating_mul(1_000);
    run_watch(ms, render_fn, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that passing a pre-fired cancel oneshot to `run_watch` causes
    /// the loop to exit cleanly without panic or error.
    #[tokio::test]
    async fn test_watch_exits_on_signal() {
        let (tx, rx) = oneshot::channel::<()>();

        // Fire the cancel signal *before* starting the watch loop so the
        // receiver is already resolved when `tokio::select!` polls it.
        tx.send(()).expect("send should not fail");

        let result = run_watch(
            0, // 0 ms → clamped to 1 ms
            || async { Ok("frame".to_string()) },
            Some(rx),
        )
        .await;

        assert!(
            result.is_ok(),
            "watch loop should exit without error on cancel"
        );
    }
}
