# rusmes-metrics

Observability and metrics collection for RusMES. Provides lock-free Prometheus-compatible metrics using `AtomicU64` counters and gauges, a Prometheus HTTP endpoint, active-connection tracking via RAII guards, TLS session counters, and per-domain message counters.

## Status

Complete. The `MetricsCollector` is fully implemented with metrics across SMTP, IMAP, JMAP, mail processing, queue, storage, active-connections, TLS, and per-domain categories. A Prometheus HTTP endpoint is served on port 9090.

## Architecture

```
MetricsCollector (Clone, Send, Sync)
  |-- Arc<AtomicU64> for each counter/gauge (lock-free)
  |-- Arc<DashMap<String, Arc<AtomicI64>>> for active-connection gauges (per-protocol)
  |-- Arc<DashMap<String, Arc<AtomicU64>>> for per-domain message counters
  |-- inc_*() methods for counters
  |-- set_*() methods for gauges
  |-- connection_guard(protocol) -> ConnectionGuard  (RAII dec-on-drop)
  |-- export_prometheus() -> String (text exposition format)
  '-- start_http_server(port, metrics) -> tokio::task::JoinHandle
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

### Active Connections (per protocol)
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_active_connections{protocol="smtp"}` | Gauge | Current active SMTP connections |
| `rusmes_active_connections{protocol="imap"}` | Gauge | Current active IMAP connections |
| `rusmes_active_connections{protocol="jmap"}` | Gauge | Current in-flight JMAP requests |

### TLS Sessions
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_tls_sessions_total{tls="yes"}` | Counter | Sessions on implicit-TLS listeners |
| `rusmes_tls_sessions_total{tls="no"}` | Counter | Plaintext sessions |
| `rusmes_tls_sessions_total{tls="starttls"}` | Counter | Sessions upgraded via STARTTLS |

### Per-domain Message Counters
| Metric | Type | Description |
|--------|------|-------------|
| `rusmes_messages_per_domain_total{domain="..."}` | Counter | Messages processed per recipient domain |

## API

```rust
use rusmes_metrics::{MetricsCollector, global_metrics, tls_label};

// Standard counter/gauge usage
let metrics = MetricsCollector::new();
metrics.inc_smtp_connections();
metrics.inc_smtp_messages_received();
metrics.set_queue_size(42);

// Active connections — RAII guard decrements on drop
let _guard = metrics.connection_guard("smtp");
// gauge is incremented; drops to previous value when _guard is dropped

// Global singleton (for use in protocol crates without threading the handle)
let _guard = global_metrics().connection_guard("imap");

// TLS session counters
metrics.inc_tls_session(tls_label::STARTTLS);
metrics.inc_tls_session(tls_label::NO);

// Per-domain counters via callback source
metrics.set_domain_stats_source(Arc::new(move || queue.queue_stats_per_domain()));

// Prometheus text exposition
let prometheus_text = metrics.export_prometheus();

// HTTP endpoint on port 9090
start_http_server(9090, metrics).await;
```

## Prometheus HTTP Endpoint

`start_http_server(port, metrics)` spawns an axum server that serves:

| Path | Description |
|------|-------------|
| `GET /metrics` | Prometheus text exposition (optional Basic auth) |
| `GET /health` | Kubernetes liveness / readiness probe |
| `GET /ready` | Readiness probe |
| `GET /live` | Liveness probe |

Optional Basic auth is configured via `[metrics.basic_auth] { username, password_hash }` in `rusmes.toml`. Omitting the block serves the endpoint anonymously (backward-compatible default).

## Dependencies
- `rusmes-proto` - `Mail` type for `record_mail_completed()`
- `dashmap` - lock-free per-protocol and per-domain maps
- `axum` / `tokio` - async HTTP server for the `/metrics` endpoint
- `bcrypt` - password hash verification for optional Basic auth

## Tests

```bash
cargo nextest run -p rusmes-metrics --all-features   # 26 tests
```
