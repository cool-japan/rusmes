# rusmes-auth TODO

## Implemented ✅
### Trait & User Management
- [x] `AuthBackend` trait: authenticate, verify_identity, list/create/delete users, change_password
- [x] Optional SCRAM/APOP credential methods

### Backends
- [x] File backend (bcrypt password hashing, 221 lines)
- [x] LDAP backend (bind authentication, user search, 802 lines)
- [x] SQL backend (PostgreSQL/MySQL/SQLite, configurable queries, 1,154 lines)
- [x] OAuth2/OIDC backend (token introspection, JWKS validation, 1,469 lines)
- [x] PAM backend (Linux Pluggable Authentication Modules, feature flag)

### SASL Framework (1,495 lines)
- [x] PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2

### Security (885 lines)
- [x] Brute force protection (account lockout after N failures)
- [x] Password strength validation (entropy calculation)
- [x] Audit logging (authentication success/failure events)
- [x] Rate limiting on auth attempts per IP

## Remaining
- [-] **Server integration**: LDAP/SQL/OAuth2 backends have code but `rusmes-server` falls back to `DummyAuthBackend` — only file-based auth works end-to-end
- [ ] Argon2 password hashing support (currently bcrypt only for file backend)