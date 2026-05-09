# rusmes-imap TODO

## Implemented ✅

- [x] **IMAP COMPRESS=DEFLATE (RFC 4978) — promote and activate** (landed 2026-05-06)
  - `oxiarc-deflate 0.2.7` `RawDeflateWriter`/`RawInflateReader` API confirmed and wired.
  - `CAPABILITY` advertises `COMPRESS=DEFLATE`; `handle_compress` in `handler.rs` sets `compress_pending`.
  - `imap_session_loop` in `server.rs` swaps reader/writer via `std::mem::replace` + sentinels.
  - Stale `#[ignore]` at `src/mailbox_registry.rs` replaced with `test_compress_deflate_roundtrip_streaming` and `test_compress_deflate_lz77_window_persists`.
  - New integration tests in `tests/compress_deflate_e2e.rs`: handler OK/NO/double-compress, transport roundtrip via duplex.

## Remaining
