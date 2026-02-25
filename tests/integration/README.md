# RusMES Integration Tests

Comprehensive end-to-end integration tests for the RusMES mail server.

## Overview

These integration tests verify the complete email workflow across all protocols:
- **SMTP** (Simple Mail Transfer Protocol)
- **IMAP** (Internet Message Access Protocol)
- **POP3** (Post Office Protocol v3)
- **JMAP** (JSON Meta Application Protocol)

## Test Suites

### 1. SMTP → IMAP Workflow (`smtp_to_imap.rs`)
Tests the complete delivery pipeline:
- Send email via SMTP (port 25/587)
- Process through mailet chain (DKIM, SPF, spam filtering)
- Store in mailbox
- Retrieve via IMAP (port 143/993)
- Verify headers, flags, and content

**Key Tests:**
- Basic delivery
- Multiple messages
- Headers verification
- Multipart messages
- Concurrent delivery
- Message flags

### 2. JMAP → IMAP → POP3 Workflow (`jmap_to_pop3.rs`)
Tests protocol interoperability:
- Create email via JMAP API
- Fetch via IMAP
- Retrieve via POP3
- Verify consistency across protocols

**Key Tests:**
- JMAP create and query
- JMAP to IMAP workflow
- JMAP to POP3 workflow
- Multiple recipients
- Attachments handling

### 3. Multi-User Concurrent Access (`concurrent_users.rs`)
Tests system behavior under concurrent load:
- 100+ concurrent users
- Simultaneous send/receive
- Race condition detection
- Message isolation verification

**Key Tests:**
- Concurrent SMTP connections (50+)
- Concurrent IMAP connections (50+)
- Concurrent POP3 connections (50+)
- Mixed protocol access
- User isolation
- High concurrency stress (100+)

### 4. Failover and Recovery (`failover.rs`)
Tests system resilience:
- Graceful shutdown/restart
- Crash recovery
- Queue persistence
- Connection handling during shutdown

**Key Tests:**
- Graceful shutdown and restart
- Recovery after crash
- Message delivery during restart
- Data consistency after multiple restarts
- Queue persistence

### 5. Storage Migration (`migration.rs`)
Tests storage backend operations:
- Filesystem backend
- Data persistence
- Concurrent storage operations
- Large message handling

**Key Tests:**
- Filesystem backend basic operations
- Data persistence across restarts
- Concurrent read/write operations
- Large message storage (100KB+)
- Storage integrity under load

### 6. Authentication (`authentication.rs`)
Tests authentication across all protocols:
- Success/failure cases
- Concurrent authentication
- Multiple users
- Edge cases

**Key Tests:**
- IMAP authentication (success/failure)
- POP3 authentication (success/failure)
- SMTP authentication
- JMAP authentication
- Concurrent authentication (20+ simultaneous)
- Different users
- Invalid credentials handling
- Case sensitivity

## Running Tests

### Prerequisites

1. **Rust toolchain** (1.75+)
2. **Docker and Docker Compose** (for full integration tests)
3. **Network ports available**: 25, 143, 110, 8080, 5432, 6379

### Run All Integration Tests

```bash
# From project root
cargo test --test '*' --features integration

# Run specific test suite
cargo test --test smtp_to_imap
cargo test --test concurrent_users
cargo test --test failover
```

### Run with Docker Compose

```bash
# Start test environment
cd tests/integration
docker-compose up -d

# Wait for services to be ready
docker-compose ps

# Run tests
cargo test --test '*' --features integration

# Clean up
docker-compose down -v
```

### Run Individual Tests

```bash
# Run a specific test
cargo test test_smtp_to_imap_basic_delivery

# Run with output
cargo test test_concurrent_smtp_connections -- --nocapture

# Run with logging
RUST_LOG=debug cargo test test_failover_recovery
```

## Test Infrastructure

### Common Test Helpers (`common/mod.rs`)

- **TestServer**: Manages test server lifecycle
  - Auto port allocation
  - Start/stop functionality
  - Automatic cleanup

