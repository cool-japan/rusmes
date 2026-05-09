//! Size and duration string parsers, plus shared default value functions.
//!
//! These are `pub(crate)` helpers used by the struct definitions in
//! `listeners.rs`, `runtime.rs`, and `env_overrides.rs`.

/// Parse a size string like `"50MB"`, `"1GB"`, `"1024KB"` into bytes.
pub(crate) fn parse_size(s: &str) -> anyhow::Result<usize> {
    let s = s.trim().to_uppercase();

    if let Some(rest) = s.strip_suffix("GB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0 * 1024.0 * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("MB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0 * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("KB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix('B') {
        let num: usize = rest.trim().parse()?;
        Ok(num)
    } else {
        // Assume bytes
        let num: usize = s.parse()?;
        Ok(num)
    }
}

/// Parse a duration string like `"60s"`, `"30m"`, `"1h"` into seconds.
pub(crate) fn parse_duration(s: &str) -> anyhow::Result<u64> {
    let s = s.trim().to_lowercase();

    if let Some(rest) = s.strip_suffix('h') {
        let num: u64 = rest.trim().parse()?;
        Ok(num * 3600)
    } else if let Some(rest) = s.strip_suffix('m') {
        let num: u64 = rest.trim().parse()?;
        Ok(num * 60)
    } else if let Some(rest) = s.strip_suffix('s') {
        let num: u64 = rest.trim().parse()?;
        Ok(num)
    } else {
        // Assume seconds
        let num: u64 = s.parse()?;
        Ok(num)
    }
}

// --- Shared serde default functions ---
// These are referenced by #[serde(default = "...")] attributes in both
// listeners.rs and runtime.rs, so they must be in a common location.

pub(crate) fn default_max_connections_per_ip() -> usize {
    10
}

pub(crate) fn default_max_total_connections() -> usize {
    1000
}

pub(crate) fn default_idle_timeout() -> String {
    "300s".to_string()
}

pub(crate) fn default_reaper_interval() -> String {
    "60s".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("50MB").unwrap(), 50 * 1024 * 1024);
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("60").unwrap(), 60);
        assert_eq!(parse_duration("60s").unwrap(), 60);
        assert_eq!(parse_duration("5m").unwrap(), 300);
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
    }
}
