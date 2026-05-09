# rusmes-auth TODO

## Implemented ✅
### Trait & User Management
- [x] `AuthBackend` trait: authenticate, verify_identity, list/create/delete users, change_password
- [x] Optional SCRAM/APOP credential methods

### Backends
- [x] File backend (bcrypt **and** argon2id password hashing, auto-detected by PHC prefix; SCRAM-SHA-256 credential bundle storage)
- [x] LDAP backend (bind authentication, user search, 802 lines)
- [x] SQL backend (PostgreSQL/MySQL/SQLite, configurable queries, 1,154 lines)
- [x] OAuth2/OIDC backend (token introspection, JWKS validation, 1,469 lines)
- [x] PAM backend (Linux Pluggable Authentication Modules, feature flag)

### SASL Framework (1,495 lines)
- [x] PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2
- [x] `verify_bearer_token()` on AuthBackend trait (default returns Unauthorized); OAuth2Backend override dispatching to internal JWT verifier; fixed JWT base64 double-encoding in validate_jwt (landed 2026-05-05)

### Security (885 lines)
- [x] Brute force protection (account lockout after N failures)
- [x] Password strength validation (entropy calculation)
- [x] Audit logging (authentication success/failure events)
- [x] Rate limiting on auth attempts per IP

## Remaining
- [x] **Server integration**: `AuthBackendKind` config-driven factory + `fetch_scram_credentials` trait method + file-backend SCRAM impl landed (2026-05-05)
  - `AuthBackendKind { File(FileBackendConfig), Sql, Ldap, OAuth2 }` enum with `async fn build(self) -> Arc<dyn AuthBackend>` in `src/lib.rs`
  - `ScramCredentials { salt, iteration_count, stored_key, server_key }` struct in `src/lib.rs`
  - `fetch_scram_credentials(&self, user) -> Result<Option<ScramCredentials>>` on `AuthBackend` trait (default: `Ok(None)`)
  - File backend (`src/file.rs`) overrides with real impl: parses RFC 5802 SCRAM bundle from tab-separated passwd format; `set_scram_credentials` for admin writes
  - LDAP, SQL, OAuth2 inherit `Ok(None)` default — no code changes needed in those backends
  - `rusmes-server/src/bootstrap.rs` already uses `AuthBackendKind::build` via `auth_kind_from_config` — no hardcoded DummyAuthBackend fallback present
  - Tests: `auth_backend_kind_build_file`, `file_backend_scram_credentials_roundtrip`, `default_fetch_scram_credentials_returns_none`, `argon2_roundtrip_and_bcrypt_compat` (250 passed, 0 failures)
- [x] Argon2 password hashing support for the file backend (landed 2026-05-03)
  - File backend supports both bcrypt and argon2id; algorithm dispatch is by
    PHC prefix on read (`$2y$…` / `$2b$…` / `$2a$…` → bcrypt;
    `$argon2id$…` / `$argon2i$…` / `$argon2d$…` → argon2). New password writes
    use whichever algorithm is selected via `FileBackendConfig.hash_algorithm`
    (or `FileAuthBackend::with_algorithm`). The on-disk
    `[auth.file.hash_algorithm]` config accepts `"bcrypt"` (default) or
    `"argon2"` / `"argon2id"`. Existing bcrypt hashes continue to authenticate
    after switching the configured algorithm to argon2.