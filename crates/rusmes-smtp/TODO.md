# rusmes-smtp TODO

## Implemented ✅
- [x] nom-based SMTP command parser (HELO/EHLO/MAIL/RCPT/DATA/BDAT/AUTH/STARTTLS etc.)
- [x] Full session state machine (Initial → Connected → Authenticated → MailTransaction → Data → Quit)
- [x] SMTP response builder
- [x] Async server with TLS (rustls) and rate limiter integration
- [x] PIPELINING (RFC 2920), 8BITMIME (RFC 6152), SMTPUTF8 (RFC 6531)
- [x] SIZE (RFC 1870) — declared and enforced
- [x] DSN (RFC 3461) — delivery status notifications (586 lines)
- [x] BDAT/CHUNKING (RFC 3030) — binary data transfer (769 lines)
- [x] AUTH: PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256
- [x] Relay authorization based on `relay_networks` config
- [x] Recipient validation against storage backend
- [x] Submission server (port 587, mandatory STARTTLS + AUTH)
- [x] Connection timeout enforcement

## Remaining
- [-] SCRAM-SHA-256: salt/iteration count are placeholder values — needs real credential storage
- [ ] Mutual TLS (client certificate verification)
- [ ] Reject connections from blocked IPs (configuration-driven)
- [ ] Max connections per IP enforcement
- [ ] Idle connection reaping
- [ ] Outbound connection pooling for remote delivery
- [ ] Per-session structured logging (session ID, client IP)
- [ ] Per-event metrics recording (connect, auth, message, error)