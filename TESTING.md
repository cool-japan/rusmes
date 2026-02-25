# RusMES Wave 12: Testing & Quality Assurance

Complete testing infrastructure for RusMES mail server.

## Overview

This wave implements comprehensive testing across 6 major categories:
1. Unit Tests (300+ tests)
2. Integration Tests (6 test suites)
3. RFC Compliance Tests (6 RFC test suites, 100+ tests)
4. Performance Benchmarks (4 benchmark suites)
5. Fuzz Testing (5 fuzz targets)
6. Load Testing Tool (rusmes-loadtest crate)

**Total Test Code: ~5,000+ lines**

## 1. Unit Tests

### Location
- `crates/*/tests/*.rs`
- Embedded tests in source files with `#[cfg(test)]`

### Coverage
- **rusmes-core**: Mailet tests (20+ tests per mailet)
  - AddHeaderMailet
  - VirusScanMailet
  - SpamAssassinMailet
  - DkimVerifyMailet
  - DmarcVerifyMailet
  - SpfCheckMailet
  - LocalDeliveryMailet
  - RemoteDeliveryMailet

- **rusmes-storage**: Backend tests
  - MemoryBackend
  - FilesystemBackend
  - S3Backend
  - PostgreSQL operations
  - Mailbox operations

- **rusmes-smtp**: Protocol tests
  - Command parsing
  - Response codes
  - AUTH mechanisms
  - Extensions (SIZE, 8BITMIME, STARTTLS)

- **rusmes-auth**: Authentication tests
  - Password hashing/verification
  - LDAP backend
  - Database backend
  - PAM backend
  - Session management
  - Rate limiting
  - 2FA/TOTP

### Running Unit Tests
```bash
# All tests
cargo test

# Specific crate
cargo test --package rusmes-core

# Specific test
cargo test --package rusmes-core test_add_header_mailet

# With output
cargo test -- --nocapture
```

## 2. Integration Tests

### Location
- `tests/integration/*.rs`

### Test Suites

#### 1. SMTP to IMAP Workflow (`smtp_to_imap_workflow.rs`)
Tests complete email delivery:
- SMTP sending
- Mailet processing
- Storage
- IMAP retrieval
- Multiple messages
- Concurrent delivery
- Error handling

#### 2. JMAP Workflow (`jmap_workflow.rs`)
Tests JMAP interoperability:
- JMAP create
- IMAP fetch
- POP3 retrieve
- Multiple recipients
- Batch operations
- JSON format validation

#### 3. Concurrent Access (`concurrent_access.rs`)
Tests multi-user scenarios:
- Concurrent user creation
- Concurrent message sending
- Read/write concurrency
- Multiple users
- High concurrency (100+ operations)

#### 4. Failover & Recovery (`failover_recovery.rs`)
Tests system resilience:
- Failure detection
- Automatic failover
- Primary recovery
- Cascading failures
- Partial recovery

#### 5. Storage Migration (`storage_migration.rs`)
Tests backend migration:
- Memory to filesystem
- Large dataset migration
- Bidirectional migration
- Incremental migration
- Concurrent migration

#### 6. Authentication Across Protocols (`auth_across_protocols.rs`)
Tests unified authentication:
- SMTP AUTH
- IMAP LOGIN
- POP3 USER/PASS
- JMAP bearer tokens
- Account lockout
- Password validation

### Running Integration Tests
```bash
# All integration tests
cargo test --test '*'

# Specific suite
cargo test --test smtp_to_imap_workflow

# Specific test
cargo test --test concurrent_access test_high_concurrency
```

## 3. RFC Compliance Tests

### Location
- `tests/rfc_compliance/*.rs`

### Test Suites

#### 1. SMTP RFC 5321 (`smtp_rfc5321.rs`)
- Command format (EHLO, MAIL FROM, RCPT TO, DATA, QUIT)
- Response codes (2xx, 3xx, 4xx, 5xx)
- Multiline responses
- Line length limits
- Path syntax
- Email address format
- Case insensitivity

#### 2. IMAP RFC 9051 (`imap_rfc9051.rs`)
- Command format (tag + command)
- All IMAP commands (LOGIN, SELECT, FETCH, SEARCH, etc.)
- Response format (tagged/untagged)
- Flags (\\Seen, \\Answered, etc.)
- Sequence sets
- Quoted strings
- Literal strings
- Date format
- Search criteria

#### 3. POP3 RFC 1939 (`pop3_rfc1939.rs`)
- Commands (USER, PASS, STAT, LIST, RETR, DELE, QUIT)
- Responses (+OK, -ERR)
- Greeting format

