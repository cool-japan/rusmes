# rusmes-metrics TODO

## Implemented ✅
### Prometheus Metrics (664 lines)
- [x] SMTP/IMAP/JMAP/Queue/Storage counters + histograms
- [x] Prometheus text format exporter
- [x] `/metrics` HTTP endpoint (axum, configurable bind address)
- [x] Health check endpoints (`/health`, `/ready`, `/live`) for Kubernetes probes
- [x] Histogram for message processing latency
- [x] Histogram for SMTP session duration

### OpenTelemetry (477 lines)
- [x] OTLP exporter (gRPC + HTTP)
- [x] SMTP/IMAP/JMAP/Mailet pipeline span generation
- [x] Distributed tracing with span propagation

### Dashboards
- [x] Grafana dashboard JSON template (16 panels)
- [x] Alert rules

## Remaining
- [ ] Basic auth for metrics endpoint (optional)
- [ ] Per-domain message counters
- [ ] TLS connection counters (plaintext vs encrypted)
- [ ] Active connections gauge per protocol (SMTP, IMAP, JMAP, POP3)