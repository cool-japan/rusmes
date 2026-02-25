# RusMES Grafana Dashboard

This directory contains a production-ready Grafana dashboard and alert rules for monitoring the RusMES mail server.

## Contents

- `dashboard.json` - Complete Grafana dashboard with 20+ panels
- `alerts.yaml` - Prometheus alert rules for critical conditions
- `README.md` - This documentation file

## Dashboard Overview

The RusMES monitoring dashboard provides comprehensive visibility into:

### Row 1: Overview
- **Messages per Second** - Real-time message throughput across SMTP, IMAP, and JMAP
- **Active Connections** - Current active connections by protocol
- **Queue Depth** - Mail queue size with alerting threshold

### Row 2: Performance
- **Message Processing Latency** - p50, p95, p99 percentile latency tracking
- **SMTP Session Duration** - Session length percentiles
- **Storage Usage** - Disk space utilization gauge

### Row 3: System Resources
- **Memory Usage** - Process memory consumption
- **CPU Usage** - CPU utilization percentage
- **Disk I/O** - Read/write throughput

### Row 4: Errors & Security
- **Error Rate** - Errors per second by protocol
- **Spam/Virus Blocked** - Messages blocked by security filters
- **Mail Processing Status** - Pie chart of delivered/bounced/dropped mail

### Row 5: Protocol-Specific Metrics
- **SMTP Message Flow** - Inbound vs outbound SMTP traffic
- **IMAP Command Rate** - IMAP command frequency
- **JMAP Request Rate** - JMAP API request frequency

### Row 6: Summary Statistics
- **Total Messages Processed** - Lifetime message counter
- **Total Mailboxes** - Number of mailboxes
- **Total Messages Stored** - Current message count
- **Uptime** - Service uptime duration

## Installation

### Prerequisites

- Grafana 10.x or later
- Prometheus data source configured
- RusMES metrics endpoint enabled (default: `http://localhost:9090/metrics`)

### Step 1: Configure Prometheus Data Source

1. Log into Grafana
2. Navigate to **Configuration** → **Data Sources**
3. Click **Add data source**
4. Select **Prometheus**
5. Configure:
   - **Name**: `Prometheus` (or your preferred name)
   - **URL**: `http://prometheus:9090` (adjust for your setup)
   - **Scrape interval**: `15s` or `30s`
6. Click **Save & Test**

### Step 2: Import Dashboard

#### Method 1: Import from JSON

1. Navigate to **Dashboards** → **Import**
2. Click **Upload JSON file**
3. Select `dashboard.json` from this directory
4. Configure:
   - **Name**: Leave as "RusMES Mail Server Monitoring" or customize
   - **Folder**: Select or create a folder
   - **Prometheus**: Select your Prometheus data source
5. Click **Import**

#### Method 2: Copy-Paste JSON

1. Navigate to **Dashboards** → **Import**
2. Copy the contents of `dashboard.json`
3. Paste into the **Import via panel json** text area
4. Click **Load**
5. Select your Prometheus data source
6. Click **Import**

### Step 3: Configure Prometheus Scraping

Add the following job to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'rusmes'
    static_configs:
      - targets: ['localhost:9090']  # Adjust host and port
    scrape_interval: 30s
    scrape_timeout: 10s
    metrics_path: /metrics
```

Reload Prometheus configuration:

```bash
curl -X POST http://localhost:9090/-/reload
```

Or restart Prometheus:

```bash
systemctl restart prometheus
```

### Step 4: Load Alert Rules

Add the alert rules to Prometheus:

1. Copy `alerts.yaml` to your Prometheus configuration directory:

```bash
sudo cp alerts.yaml /etc/prometheus/rules/
```

2. Update `prometheus.yml` to include the rules:

```yaml
rule_files:
  - "rules/alerts.yaml"