#### 4. JMAP RFC 8620/8621 (`jmap_rfc8620.rs`)
- Request structure
- Method calls
- Capabilities
- Response structure
- Error responses

#### 5. MIME RFC 2045 (`mime_rfc2045.rs`)
- Content-Type headers
- Content-Transfer-Encoding
- Multipart boundaries
- MIME version

#### 6. DSN RFC 3464 (`dsn_rfc3464.rs`)
- DSN content type
- Action values
- Status codes
- DSN fields

### Running RFC Tests
```bash
# All RFC compliance tests
cargo test --test 'rfc_*'

# Specific RFC
cargo test --test smtp_rfc5321
```

## 4. Performance Benchmarks

### Location
- `benches/*.rs`

### Benchmark Suites

#### 1. Throughput Benchmark (`throughput_benchmark.rs`)
Measures messages per second:
- Single message parsing
- Batch processing (10, 100, 1000 messages)
- Various message sizes (1KB, 10KB, 100KB)

#### 2. Concurrent Connections (`concurrent_connections.rs`)
Measures connection handling:
- Connection pool performance
- Concurrent acquire/release
- Scalability (10, 100, 1000 connections)

#### 3. Search Performance (`search_performance.rs`)
Measures search operations:
- Indexing performance
- Query execution
- Large dataset handling

#### 4. Mailet Pipeline (`mailet_pipeline.rs`)
Measures processing latency:
- Single mailet execution
- Full pipeline (multiple mailets)
- Processing overhead

### Running Benchmarks
```bash
# All benchmarks
cargo bench

# Specific benchmark
cargo bench --bench throughput_benchmark

# With HTML reports
cargo bench -- --save-baseline baseline_v1
```

## 5. Fuzz Testing

### Location
- `fuzz/fuzz_targets/*.rs`

### Fuzz Targets

#### 1. SMTP Parser (`fuzz_smtp_parser.rs`)
Fuzzes SMTP command parsing for crashes and panics

#### 2. IMAP Parser (`fuzz_imap_parser.rs`)
Fuzzes IMAP command parsing

#### 3. MIME Parser (`fuzz_mime_parser.rs`)
Fuzzes MIME header parsing

#### 4. Email Address Parser (`fuzz_email_address.rs`)
Fuzzes email address validation

#### 5. Sieve Parser (`fuzz_sieve_parser.rs`)
Fuzzes Sieve script parsing

### Running Fuzz Tests
```bash
# Install cargo-fuzz
cargo install cargo-fuzz

# Run specific fuzz target
cargo fuzz run fuzz_smtp_parser

# Run with corpus
cargo fuzz run fuzz_smtp_parser -- -runs=1000000

# Minimize corpus
cargo fuzz cmin fuzz_smtp_parser
```

## 6. Load Testing Tool (rusmes-loadtest)

### Location
- `crates/rusmes-loadtest/`

### Features
- Configurable message rate
- Multiple concurrent senders
- Random message sizes
- Multiple protocols (SMTP, IMAP, JMAP)
- Latency percentiles (p50, p95, p99, p99.9)
- Error rate tracking
- Resource usage monitoring

### Components

#### Configuration (`config.rs`)
- Target host/port
- Test duration
- Concurrency level
- Message rate
- Message size range

#### Metrics (`metrics.rs`)
- Total requests
- Success/failure counts
- Latency histogram
- Throughput calculation
- Detailed statistics

#### Scenarios (`scenarios.rs`)
- SMTP Throughput
- Concurrent Connections
- Mixed Protocol
- Sustained Load

#### Message Generation (`generators.rs`)
- Random message generation
- Configurable sizes
- Attachment support

#### Protocol Clients (`protocols.rs`)
- SMTP client
- IMAP client
- JMAP client

### Usage

#### CLI
```bash
# Basic test
rusmes-loadtest --host localhost --port 25 --duration 60

# Custom configuration
rusmes-loadtest \
  --host mail.example.com \
  --port 25 \
  --scenario smtp-throughput \
  --duration 300 \
  --concurrency 50 \
  --rate 1000 \
  --min-size 1024 \
  --max-size 102400

# Different scenarios
rusmes-loadtest --scenario concurrent-connections --concurrency 1000
rusmes-loadtest --scenario mixed-protocol --duration 600
rusmes-loadtest --scenario sustained-load --rate 100
```

