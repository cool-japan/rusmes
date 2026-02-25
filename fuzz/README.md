# Fuzz Testing for Rusmes Parsers

This directory contains comprehensive fuzz tests for all parsers in the rusmes email server. Fuzzing helps discover crashes, panics, infinite loops, and other bugs by feeding parsers with random or malformed input.

## Overview

Fuzz testing is a critical part of ensuring the security and reliability of email servers. Our parsers handle untrusted input from the network, so they must be robust against all possible inputs, including malicious ones.

## Fuzz Targets

We have 6 comprehensive fuzz targets:

1. **fuzz_smtp_parser** - SMTP command parser (RFC 5321)
   - Tests all SMTP commands (HELO, EHLO, MAIL, RCPT, DATA, BDAT, etc.)
   - Tests ESMTP parameters (SIZE, BODY, SMTPUTF8, etc.)
   - Tests invalid syntax, buffer overflows, UTF-8 edge cases
   - Tests extremely long lines and special characters

2. **fuzz_imap_parser** - IMAP command parser (RFC 3501, RFC 9051)
   - Tests all IMAP commands
   - Tests literal syntax ({size} and {size+})
   - Tests quoted strings with escapes
   - Tests parenthesized lists and deep nesting
   - Tests malformed responses

3. **fuzz_mime_parser** - MIME parser (RFC 2045, RFC 5322)
   - Tests multipart boundary handling
   - Tests Content-Transfer-Encoding (base64, quoted-printable)
   - Tests nested multipart messages
   - Tests invalid headers and long header lines
   - Tests binary data in text parts

4. **fuzz_email_address** - Email address parser (RFC 5322)
   - Tests RFC 5322 address syntax
   - Tests quoted local parts
   - Tests international addresses (IDN)
   - Tests invalid syntax and edge cases
   - Tests extremely long local parts and domains

5. **fuzz_sieve_parser** - Sieve script parser (RFC 5228)
   - Tests Sieve script syntax
   - Tests nested conditions (if/elsif/else)
   - Tests string literals with escapes
   - Tests multi-line strings
   - Tests deep recursion in conditions

6. **fuzz_jmap_json** - JMAP JSON parser (RFC 8620)
   - Tests JMAP request/response JSON
   - Tests deeply nested objects
   - Tests large arrays
   - Tests invalid UTF-8 in JSON
   - Tests malformed JSON and type confusion

## Installation

First, install cargo-fuzz:

```bash
cargo install cargo-fuzz
```

Note: cargo-fuzz requires a nightly Rust toolchain:

```bash
rustup install nightly
```

## Running Fuzz Tests

### Run a single fuzz target

```bash
cd /media/kitasan/Backup/rusmes/fuzz
cargo +nightly fuzz run fuzz_smtp_parser
```

### Run with time limit (recommended)

Run for 1 hour (3600 seconds):

```bash
cargo +nightly fuzz run fuzz_smtp_parser -- -max_total_time=3600
```

### Run with iteration limit

Run for 1 million iterations:

```bash
cargo +nightly fuzz run fuzz_smtp_parser -- -runs=1000000
```

### Run all targets

```bash
#!/bin/bash
TARGETS=(
    fuzz_smtp_parser
    fuzz_imap_parser
    fuzz_mime_parser
    fuzz_email_address
    fuzz_sieve_parser
    fuzz_jmap_json
)

for target in "${TARGETS[@]}"; do
    echo "Fuzzing $target for 1 hour..."
    cargo +nightly fuzz run "$target" -- -max_total_time=3600 || true
done
```

## Understanding Results

### Successful fuzzing

If no crashes are found, you'll see output like:

```
#1000000 DONE   cov: 1234 ft: 5678 corp: 100/50Kb exec/s: 10000
```

This means:
- 1M iterations completed
- 1234 code coverage points
- 5678 features discovered
- 100 interesting inputs in corpus
- 10K executions per second

### When a crash is found

If a crash is found, cargo-fuzz will:
1. Stop execution
2. Save the crashing input to `fuzz/artifacts/<target>/crash-*`
3. Print a stack trace

Example:

```
==1234==ERROR: AddressSanitizer: heap-buffer-overflow
    #0 0x... in rusmes_smtp::parser::parse_command
```

### Reproducing crashes

To reproduce a crash:

```bash
cargo +nightly fuzz run fuzz_smtp_parser fuzz/artifacts/fuzz_smtp_parser/crash-abc123
```

## Seed Corpus

The `corpus/` directory contains seed inputs that provide good initial coverage:

- `corpus/fuzz_smtp_parser/` - Valid SMTP commands
- `corpus/fuzz_imap_parser/` - Valid IMAP commands
- `corpus/fuzz_mime_parser/` - Valid MIME messages
- `corpus/fuzz_email_address/` - Valid email addresses
- `corpus/fuzz_sieve_parser/` - Valid Sieve scripts
- `corpus/fuzz_jmap_json/` - Valid JMAP requests

