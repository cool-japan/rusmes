# rusmes-acme TODO

## Remaining

## Implemented

- [x] JWK thumbprint (RFC 7638) — implement proper SHA-256 thumbprint for ACME account key (done 2026-05-03)
  - **Goal:** Implement RFC 7638 JWK thumbprint properly so that real Let's Encrypt issuance succeeds. Thumbprint must be the SHA-256 hash (URL-safe base64, no padding) of the canonical JWK JSON: required members in lexicographic order, no whitespace, RSA: `e/kty/n`; EC: `crv/kty/x/y`.
  - **Files:** `src/jwk.rs` (new — `pub enum Jwk`, `pub fn jwk_thumbprint(jwk: &Jwk) -> String`; canonical JSON built manually with fixed field order per `kty`; hashed with `sha2::Sha256`; encoded with `base64::engine::general_purpose::URL_SAFE_NO_PAD`), `src/lib.rs` (module registration + re-export), `Cargo.toml` (`sha2 = { workspace = true }`).
  - **Tests:** `jwk_thumbprint_rsa_rfc7638_appendix_a1` (RFC 7638 §3.1 vector — `NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs`), `jwk_thumbprint_ec_p256` (43-char URL-safe base64, no padding, correct alphabet), `jwk_thumbprint_canonical_ordering` (lexicographic order assertion for RSA), `jwk_thumbprint_no_padding` (no `=` in RSA or EC output).
  - **Result:** 74/74 tests pass; clippy clean (0 warnings).