#### Programmatic
```rust
use rusmes_loadtest::{LoadTestConfig, LoadTester};
use rusmes_loadtest::scenarios::ScenarioType;

#[tokio::main]
async fn main() {
    let config = LoadTestConfig {
        target_host: "localhost".to_string(),
        target_port: 25,
        scenario: ScenarioType::SmtpThroughput,
        duration_secs: 60,
        concurrency: 10,
        message_rate: 100,
        message_size_min: 1024,
        message_size_max: 102400,
    };

    let tester = LoadTester::new(config);
    let metrics = tester.run().await.unwrap();

    metrics.print_summary();
}
```

### Load Test Output
```
=== Load Test Results ===

Duration: 60.00s
Total Requests: 6000
Successful: 5950
Failed: 50
Success Rate: 99.17%
Throughput: 100.00 req/s

Data Transfer:
  Sent: 307200000 bytes (307.20 MB)
  Received: 102400 bytes (0.10 MB)

Latency:
  Min: 5ms
  Mean: 15ms
  Max: 250ms
  p50: 12ms
  p95: 45ms
  p99: 120ms
  p99.9: 235ms
```

## Test Statistics

### Total Test Coverage
- **Unit Tests**: 300+ tests across all crates
- **Integration Tests**: 6 comprehensive test suites with 30+ tests
- **RFC Compliance**: 100+ compliance tests across 6 RFCs
- **Benchmarks**: 4 benchmark suites with multiple scenarios
- **Fuzz Targets**: 5 fuzz testing targets
- **Load Testing**: Full-featured load testing tool

### Lines of Code
- **Test Code**: ~3,700 lines
- **Load Testing Tool**: ~1,300 lines
- **Total**: ~5,000+ lines

## CI/CD Integration

### GitHub Actions
```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run tests
        run: cargo test --all
      - name: Run benchmarks
        run: cargo bench --all

  compliance:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - name: RFC compliance tests
        run: cargo test --test 'rfc_*'

  fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz
      - name: Fuzz tests
        run: cargo fuzz run --all-targets -- -runs=10000
```

### Docker Test Environment
```yaml
version: '3.8'

services:
  rusmes-test:
    build: .
    environment:
      - RUST_LOG=debug
      - RUST_BACKTRACE=1
    command: cargo test --all

  rusmes-bench:
    build: .
    command: cargo bench --all

  rusmes-loadtest:
    build: .
    command: |
      rusmes-loadtest \
        --host rusmes-server \
        --port 25 \
        --duration 60 \
        --concurrency 10
    depends_on:
      - rusmes-server
```

## Best Practices

### Writing Tests
1. Use descriptive test names
2. Test one thing per test
3. Use fixtures for common setup
4. Mock external dependencies
5. Test error conditions
6. Test edge cases

### Running Tests
1. Run tests before committing
2. Run full suite before releases
3. Monitor test coverage
4. Keep tests fast
5. Isolate integration tests

### Maintaining Tests
1. Keep tests up to date with code
2. Remove obsolete tests
3. Refactor duplicated test code
4. Document complex test scenarios
5. Review test failures promptly

## Troubleshooting

### Slow Tests
```bash
# Run tests in parallel
cargo test -- --test-threads=8

# Skip slow tests
cargo test -- --skip slow_

# Profile tests
cargo test -- --nocapture --test-threads=1 --show-output
```

### Flaky Tests
```bash
# Run specific test multiple times
cargo test test_name -- --ignored --nocapture --test-threads=1

# Enable detailed logging
RUST_LOG=debug cargo test
```

### Memory Issues
```bash
# Limit memory
cargo test --release

# Run tests sequentially
cargo test -- --test-threads=1
```

## Future Enhancements

### Planned
1. Chaos engineering tests
2. Performance regression detection
3. Automated test generation
4. Mutation testing
5. Property-based testing with proptest
6. Contract testing for APIs
7. Snapshot testing for outputs
8. Visual regression testing for UI

### Test Coverage Goals
- 95%+ code coverage
- 100% critical path coverage
- All RFC requirements tested
- All mailets tested
- All backends tested

## Resources

### Documentation
- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [cargo-fuzz Documentation](https://rust-fuzz.github.io/book/cargo-fuzz.html)

### Related RFCs
- RFC 5321 (SMTP)
- RFC 9051 (IMAP)
- RFC 1939 (POP3)
- RFC 8620/8621 (JMAP)
- RFC 2045 (MIME)
- RFC 3464 (DSN)

## Contributing

When adding new features:
1. Write tests first (TDD)
2. Ensure tests pass
3. Add integration tests
4. Add RFC compliance tests if applicable
5. Update benchmarks
6. Add fuzz targets for parsers
7. Update this documentation

---

For questions or issues, please refer to the project documentation or open an issue on GitHub.
