# rusmes-pop3 TODO

## Implemented ✅
### Core Commands (RFC 1939)
- [x] USER / PASS authentication
- [x] APOP authentication (MD5 challenge-response)
- [x] STAT, LIST, RETR, DELE, NOOP, RSET, QUIT
- [x] TOP (headers + N lines), UIDL (unique ID listing)

### Extensions
- [x] STLS (RFC 2595) — STARTTLS upgrade
- [x] CAPA (RFC 2449) — capability negotiation

### Server
- [x] TCP listener with session dispatch
- [x] Session state machine (Authorization → Transaction → Update)
- [x] Command parser + response builder
- [x] Integration with rusmes-storage

## Remaining
- [ ] Maildrop locking (prevent concurrent POP3 sessions for same user)
- [ ] AUTH (RFC 1734) — SASL authentication for POP3