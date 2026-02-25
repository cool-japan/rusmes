# rusmes-config TODO

## Implemented ✅
- [x] TOML/YAML configuration loading (auto-detect, 1,639 lines)
- [x] All config sections: SMTP, IMAP, JMAP, POP3, Storage, Auth, Queue, Security, Metrics, Tracing, Connection Limits
- [x] Environment variable overrides (30+ `RUSMES_*` variables)
- [x] Configuration validation on load (domain, email, port, paths, processor names)
- [x] Hot-reload on SIGHUP
- [x] Size string parser ("50MB", "1GB"), duration parser ("60s", "30m", "1h")
- [x] Log rotation config (daily/hourly/size-based, JSON/Text format)

## Remaining
- [ ] `[performance]` section (worker threads, pool sizes, buffer sizes)
- [ ] TLS certificate paths per protocol (currently shared)
- [ ] Default value documentation in struct fields
- [ ] Warn on unknown configuration keys