```

3. Reload Prometheus:

```bash
curl -X POST http://localhost:9090/-/reload
```

4. Verify alerts are loaded:

Navigate to Prometheus UI: `http://localhost:9090/alerts`

## Alert Configuration

The alert rules include the following conditions:

### Queue Alerts
- **HighQueueDepth**: Queue > 1000 messages for 5 minutes (warning)
- **CriticalQueueDepth**: Queue > 5000 messages for 2 minutes (critical)
- **QueueRetryStorm**: Retry rate > 50/sec for 2 minutes (warning)

### Error Alerts
- **HighSMTPErrorRate**: SMTP errors > 10/sec for 1 minute (critical)
- **HighIMAPErrorRate**: IMAP errors > 10/sec for 1 minute (critical)
- **HighJMAPErrorRate**: JMAP errors > 10/sec for 1 minute (critical)

### Storage Alerts
- **HighStorageUsage**: Storage > 90% for 5 minutes (critical)
- **StorageUsageWarning**: Storage > 70% for 10 minutes (warning)

### Performance Alerts
- **HighMemoryUsage**: Memory > 1.5GB for 5 minutes (warning)
- **CriticalMemoryUsage**: Memory > 3GB for 2 minutes (critical)
- **HighCPUUsage**: CPU > 80% for 10 minutes (warning)
- **CriticalCPUUsage**: CPU > 95% for 5 minutes (critical)
- **HighMessageProcessingLatency**: P95 latency > 5s for 5 minutes (warning)
- **HighSMTPSessionDuration**: P95 duration > 120s for 5 minutes (warning)

### Security Alerts
- **BruteForceAttackDetected**: Auth failure rate > 50/sec for 2 minutes (critical)
- **HighSpamRate**: Blocked messages > 100/sec for 5 minutes (warning)

### Availability Alerts
- **ServiceDown**: Service not responding for 1 minute (critical)
- **HighConnectionFailureRate**: > 10% SMTP connection failures for 5 minutes (warning)
- **NoMessagesProcessed**: No mail processed in 1 hour (warning)

### Delivery Alerts
- **HighBouncedMailRate**: > 20% bounce rate for 10 minutes (warning)
- **CriticalBouncedMailRate**: > 50% bounce rate for 5 minutes (critical)

### Health Alerts
- **HealthCheckFailing**: Health endpoint failing for 2 minutes (critical)
- **ReadinessCheckFailing**: Readiness endpoint failing for 1 minute (warning)

## Notification Channels

To receive alert notifications, configure notification channels in Grafana:

### Email Notifications

1. Navigate to **Alerting** → **Contact points**
2. Click **New contact point**
3. Configure:
   - **Name**: `Email Alerts`
   - **Type**: `Email`
   - **Addresses**: Your email addresses
4. Click **Save contact point**

### Slack Notifications

1. Create a Slack webhook URL in your Slack workspace
2. Navigate to **Alerting** → **Contact points**
3. Click **New contact point**
4. Configure:
   - **Name**: `Slack Alerts`
   - **Type**: `Slack`
   - **Webhook URL**: Your Slack webhook URL
5. Click **Save contact point**

### PagerDuty Integration

1. Navigate to **Alerting** → **Contact points**
2. Click **New contact point**
3. Configure:
   - **Name**: `PagerDuty`
   - **Type**: `PagerDuty`
   - **Integration Key**: Your PagerDuty integration key
4. Click **Save contact point**

### Configure Alert Rules to Use Notification Channels

1. Navigate to **Alerting** → **Notification policies**
2. Edit the default policy or create new policies
3. Map alert labels to notification channels:
   - `severity=critical` → PagerDuty
   - `severity=warning` → Email/Slack
4. Save your notification policy

## Dashboard Variables

The dashboard includes template variables for filtering:

### $datasource
- **Type**: Data source selector
- **Purpose**: Select Prometheus data source
- **Default**: Prometheus

