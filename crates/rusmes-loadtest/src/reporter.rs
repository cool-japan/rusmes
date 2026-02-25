//! Report generation for load test results

use crate::metrics::LoadTestMetrics;
use anyhow::Result;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Report generator
pub struct Reporter;

impl Reporter {
    /// Generate JSON report
    pub fn generate_json(metrics: &LoadTestMetrics, path: &Path) -> Result<()> {
        let report = JsonReport::from_metrics(metrics);
        let json = serde_json::to_string_pretty(&report)?;

        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;

        Ok(())
    }

    /// Generate CSV report
    pub fn generate_csv(metrics: &LoadTestMetrics, path: &Path) -> Result<()> {
        let mut file = File::create(path)?;

        writeln!(file, "metric,value")?;
        writeln!(file, "total_requests,{}", metrics.total_requests)?;
        writeln!(file, "successful_requests,{}", metrics.successful_requests)?;
        writeln!(file, "failed_requests,{}", metrics.failed_requests)?;
        writeln!(file, "success_rate,{:.4}", metrics.success_rate())?;
        writeln!(
            file,
            "requests_per_second,{:.2}",
            metrics.requests_per_second()
        )?;
        writeln!(file, "bytes_sent,{}", metrics.bytes_sent)?;
        writeln!(file, "bytes_received,{}", metrics.bytes_received)?;

        if let Some(duration) = metrics.duration() {
            writeln!(file, "duration_secs,{:.2}", duration.as_secs_f64())?;
        }

        let stats = metrics.latency_stats();
        writeln!(
            file,
            "latency_min_ms,{:.2}",
            stats.min.as_secs_f64() * 1000.0
        )?;
        writeln!(
            file,
            "latency_max_ms,{:.2}",
            stats.max.as_secs_f64() * 1000.0
        )?;
        writeln!(
            file,
            "latency_mean_ms,{:.2}",
            stats.mean.as_secs_f64() * 1000.0
        )?;
        writeln!(
            file,
            "latency_p50_ms,{:.2}",
            stats.p50.as_secs_f64() * 1000.0
        )?;
        writeln!(
            file,
            "latency_p95_ms,{:.2}",
            stats.p95.as_secs_f64() * 1000.0
        )?;
        writeln!(
            file,
            "latency_p99_ms,{:.2}",
            stats.p99.as_secs_f64() * 1000.0
        )?;
        writeln!(
            file,
            "latency_p999_ms,{:.2}",
            stats.p999.as_secs_f64() * 1000.0
        )?;

        Ok(())
    }

