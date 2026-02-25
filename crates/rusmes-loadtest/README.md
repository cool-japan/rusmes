# RusMES Load Testing Tool

Comprehensive load testing tool for RusMES mail server, supporting multiple protocols and workload patterns.

## Features

- **Multi-Protocol Support**: SMTP, IMAP, JMAP, POP3, and mixed workloads
- **Flexible Workload Patterns**: Steady load, spike tests, ramp-up, stress tests, wave patterns
- **Detailed Metrics**: HDR Histogram-based latency tracking (p50, p95, p99, p99.9)
- **Multiple Report Formats**: JSON, HTML, CSV, and Prometheus metrics export
- **Realistic Message Generation**: Configurable message sizes and content types
- **Concurrent Testing**: Support for high concurrency scenarios

## Installation

```bash
cargo install --path .
```

## Quick Start

### Basic SMTP Load Test

```bash
rusmes-loadtest \
  --host localhost \
  --port 25 \
  --protocol smtp \
  --rate 100 \
  --concurrency 10 \
  --duration 60
```

### High-Throughput Test

```bash
rusmes-loadtest \
  --host localhost \
  --port 25 \
  --protocol smtp \
  --rate 10000 \
  --concurrency 1000 \
  --duration 300 \
  --ramp-up 30 \
  --output-json results.json \
  --output-html report.html
```

### Mixed Protocol Test

```bash
rusmes-loadtest \
  --protocol mixed \
  --smtp-weight 70 \
  --imap-weight 20 \
  --jmap-weight 10 \
  --rate 5000 \
  --concurrency 500 \
  --duration 300
```

## Command-Line Options

### Core Options

- `-H, --host <HOST>`: Target host (default: localhost)
- `-p, --port <PORT>`: Target port (default: 25)
- `--protocol <PROTOCOL>`: Protocol to test (smtp, imap, jmap, pop3, mixed)
- `-s, --scenario <SCENARIO>`: Test scenario (smtp-throughput, concurrent-connections, mixed-protocol, sustained-load)
- `-d, --duration <SECONDS>`: Test duration in seconds (default: 60)
- `-c, --concurrency <NUM>`: Number of concurrent workers (default: 10)
- `-r, --rate <RATE>`: Target message rate in msg/s (default: 100)

### Advanced Options

- `--ramp-up <SECONDS>`: Gradual ramp-up duration (default: 0)
- `--min-size <BYTES>`: Minimum message size (default: 1024)
- `--max-size <BYTES>`: Maximum message size (default: 102400)
- `--content <TYPE>`: Message content type (random, template, real-world)

### Report Options

- `--output-json <PATH>`: Generate JSON report
- `--output-html <PATH>`: Generate HTML report
- `--output-csv <PATH>`: Generate CSV report
- `--prometheus`: Output Prometheus metrics format
- `--prometheus-port <PORT>`: Prometheus export port (default: 9090)

### Mixed Protocol Weights

- `--smtp-weight <0-100>`: SMTP percentage (default: 70)
- `--imap-weight <0-100>`: IMAP percentage (default: 20)
- `--jmap-weight <0-100>`: JMAP percentage (default: 10)
- `--pop3-weight <0-100>`: POP3 percentage (default: 0)

## Test Scenarios

### 1. Throughput Test

Tests maximum message processing rate:

```bash
rusmes-loadtest \
  --scenario smtp-throughput \
  --rate 50000 \
  --concurrency 1000 \
  --duration 300
```

### 2. Concurrent Connections

Tests server connection handling:

```bash
rusmes-loadtest \
  --scenario concurrent-connections \
  --concurrency 10000 \
  --duration 60
```

### 3. Sustained Load

Long-running test for stability:

```bash
rusmes-loadtest \
  --scenario sustained-load \
  --rate 10000 \
  --duration 86400  # 24 hours
```

### 4. Spike Test

Sudden traffic increase:

```bash
rusmes-loadtest \
  --rate 100000 \
  --concurrency 5000 \
  --duration 60 \
  --ramp-up 5
```

## Report Formats

### JSON Report

Machine-readable format for automation:

```json
{
  "duration_secs": 60.5,
  "total_requests": 6000,
  "successful_requests": 5980,
  "failed_requests": 20,
  "success_rate": 0.9967,
  "requests_per_second": 99.17,
  "latency": {
    "min_ms": 1.2,
    "mean_ms": 15.3,
    "max_ms": 125.7,
    "p50_ms": 12.1,
    "p95_ms": 45.2,
    "p99_ms": 78.9,
    "p999_ms": 110.3
  }
}
```

