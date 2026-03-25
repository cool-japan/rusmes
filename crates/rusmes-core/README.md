# rusmes-core

Message processing engine for RusMES, implementing the mailet-based pipeline architecture inspired by Apache JAMES.

## Architecture

Mail flows through a chain of **Matcher-Mailet** pairs organized into **Processors**. A **Router** dispatches mail to the correct processor based on its current state.

```
Mail (state=Root)
  |
  v
MailProcessorRouter
  |-- Processor "root" (state=Root)
  |     |-- [AllMatcher] -> SpfCheckMailet
  |     |-- [AllMatcher] -> DkimVerifyMailet
  |     |-- [AllMatcher] -> SpamAssassinMailet
  |     '-- [AllMatcher] -> AddHeaderMailet
  |
  |-- Processor "transport" (state=Transport)
  |     |-- [RecipientIsLocal] -> LocalDeliveryMailet
  |     '-- [All] -> RemoteDeliveryMailet
  |
  '-- Processor "error" (state=Error)
        '-- [All] -> BounceMailet
```

## Modules

| Module | Description |
|--------|-------------|
| `mailet` | `Mailet` trait, `MailetAction`, `MailetConfig` |
| `matcher` | `Matcher` trait, `AllMatcher`, `NoneMatcher` |
| `processor` | `Processor` chain, `ProcessingStep` |
| `router` | `MailProcessorRouter` state-based dispatch |
| `factory` | `create_mailet()` / `create_matcher()` factory functions |
| `queue` | `MailQueue` with retry and delay support |
| `bounce` | Bounce message generation |
| `rate_limit` | Per-IP connection and message rate limiting |
| `mailets/` | Standard mailet implementations |
| `matchers/` | Standard matcher implementations |

## Mailets (16 implemented)

| Mailet | Description |
|--------|-------------|
| `AddHeader` | Add/modify message headers |
| `LocalDelivery` | Deliver to local mailboxes |
| `RemoteDelivery` | Relay to external SMTP servers |
| `SpamAssassin` | Spam scoring (configurable threshold) |
| `VirusScan` | Virus detection (stub for ClamAV) |
| `DkimVerify` | DKIM signature verification (stub) |
| `SpfCheck` | SPF record validation (stub) |
| `DmarcVerify` | DMARC alignment check |
| `Bounce` | Generate DSN bounce messages |
| `RemoveMimeHeader` | Strip specified MIME headers |
| `Forward` | Forward mail to additional recipients |
| `SieveMailet` | Execute Sieve scripts (RFC 5228) |
| `OxiFYMailet` | AI-powered mail analysis |
| `LegalisMailet` | Legal archiving integration |
| `DNSBL` | DNS Blocklist spam prevention |
| `Greylist` | Greylisting spam prevention |

## Matchers (11 implemented)

| Matcher | Description |
|---------|-------------|
| `All` | Matches all recipients |
| `None` | Matches no recipients |
| `RecipientIsLocal` | Matches recipients in configured local domains |
| `SenderIs` | Matches by sender address pattern |
| `HasAttachment` | Matches messages with attachments |
| `SizeGreaterThan` | Matches messages above a size threshold |
| `HeaderContains` | Matches by header value pattern |
| `RemoteAddress` | Matches by client IP / CIDR range |
| `IsInWhitelist` | Matches senders in whitelist |
| `IsInBlacklist` | Matches senders in blocklist |
| `And` / `Or` / `Not` | Composite matchers |

## Key Traits

```rust
#[async_trait]
pub trait Mailet: Send + Sync {
    async fn init(&mut self, config: MailetConfig) -> Result<()>;
    async fn service(&self, mail: &mut Mail) -> Result<MailetAction>;
    async fn destroy(&mut self) -> Result<()>;
    fn name(&self) -> &str;
}

#[async_trait]
pub trait Matcher: Send + Sync {
    async fn match_mail(&self, mail: &Mail) -> Result<Vec<MailAddress>>;
    fn name(&self) -> &str;
}
```

## Dependencies
- `rusmes-proto` - core types
- `rusmes-storage` - storage traits (for LocalDelivery)
- `rusmes-metrics` - metrics collection
- `tokio` - async runtime
- `tracing` - structured logging

## Tests

```bash
cargo test -p rusmes-core   # 20 tests
```
