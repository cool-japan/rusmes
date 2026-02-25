# RusMES Configuration Examples

This directory contains example configurations for different deployment scenarios.

## Configuration Files

### `rusmes-minimal.toml`
**Use case:** Development and testing

- Non-privileged ports (2525 for SMTP, 1143 for IMAP)
- Filesystem storage
- Minimal processing chain
- Debug logging
- Local file authentication

**Quick start:**
```bash
rusmes-server -c examples/rusmes-minimal.toml
```

### `rusmes-full.toml`
**Use case:** Reference for all available options

- Comprehensive documentation of every configuration option
- Multiple backend examples (filesystem, PostgreSQL, AmateRS)
- All mailet types demonstrated
- Complete security and performance tuning options

**Note:** This file is meant as documentation, not for direct use.

### `rusmes-production.toml`
**Use case:** Production deployment with security hardening

- Standard privileged ports (25, 143, 587, 993)
- PostgreSQL backend
- LDAP authentication
- Aggressive spam/virus filtering
- Strict SPF/DKIM/DMARC enforcement
- JSON logging with rotation
- Prometheus metrics
- Conservative rate limiting

**Before using in production:**
1. Change all `CHANGEME` passwords
2. Update domain names
3. Configure TLS certificates
4. Set up PostgreSQL database
5. Configure LDAP connection
6. Review and adjust rate limits

## Configuration Sections

### Server Identity
```toml
domain = "mail.example.com"
postmaster = "postmaster@example.com"
hostname = "mx1.example.com"
```

### Protocol Servers
- `[smtp]` - SMTP server configuration
- `[imap]` - IMAP server configuration
- `[jmap]` - JMAP API configuration

### Storage
- `[storage]` - Backend selection and configuration
- Supported backends: filesystem, postgres, amaters

### Mail Processing
- `[[processors]]` - Processing pipelines
- `[[processors.mailets]]` - Individual processing steps

### Authentication
- `[auth]` - Authentication backend
- Supported: file, ldap, sql, oauth2

### Observability
- `[metrics]` - Prometheus metrics
  - HTTP endpoint for Prometheus scraping
  - Configurable bind address and path
  - Histograms for message processing latency and SMTP session duration
  - Example: `curl http://localhost:9090/metrics`
- `[logging]` - Logging configuration

### Security
- `[security]` - Security policies
- `[domains]` - Local domain configuration

## Testing Configurations

To validate a configuration file:

```bash
rusmes-server --check-config -c examples/rusmes-production.toml
```

## Migration Guide

### From Postfix/Dovecot

1. Export mailboxes in maildir format
2. Copy to RusMES mailbox directory
3. Configure `[storage.filesystem]` to point to mailbox path
4. Import users to authentication backend

### From Other Mail Servers

1. Use IMAP migration tools (e.g., imapsync)
2. Set up parallel running initially
3. Gradually migrate domains
4. Update MX records when ready

## Environment Variables

Configuration can be overridden with environment variables using the convention `RUSMES_SECTION_KEY`.
Priority: environment variables > config file > defaults

### Supported Environment Variables

#### Server Identity
- `RUSMES_DOMAIN` - Server domain name
- `RUSMES_POSTMASTER` - Postmaster email address

#### SMTP Server
- `RUSMES_SMTP_HOST` - SMTP bind address
- `RUSMES_SMTP_PORT` - SMTP port (default: 25)
- `RUSMES_SMTP_TLS_PORT` - SMTP TLS port (default: 587)
- `RUSMES_SMTP_MAX_MESSAGE_SIZE` - Max message size (e.g., "50MB")
- `RUSMES_SMTP_REQUIRE_AUTH` - Require authentication (true/false)
- `RUSMES_SMTP_ENABLE_STARTTLS` - Enable STARTTLS (true/false)
- `RUSMES_SMTP_RATE_LIMIT_MAX_CONNECTIONS_PER_IP` - Max connections per IP address
- `RUSMES_SMTP_RATE_LIMIT_MAX_MESSAGES_PER_HOUR` - Rate limit max messages
- `RUSMES_SMTP_RATE_LIMIT_WINDOW_DURATION` - Rate limit window (e.g., "1h")

#### IMAP Server
- `RUSMES_IMAP_HOST` - IMAP bind address
- `RUSMES_IMAP_PORT` - IMAP port (default: 143)
- `RUSMES_IMAP_TLS_PORT` - IMAP TLS port (default: 993)

#### JMAP Server
- `RUSMES_JMAP_HOST` - JMAP bind address
- `RUSMES_JMAP_PORT` - JMAP port (default: 8080)
- `RUSMES_JMAP_BASE_URL` - JMAP base URL

#### Storage
- `RUSMES_STORAGE_PATH` - Storage path (filesystem backend only)

#### Logging
- `RUSMES_LOG_LEVEL` - Log level (trace, debug, info, warn, error)
- `RUSMES_LOG_FORMAT` - Log format (json, text)
- `RUSMES_LOG_OUTPUT` - Log output (stdout, stderr, or file path)

#### Queue
- `RUSMES_QUEUE_INITIAL_DELAY` - Initial retry delay (e.g., "60s")
- `RUSMES_QUEUE_MAX_DELAY` - Maximum retry delay (e.g., "3600s")
- `RUSMES_QUEUE_BACKOFF_MULTIPLIER` - Backoff multiplier (e.g., "2.0")
- `RUSMES_QUEUE_MAX_ATTEMPTS` - Maximum retry attempts
- `RUSMES_QUEUE_WORKER_THREADS` - Number of worker threads
- `RUSMES_QUEUE_BATCH_SIZE` - Batch processing size

#### Metrics
- `RUSMES_METRICS_ENABLED` - Enable metrics (true/false)
- `RUSMES_METRICS_BIND_ADDRESS` - Metrics endpoint bind address (e.g., "0.0.0.0:9090")
- `RUSMES_METRICS_PATH` - Metrics endpoint path (e.g., "/metrics")

### Examples

Override SMTP port for development:
```bash
RUSMES_SMTP_PORT=2525 rusmes-server -c rusmes.toml
```

Override log level for debugging:
```bash
RUSMES_LOG_LEVEL=debug rusmes-server -c rusmes.toml
```

Multiple overrides:
```bash
RUSMES_SMTP_PORT=2525 \
RUSMES_IMAP_PORT=1143 \
RUSMES_LOG_LEVEL=debug \
rusmes-server -c rusmes.toml
```

Docker deployment with environment variables:
```bash
docker run -e RUSMES_SMTP_PORT=25 \
           -e RUSMES_IMAP_PORT=143 \
           -e RUSMES_LOG_LEVEL=info \
           -e RUSMES_STORAGE_PATH=/var/mail \
           rusmes/rusmes:latest
```

## Best Practices

1. **Start minimal** - Begin with `rusmes-minimal.toml` and add features
2. **Test thoroughly** - Use staging environment before production
3. **Monitor closely** - Enable metrics from day one
4. **Rotate logs** - Configure log rotation to prevent disk fill
5. **Secure secrets** - Never commit passwords to version control
6. **Update regularly** - Keep RusMES and dependencies up to date
7. **Backup configuration** - Keep versioned copies of working configs

## Getting Help

- Documentation: https://rusmes.org/docs
- Issues: https://github.com/rusmes/rusmes/issues
- Community: https://discord.gg/rusmes
