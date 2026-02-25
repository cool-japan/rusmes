# rusmes-jmap TODO

## Implemented ✅
### Core (RFC 8620)
- [x] Session endpoint (`/.well-known/jmap`) with capabilities
- [x] Request validation (using, methodCalls structure)
- [x] Error responses (unknownMethod, invalidArguments, unknownCapability, limit, etc.)
- [x] Account discovery

### Email Methods (RFC 8621)
- [x] `Email/get`, `Email/set`, `Email/query`
- [x] `Email/changes`, `Email/copy`, `Email/import`, `Email/parse`
- [x] `EmailSubmission/*` (1,629 lines)

### Mailbox Methods
- [x] `Mailbox/get`, `Mailbox/set`, `Mailbox/query`, `Mailbox/changes` (1,932 lines)

### Other Methods
- [x] `Thread/get`, `Thread/changes`
- [x] `SearchSnippet/get`
- [x] `Identity/get`, `Identity/set`
- [x] `VacationResponse/get`, `VacationResponse/set`

### Blob & Push
- [x] Blob download endpoint (`/download/:account/:blob/:name`)
- [x] Blob upload endpoint (`/upload/:account`)
- [x] EventSource (Server-Sent Events) with broadcast channel

## Remaining
### Critical
- [-] **Authentication**: Basic/Bearer is hardcoded (DEVELOPMENT ONLY) — needs real `AuthBackend` integration
- [ ] Blob storage persistence (currently in-memory, lost on restart)

### Important
- [ ] Back-reference resolution between method calls (RFC 8620 §3.7)
- [ ] `Email/queryChanges` — incremental query updates
- [ ] Account permission enforcement
- [ ] Blob size limits enforcement
- [ ] Push subscription management (WebPush)