//! Check server status

use anyhow::{Context, Result};
use colored::*;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

#[cfg(target_os = "linux")]
use std::io::Read;

const SMTP_DEFAULT_PORT: u16 = 25;
const IMAP_DEFAULT_PORT: u16 = 143;
const METRICS_DEFAULT_PORT: u16 = 9090;

/// Serialisable status snapshot used for `--json` output.
#[derive(Debug, Serialize)]
pub struct ServerStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub smtp_listening: bool,
    pub imap_listening: bool,
    pub pid_file_path: String,
    pub status_message: String,
    pub active_connections: Option<HashMap<String, i64>>,
}

/// Query the metrics endpoint and extract active connection counts per protocol.
///
/// Returns `None` if the endpoint is unavailable (server not running, metrics
/// disabled, or network error).
fn fetch_active_connections(metrics_port: u16) -> Option<HashMap<String, i64>> {
    let url = format!("http://127.0.0.1:{}/metrics", metrics_port);
    let resp = reqwest::blocking::get(&url).ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().ok()?;
    let mut map = HashMap::new();
    for line in body.lines() {
        // Match lines like: rusmes_active_connections{protocol="smtp"} 2
        if line.starts_with("rusmes_active_connections{") {
            if let Some(rest) = line.strip_prefix("rusmes_active_connections{protocol=\"") {
                if let Some(end) = rest.find('"') {
                    let protocol = &rest[..end];
                    let after = &rest[end..];
                    if let Some(val_str) = after.split('}').nth(1) {
                        if let Ok(count) = val_str.trim().parse::<i64>() {
                            map.insert(protocol.to_string(), count);
                        }
                    }
                }
            }
        }
    }
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// Render a status frame as a human-readable `String` (used by `--watch`).
///
/// When `json` is `true`, the returned string is JSON-formatted.
pub fn render(runtime_dir: &str, json: bool) -> Result<String> {
    let pid_file = format!("{}/rusmes.pid", runtime_dir);
    let pid = check_pid_file(&pid_file)?;

    let process_running = if let Some(p) = pid {
        check_process_running(p)?
    } else {
        false
    };

    let smtp_listening = check_port_listening("127.0.0.1", SMTP_DEFAULT_PORT);
    let imap_listening = check_port_listening("127.0.0.1", IMAP_DEFAULT_PORT);

    let uptime_secs = if process_running {
        pid.and_then(|p| get_process_uptime(p).ok())
    } else {
        None
    };

    let status_message = if process_running {
        "RUNNING".to_string()
    } else if pid.is_some() {
        "STOPPED (stale PID file)".to_string()
    } else {
        "Server may not be running (PID file not found)".to_string()
    };

    let active_connections = if process_running {
        fetch_active_connections(METRICS_DEFAULT_PORT)
    } else {
        None
    };

    let snapshot = ServerStatus {
        running: process_running,
        pid,
        uptime_secs,
        smtp_listening,
        imap_listening,
        pid_file_path: pid_file.clone(),
        status_message: status_message.clone(),
        active_connections: active_connections.clone(),
    };

    if json {
        return Ok(serde_json::to_string_pretty(&snapshot)?);
    }

    // Human-readable rendering.
    let mut out = String::new();
    out.push_str("Checking RusMES server status...\n\n");
    out.push_str("Server status:\n");

    if process_running {
        out.push_str(&format!("  Status: {}\n", "RUNNING".green().bold()));
        if let Some(p) = pid {
            out.push_str(&format!("  PID: {}\n", p));
        }
    } else if pid.is_some() {
        out.push_str(&format!(
            "  Status: {}\n",
            "STOPPED (stale PID file)".yellow()
        ));
    } else {
        out.push_str(&format!(
            "  Status: {}\n",
            "Server may not be running (PID file not found)".yellow()
        ));
    }

    out.push_str(&format!("  PID file: {}\n", pid_file));
    out.push('\n');
    out.push_str("Service status:\n");
    out.push_str(&format!(
        "  SMTP (port {}): {}\n",
        SMTP_DEFAULT_PORT,
        if smtp_listening {
            "listening".green().to_string()
        } else {
            "not listening".red().to_string()
        }
    ));
    out.push_str(&format!(
        "  IMAP (port {}): {}\n",
        IMAP_DEFAULT_PORT,
        if imap_listening {
            "listening".green().to_string()
        } else {
            "not listening".red().to_string()
        }
    ));

    if let Some(uptime) = uptime_secs {
        out.push_str(&format!("\nUptime: {}\n", format_uptime(uptime)));
    }

    if process_running {
        match active_connections {
            Some(ref conns) => {
                out.push_str("\nActive connections:\n");
                let mut protocols: Vec<_> = conns.iter().collect();
                protocols.sort_by_key(|(k, _)| k.as_str());
                for (proto, count) in protocols {
                    out.push_str(&format!("  {}: {}\n", proto, count));
                }
            }
            None => {
                out.push_str(
                    "\nActive connections: unavailable (metrics endpoint not responding)\n",
                );
            }
        }
    }

    Ok(out)
}

/// Check server status and print to stdout.
pub fn run(runtime_dir: &str, json: bool) -> Result<()> {
    let output = render(runtime_dir, json)?;
    print!("{}", output);
    Ok(())
}

/// Check PID file and return PID if exists
fn check_pid_file(pid_file: &str) -> Result<Option<u32>> {
    let path = Path::new(pid_file);
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).context("Failed to read PID file")?;
    let pid: u32 = content.trim().parse().context("Invalid PID in PID file")?;
    Ok(Some(pid))
}

