# rusmes-imap TODO

## Implemented ✅
### Core Commands (RFC 9051)
- [x] LOGIN, AUTHENTICATE (multi-step SASL)
- [x] SELECT, EXAMINE
- [x] FETCH (all data items: BODY, ENVELOPE, FLAGS, etc.)
- [x] STORE (+FLAGS, -FLAGS, FLAGS)
- [x] SEARCH (full criteria support)
- [x] APPEND (RFC 9051 compliant, APPENDUID response)
- [x] LIST (with patterns), LSUB, SUBSCRIBE/UNSUBSCRIBE
- [x] CREATE, DELETE, RENAME
- [x] COPY, MOVE (RFC 6851)
- [x] EXPUNGE, CLOSE
- [x] NOOP, CAPABILITY, LOGOUT
- [x] UID variants of FETCH, STORE, SEARCH, COPY, MOVE, EXPUNGE

### Extensions
- [x] IDLE (RFC 2177) — push notifications
- [x] NAMESPACE (RFC 2342)
- [x] CONDSTORE (RFC 7162, 470 lines)
- [x] QRESYNC (RFC 7162, 1,063 lines)
- [x] LITERAL+ (RFC 7888)
- [x] UIDPLUS (RFC 4315) — APPENDUID, COPYUID
- [x] SPECIAL-USE (RFC 6154)

### Parser
- [x] Full IMAP command parser (1,222 lines, nom-based, LITERAL+ aware)
- [x] Literal string, quoted string, atom, sequence set, FETCH data item, SEARCH criteria parsing

## Remaining
- [ ] COMPRESS=DEFLATE (RFC 4978)
- [ ] Untagged responses (EXISTS, RECENT, FLAGS updates) for concurrent access
- [ ] Concurrent mailbox access handling (cross-session notifications)
- [ ] Mailbox change notifications across sessions