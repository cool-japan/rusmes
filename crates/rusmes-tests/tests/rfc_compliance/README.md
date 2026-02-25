# RFC Compliance Test Suites

This directory contains comprehensive RFC compliance test suites for all protocols implemented in rusmes. These tests validate that the implementation strictly adheres to the relevant RFC specifications.

## Overview

The test suites cover the following protocols and RFCs:

1. **SMTP** - RFC 5321 (Simple Mail Transfer Protocol)
2. **IMAP** - RFC 9051 (Internet Message Access Protocol version 4rev2)
3. **POP3** - RFC 1939 (Post Office Protocol version 3)
4. **JMAP** - RFC 8620/8621 (JSON Meta Application Protocol)
5. **MIME** - RFC 2045/5322 (Multipurpose Internet Mail Extensions)
6. **DSN** - RFC 3464 (Delivery Status Notifications)

## Test Files

### smtp_rfc5321.rs

Comprehensive SMTP protocol compliance tests covering:

- **Commands**: HELO, EHLO, MAIL FROM, RCPT TO, DATA, QUIT, RSET, VRFY, EXPN, HELP, NOOP
- **Response Codes**: 2xx (success), 3xx (intermediate), 4xx (transient failure), 5xx (permanent failure)
- **Command Syntax**: Validation of all command formats and parameters
- **Line Length Limits**: 512 octets maximum (RFC 5321 Section 4.5.3.1.6)
- **Pipelining**: RFC 2920 compliance
- **Error Handling**: Invalid commands, malformed syntax
- **Edge Cases**: Null sender, postmaster special cases, dot-stuffing
- **Extensions**: SIZE, 8BITMIME, DSN parameters

**Key Test Categories**:
- Command format validation (case insensitivity, parameter parsing)
- Email address validation (local-part and domain-part)
- Path syntax (reverse-path and forward-path)
- Multi-line response handling
- State machine validation (EHLO → MAIL → RCPT → DATA → QUIT)

### imap_rfc9051.rs

IMAP4rev2 torture tests covering:

- **All Commands**: CAPABILITY, NOOP, LOGOUT, LOGIN, SELECT, EXAMINE, CREATE, DELETE, RENAME, SUBSCRIBE, UNSUBSCRIBE, LIST, LSUB, STATUS, APPEND, CHECK, CLOSE, EXPUNGE, SEARCH, FETCH, STORE, COPY, UID
- **Malformed Commands**: Missing tags, invalid syntax, extra arguments
- **Literal Handling**: Synchronizing literals `{size}` and non-synchronizing literals `{size+}`
- **Boundary Conditions**: Empty strings, maximum lengths, special characters
- **Quote Handling**: Quoted strings, escaped quotes, quoted-printable
- **Flags**: System flags (`\Seen`, `\Answered`, etc.) and custom flags
- **Search Criteria**: ALL, ANSWERED, BCC, BEFORE, BODY, CC, DELETED, DRAFT, FLAGGED, FROM, HEADER, KEYWORD, LARGER, NEW, NOT, OLD, ON, OR, RECENT, SEEN, SENTBEFORE, SENTON, SENTSINCE, SINCE, SMALLER, SUBJECT, TEXT, TO, UID, UNANSWERED, UNDELETED, UNDRAFT, UNFLAGGED, UNKEYWORD, UNSEEN

**Key Test Categories**:
- Command format (tag + command + arguments)
- Response format (tagged, untagged, continuation)
- Message sequence numbers (single, range, wildcard)
- Mailbox names and hierarchy delimiters
- Date format (dd-Mon-yyyy)
- IDLE and ENABLE extensions

### pop3_rfc1939.rs

POP3 protocol compliance tests covering:

- **All Commands**: USER, PASS, STAT, LIST, RETR, DELE, NOOP, RSET, QUIT, TOP, UIDL, APOP
- **State Transitions**: AUTHORIZATION → TRANSACTION → UPDATE
- **Response Format**: +OK/-ERR responses
- **Multi-line Responses**: Byte-stuffing, termination with `.`
- **Message Numbers**: Validation of message numbering
- **Octets Format**: Byte count validation

