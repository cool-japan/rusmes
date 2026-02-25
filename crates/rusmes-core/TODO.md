# rusmes-core TODO

## Implemented ✅
### Mailet Engine
- [x] `Mailet` trait (async init/service/destroy), `MailetAction` enum
- [x] `Matcher` trait + `MatchResult`
- [x] Processor chain with fork/split support (partial match handling)
- [x] Mail processor router with state-based routing (depth limit = 100)

### Mailets (16)
- [x] AddHeader, Bounce, DkimVerify (962 lines), DmarcVerify, DNSBL
- [x] Forward (1,121 lines), Greylist, Legalis (1,040 lines), LocalDelivery
- [x] OxiFY (1,411 lines), RemoteDelivery, RemoveMimeHeader, Sieve
- [x] SpamAssassin (spamd protocol), SpfCheck (972 lines), VirusScan (ClamAV clamd)

### Matchers (11)
- [x] All, None, RecipientIsLocal, SenderIs, HasAttachment
- [x] SizeGreaterThan, HeaderContains, RemoteAddress (CIDR)
- [x] IsInWhitelist, IsInBlacklist, Composite (And/Or/Not)

### Other
- [x] DSN bounce message generation (RFC 3464)
- [x] Rate limiter (per-IP, hot-reload capable)
- [x] Persistent queue + dead letter queue + priority queue
- [x] Sieve scripting engine (RFC 5228 parser + interpreter)

## Remaining
- [ ] Mailet execution timeout (configurable per-mailet)
- [ ] Mailet error handling policy (skip, abort, retry)
- [ ] Async mailet loading from shared libraries (plugin system / WASM)
- [ ] Queue priority levels (currently flat)
- [ ] Queue statistics per destination domain
- [ ] Per-sender rate limiting (currently per-IP only)
- [ ] Persistent rate limit state (survive restarts)