    /// Generate HTML report
    pub fn generate_html(metrics: &LoadTestMetrics, path: &Path) -> Result<()> {
        let stats = metrics.latency_stats();
        let duration = metrics
            .duration()
            .map(|d| format!("{:.2}s", d.as_secs_f64()))
            .unwrap_or_else(|| "N/A".to_string());

        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Load Test Report</title>
    <style>
        body {{
            font-family: Arial, sans-serif;
            margin: 20px;
            background-color: #f5f5f5;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
            background-color: white;
            padding: 20px;
            border-radius: 5px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }}
        h1 {{
            color: #333;
            border-bottom: 2px solid #4CAF50;
            padding-bottom: 10px;
        }}
        h2 {{
            color: #555;
            margin-top: 30px;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
            margin: 20px 0;
        }}
        th, td {{
            padding: 12px;
            text-align: left;
            border-bottom: 1px solid #ddd;
        }}
        th {{
            background-color: #4CAF50;
            color: white;
        }}
        tr:hover {{
            background-color: #f5f5f5;
        }}
        .success {{
            color: #4CAF50;
        }}
        .error {{
            color: #f44336;
        }}
        .metric-value {{
            font-weight: bold;
            font-size: 1.2em;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Load Test Report</h1>

        <h2>Summary</h2>
        <table>
            <tr>
                <th>Metric</th>
                <th>Value</th>
            </tr>
            <tr>
                <td>Duration</td>
                <td class="metric-value">{}</td>
            </tr>
            <tr>
                <td>Total Requests</td>
                <td class="metric-value">{}</td>
            </tr>
            <tr>
                <td>Successful Requests</td>
                <td class="metric-value success">{}</td>
            </tr>
            <tr>
                <td>Failed Requests</td>
                <td class="metric-value error">{}</td>
            </tr>
            <tr>
                <td>Success Rate</td>
                <td class="metric-value">{:.2}%</td>
            </tr>
            <tr>
                <td>Throughput</td>
                <td class="metric-value">{:.2} req/s</td>
            </tr>
        </table>

        <h2>Data Transfer</h2>
        <table>
            <tr>
                <th>Metric</th>
                <th>Value</th>
            </tr>
            <tr>
                <td>Bytes Sent</td>
                <td class="metric-value">{} bytes ({:.2} MB)</td>
            </tr>
            <tr>
                <td>Bytes Received</td>
                <td class="metric-value">{} bytes ({:.2} MB)</td>
            </tr>
        </table>

        <h2>Latency Statistics</h2>
        <table>
            <tr>
                <th>Percentile</th>
                <th>Latency</th>
            </tr>
            <tr>
                <td>Minimum</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
            <tr>
                <td>Mean</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
            <tr>
                <td>Maximum</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
            <tr>
                <td>p50</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
            <tr>
                <td>p95</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
            <tr>
                <td>p99</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
            <tr>
                <td>p99.9</td>
                <td class="metric-value">{:.2}ms</td>
            </tr>
        </table>

        <h2>Errors</h2>
        <p>Total Errors: {}</p>
        {}
    </div>
</body>
</html>"#,
            duration,
            metrics.total_requests,
            metrics.successful_requests,
            metrics.failed_requests,
            metrics.success_rate() * 100.0,
            metrics.requests_per_second(),
            metrics.bytes_sent,
            metrics.bytes_sent as f64 / 1_000_000.0,
            metrics.bytes_received,
            metrics.bytes_received as f64 / 1_000_000.0,
            stats.min.as_secs_f64() * 1000.0,
            stats.mean.as_secs_f64() * 1000.0,
            stats.max.as_secs_f64() * 1000.0,
            stats.p50.as_secs_f64() * 1000.0,
            stats.p95.as_secs_f64() * 1000.0,
            stats.p99.as_secs_f64() * 1000.0,
            stats.p999.as_secs_f64() * 1000.0,
            metrics.errors.len(),
            if metrics.errors.is_empty() {
                "<p>No errors occurred during the test.</p>".to_string()
            } else {
                let mut error_html = String::from("<ul>");
                for error in metrics.errors.iter().take(20) {
                    error_html.push_str(&format!("<li>{}</li>", error));
                }
                error_html.push_str("</ul>");
                error_html
            }
        );

        let mut file = File::create(path)?;
        file.write_all(html.as_bytes())?;

        Ok(())
    }

    /// Generate Prometheus metrics format
    pub fn generate_prometheus_metrics(metrics: &LoadTestMetrics) -> String {
        let stats = metrics.latency_stats();

        format!(
            "# HELP loadtest_total_requests Total number of requests\n\
             # TYPE loadtest_total_requests counter\n\
             loadtest_total_requests {}\n\
             # HELP loadtest_successful_requests Number of successful requests\n\
             # TYPE loadtest_successful_requests counter\n\
             loadtest_successful_requests {}\n\
             # HELP loadtest_failed_requests Number of failed requests\n\
             # TYPE loadtest_failed_requests counter\n\
             loadtest_failed_requests {}\n\
             # HELP loadtest_success_rate Success rate (0.0-1.0)\n\
             # TYPE loadtest_success_rate gauge\n\
             loadtest_success_rate {:.4}\n\
             # HELP loadtest_requests_per_second Request throughput\n\
             # TYPE loadtest_requests_per_second gauge\n\
             loadtest_requests_per_second {:.2}\n\
             # HELP loadtest_bytes_sent Total bytes sent\n\
             # TYPE loadtest_bytes_sent counter\n\
             loadtest_bytes_sent {}\n\
             # HELP loadtest_bytes_received Total bytes received\n\
             # TYPE loadtest_bytes_received counter\n\
             loadtest_bytes_received {}\n\
             # HELP loadtest_latency_seconds Latency in seconds\n\
             # TYPE loadtest_latency_seconds summary\n\
             loadtest_latency_seconds{{quantile=\"0.5\"}} {:.6}\n\
             loadtest_latency_seconds{{quantile=\"0.95\"}} {:.6}\n\
             loadtest_latency_seconds{{quantile=\"0.99\"}} {:.6}\n\
             loadtest_latency_seconds{{quantile=\"0.999\"}} {:.6}\n\
             loadtest_latency_seconds_sum {:.6}\n\
             loadtest_latency_seconds_count {}\n",
            metrics.total_requests,
            metrics.successful_requests,
            metrics.failed_requests,
            metrics.success_rate(),
            metrics.requests_per_second(),
            metrics.bytes_sent,
            metrics.bytes_received,
            stats.p50.as_secs_f64(),
            stats.p95.as_secs_f64(),
            stats.p99.as_secs_f64(),
            stats.p999.as_secs_f64(),
            stats.mean.as_secs_f64() * metrics.successful_requests as f64,
            metrics.successful_requests,
        )
    }
}

/// JSON report structure
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct JsonReport {
    duration_secs: f64,
    total_requests: u64,
    successful_requests: u64,
    failed_requests: u64,
    success_rate: f64,
    requests_per_second: f64,
    bytes_sent: u64,
    bytes_received: u64,
    latency: JsonLatencyStats,
    errors: Vec<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct JsonLatencyStats {
    min_ms: f64,
    max_ms: f64,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    p999_ms: f64,
}

impl JsonReport {
    fn from_metrics(metrics: &LoadTestMetrics) -> Self {
        let stats = metrics.latency_stats();

        Self {
            duration_secs: metrics.duration().map(|d| d.as_secs_f64()).unwrap_or(0.0),
            total_requests: metrics.total_requests,
            successful_requests: metrics.successful_requests,
            failed_requests: metrics.failed_requests,
            success_rate: metrics.success_rate(),
            requests_per_second: metrics.requests_per_second(),
            bytes_sent: metrics.bytes_sent,
            bytes_received: metrics.bytes_received,
            latency: JsonLatencyStats {
                min_ms: stats.min.as_secs_f64() * 1000.0,
                max_ms: stats.max.as_secs_f64() * 1000.0,
                mean_ms: stats.mean.as_secs_f64() * 1000.0,
                p50_ms: stats.p50.as_secs_f64() * 1000.0,
                p95_ms: stats.p95.as_secs_f64() * 1000.0,
                p99_ms: stats.p99.as_secs_f64() * 1000.0,
                p999_ms: stats.p999.as_secs_f64() * 1000.0,
            },
            errors: metrics.errors.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn test_json_report_generation() {
        let temp_dir = TempDir::new().unwrap();
        let report_path = temp_dir.path().join("report.json");

        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);
        metrics.record_success(Duration::from_millis(20), 100, 50);

        let result = Reporter::generate_json(&metrics, &report_path);
        assert!(result.is_ok());
        assert!(report_path.exists());

        let content = std::fs::read_to_string(&report_path).unwrap();
        assert!(content.contains("total_requests"));
        assert!(content.contains("latency"));
    }

    #[test]
    fn test_csv_report_generation() {
        let temp_dir = TempDir::new().unwrap();
        let report_path = temp_dir.path().join("report.csv");

        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);

        let result = Reporter::generate_csv(&metrics, &report_path);
        assert!(result.is_ok());
        assert!(report_path.exists());

        let content = std::fs::read_to_string(&report_path).unwrap();
        assert!(content.contains("metric,value"));
        assert!(content.contains("total_requests"));
    }

    #[test]
    fn test_html_report_generation() {
        let temp_dir = TempDir::new().unwrap();
        let report_path = temp_dir.path().join("report.html");

        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);

        let result = Reporter::generate_html(&metrics, &report_path);
        assert!(result.is_ok());
        assert!(report_path.exists());

        let content = std::fs::read_to_string(&report_path).unwrap();
        assert!(content.contains("<html>"));
        assert!(content.contains("Load Test Report"));
    }

    #[test]
    fn test_prometheus_metrics_generation() {
        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);
        metrics.record_success(Duration::from_millis(20), 100, 50);

        let prometheus = Reporter::generate_prometheus_metrics(&metrics);
        assert!(prometheus.contains("loadtest_total_requests"));
        assert!(prometheus.contains("loadtest_latency_seconds"));
        assert!(prometheus.contains("quantile"));
    }
}