**Key Test Categories**:
- Command sequences and state transitions
- Response format (+OK, -ERR)
- Multi-line response handling
- Optional commands (TOP, UIDL, APOP)
- Timeout requirements
- Error cases (no such message, invalid password, locked mailbox)

### jmap_rfc8620.rs

JMAP protocol conformance tests covering:

- **Core Methods**: Session, capabilities, standard methods
- **Email Methods**: Email/get, Email/set, Email/query, Email/changes
- **Mailbox Methods**: Mailbox/get, Mailbox/set, Mailbox/query, Mailbox/changes
- **Thread Methods**: Thread/get, Thread/changes
- **Identity Methods**: Identity/get, Identity/set
- **Submission Methods**: EmailSubmission/set, EmailSubmission/get
- **VacationResponse**: VacationResponse/get, VacationResponse/set
- **SearchSnippet**: SearchSnippet/get

**Key Test Categories**:
- Request/response structure (JSON format validation)
- Method call format [name, arguments, callId]
- Error handling (invalidArguments, notFound, serverFail, etc.)
- State tracking (sessionState, ifInState)
- Batch operations and result references
- Blob upload/download
- Filter operators (AND, OR, NOT)
- Sort comparators
- Pagination (position, limit, anchor, anchorOffset)

### mime_rfc2045.rs

MIME parsing compliance tests covering:

- **Multipart Messages**: multipart/mixed, multipart/alternative, multipart/digest, multipart/parallel
- **Content-Transfer-Encoding**: 7bit, 8bit, binary, quoted-printable, base64
- **Content-Type Parsing**: Type/subtype, parameters, charset
- **Boundary Handling**: Delimiter format, closing delimiter
- **Nested Multipart**: Recursive multipart structure
- **Invalid MIME Structures**: Missing boundaries, malformed headers
- **Header Folding**: RFC 5322 folding/unfolding
- **Address Parsing**: Individual addresses, address lists, groups

**Key Test Categories**:
- MIME-Version validation
- Content-Type header parsing (type, subtype, parameters)
- Boundary validation (max 70 characters, no leading/trailing spaces)
- Content-Transfer-Encoding validation
- Content-Disposition (inline, attachment, filename)
- Quoted-printable encoding (soft line breaks, special characters)
- Base64 encoding (padding, line length)
- Message structure (text, image, application, message types)
- Charset parameter extraction

### dsn_rfc3464.rs

Delivery Status Notification format validation tests covering:

- **DSN Structure**: multipart/report with three parts
  1. Human-readable text/plain
  2. Machine-readable message/delivery-status
  3. Original message (message/rfc822 or text/rfc822-headers)
- **Per-Message Fields**: Reporting-MTA, DSN-Gateway, Received-From-MTA, Arrival-Date
- **Per-Recipient Fields**: Original-Recipient, Final-Recipient, Action, Status, Remote-MTA, Diagnostic-Code, Last-Attempt-Date, Will-Retry-Until
- **Action Values**: failed, delayed, delivered, relayed, expanded
- **Status Codes**: RFC 3463 enhanced status codes (class.subject.detail)

**Key Test Categories**:
- Content-Type validation (multipart/report; report-type=delivery-status)
- DSN structure validation (3-part structure)
- Action field validation (5 standard actions)
- Status code format (X.Y.Z where X=2/4/5)
- Status code classes (2.x.x=success, 4.x.x=transient, 5.x.x=permanent)
- Field format validation (type; value)
- Common status codes (5.1.1, 5.2.2, 4.2.1, etc.)

## Running the Tests

Run all RFC compliance tests:

```bash
cargo test --test rfc_compliance
```

Run tests for a specific protocol:

```bash
# SMTP tests
cargo test --test rfc_compliance::smtp_rfc5321

# IMAP tests
cargo test --test rfc_compliance::imap_rfc9051

# POP3 tests
cargo test --test rfc_compliance::pop3_rfc1939

# JMAP tests
cargo test --test rfc_compliance::jmap_rfc8620

# MIME tests
cargo test --test rfc_compliance::mime_rfc2045

# DSN tests
cargo test --test rfc_compliance::dsn_rfc3464
```

Run with verbose output:

