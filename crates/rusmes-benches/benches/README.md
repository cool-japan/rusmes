# Rusmes Performance Benchmarks

This directory contains comprehensive performance benchmarks for the Rusmes mail server using Criterion.rs.

## Benchmark Categories

### 1. Throughput (`throughput.rs`)
**Target: >50,000 msg/sec**

Measures messages per second throughput for:
- SMTP ingest rate (parallel connections)
- IMAP fetch rate (concurrent users)
- Queue processing rate
- Batch message ingestion
- Message size impact (1KB - 10MB)

### 2. Concurrent Connections (`connections.rs`)
**Target: 10,000+ concurrent IMAP connections**

Benchmarks connection handling:
- Connection establishment latency
- Memory per connection (<10KB target)
- Connection pool performance
- Cleanup after disconnect
- Max concurrent connections before degradation

### 3. Search Performance (`search.rs`)
**Target: <50ms p95 search query latency**

Tests full-text search capabilities:
- Index build performance (100 - 100K messages)
- Simple search (single term)
- Complex search (multiple criteria with AND/OR)
- Index size vs message count (scalability)
- Index update performance
- Search result pagination

### 4. Mailet Pipeline Latency (`mailets.rs`)
**Target: <50ms avg latency**

Measures mail processing pipeline:
- DKIM verification overhead
- SPF verification overhead
- DMARC verification overhead
- ClamAV scanning latency (simulated)
- SpamAssassin checking latency (simulated)
- Sieve script execution time
- Total pipeline latency (all mailets)

### 5. Protocol Parsing (`parsing.rs`)

Benchmarks parsing performance:
- SMTP command parsing
- IMAP command parsing (with literals)
- MIME message parsing
- Email address parsing
- Header parsing
- JSON parsing (JMAP requests)

### 6. Storage Operations (`storage.rs`)

Tests storage backend performance:
- Message append (1KB - 10MB)
- Message retrieval by UID
- Mailbox listing
- Flag updates
- Message copy
- Message delete
- Batch operations

### 7. Authentication (`auth.rs`)

Benchmarks authentication methods:
- bcrypt verification (various costs)
- LDAP bind (simulated)
- SQL query (simulated)
- OAuth2 token validation (simulated)
- Concurrent authentication
- Authentication cache performance

## Running Benchmarks

### Run All Benchmarks
```bash
cargo bench --workspace
```

### Run Specific Benchmark
```bash
cargo bench --bench throughput
cargo bench --bench connections
cargo bench --bench search
cargo bench --bench mailets
cargo bench --bench parsing
cargo bench --bench storage
cargo bench --bench auth
```

### Run Specific Test Within Benchmark
```bash
cargo bench --bench throughput -- smtp_ingest
cargo bench --bench search -- simple_search
```

## Benchmark Output

Criterion generates HTML reports in:
```
target/criterion/
```

Open `target/criterion/report/index.html` in your browser to view detailed results.

## Performance Targets

| Category | Metric | Target |
|----------|--------|--------|
| SMTP Ingest | Throughput | >50,000 msg/sec |
| IMAP Fetch | Throughput | >10,000 msg/sec |
| Search Query | Latency (p95) | <50ms |
| Message Processing | Avg Latency | <50ms |
| Concurrent Connections | Count | 10,000+ |
| Memory per Connection | Usage | <10KB |
| Storage Append | Latency | <10ms |
| Storage Retrieval | Latency | <5ms |

## CI Integration

Benchmarks are integrated into the CI/CD pipeline:
- Run on every release tag
- Track performance regression
- Store historical results
- Alert on significant degradation

## Benchmark Configuration

All benchmarks use:
- **Measurement time**: 10 seconds per test
- **Warm-up time**: 3 seconds
- **Sample size**: Auto-determined by Criterion
- **Confidence level**: 95%

## Comparing Results

To compare with a baseline:
```bash
# Save current results as baseline
cargo bench --bench throughput -- --save-baseline main

# Make changes, then compare
cargo bench --bench throughput -- --baseline main
```

## Hardware Specifications

For reproducible benchmarks, document your hardware:
- CPU: AMD Ryzen / Intel Core (specify model)
- RAM: 16GB+ recommended
- Storage: SSD recommended
- OS: Linux kernel 5.x+

## Performance Tuning Recommendations

Based on benchmark results:

1. **Connection Handling**
   - Use connection pooling
   - Implement backpressure for >10K connections
   - Consider using io_uring for improved I/O

2. **Message Processing**
   - Parallelize mailet execution where possible
   - Cache DKIM/SPF/DMARC lookups
   - Use async I/O for external services

3. **Search Performance**
   - Batch index updates
   - Use commit buffering
   - Consider index sharding for >1M messages

4. **Storage Optimization**
   - Batch write operations
   - Use memory-mapped files for hot data
   - Implement LRU caching for frequently accessed messages

## Comparison with Apache JAMES

See [RESULTS.md](RESULTS.md) for detailed performance comparisons.

## Contributing

When adding new benchmarks:
1. Follow existing benchmark structure
2. Use realistic test data
3. Document performance targets
4. Add to this README
5. Update RESULTS.md with baseline results

## License

Apache-2.0
