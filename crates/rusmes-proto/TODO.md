# rusmes-proto TODO

## Implemented ✅
- [x] `MailAddress` with validation, `Domain`, `Username` types
- [x] `MailAddress::is_local(&HashSet<String>) -> bool` — case-insensitive,
      IDN-normalized via `idna::domain_to_ascii` (RFC 3490)
- [x] `Mail` struct with envelope, state machine, `MessageId` (UUID)
- [x] `MailState` enum (Root/Transport/LocalDelivery/Error/Ghost)
- [x] `MessageBody` (Small/Large), `MessageHeaders`, `MimeMessage`
- [x] `MimeMessage::size_with_headers()` / `body_size()` split for
      RFC-correct SMTP `SIZE` / IMAP `RFC822.SIZE` / quota accounting
- [x] MIME multipart parsing (RFC 2045, 613 lines)
- [x] Header folding/unfolding per RFC 5322
- [x] Content-Transfer-Encoding decoding (base64, quoted-printable)
- [x] Error types: InvalidAddress, Parse, MessageTooLarge, etc.
- [x] `Ord` implementation for `MailAddress`

## Remaining
- [x] `MessageBody::Large` streaming implementation (AsyncRead-based for >100MB) (landed 2026-05-05)
  - **Goal:** `MessageBody::Large` becomes a fully pollable streaming body type backed by an async reader, enabling correct handling of messages > 100 MB without loading them into memory.
  - **Design:** Replaced `Large(Arc<dyn AsyncRead + Send + Sync>)` with `Large(LargeBody)` where `LargeBody { reader: Arc<tokio::sync::Mutex<Pin<Box<dyn AsyncRead + Send + Sync>>>>, size: u64, digest: Option<[u8; 32]> }`. Added factory methods `LargeBody::from_path(path) -> Result<Self>` and `LargeBody::from_reader(r, size) -> Self`. Converted `extract_text()`, `decode_body()`, `parse_multipart()` to `async fn`. Added `MessageBody::from_path_with_threshold(path, threshold) -> Result<Self>`. Filled every `Large(_) => Err(...)` / unimplemented match arm across the workspace with a real streaming implementation.
  - **Files:** `crates/rusmes-proto/src/message.rs`, `crates/rusmes-storage/src/backends/filesystem/message_helpers.rs`, `crates/rusmes-storage/src/backends/filesystem/mod.rs`, `crates/rusmes-search/src/lib.rs`, `crates/rusmes-imap/src/handler_message.rs`, `crates/rusmes-jmap/src/methods/email.rs`, `crates/rusmes-jmap/src/methods/email_advanced.rs`, `crates/rusmes-pop3/src/session.rs`, `crates/rusmes-smtp/src/transport.rs`, `crates/rusmes-core/src/mailets/remote_delivery.rs`, `crates/rusmes-core/src/mailets/spam_assassin.rs`, `crates/rusmes-core/src/mailets/virus_scan.rs`
  - **Tests added:** `test_largebody_from_path_roundtrip`, `test_messagebody_threshold_chooses_small`, `test_messagebody_threshold_chooses_large`, `test_largebody_extract_text` (all in `rusmes-proto::message::large_body_tests`).
- [x] Internationalized email address support (RFC 6531 / SMTPUTF8) — parser-level (landed 2026-05-05)
  - **Goal:** `MailAddress` accepts non-ASCII UTF-8 local-parts when explicitly created via a SMTPUTF8-aware constructor, while `FromStr` / `new` remain ASCII-only by default (preserving existing behavior).
  - **Design:** Add `MailError::NonAsciiLocalPartRequiresSMTPUTF8`. `MailAddress::new` rejects bytes ≥ 0x80 in local-part with this error. New `MailAddress::new_smtputf8(local, domain)` and `MailAddress::from_str_smtputf8(s)` accept UTF-8 local-parts (still enforcing 64-byte octet limit per RFC 5321 §4.5.3.1.1 and rejecting C0/C1 controls). SMTP parser (`crates/rusmes-smtp/src/parser.rs`) exposes `parse_command_smtputf8(input, bool)` and threads the flag through `mail_command`/`rcpt_command`; `session.rs` sets `ehlo_used` on EHLO and activates SMTPUTF8-mode parsing when the client used EHLO (RFC 6531 §3).
  - **Files:** `crates/rusmes-proto/src/address.rs`, `crates/rusmes-proto/src/error.rs`, `crates/rusmes-smtp/src/parser.rs`, `crates/rusmes-smtp/src/session.rs`
  - **Tests added:** `test_mailaddress_ascii_only_rejects_unicode`, `test_mailaddress_smtputf8_roundtrip`, `test_mailaddress_64_byte_limit_unicode`, `test_mailaddress_smtputf8_rejects_control_chars`, `test_mailaddress_new_rejects_control_chars`, `test_parse_mail_from_ascii_rejects_unicode`, `test_parse_mail_from_smtputf8_accepts_unicode`, `test_parse_mail_from_smtputf8_false_rejects_unicode`, `test_parse_mail_from_smtputf8_with_param`, `test_parse_rcpt_to_smtputf8_accepts_unicode`, `test_ehlo_advertises_smtputf8`, `test_smtputf8_requires_ehlo_not_helo`.
  - **File size check:** session.rs at 1,910 lines (< 2000 — no split required).
- [x] Message size calculation including headers (landed 2026-05-03)
  - `MimeMessage::size_with_headers()` returns the canonical on-wire byte count
    (header block + final CRLF + body); `MimeMessage::body_size()` preserves
    body-only semantics; `MimeMessage::size()` is now an alias for
    `size_with_headers()` so storage / IMAP `RFC822.SIZE` / JMAP `Email/get` /
    quota accounting all report the correct value without further audits.
    `Mail::size()` and a new `Mail::body_size()` mirror the same split.
- [x] `MailAddress::is_local()` helper with domain list parameter (landed 2026-05-03)
  - `MailAddress::is_local(&HashSet<String>) -> bool` performs case-insensitive,
    IDN-normalized domain comparison via `idna::domain_to_ascii` (RFC 3490).
    Punycode and Unicode entries are interchangeable in the supplied set.

## Proposed follow-ups

- [x] SMTP DATA threshold path: if message > 1 MiB, write to temp file and emit `Large(LargeBody::from_path(...))` — implemented 2026-05-05.
  - **Goal:** When SMTP DATA payload exceeds a configurable threshold (default 1 MiB), spill to a tempfile and emit `MessageBody::Large(LargeBody::from_path(...))` instead of buffering in memory. Below threshold, behavior unchanged.
  - **Design:** Hybrid sink in `handle_data_input()` (extracted to `session/data.rs` after `session.rs` split). Buffer in `Vec<u8>` below threshold; on crossover, create `tempfile::NamedTempFile`, flush buffer, continue appending. On `.<CRLF>`: build `MessageBody::Bytes` or `MessageBody::Large` accordingly. Config field `SmtpConfig::data_tempfile_threshold: usize` default 1 MiB. (planned 2026-05-05)
  - **Files:** `crates/rusmes-smtp/src/session.rs` → `session/mod.rs` + `session/tests.rs` + `session/data.rs`; `crates/rusmes-config/src/...` (SmtpConfig); `crates/rusmes-smtp/Cargo.toml` (tempfile dep)
  - **Tests:** `test_data_input_spills_above_threshold`, `test_data_input_stays_in_memory_below_threshold`, `test_data_input_threshold_boundary`