### $instance
- **Type**: Query-based multi-select
- **Purpose**: Filter by server instance
- **Query**: `label_values(rusmes_smtp_connections_total, instance)`
- **Options**: Multi-select with "All" option

### $interval
- **Type**: Interval selector
- **Purpose**: Adjust query aggregation interval
- **Options**: 1m, 5m, 10m, 30m, 1h
- **Default**: Auto (based on time range)

## Customization Guide

### Adjusting Time Range

The default time range is 6 hours. To change:

1. Click the time picker in the upper-right corner
2. Select a preset or custom range
3. Click **Apply**

To change the default:

1. Click the dashboard settings gear icon
2. Go to **General** → **Time options**
3. Set **Default time range** to your preference
4. Click **Save dashboard**

### Modifying Panels

To edit a panel:

1. Hover over the panel title
2. Click the three dots (⋮)
3. Select **Edit**
4. Modify queries, visualization, or thresholds
5. Click **Apply** and save the dashboard

### Adding New Panels

To add custom panels:

1. Click **Add panel** in the top menu
2. Select **Add a new panel**
3. Configure your query and visualization
4. Click **Apply**
5. Save the dashboard

### Adjusting Alert Thresholds

To modify alert thresholds:

1. Edit the panel with the alert
2. Go to the **Alert** tab
3. Modify conditions and thresholds
4. Click **Apply**

Or edit `alerts.yaml` and reload Prometheus.

### Color Schemes

The dashboard uses Grafana's palette-classic theme with custom thresholds:

- **Green**: Normal/healthy state
- **Yellow**: Warning state (70-90% capacity)
- **Orange**: High warning
- **Red**: Critical state (>90% capacity)

To customize colors:

1. Edit panel → **Field** tab
2. Modify **Thresholds** settings
3. Adjust colors for each threshold step

## Annotations

The dashboard supports two annotation types:

### Alert Annotations
- **Enabled by default**
- Shows when alerts fire/resolve
- Color: Red
- Query: `ALERTS{alertstate="firing",instance=~"$instance"}`

### Deployment Annotations
- **Disabled by default**
- Enable to track deployments
- Color: Blue
- Configure your deployment tracking system to push annotations

To enable deployment tracking:

1. Dashboard settings → **Annotations**
2. Enable **Deployments**
3. Configure query or use external tool like Grafana API

## Best Practices

### Monitoring

1. **Set up alerting** - Configure notification channels before production use
2. **Review alerts regularly** - Tune thresholds based on your workload
3. **Monitor trends** - Use longer time ranges to identify patterns
4. **Correlate metrics** - Use multiple panels to diagnose issues

### Performance

1. **Adjust scrape interval** - 30s is recommended for production
2. **Use recording rules** - For complex queries used in multiple panels
3. **Set appropriate retention** - Balance storage vs historical data needs

### Security

1. **Enable authentication** - Protect your Grafana instance
2. **Use HTTPS** - Encrypt dashboard access
3. **Limit access** - Use Grafana's role-based access control
4. **Audit changes** - Enable dashboard versioning

## Troubleshooting

### No Data Appearing

1. Verify Prometheus is scraping RusMES:
   ```bash
   curl http://localhost:9090/api/v1/targets
   ```

2. Check RusMES metrics endpoint:
   ```bash
   curl http://localhost:9090/metrics
   ```

3. Verify Grafana data source connection:
   - Go to **Data Sources** → **Prometheus** → **Test**

4. Check time range - ensure it covers when metrics were collected

### Alerts Not Firing

1. Verify alerts are loaded in Prometheus:
   ```bash
   curl http://localhost:9090/api/v1/rules
   ```

2. Check alert evaluation:
   - Visit Prometheus UI: `http://localhost:9090/alerts`

3. Verify notification channels are configured:
   - Grafana → **Alerting** → **Contact points**

### High Memory Usage in Grafana

1. Reduce dashboard auto-refresh rate
2. Decrease time range
3. Optimize queries (use recording rules)
4. Increase Grafana server resources