You can add your own seed inputs by creating files in these directories.

## Adding New Fuzz Targets

1. Create a new file in `fuzz_targets/`:

```rust
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Call your parser here
        let _ = your_parser::parse(s);
    }
});
```

2. Add a new `[[bin]]` section to `Cargo.toml`:

```toml
[[bin]]
name = "fuzz_your_parser"
path = "fuzz_targets/fuzz_your_parser.rs"
test = false
doc = false
```

3. Create a corpus directory:

```bash
mkdir -p corpus/fuzz_your_parser
```

4. Add seed inputs to the corpus.

## Continuous Integration

### GitHub Actions

Add to `.github/workflows/fuzz.yml`:

```yaml
name: Fuzz Testing

on:
  schedule:
    - cron: '0 2 * * *'  # Run nightly at 2 AM
  workflow_dispatch:

jobs:
  fuzz:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        target:
          - fuzz_smtp_parser
          - fuzz_imap_parser
          - fuzz_mime_parser
          - fuzz_email_address
          - fuzz_sieve_parser
          - fuzz_jmap_json
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo install cargo-fuzz
      - run: cd fuzz && cargo +nightly fuzz run ${{ matrix.target }} -- -max_total_time=3600
      - uses: actions/upload-artifact@v4
        if: failure()
        with:
          name: fuzz-artifacts-${{ matrix.target }}
          path: fuzz/artifacts/
```

### Local Nightly Fuzzing

Create a script `scripts/fuzz-nightly.sh`:

```bash
#!/bin/bash
set -e

cd "$(dirname "$0")/../fuzz"

TARGETS=(
    fuzz_smtp_parser
    fuzz_imap_parser
    fuzz_mime_parser
    fuzz_email_address
    fuzz_sieve_parser
    fuzz_jmap_json
)

for target in "${TARGETS[@]}"; do
    echo "========================================="
    echo "Fuzzing $target for 1 hour..."
    echo "========================================="

    cargo +nightly fuzz run "$target" -- -max_total_time=3600 || {
        echo "CRASH FOUND in $target!"
        echo "Artifacts saved to fuzz/artifacts/$target/"
        exit 1
    }
done

echo "========================================="
echo "All fuzz targets completed successfully!"
echo "========================================="
```

## Fixing Issues Found by Fuzzing

When fuzzing finds an issue:

1. **Reproduce the crash**:
   ```bash
   cargo +nightly fuzz run fuzz_smtp_parser artifacts/fuzz_smtp_parser/crash-abc123
   ```

2. **Debug with a normal build**:
   ```bash
   # Create a test case in the appropriate crate
   #[test]
   fn test_crash_abc123() {
       let input = include_bytes!("../fuzz/artifacts/fuzz_smtp_parser/crash-abc123");
       let _ = parse_command(std::str::from_utf8(input).unwrap());
   }
   ```

3. **Fix the issue** in the parser code

4. **Verify the fix**:
   ```bash
   cargo test
   cargo +nightly fuzz run fuzz_smtp_parser artifacts/fuzz_smtp_parser/crash-abc123
   ```

5. **Add the crashing input to the corpus** (optional):
   ```bash
   cp artifacts/fuzz_smtp_parser/crash-abc123 corpus/fuzz_smtp_parser/
   ```

## Success Criteria

Target: **0 crashes after 1M iterations per target**

- No panics
- No out-of-bounds access
- No infinite loops
- No excessive memory usage
- No stack overflows

## Coverage Analysis

To see which code is being exercised:

```bash
cargo +nightly fuzz coverage fuzz_smtp_parser
cargo cov -- show fuzz/target/*/release/fuzz_smtp_parser \
    --format=html -instr-profile=fuzz/coverage/fuzz_smtp_parser/coverage.profdata \
    > coverage.html
```

## Performance Tips

1. **Use `--release` mode** (default for cargo-fuzz)
2. **Use multiple cores**: `cargo +nightly fuzz run target -- -jobs=8`
3. **Use a RAM disk** for corpus: `mount -t tmpfs -o size=1G tmpfs corpus/`
4. **Minimize corpus**: `cargo +nightly fuzz cmin target`

## Fuzzing Dictionaries

For better coverage, you can provide dictionaries of common tokens:

Create `fuzz_smtp_parser.dict`:

```
"HELO"
"EHLO"
"MAIL FROM:"
"RCPT TO:"
"DATA"
"BDAT"
"QUIT"
"<"
">"
"@"
"\r\n"
```

Use it:

```bash
cargo +nightly fuzz run fuzz_smtp_parser -- -dict=fuzz_smtp_parser.dict
```

## Resources

- [cargo-fuzz book](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [libFuzzer documentation](https://llvm.org/docs/LibFuzzer.html)
- [Fuzzing in Rust](https://rust-fuzz.github.io/book/)
- [AFL++ (alternative fuzzer)](https://github.com/AFLplusplus/AFLplusplus)

## License

Same as the main rusmes project (Apache-2.0).