/// Check if process with given PID is running
fn check_process_running(_pid: u32) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        let proc_path = format!("/proc/{}", _pid);
        Ok(Path::new(&proc_path).exists())
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Signal 0 check is not portable without libc — return false conservatively.
        Ok(false)
    }
}

/// Check if a port is listening
fn check_port_listening(host: &str, port: u16) -> bool {
    let address = format!("{}:{}", host, port);
    address
        .parse()
        .ok()
        .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok())
        .unwrap_or(false)
}

/// Get process uptime in seconds
fn get_process_uptime(_pid: u32) -> Result<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat_path = format!("/proc/{}/stat", _pid);
        let mut file = fs::File::open(&stat_path).context("Failed to open process stat file")?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .context("Failed to read process stat file")?;

        // Parse stat file - start time is the 22nd field
        let fields: Vec<&str> = content.split_whitespace().collect();
        if fields.len() < 22 {
            anyhow::bail!("Invalid stat file format");
        }

        let start_time: u64 = fields[21]
            .parse()
            .context("Failed to parse process start time")?;

        let uptime_content =
            fs::read_to_string("/proc/uptime").context("Failed to read system uptime")?;
        let uptime_fields: Vec<&str> = uptime_content.split_whitespace().collect();
        let system_uptime: f64 = uptime_fields
            .first()
            .ok_or_else(|| anyhow::anyhow!("Empty /proc/uptime"))?
            .parse()
            .context("Failed to parse system uptime")?;

        // Clock ticks per second (usually 100 on Linux).
        let clock_ticks: u64 = 100;
        let start_time_seconds = start_time / clock_ticks;
        let current_time = system_uptime as u64;
        let process_uptime = current_time.saturating_sub(start_time_seconds);

        Ok(process_uptime)
    }

    #[cfg(not(target_os = "linux"))]
    {
        Ok(0)
    }
}

/// Format uptime in human-readable format
fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, minutes, secs)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_uptime_seconds() {
        let uptime = format_uptime(45);
        assert_eq!(uptime, "45s");
    }

    #[test]
    fn test_format_uptime_minutes() {
        let uptime = format_uptime(150);
        assert_eq!(uptime, "2m 30s");
    }

    #[test]
    fn test_format_uptime_hours() {
        let uptime = format_uptime(3665);
        assert_eq!(uptime, "1h 1m 5s");
    }

    #[test]
    fn test_format_uptime_days() {
        let uptime = format_uptime(90061);
        assert_eq!(uptime, "1d 1h 1m 1s");
    }

    #[test]
    fn test_format_uptime_zero() {
        let uptime = format_uptime(0);
        assert_eq!(uptime, "0s");
    }

    #[test]
    fn test_format_uptime_exact_minute() {
        let uptime = format_uptime(60);
        assert_eq!(uptime, "1m 0s");
    }

    #[test]
    fn test_format_uptime_exact_hour() {
        let uptime = format_uptime(3600);
        assert_eq!(uptime, "1h 0m 0s");
    }

    #[test]
    fn test_format_uptime_exact_day() {
        let uptime = format_uptime(86400);
        assert_eq!(uptime, "1d 0h 0m 0s");
    }

    #[test]
    fn test_format_uptime_multiple_days() {
        let uptime = format_uptime(259200); // 3 days
        assert_eq!(uptime, "3d 0h 0m 0s");
    }

    /// `status --json` should produce parseable JSON even when the server is
    /// not running.
    #[test]
    fn json_output_parses_as_json() {
        let tmp = std::env::temp_dir().join("rusmes_status_test_no_pid_dir");
        let dir_str = tmp.to_string_lossy().to_string();

        let output = render(&dir_str, true).expect("render should not error");
        let _: serde_json::Value =
            serde_json::from_str(&output).expect("status --json should produce parseable JSON");
    }

    /// When `NO_COLOR` is set, the text output should not contain ANSI escapes.
    #[test]
    fn color_disabled_when_no_color_env() {
        // Force color off for this test.
        colored::control::set_override(false);

        let tmp = std::env::temp_dir().join("rusmes_status_no_color_test");
        let dir_str = tmp.to_string_lossy().to_string();

        let output = render(&dir_str, false).expect("render should not error");

        // ANSI escape sequences start with ESC (\x1b).
        assert!(
            !output.contains('\x1b'),
            "output should not contain ANSI escapes when color is disabled"
        );

        // Restore so other tests are not affected.
        colored::control::unset_override();
    }
}