### Missing Metrics

If certain metrics are not appearing:

1. Verify RusMES version supports all metrics
2. Check if metrics are being collected:
   ```bash
   curl http://localhost:9090/metrics | grep rusmes_
   ```

3. Update dashboard to use available metrics
4. Check RusMES configuration for metrics enablement

## Metrics Reference

### SMTP Metrics
- `rusmes_smtp_connections_total` - Total SMTP connections
- `rusmes_smtp_messages_received_total` - Messages received via SMTP
- `rusmes_smtp_messages_sent_total` - Messages sent via SMTP
- `rusmes_smtp_errors_total` - SMTP protocol errors
- `rusmes_smtp_session_duration_seconds_bucket` - SMTP session duration histogram

### IMAP Metrics
- `rusmes_imap_connections_total` - Total IMAP connections
- `rusmes_imap_commands_total` - IMAP commands processed
- `rusmes_imap_errors_total` - IMAP protocol errors

### JMAP Metrics
- `rusmes_jmap_requests_total` - Total JMAP requests
- `rusmes_jmap_errors_total` - JMAP request errors

### Mail Processing Metrics
- `rusmes_mail_processed_total` - Total mail processed
- `rusmes_mail_delivered_total` - Mail successfully delivered
- `rusmes_mail_bounced_total` - Mail bounced
- `rusmes_mail_dropped_total` - Mail dropped (spam/virus)
- `rusmes_message_processing_latency_seconds_bucket` - Processing latency histogram

### Queue Metrics
- `rusmes_queue_size` - Current queue depth
- `rusmes_queue_retries_total` - Queue delivery retries

### Storage Metrics
- `rusmes_mailboxes_total` - Number of mailboxes
- `rusmes_messages_total` - Number of stored messages
- `rusmes_storage_bytes` - Storage space used

### System Metrics
- `process_resident_memory_bytes` - Process memory usage
- `process_cpu_seconds_total` - CPU time consumed
- `process_start_time_seconds` - Process start timestamp

## Advanced Configuration

### Recording Rules

For better performance, create recording rules in Prometheus:

```yaml
groups:
  - name: rusmes_recording_rules
    interval: 30s
    rules:
      - record: rusmes:message_rate:5m
        expr: rate(rusmes_mail_processed_total[5m])

      - record: rusmes:error_rate:5m
        expr: |
          sum(
            rate(rusmes_smtp_errors_total[5m]) +
            rate(rusmes_imap_errors_total[5m]) +
            rate(rusmes_jmap_errors_total[5m])
          )

      - record: rusmes:latency:p95:5m
        expr: |
          histogram_quantile(0.95,
            sum(rate(rusmes_message_processing_latency_seconds_bucket[5m])) by (le)
          )
```

Then update dashboard queries to use recording rules for faster rendering.

### Grafana Provisioning

For automated deployment, use Grafana provisioning:

Create `grafana/provisioning/dashboards/rusmes.yml`:

```yaml
apiVersion: 1

providers:
  - name: 'RusMES'
    orgId: 1
    folder: 'Mail Server'
    type: file
    disableDeletion: false
    updateIntervalSeconds: 10
    allowUiUpdates: true
    options:
      path: /etc/grafana/dashboards/rusmes
```

Place `dashboard.json` in the configured path.

### Kubernetes Deployment

For Kubernetes deployments, use ConfigMaps:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: rusmes-grafana-dashboard
  namespace: monitoring
data:
  rusmes-dashboard.json: |
    <paste dashboard.json contents>
```

Mount in Grafana deployment and configure provisioning.

## Support

For issues, questions, or contributions:

- GitHub Issues: https://github.com/yourusername/rusmes
- Documentation: See main README.md
- Metrics Documentation: See crates/rusmes-metrics/README.md

## License

This dashboard and alert configuration are part of the RusMES project and are distributed under the same license as the main project.