```bash
cargo test --test rfc_compliance -- --nocapture
```

## Test Strategy

### Positive Tests
- Valid command formats
- Correct response codes
- Proper syntax for all protocol elements
- Edge cases from RFC appendices

### Negative Tests
- Invalid commands
- Malformed syntax
- Missing required parameters
- Out-of-sequence commands
- Invalid state transitions

### Boundary Tests
- Maximum line lengths
- Empty values
- Special characters
- Unicode handling
- Case sensitivity/insensitivity

### RFC Errata
Tests include cases from published RFC errata to ensure compliance with corrections and clarifications.

## Coverage Goals

- **Command Coverage**: 100% of all commands defined in each RFC
- **Response Coverage**: All defined response codes and formats
- **Error Coverage**: All error conditions specified in RFCs
- **Edge Cases**: All examples and edge cases from RFC appendices
- **Extensions**: Common protocol extensions (PIPELINING, LITERAL+, etc.)

## Test Vectors

Test vectors are derived from:
- RFC appendices (example sessions)
- RFC errata (corrections and clarifications)
- Real-world protocol traces
- Known edge cases and corner cases
- Invalid inputs for robustness testing

## Assertion Strategy

### Exact Matching
- Protocol commands must match exactly (case-insensitive)
- Response codes must match expected values
- Syntax must be strictly validated

### Field-by-Field Validation
- Structured data (JMAP JSON, DSN fields) validated field-by-field
- Required fields must be present
- Optional fields validated when present

### State Transition Validation
- Command sequences must follow state machines
- Invalid transitions must be rejected
- State must be properly maintained

### Error Code Validation
- Error responses must use correct codes
- Error messages should be informative
- Error types must match RFC definitions

## Continuous Integration

These tests run automatically on:
- Every commit to main branch
- All pull requests
- Nightly builds
- Release candidates

## Contributing

When adding new RFC compliance tests:

1. Reference the specific RFC section being tested
2. Include both positive and negative test cases
3. Test boundary conditions
4. Add comments explaining complex test cases
5. Use descriptive test names
6. Group related tests in submodules

## References

- [RFC 5321](https://www.rfc-editor.org/rfc/rfc5321.html) - Simple Mail Transfer Protocol
- [RFC 9051](https://www.rfc-editor.org/rfc/rfc9051.html) - Internet Message Access Protocol (IMAP) - Version 4rev2
- [RFC 1939](https://www.rfc-editor.org/rfc/rfc1939.html) - Post Office Protocol - Version 3
- [RFC 8620](https://www.rfc-editor.org/rfc/rfc8620.html) - The JSON Meta Application Protocol (JMAP)
- [RFC 8621](https://www.rfc-editor.org/rfc/rfc8621.html) - The JSON Meta Application Protocol (JMAP) for Mail
- [RFC 2045](https://www.rfc-editor.org/rfc/rfc2045.html) - Multipurpose Internet Mail Extensions (MIME) Part One
- [RFC 2046](https://www.rfc-editor.org/rfc/rfc2046.html) - MIME Part Two: Media Types
- [RFC 5322](https://www.rfc-editor.org/rfc/rfc5322.html) - Internet Message Format
- [RFC 3464](https://www.rfc-editor.org/rfc/rfc3464.html) - An Extensible Message Format for Delivery Status Notifications
- [RFC 3463](https://www.rfc-editor.org/rfc/rfc3463.html) - Enhanced Mail System Status Codes
- [RFC 2920](https://www.rfc-editor.org/rfc/rfc2920.html) - SMTP Service Extension for Command Pipelining
- [RFC 7162](https://www.rfc-editor.org/rfc/rfc7162.html) - IMAP Extensions: Quick Flag Changes Resynchronization (CONDSTORE) and Quick Mailbox Resynchronization (QRESYNC)
- [RFC 7888](https://www.rfc-editor.org/rfc/rfc7888.html) - IMAP4 Non-synchronizing Literals
- [RFC 6152](https://www.rfc-editor.org/rfc/rfc6152.html) - SMTP Service Extension for 8-bit MIME Transport

## License

These tests are part of the rusmes project and are licensed under the same terms (Apache-2.0).
