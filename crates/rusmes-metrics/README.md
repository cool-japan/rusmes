# rusmes-metrics

Observability and metrics collection for RusMES. Provides lock-free Prometheus-compatible metrics using `AtomicU64` counters and gauges.

## Status

Complete. The `MetricsCollector` is fully implemented with 18 metrics across SMTP, IMAP, JMAP, mail processing, queue, and storage categories.

## Architecture

```
MetricsCollector (Clone, Send, Sync)
  |-- Arc<AtomicU64> for each metric (lock-free)
  |-- inc_*() methods for counters
  |-- set_*() methods for gauges
  '-- export_prometheus() -> String (text format)
```

## Metrics

### SMTP
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_smtp_connections_total` | Counter | Total SMTP connections accepted |
| `rusmes_smtp_messages_received_total` | Counter | Total messages received via SMTP |
| `rusmes_smtp_messages_sent_total` | Counter | Total messages relayed outbound |
| `rusmes_smtp_errors_total` | Counter | Total SMTP errors |

### IMAP
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_imap_connections_total` | Counter | Total IMAP connections |
| `rusmes_imap_commands_total` | Counter | Total IMAP commands processed |
| `rusmes_imap_errors_total` | Counter | Total IMAP errors |

### JMAP
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_jmap_requests_total` | Counter | Total JMAP API requests |
| `rusmes_jmap_errors_total` | Counter | Total JMAP errors |

### Mail Processing
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_mail_processed_total` | Counter | Total mail processed through pipeline |
| `rusmes_mail_delivered_total` | Counter | Total mail delivered to local mailboxes |
| `rusmes_mail_bounced_total` | Counter | Total bounce messages generated |
| `rusmes_mail_dropped_total` | Counter | Total mail dropped (Ghost state) |

### Queue
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_queue_size` | Gauge | Current number of messages in queue |
| `rusmes_queue_retries_total` | Counter | Total queue retry attempts |

### Storage
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_mailboxes_total` | Gauge | Total mailboxes |
| `rusmes_messages_total` | Gauge | Total stored messages |
| `rusmes_storage_bytes` | Gauge | Total storage usage in bytes |

## Usage

```rust
use rusmes_metrics::MetricsCollector;

let metrics = MetricsCollector::new();
metrics.inc_smtp_connections();
metrics.inc_smtp_messages_received();
metrics.set_queue_size(42);

let prometheus_text = metrics.export_prometheus();
// Returns Prometheus text exposition format
```

## Dependencies
- `rusmes-proto` - `Mail` type for `record_mail_completed()`
- `prometheus` (declared, custom implementation used)
- `opentelemetry` (declared, for future tracing integration)

## Tests

```bash
cargo test -p rusmes-metrics   # 2 tests
```
