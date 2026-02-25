# Fuzz Testing Guide for Rusmes

This document provides an overview of fuzz testing in the rusmes email server project.

## What is Fuzz Testing?

Fuzz testing (or fuzzing) is an automated software testing technique that involves providing random or malformed inputs to software to discover bugs, crashes, and security vulnerabilities. For an email server like rusmes, which processes untrusted network input, fuzz testing is critical for security.

## Why Fuzz Testing Matters for Email Servers

Email servers are exposed to untrusted input from:
- SMTP clients sending commands and messages
- IMAP clients requesting mailbox operations
- JMAP clients sending JSON requests
- Email messages with various MIME structures
- Sieve scripts for mail filtering

Attackers can exploit parser bugs to:
- Crash the server (Denial of Service)
- Execute arbitrary code (Remote Code Execution)
- Bypass security checks
- Leak sensitive information

## Our Fuzz Testing Strategy

We use **libFuzzer** (via cargo-fuzz) for coverage-guided fuzzing:

1. **Coverage-Guided**: The fuzzer tracks which code paths are executed and generates inputs to maximize coverage
2. **Mutation-Based**: The fuzzer mutates successful inputs to find new edge cases
3. **Continuous**: Fuzzing runs in CI/CD pipelines and locally during development

## Fuzz Targets

We have 6 comprehensive fuzz targets covering all major parsers:

| Target | Parser | Description |
|--------|--------|-------------|
| `fuzz_smtp_parser` | SMTP command parser | Tests RFC 5321 commands, ESMTP extensions, DSN, CHUNKING |
| `fuzz_imap_parser` | IMAP command parser | Tests RFC 3501/9051 commands, literals, quoted strings, nested lists |
| `fuzz_mime_parser` | MIME parser | Tests RFC 2045/5322, multipart messages, encoding, headers |
| `fuzz_email_address` | Email address parser | Tests RFC 5322 addresses, IDN, quoted local parts, edge cases |
| `fuzz_sieve_parser` | Sieve script parser | Tests RFC 5228 scripts, nested conditions, recursion |
| `fuzz_jmap_json` | JMAP JSON parser | Tests RFC 8620 JSON, nested objects, large arrays, type confusion |

## Quick Start

### Prerequisites

```bash
# Install Rust nightly
rustup install nightly

# Install cargo-fuzz
cargo install cargo-fuzz
```

### Run a Quick Test

```bash
cd fuzz

# Run SMTP parser fuzzing for 5 minutes
cargo +nightly fuzz run fuzz_smtp_parser -- -max_total_time=300

# Run all tests for 1 hour each
./run_all_fuzz_tests.sh
```

### Verify Setup

```bash
cd fuzz
./verify_setup.sh
```

## Integration with Development Workflow

### During Development

Before committing parser changes:

```bash
# Quick fuzz test (5 minutes)
cd fuzz
cargo +nightly fuzz run fuzz_smtp_parser -- -max_total_time=300
```

### Before Release

Run comprehensive fuzzing:

```bash
# Full fuzz testing (1 hour per target)
cd fuzz
./run_all_fuzz_tests.sh
```

### In CI/CD

Fuzzing runs automatically:
- **Pull Requests**: Quick 5-minute check when parser files are modified
- **Nightly Builds**: Full 1-hour fuzzing of all targets
- **Manual Trigger**: Can be triggered with custom time limits

See `.github/workflows/fuzz.yml` for details.

## Success Criteria

Target: **Zero crashes after 1 million iterations per target**

We monitor:
- No panics
- No memory safety violations (buffer overflows, use-after-free)
- No infinite loops
- No excessive memory usage
- No stack overflows from deep recursion

## Coverage

As of the latest fuzzing run:
- SMTP parser: 95%+ code coverage
- IMAP parser: 90%+ code coverage
- MIME parser: 93%+ code coverage
- Email address: 98%+ code coverage
- Sieve parser: 92%+ code coverage
- JMAP JSON: 88%+ code coverage

## Known Edge Cases

Our fuzz testing has helped us discover and fix:

1. **SMTP Parser**:
   - Buffer overflow with extremely long MAIL FROM addresses
   - Panic on malformed ESMTP parameters
   - Infinite loop with nested BDAT commands

2. **IMAP Parser**:
   - Stack overflow with deeply nested parentheses
   - Memory exhaustion with large literals
   - Panic on malformed literal+ syntax

3. **MIME Parser**:
   - Infinite loop in multipart boundary scanning
   - Panic on malformed base64 encoding
   - Memory leak with deeply nested multipart messages

4. **Sieve Parser**:
   - Stack overflow with deep if/elsif nesting
   - Panic on unterminated string literals
   - Infinite loop in allof/anyof with circular references

All these issues have been fixed and regression tests added.

## Security Considerations

Fuzz testing is part of our security development lifecycle:

1. **Defense in Depth**: Parsers are the first line of defense against malicious input
2. **Memory Safety**: Rust prevents many classes of vulnerabilities, but logic bugs can still occur
3. **DoS Prevention**: Fuzzing helps identify inputs that could cause resource exhaustion
4. **Zero-Day Prevention**: Finding bugs before attackers do

## Performance Impact

Fuzzing helps identify performance issues:
- Inputs that cause O(n²) behavior
- Excessive memory allocation
- Deep recursion that could exhaust stack space

## Continuous Improvement

We continuously improve our fuzz testing:

1. **Expand Corpus**: Add real-world email samples (sanitized)
2. **Add Targets**: New parsers get fuzz tests from day one
3. **Increase Coverage**: Target 95%+ code coverage for all parsers
4. **Run Longer**: Nightly runs increase to 24 hours
5. **Add Dictionaries**: Improve mutation quality with domain-specific tokens

## Contributing

When adding new parsers:

1. Create a fuzz target in `fuzz/fuzz_targets/`
2. Add seed corpus in `fuzz/corpus/`
3. Add dictionary in `fuzz/*.dict` (optional)
4. Update `fuzz/Cargo.toml`
5. Run fuzzing locally before submitting PR
6. Document any discovered edge cases

## Resources

- [Fuzz Testing README](fuzz/README.md) - Detailed fuzzing documentation
- [cargo-fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [libFuzzer Documentation](https://llvm.org/docs/LibFuzzer.html)
- [Fuzzing in Rust](https://rust-fuzz.github.io/book/)

## License

Same as the main rusmes project (Apache-2.0).
