# rusmes-auth

Authentication backend abstraction for RusMES. Defines a pluggable `AuthBackend` trait used by all protocol servers (SMTP, IMAP, JMAP, POP3).

## Status

Complete. Implements `AuthBackend` trait with five production backends, full SASL
framework, and security hardening.

## Key Trait

```rust
#[async_trait]
pub trait AuthBackend: Send + Sync {
    async fn authenticate(&self, username: &Username, password: &str) -> Result<bool>;
    async fn verify_identity(&self, username: &Username) -> Result<bool>;
    async fn create_user(&self, username: &Username, password: &str) -> Result<()>;
    async fn delete_user(&self, username: &Username) -> Result<()>;
    async fn change_password(&self, username: &Username, new_password: &str) -> Result<()>;
    async fn list_users(&self) -> Result<Vec<Username>>;
    // Optional: SCRAM/APOP credential methods
}
```

All protocol servers receive an `Arc<dyn AuthBackend>` and call `authenticate()`
 during login/auth flows.

## Backends

| Backend | Description | Status |
|---------|-------------|--------|
| File | htpasswd-style file with bcrypt hashes | ✅ Complete |
| LDAP | LDAP/LDAPS bind authentication (802 lines) | ✅ Complete |
| SQL | Query-based auth against PostgreSQL/MySQL/SQLite (1,154 lines) | ✅ Complete |
| OAuth2/OIDC | Token introspection and JWKS validation (1,469 lines) | ✅ Complete |
| PAM | Linux PAM integration (feature-gated) | ✅ Complete |

### SASL Mechanisms (1,495 lines)
PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2

### Security (885 lines)
Brute-force protection, password strength validation, audit logging, IP rate limiting.

> **Note**: Only file-based auth is fully integrated end-to-end in `rusmes-server`.
> LDAP/SQL/OAuth2 backends work independently but fall back to `DummyAuthBackend`
> in the main server binary.

## Dependencies
- `rusmes-proto` - `Username` type
- `async-trait` - async trait support
- `anyhow` - error handling
