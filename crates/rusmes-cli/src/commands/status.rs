//! Check server status

use anyhow::{Context, Result};
use std::fs;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

#[cfg(target_os = "linux")]
use std::io::Read;

const PID_FILE: &str = "./data/rusmes.pid";
const SMTP_DEFAULT_PORT: u16 = 25;
const IMAP_DEFAULT_PORT: u16 = 143;

/// Check server status
pub fn run() -> Result<()> {
    println!("Checking RusMES server status...");
    println!();

    // Check PID file
    let pid = check_pid_file()?;

    // Check if process is running
    let process_running = if let Some(pid) = pid {
        check_process_running(pid)?
    } else {
        false
    };

    // Check if ports are listening
    let smtp_listening = check_port_listening("127.0.0.1", SMTP_DEFAULT_PORT);
    let imap_listening = check_port_listening("127.0.0.1", IMAP_DEFAULT_PORT);

    // Display status
    println!("Server status:");
    if process_running {
        println!("  Status: RUNNING");
        if let Some(pid) = pid {
            println!("  PID: {}", pid);
        }
    } else if pid.is_some() {
        println!("  Status: STOPPED (stale PID file)");
    } else {
        println!("  Status: STOPPED");
    }

    println!();
    println!("Service status:");
    println!(
        "  SMTP (port {}): {}",
        SMTP_DEFAULT_PORT,
        if smtp_listening {
            "listening"
        } else {
            "not listening"
        }
    );
    println!(
        "  IMAP (port {}): {}",
        IMAP_DEFAULT_PORT,
        if imap_listening {
            "listening"
        } else {
            "not listening"
        }
    );

    // Get uptime if running
    if process_running {
        if let Some(pid_val) = pid {
            if let Ok(uptime) = get_process_uptime(pid_val) {
                println!();
                println!("Uptime: {}", format_uptime(uptime));
            }
        }
    }

    // Connection count (placeholder - would need actual implementation)
    if process_running {
        println!();
        println!("Active connections: N/A (not implemented)");
    }

    Ok(())
}

/// Check PID file and return PID if exists
fn check_pid_file() -> Result<Option<u32>> {
    let path = Path::new(PID_FILE);
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).context("Failed to read PID file")?;

    let pid: u32 = content.trim().parse().context("Invalid PID in PID file")?;

    Ok(Some(pid))
}

/// Check if process with given PID is running
fn check_process_running(_pid: u32) -> Result<bool> {
    // On Linux, check if /proc/PID exists
    #[cfg(target_os = "linux")]
    {
        let proc_path = format!("/proc/{}", _pid);
        Ok(Path::new(&proc_path).exists())
    }

    // On other platforms, use a different method
    #[cfg(not(target_os = "linux"))]
    {
        // Try to send signal 0 (null signal) to check if process exists
        // This is a placeholder - would need platform-specific implementation
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

        // Get system uptime
        let uptime_content =
            fs::read_to_string("/proc/uptime").context("Failed to read system uptime")?;
        let uptime_fields: Vec<&str> = uptime_content.split_whitespace().collect();
        let system_uptime: f64 = uptime_fields[0]
            .parse()
            .context("Failed to parse system uptime")?;

        // Get clock ticks per second
        let clock_ticks = 100; // Usually 100 on Linux

        // Calculate process uptime
        let start_time_seconds = start_time / clock_ticks;
        let current_time = system_uptime as u64;
        let process_uptime = current_time.saturating_sub(start_time_seconds);

        Ok(process_uptime)
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Placeholder for other platforms
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
}
