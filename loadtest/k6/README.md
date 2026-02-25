# K6 Load Testing Scripts

Alternative load testing scripts using [k6](https://k6.io/).

## Prerequisites

Install k6:

```bash
# Linux
sudo apt-key adv --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69
echo "deb https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
sudo apt-get update
sudo apt-get install k6

# macOS
brew install k6

# Docker
docker pull grafana/k6
```

## Usage

### Basic SMTP Load Test

```bash
k6 run smtp_load.js
```

### Custom Parameters

```bash
# Custom target and duration
k6 run --vus 100 --duration 60s smtp_load.js

# Environment variables
SMTP_HOST=mail.example.com \
SMTP_PORT=25 \
SMTP_FROM=sender@example.com \
SMTP_TO=recipient@example.com \
k6 run smtp_load.js

# Output to InfluxDB
k6 run --out influxdb=http://localhost:8086/k6 smtp_load.js

# Generate HTML report
k6 run --out json=results.json smtp_load.js
k6 convert results.json --output report.html
```

### Advanced Scenarios

```bash
# Spike test
k6 run --stage 30s:100,10s:1000,60s:1000,30s:100 smtp_load.js

# Stress test
k6 run --stage 60s:1000,120s:1000,60s:0 smtp_load.js

# Soak test (6 hours)
k6 run --stage 60s:100,6h:100,60s:0 smtp_load.js
```

## Comparison: rusmes-loadtest vs k6

| Feature | rusmes-loadtest | k6 |
|---------|----------------|-----|
| Language | Rust | JavaScript |
| Performance | Higher (native) | Good (Go runtime) |
| Setup | Cargo install | Separate install |
| SMTP Support | Native | Via extension |
| IMAP Support | Native | Via extension |
| JMAP Support | Native | Custom HTTP |
| Metrics | HDR Histogram | Built-in |
| Reporting | JSON/HTML/CSV | JSON/InfluxDB |
| Scripting | Limited | Full JS |

## When to Use Each

### Use rusmes-loadtest when:
- Testing rusmes-specific features
- Need maximum performance
- Want tight integration with rusmes
- Prefer native binary
- Need detailed protocol-level metrics

### Use k6 when:
- Need complex scenarios
- Want JavaScript flexibility
- Integration with Grafana/InfluxDB
- Industry-standard tool
- Cloud testing (k6 Cloud)

## Metrics

K6 provides:
- `smtp_latency`: Response time trend
- `smtp_errors`: Error counter
- `smtp_success`: Success counter

Custom thresholds:
```javascript
thresholds: {
  'smtp_latency': ['p(95)<500'],
  'smtp_errors': ['count<100'],
}
```

## Cloud Testing

Upload to k6 Cloud for distributed testing:

```bash
k6 cloud smtp_load.js
```

## CI/CD Integration

GitHub Actions:
```yaml
- uses: grafana/k6-action@v0.3.0
  with:
    filename: loadtest/k6/smtp_load.js
```

GitLab CI:
```yaml
k6_test:
  image: grafana/k6:latest
  script:
    - k6 run loadtest/k6/smtp_load.js
```