- **SmtpClient**: SMTP protocol client
  - EHLO, MAIL FROM, RCPT TO, DATA commands
  - Connection management

- **ImapClient**: IMAP protocol client
  - LOGIN, SELECT, FETCH, LOGOUT commands
  - Tagged command handling

- **Pop3Client**: POP3 protocol client
  - USER, PASS, STAT, LIST, RETR commands

- **JmapClient**: JMAP API client
  - HTTP/JSON based communication
  - Session management
  - Method calls

- **MessageGenerator**: Test message creation
  - Simple messages
  - Multipart messages
  - Messages with attachments

### Docker Services

- **rusmes**: Main mail server (ports 25, 143, 110, 8080)
- **postgres**: Database backend (port 5432)
- **redis**: Cache backend (port 6379)
- **clamav**: Antivirus scanning (port 3310)
- **spamassassin**: Spam filtering (port 783)
- **ldap**: LDAP authentication (ports 389, 636)
- **mailhog**: SMTP test sink (ports 1025, 8025)

## CI/CD Integration

### GitHub Actions

The `.github/workflows/integration-tests.yml` workflow:
- Runs on every PR
- Uses Docker Compose for dependencies
- Parallel test execution
- Test result reporting
- Coverage reporting

### Test Reports

Test results are available in:
- GitHub Actions artifacts
- JUnit XML format
- HTML coverage reports

## Performance Benchmarks

Integration tests also serve as performance benchmarks:

- **Concurrent connections**: 100+ simultaneous connections
- **Message throughput**: Messages/second under load
- **Latency**: Average delivery time
- **Recovery time**: Time to restart and resume

## Debugging Tests

### Enable Debug Logging

```bash
RUST_LOG=debug,rusmes=trace cargo test test_name -- --nocapture
```

### Connect to Test Server

```bash
# While tests are running (add a sleep in test code)
telnet localhost 25  # SMTP
telnet localhost 143 # IMAP
telnet localhost 110 # POP3
```

### Inspect Docker Logs

```bash
docker-compose logs -f rusmes
docker-compose logs postgres
```

## Writing New Tests

### Test Structure

```rust
mod common;

use common::{TestServer, SmtpClient};
use std::net::SocketAddr;

#[tokio::test]
async fn test_my_feature() {
    // 1. Create and start server
    let server = TestServer::new().expect("Failed to create test server");
    server.start().await.expect("Failed to start server");

    // 2. Connect client
    let addr = SocketAddr::from(([127, 0, 0, 1], server.smtp_port()));
    let mut client = SmtpClient::connect(addr).await.expect("Failed to connect");

    // 3. Perform test operations
    // ... your test code ...

    // 4. Verify results
    assert_eq!(result, expected);

    // 5. Cleanup
    server.stop().await.expect("Failed to stop server");
}
```

### Best Practices

1. **Always cleanup**: Use `server.stop()` or rely on Drop
2. **Use random ports**: TestServer handles this automatically
3. **Add delays**: Give server time to process (`tokio::time::sleep`)
4. **Verify state**: Check both success and failure cases
5. **Test isolation**: Each test should be independent
6. **Concurrent tests**: Test under realistic load
7. **Error handling**: Test failure scenarios

## Troubleshooting

### Port Already in Use

```bash
# Find process using port
lsof -i :25
kill -9 <PID>
```

### Docker Compose Issues

```bash
# Reset everything
docker-compose down -v
docker system prune -f

# Rebuild images
docker-compose build --no-cache
```

### Test Timeouts

- Increase timeout in test with `#[tokio::test(flavor = "multi_thread")]`
- Add delays with `tokio::time::sleep(Duration::from_secs(5))`
- Check server logs for startup issues

## Coverage

Integration tests contribute to overall code coverage:

```bash
# Generate coverage report
cargo tarpaulin --out Html --output-dir coverage

# View report
open coverage/index.html
```

## Contributing

When adding new integration tests:

1. Follow existing test patterns
2. Add documentation for new test suites
3. Update this README
4. Ensure tests pass in CI
5. Add to appropriate test suite or create new one

## License

Apache-2.0
