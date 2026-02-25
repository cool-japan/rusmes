# rusmes-proto TODO

## Implemented ✅
- [x] `MailAddress` with validation, `Domain`, `Username` types
- [x] `Mail` struct with envelope, state machine, `MessageId` (UUID)
- [x] `MailState` enum (Root/Transport/LocalDelivery/Error/Ghost)
- [x] `MessageBody` (Small/Large), `MessageHeaders`, `MimeMessage`
- [x] MIME multipart parsing (RFC 2045, 613 lines)
- [x] Header folding/unfolding per RFC 5322
- [x] Content-Transfer-Encoding decoding (base64, quoted-printable)
- [x] Error types: InvalidAddress, Parse, MessageTooLarge, etc.
- [x] `Ord` implementation for `MailAddress`

## Remaining
- [ ] `MessageBody::Large` streaming implementation (AsyncRead-based for >100MB)
- [ ] Internationalized email address support (RFC 6531 / SMTPUTF8) — parser-level
- [ ] Message size calculation including headers
- [ ] `MailAddress::is_local()` helper with domain list parameter