### HTML Report

Human-readable visual report with charts and tables.

### CSV Report

Spreadsheet-compatible format:

```csv
metric,value
total_requests,6000
successful_requests,5980
latency_p99_ms,78.9
```

### Prometheus Metrics

Compatible with Prometheus monitoring:

```
loadtest_total_requests 6000
loadtest_successful_requests 5980
loadtest_latency_seconds{quantile="0.99"} 0.0789
```

## Performance Metrics

The tool tracks:

- **Throughput**: Requests per second
- **Latency**: Min, max, mean, p50, p95, p99, p99.9
- **Success Rate**: Percentage of successful requests
- **Data Transfer**: Bytes sent and received
- **Error Details**: First 100 unique errors

## Example Test Plans

### Development Testing

```bash
# Quick smoke test
rusmes-loadtest --rate 10 --duration 10

# Standard dev test
rusmes-loadtest --rate 100 --duration 60
```

### Staging Environment

```bash
# Capacity test
rusmes-loadtest --rate 5000 --concurrency 500 --duration 300

# Stress test
rusmes-loadtest --rate 10000 --concurrency 1000 --duration 600
```

### Production Validation

```bash
# Peak traffic simulation
rusmes-loadtest \
  --rate 50000 \
  --concurrency 5000 \
  --duration 3600 \
  --ramp-up 300 \
  --output-json prod-test.json \
  --output-html prod-test.html

# Soak test (24 hours)
rusmes-loadtest \
  --rate 10000 \
  --concurrency 1000 \
  --duration 86400 \
  --output-json soak-test.json
```

## Integration with CI/CD

### GitHub Actions

```yaml
- name: Load Test
  run: |
    rusmes-loadtest \
      --host ${{ secrets.TEST_HOST }} \
      --rate 1000 \
      --duration 60 \
      --output-json loadtest-results.json

- name: Upload Results
  uses: actions/upload-artifact@v3
  with:
    name: loadtest-results
    path: loadtest-results.json
```

### GitLab CI

```yaml
load_test:
  script:
    - rusmes-loadtest --rate 1000 --duration 60 --output-json results.json
  artifacts:
    reports:
      junit: results.json
```

## Advanced Usage

### Custom Workload Patterns

The tool supports various workload patterns in code:

```rust
use rusmes_loadtest::workload::WorkloadPattern;

// Steady load
let pattern = WorkloadPattern::Steady { rate: 1000 };

// Spike test
let pattern = WorkloadPattern::Spike {
    baseline: 100,
    peak: 10000,
    spike_duration: Duration::from_secs(30),
    spike_start: Duration::from_secs(60),
};

// Ramp-up
let pattern = WorkloadPattern::RampUp {
    start_rate: 100,
    end_rate: 10000,
    duration: Duration::from_secs(300),
};

// Wave pattern
let pattern = WorkloadPattern::Wave {
    min_rate: 1000,
    max_rate: 5000,
    period: Duration::from_secs(120),
};
```

## Troubleshooting

### High Connection Failures

If you see many connection failures:

1. Check system file descriptor limits: `ulimit -n`
2. Increase limits: `ulimit -n 65536`
3. Check server connection limits
4. Reduce concurrency or rate

### Memory Issues

For very high concurrency:

1. Monitor memory usage with `top` or `htop`
2. Reduce concurrency
3. Reduce message size
4. Use shorter test duration

### Network Saturation

If network becomes saturated:

1. Check bandwidth with `iperf3`
2. Reduce message size
3. Reduce rate
4. Use multiple test clients

## Architecture

```
rusmes-loadtest/
├── src/
│   ├── main.rs           # CLI entry point
│   ├── lib.rs            # Library root
│   ├── config.rs         # Configuration
│   ├── metrics.rs        # Metrics collection (HDR Histogram)
│   ├── reporter.rs       # Report generation
│   ├── workload.rs       # Workload patterns
│   ├── generators.rs     # Message generation
│   ├── scenarios.rs      # Test scenarios
│   └── protocols/        # Protocol clients
│       ├── smtp.rs
│       ├── imap.rs
│       ├── jmap.rs
│       └── pop3.rs
└── tests/                # Integration tests
```

## License

Same as RusMES project.

## Contributing

See main RusMES CONTRIBUTING.md for guidelines.
