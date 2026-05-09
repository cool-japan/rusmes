# rusmes-jmap TODO

## Implemented ✅
- [x] **JMAP `convert_mail_to_email` and `make_placeholder_email` placeholder fixes** (landed 2026-05-06)
  - **Goal:** Eliminate all hardcoded/placeholder values from `convert_mail_to_email` (blob_id, received_at, mailbox_ids, keywords, thread_id) and populate `make_placeholder_email` with header-derived fields. Every JMAP `Email/get` / `Email/import` / `Email/parse` response must carry real data, not stubs.
  - **Implemented:** Introduced `EmailConversionContext<'_>` struct (blob_id/received_at/mailbox_ids/keywords/thread_id); `EmailConversionContext::placeholder` for parse-only callers. Added `compute_blob_id(bytes) -> String` (SHA-256 hex) to `blob.rs`. Added `jmap_keywords_from_flags`. Used Choice B (header-based `received_at` via `parse_received_header` + `parse_date_header`) — Choice A was deferred due to 14+ backend call-sites. Updated 3 callsites. `make_placeholder_email` now extracts all RFC 5322 header fields from raw bytes.
  - **Files:** `src/methods/email.rs`, `src/methods/email_advanced/parse.rs`, `src/methods/email_advanced/import.rs`, `src/blob.rs`
  - **Tests added (17 new, 2297 total):** `test_compute_blob_id_deterministic`, `test_compute_blob_id_differs_for_different_inputs`, `test_jmap_keywords_from_flags_canonical_mapping`, `test_jmap_keywords_from_flags_all_set`, `test_convert_mail_to_email_uses_real_blob_id`, `test_convert_mail_to_email_received_at_from_context`, `test_convert_mail_to_email_keywords_from_context`, `test_convert_mail_to_email_thread_id_present`, `test_convert_mail_to_email_mailbox_ids_multi`, `test_convert_mail_to_email_thread_id_none`, `test_placeholder_context_has_inbox`

## Remaining
