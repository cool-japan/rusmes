//! POP3 session state machine and handler

use crate::command::Pop3Command;
use crate::maildrop_lock::{MaildropGuard, MaildropLockManager};
use crate::parser::parse_command;
use crate::response::Pop3Response;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use md5::{Digest, Md5};
use rusmes_auth::sasl::{SaslConfig, SaslMechanism, SaslServer, SaslStep};
use rusmes_auth::AuthBackend;
use rusmes_proto::{Mail, MessageId, Username};
use rusmes_storage::{MailboxId, MailboxPath, SearchCriteria, StorageBackend};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

/// Generate a random u64 value using the OS CSPRNG
fn random_u64() -> anyhow::Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("RNG failure: {}", e))?;
    Ok(u64::from_le_bytes(bytes))
}

/// POP3 session state
#[derive(Debug, Clone, PartialEq)]
pub enum Pop3State {
    /// Authorization state - waiting for USER/PASS or APOP
    Authorization,
    /// Transaction state - authenticated, can access mailbox
    Transaction,
    /// Update state - after QUIT, applying deletions
    Update,
}

/// POP3 session configuration
#[derive(Debug, Clone)]
pub struct Pop3Config {
    pub hostname: String,
    pub greeting: String,
    pub timeout_seconds: u64,
    pub enable_stls: bool,
    /// SASL mechanisms advertised in CAPA SASL line and accepted by AUTH.
    ///
    /// Default: `["PLAIN", "LOGIN", "CRAM-MD5", "SCRAM-SHA-256"]` — matching
    /// what the SMTP server already exposes via `rusmes-auth::sasl`.
    pub sasl_mechanisms: Vec<String>,
}

impl Default for Pop3Config {
    fn default() -> Self {
        Self {
            hostname: "localhost".to_string(),
            greeting: "POP3 server ready".to_string(),
            timeout_seconds: 600, // 10 minutes
            enable_stls: false,
            sasl_mechanisms: vec![
                "PLAIN".to_string(),
                "LOGIN".to_string(),
                "CRAM-MD5".to_string(),
                "SCRAM-SHA-256".to_string(),
            ],
        }
    }
}

/// Message info in the maildrop
#[derive(Debug, Clone)]
struct MessageInfo {
    message_id: MessageId,
    uid: u32,
    size: usize,
    deleted: bool,
}

/// POP3 session handler
pub struct Pop3Session {
    remote_addr: SocketAddr,
    state: Pop3State,
    config: Pop3Config,
    username: Option<Username>,
    mailbox_id: Option<MailboxId>,
    messages: Vec<MessageInfo>,
    auth_backend: Arc<dyn AuthBackend>,
    storage_backend: Arc<dyn StorageBackend>,
    apop_timestamp: Option<String>,
    /// Per-user maildrop lock manager shared across all sessions on this server.
    maildrop_locks: MaildropLockManager,
    /// RAII handle to this session's exclusive maildrop lock (RFC 1939 §3).
    /// Held for the entirety of the Transaction state and released on QUIT
    /// or session drop.
    maildrop_guard: Option<MaildropGuard>,
    /// Pending SASL mechanism, if a multi-step AUTH exchange is in progress.
    /// While this is `Some`, raw client lines are routed to `step()` instead
    /// of being parsed as POP3 commands.
    pending_sasl: Option<Box<dyn SaslMechanism>>,
}

impl Pop3Session {
    /// Create a new POP3 session.
    ///
    /// The `maildrop_locks` argument is shared across all sessions on a single
    /// server so that exclusive-access semantics (RFC 1939 §3) hold.
    pub fn new(
        remote_addr: SocketAddr,
        config: Pop3Config,
        auth_backend: Arc<dyn AuthBackend>,
        storage_backend: Arc<dyn StorageBackend>,
        maildrop_locks: MaildropLockManager,
    ) -> Self {
        Self {
            remote_addr,
            state: Pop3State::Authorization,
            config,
            username: None,
            mailbox_id: None,
            messages: Vec::new(),
            auth_backend,
            storage_backend,
            apop_timestamp: None,
            maildrop_locks,
            maildrop_guard: None,
            pending_sasl: None,
        }
    }

    /// Generate APOP timestamp banner
    ///
    /// Format: <process-id.clock@hostname>
    /// This timestamp is used for APOP MD5 digest authentication
    fn generate_apop_timestamp(&self) -> anyhow::Result<String> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let random = random_u64()?;
        let process_id = std::process::id();
        Ok(format!(
            "<{}.{}.{}@{}>",
            process_id, timestamp, random, self.config.hostname
        ))
    }

    /// Handle a client connection
    pub async fn handle(mut self, stream: TcpStream) -> anyhow::Result<()> {
        info!("New POP3 connection from {}", self.remote_addr);
        // Track this session in the active-connections gauge.
        // The guard's Drop decrements the gauge regardless of how this fn exits.
        let _conn_guard = rusmes_metrics::global_metrics().connection_guard("pop3");
        rusmes_metrics::global_metrics().inc_tls_session(rusmes_metrics::tls_label::NO);

        let (reader, writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut writer = BufWriter::new(writer);

        // Generate APOP timestamp for this session
        let timestamp = self.generate_apop_timestamp()?;
        self.apop_timestamp = Some(timestamp.clone());

        // Send greeting with APOP timestamp
        let greeting_msg = format!("{} {}", self.config.greeting, timestamp);
        let greeting = Pop3Response::ok(&greeting_msg);
        self.write_response(&mut writer, &greeting).await?;

        // Command loop
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    debug!("Client closed connection");
                    break;
                }
                Ok(_) => {
                    let response = self.handle_line(&line).await;
                    self.write_response(&mut writer, &response).await?;

                    // Check if we should quit
                    if self.state == Pop3State::Update {
                        break;
                    }
                }
                Err(e) => {
                    error!("Error reading from client: {}", e);
                    break;
                }
            }
        }

        // Perform update if needed
        if self.state == Pop3State::Update {
            if let Err(e) = self.apply_deletions().await {
                error!("Error applying deletions: {}", e);
            }
        }

        // Release maildrop lock explicitly. (It would also be released by the
        // session's Drop, but releasing here lets a follow-up session for the
        // same user proceed immediately without waiting on connection teardown.)
        self.release_maildrop_lock();

        info!("POP3 session ended for {}", self.remote_addr);
        Ok(())
    }

    /// Test-only entry point that exposes the per-line dispatch (which is
    /// otherwise driven from inside `handle()`) so integration tests can
    /// exercise the protocol without binding an actual TCP socket.
    ///
    /// Hidden behind `#[doc(hidden)]` to discourage non-test use.
    #[doc(hidden)]
    pub async fn handle_line_for_test(&mut self, line: &str) -> Pop3Response {
        self.handle_line(line).await
    }

    /// Test-only accessor for the current session state.
    #[doc(hidden)]
    pub fn state_for_test(&self) -> Pop3State {
        self.state.clone()
    }

    /// Handle a single command line.
    ///
    /// While a multi-step SASL exchange is in progress (`pending_sasl.is_some()`),
    /// raw client input is routed to the SASL mechanism's `step()` instead of
    /// being parsed as a POP3 command. RFC 5034 also defines `*` as the
    /// client-side abort indicator — we honor it here.
    async fn handle_line(&mut self, line: &str) -> Pop3Response {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Pop3Response::err("Empty command");
        }

        debug!("Received: {}", trimmed);

        if self.pending_sasl.is_some() {
            return self.handle_sasl_continuation(trimmed).await;
        }

        match parse_command(trimmed) {
            Ok(cmd) => self.handle_command(cmd).await,
            Err(e) => {
                warn!("Parse error: {}", e);
                Pop3Response::err("Syntax error in command")
            }
        }
    }

    /// Handle a parsed command
    async fn handle_command(&mut self, cmd: Pop3Command) -> Pop3Response {
        debug!("Command: {}", cmd);

        match cmd {
            Pop3Command::User(name) => self.handle_user(name).await,
            Pop3Command::Pass(pass) => self.handle_pass(pass).await,
            Pop3Command::Stat => self.handle_stat().await,
            Pop3Command::List(msg) => self.handle_list(msg).await,
            Pop3Command::Retr(msg) => self.handle_retr(msg).await,
            Pop3Command::Dele(msg) => self.handle_dele(msg).await,
            Pop3Command::Noop => self.handle_noop().await,
            Pop3Command::Rset => self.handle_rset().await,
            Pop3Command::Quit => self.handle_quit().await,
            Pop3Command::Top { msg, lines } => self.handle_top(msg, lines).await,
            Pop3Command::Uidl(msg) => self.handle_uidl(msg).await,
            Pop3Command::Apop { name, digest } => self.handle_apop(name, digest).await,
            Pop3Command::Capa => self.handle_capa().await,
            Pop3Command::Stls => self.handle_stls().await,
            Pop3Command::Auth {
                mechanism,
                initial_response,
            } => self.handle_auth(mechanism, initial_response).await,
        }
    }

    /// Handle USER command
    async fn handle_user(&mut self, name: String) -> Pop3Response {
        if self.state != Pop3State::Authorization {
            return Pop3Response::err("Command not valid in this state");
        }

        match Username::new(&name) {
            Ok(username) => {
                self.username = Some(username);
                Pop3Response::ok(format!("{} is a valid mailbox", name))
            }
            Err(_) => Pop3Response::err("Invalid username"),
        }
    }

    /// Handle PASS command
    async fn handle_pass(&mut self, password: String) -> Pop3Response {
        if self.state != Pop3State::Authorization {
            return Pop3Response::err("Command not valid in this state");
        }

        let username = match &self.username {
            Some(u) => u.clone(),
            None => return Pop3Response::err("No username specified"),
        };

        // Authenticate
        match self.auth_backend.authenticate(&username, &password).await {
            Ok(true) => self.transition_to_transaction(username).await,
            Ok(false) => Pop3Response::err("Authentication failed"),
            Err(e) => {
                error!("Auth error: {}", e);
                Pop3Response::err("Authentication error")
            }
        }
    }

    /// Transition the session into Transaction state for the named user.
    ///
    /// Acquires the per-user maildrop lock (RFC 1939 §3) and loads the
    /// mailbox. If another concurrent session already holds the lock, returns
    /// `-ERR maildrop locked` and leaves the session in Authorization state.
    async fn transition_to_transaction(&mut self, username: Username) -> Pop3Response {
        // Acquire exclusive maildrop lock first — refusing the session here
        // is cheap and avoids loading the mailbox just to throw it away.
        let guard = match self.maildrop_locks.try_acquire(username.as_str()).await {
            Some(g) => g,
            None => {
                warn!(
                    "Maildrop locked for user '{}' (concurrent session)",
                    username.as_str()
                );
                return Pop3Response::err("maildrop locked");
            }
        };

        match self.load_mailbox(&username).await {
            Ok(_) => {
                self.username = Some(username);
                self.maildrop_guard = Some(guard);
                self.state = Pop3State::Transaction;
                let count = self.messages.len();
                let size: usize = self.messages.iter().map(|m| m.size).sum();
                Pop3Response::ok(format!("{} messages ({} octets)", count, size))
            }
            Err(e) => {
                // Releasing `guard` here (it goes out of scope) frees the lock
                // so a retry can succeed once the storage error resolves.
                drop(guard);
                error!("Failed to load mailbox: {}", e);
                Pop3Response::err("Mailbox unavailable")
            }
        }
    }

    /// Handle STAT command
    async fn handle_stat(&mut self) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        let count = self.messages.iter().filter(|m| !m.deleted).count();
        let size: usize = self
            .messages
            .iter()
            .filter(|m| !m.deleted)
            .map(|m| m.size)
            .sum();

        Pop3Response::ok(format!("{} {}", count, size))
    }

    /// Handle LIST command
    async fn handle_list(&mut self, msg: Option<u32>) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        match msg {
            Some(n) => {
                // List single message
                if n == 0 || n as usize > self.messages.len() {
                    return Pop3Response::err("No such message");
                }
                let info = &self.messages[n as usize - 1];
                if info.deleted {
                    return Pop3Response::err("Message deleted");
                }
                Pop3Response::ok(format!("{} {}", n, info.size))
            }
            None => {
                // List all messages
                let count = self.messages.iter().filter(|m| !m.deleted).count();
                let size: usize = self
                    .messages
                    .iter()
                    .filter(|m| !m.deleted)
                    .map(|m| m.size)
                    .sum();

                let mut lines = Vec::new();
                for (idx, info) in self.messages.iter().enumerate() {
                    if !info.deleted {
                        lines.push(format!("{} {}", idx + 1, info.size));
                    }
                }

                Pop3Response::ok_multiline(format!("{} messages ({} octets)", count, size), lines)
            }
        }
    }

    /// Handle RETR command
    async fn handle_retr(&mut self, msg: u32) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        if msg == 0 || msg as usize > self.messages.len() {
            return Pop3Response::err("No such message");
        }

        let info = &self.messages[msg as usize - 1];
        if info.deleted {
            return Pop3Response::err("Message deleted");
        }

        // Retrieve the message
        let message_store = self.storage_backend.message_store();
        match message_store.get_message(&info.message_id).await {
            Ok(Some(mail)) => {
                let content = mail_to_wire(&mail).await;
                let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                Pop3Response::ok_multiline(format!("{} octets", info.size), lines)
            }
            Ok(None) => Pop3Response::err("Message not found"),
            Err(e) => {
                error!("Failed to retrieve message: {}", e);
                Pop3Response::err("Failed to retrieve message")
            }
        }
    }

    /// Handle DELE command
    async fn handle_dele(&mut self, msg: u32) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        if msg == 0 || msg as usize > self.messages.len() {
            return Pop3Response::err("No such message");
        }

        let info = &mut self.messages[msg as usize - 1];
        if info.deleted {
            return Pop3Response::err("Message already deleted");
        }

        info.deleted = true;
        Pop3Response::ok(format!("Message {} deleted", msg))
    }

    /// Handle NOOP command
    async fn handle_noop(&mut self) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }
        Pop3Response::ok("NOOP")
    }

    /// Handle RSET command
    async fn handle_rset(&mut self) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        // Unmark all deletions
        for info in &mut self.messages {
            info.deleted = false;
        }

        let count = self.messages.len();
        Pop3Response::ok(format!("Maildrop has {} messages", count))
    }

    /// Handle QUIT command.
    ///
    /// In Transaction state, transitions to Update so the command loop can
    /// apply pending deletions; the maildrop lock is released after deletions
    /// have been applied (see `handle()` and `release_maildrop_lock`).
    async fn handle_quit(&mut self) -> Pop3Response {
        match self.state {
            Pop3State::Authorization => {
                self.state = Pop3State::Update;
                Pop3Response::ok("POP3 server signing off")
            }
            Pop3State::Transaction => {
                self.state = Pop3State::Update;
                let deleted = self.messages.iter().filter(|m| m.deleted).count();
                Pop3Response::ok(format!(
                    "POP3 server signing off ({} messages deleted)",
                    deleted
                ))
            }
            Pop3State::Update => Pop3Response::ok("POP3 server signing off"),
        }
    }

    /// Explicitly drop the maildrop lock (no-op if not held).
    ///
    /// Called when transitioning out of Transaction state.  Even without this
    /// call the lock would be released when `Pop3Session` is dropped (the
    /// guard's `Drop` impl unlocks the inner `Mutex`), but releasing
    /// proactively here lets follow-up sessions for the same user proceed
    /// immediately rather than waiting for the connection to be reaped.
    fn release_maildrop_lock(&mut self) {
        self.maildrop_guard = None;
    }

    /// Handle TOP command
    async fn handle_top(&mut self, msg: u32, lines: u32) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        if msg == 0 || msg as usize > self.messages.len() {
            return Pop3Response::err("No such message");
        }

        let info = &self.messages[msg as usize - 1];
        if info.deleted {
            return Pop3Response::err("Message deleted");
        }

        // Retrieve the message
        let message_store = self.storage_backend.message_store();
        match message_store.get_message(&info.message_id).await {
            Ok(Some(mail)) => {
                let content = mail_to_wire(&mail).await;
                let all_lines: Vec<&str> = content.lines().collect();

                // Find the end of headers (blank line)
                let header_end = all_lines
                    .iter()
                    .position(|line| line.is_empty())
                    .unwrap_or(all_lines.len());

                // Include headers + blank line + n lines of body
                let end_pos = std::cmp::min(header_end + 1 + lines as usize, all_lines.len());

                let result_lines: Vec<String> =
                    all_lines[..end_pos].iter().map(|s| s.to_string()).collect();

                Pop3Response::ok_multiline("TOP", result_lines)
            }
            Ok(None) => Pop3Response::err("Message not found"),
            Err(e) => {
                error!("Failed to retrieve message: {}", e);
                Pop3Response::err("Failed to retrieve message")
            }
        }
    }

    /// Handle UIDL command
    async fn handle_uidl(&mut self, msg: Option<u32>) -> Pop3Response {
        if self.state != Pop3State::Transaction {
            return Pop3Response::err("Command not valid in this state");
        }

        match msg {
            Some(n) => {
                // UIDL for single message
                if n == 0 || n as usize > self.messages.len() {
                    return Pop3Response::err("No such message");
                }
                let info = &self.messages[n as usize - 1];
                if info.deleted {
                    return Pop3Response::err("Message deleted");
                }
                Pop3Response::ok(format!("{} {}", n, info.uid))
            }
            None => {
                // UIDL for all messages
                let mut lines = Vec::new();
                for (idx, info) in self.messages.iter().enumerate() {
                    if !info.deleted {
                        lines.push(format!("{} {}", idx + 1, info.uid));
                    }
                }
                Pop3Response::ok_multiline("UIDL", lines)
            }
        }
    }

    /// Handle APOP command
    ///
    /// APOP authenticates using MD5 digest: APOP <name> <digest>
    /// The digest is MD5(<timestamp-from-banner><shared-secret>)
    async fn handle_apop(&mut self, name: String, digest: String) -> Pop3Response {
        if self.state != Pop3State::Authorization {
            return Pop3Response::err("Command not valid in this state");
        }

        // Get the timestamp from banner
        let timestamp = match &self.apop_timestamp {
            Some(ts) => ts.clone(),
            None => {
                error!("APOP timestamp not available");
                return Pop3Response::err("APOP not available");
            }
        };

        // Parse and validate username
        let username = match Username::new(&name) {
            Ok(u) => u,
            Err(_) => return Pop3Response::err("Invalid username"),
        };

        // Get the user's secret (plaintext password) for APOP
        let secret = match self.auth_backend.get_apop_secret(&username).await {
            Ok(s) => s,
            Err(e) => {
                debug!("APOP not supported for user {}: {}", name, e);
                return Pop3Response::err("APOP authentication failed");
            }
        };

        // Compute expected digest: MD5(timestamp + secret)
        let expected_digest = compute_apop_digest(&timestamp, &secret);

        // Compare digests (constant-time comparison to prevent timing attacks)
        if !constant_time_compare(&digest.to_lowercase(), &expected_digest) {
            warn!("APOP authentication failed for user {}", name);
            return Pop3Response::err("Authentication failed");
        }

        // Authentication successful — acquire maildrop lock and transition.
        info!("APOP authentication successful for {}", name);
        self.transition_to_transaction(username).await
    }

    /// Handle CAPA command
    async fn handle_capa(&mut self) -> Pop3Response {
        // CAPA can be issued in any state (RFC 2449)
        let mut capabilities = vec!["USER".to_string(), "TOP".to_string(), "UIDL".to_string()];

        // Only advertise STLS in Authorization state and if enabled
        if self.state == Pop3State::Authorization && self.config.enable_stls {
            capabilities.push("STLS".to_string());
        }

        // SASL line per RFC 5034: "SASL <space-separated mechanism list>"
        // Only advertise in Authorization state where AUTH is meaningful.
        if self.state == Pop3State::Authorization && !self.config.sasl_mechanisms.is_empty() {
            let sasl_line = format!("SASL {}", self.config.sasl_mechanisms.join(" "));
            capabilities.push(sasl_line);
        }

        Pop3Response::ok_multiline("Capability list follows", capabilities)
    }

    /// Handle STLS command (STARTTLS for POP3)
    async fn handle_stls(&mut self) -> Pop3Response {
        // STLS can only be issued in Authorization state (RFC 2595)
        if self.state != Pop3State::Authorization {
            return Pop3Response::err("Command not valid in this state");
        }

        // Check if STLS is enabled
        if !self.config.enable_stls {
            return Pop3Response::err("STLS not available");
        }

        // After successful STLS response, the session should:
        // 1. Reset to initial Authorization state
        // 2. Clear any username that was provided with USER
        // 3. Perform TLS handshake
        //
        // Note: The actual TLS handshake must be performed by the caller
        // after receiving this response. This is similar to SMTP STARTTLS.
        self.username = None;
        self.state = Pop3State::Authorization;

        Pop3Response::ok("Begin TLS negotiation")
    }

    // ========================================================================
    // SASL AUTH (RFC 1734 + RFC 5034)
    // ========================================================================

    /// Handle the AUTH command. Drives a SASL exchange to completion. Multi-step
    /// mechanisms park the in-flight `SaslMechanism` in `pending_sasl`; subsequent
    /// raw client input lines are then fed to it via `handle_sasl_continuation`.
    async fn handle_auth(
        &mut self,
        mechanism: Option<String>,
        initial_response: Option<String>,
    ) -> Pop3Response {
        if self.state != Pop3State::Authorization {
            return Pop3Response::err("Command not valid in this state");
        }

        // Bare `AUTH` — list mechanisms in the multiline form RFC 1734 specifies.
        let mechanism_name = match mechanism {
            Some(m) => m,
            None => {
                return Pop3Response::ok_multiline(
                    "Supported authentication mechanisms",
                    self.config.sasl_mechanisms.clone(),
                );
            }
        };

        // Disallow nested AUTH while another exchange is already pending.
        if self.pending_sasl.is_some() {
            return Pop3Response::err("AUTH already in progress");
        }

        // Build the SASL server with the configured mechanism set.
        let server = SaslServer::new(SaslConfig {
            enabled_mechanisms: self.config.sasl_mechanisms.clone(),
            hostname: self.config.hostname.clone(),
        });

        if !server.is_mechanism_enabled(&mechanism_name) {
            return Pop3Response::err(format!(
                "Authentication mechanism '{}' not supported",
                mechanism_name
            ));
        }

        let mech = match server.create_mechanism(&mechanism_name) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "Failed to instantiate SASL mechanism '{}': {}",
                    mechanism_name, e
                );
                return Pop3Response::err("Authentication mechanism unavailable");
            }
        };

        // Decode initial response (if present) per RFC 5034. The literal `=`
        // token denotes an empty initial response.
        let initial_input: Option<Vec<u8>> = match initial_response.as_deref() {
            None => None,
            Some("=") => Some(Vec::new()),
            Some(ir) => match BASE64.decode(ir.as_bytes()) {
                Ok(b) => Some(b),
                Err(e) => {
                    warn!("Invalid base64 in AUTH initial response: {}", e);
                    return Pop3Response::err("Invalid base64 in initial response");
                }
            },
        };

        // Single source of truth: park the mechanism, then drive the first
        // step (either with the initial response, or — for mechanisms that
        // produce a challenge first — with empty input). For PLAIN without an
        // initial response, the canonical POP3 server replies `+ ` so the
        // client transmits the credentials on the next line; we therefore
        // skip the first step entirely and just send the empty continuation.
        if mechanism_name.eq_ignore_ascii_case("PLAIN") && initial_input.is_none() {
            self.pending_sasl = Some(mech);
            return Pop3Response::cont("");
        }

        self.pending_sasl = Some(mech);
        self.drive_pending_sasl(initial_input.unwrap_or_default())
            .await
    }

    /// Drive one step of the parked SASL mechanism with `input` and translate
    /// the outcome into a POP3 response. The mechanism is taken out of
    /// `pending_sasl`, stepped, and re-parked iff the exchange is still in
    /// progress (the mechanism returned a continuation).
    async fn drive_pending_sasl(&mut self, input: Vec<u8>) -> Pop3Response {
        let mut mech = match self.pending_sasl.take() {
            Some(m) => m,
            None => return Pop3Response::err("No SASL exchange in progress"),
        };

        let step_result = mech.step(&input, &*self.auth_backend).await;

        match step_result {
            Ok(SaslStep::Done {
                success,
                username: maybe_user,
            }) => {
                if success {
                    if let Some(user) = maybe_user {
                        self.transition_to_transaction(user).await
                    } else {
                        warn!("SASL reported success but no username");
                        Pop3Response::err("Authentication failed")
                    }
                } else {
                    Pop3Response::err("Authentication failed")
                }
            }
            Ok(SaslStep::Challenge { data }) => {
                // Park the mechanism so the next client line continues the
                // exchange. Mechanisms emit already-base64-encoded data as
                // bytes, so we forward as a UTF-8 string (the SASL framework
                // encodes inside the mechanisms — see e.g. LoginMechanism).
                let payload = String::from_utf8_lossy(&data).into_owned();
                self.pending_sasl = Some(mech);
                Pop3Response::cont(payload)
            }
            Ok(SaslStep::Continue) => {
                // Mechanism wants more input but has no challenge to send.
                self.pending_sasl = Some(mech);
                Pop3Response::cont("")
            }
            Err(e) => {
                warn!("SASL step error: {}", e);
                Pop3Response::err("Authentication exchange failed")
            }
        }
    }

    /// Handle a raw client line while a SASL exchange is in progress.
    ///
    /// RFC 5034 §3: the literal token `*` cancels the exchange and the server
    /// must respond with a negative status indicator.
    async fn handle_sasl_continuation(&mut self, line: &str) -> Pop3Response {
        // Honour client-side abort (RFC 5034 §3 / RFC 4954).
        if line == "*" {
            self.pending_sasl = None;
            return Pop3Response::err("Authentication aborted");
        }

        // Decode base64 client response. The bare `=` token (per RFC 5034) is
        // an empty payload.
        let decoded = if line == "=" {
            Vec::<u8>::new()
        } else {
            match BASE64.decode(line.as_bytes()) {
                Ok(b) => b,
                Err(e) => {
                    warn!("Invalid base64 in SASL response: {}", e);
                    self.pending_sasl = None;
                    return Pop3Response::err("Invalid base64 in SASL response");
                }
            }
        };

        self.drive_pending_sasl(decoded).await
    }

    /// Load the user's mailbox
    async fn load_mailbox(&mut self, username: &Username) -> anyhow::Result<()> {
        // Get the INBOX for this user
        let _mailbox_path = MailboxPath::new(username.clone(), vec!["INBOX".to_string()]);

        // Get mailbox from storage
        let mailbox_store = self.storage_backend.mailbox_store();
        let inbox_id = mailbox_store
            .get_user_inbox(username)
            .await?
            .ok_or_else(|| anyhow::anyhow!("INBOX not found for user"))?;

        self.mailbox_id = Some(inbox_id);

        // Search for all messages in INBOX
        let message_store = self.storage_backend.message_store();
        let message_ids = message_store.search(&inbox_id, SearchCriteria::All).await?;

        // Load message metadata
        self.messages.clear();
        for (idx, message_id) in message_ids.iter().enumerate() {
            // Get message to determine size
            if let Some(mail) = message_store.get_message(message_id).await? {
                let size = mail_to_wire(&mail).await.len();
                self.messages.push(MessageInfo {
                    message_id: *message_id,
                    uid: (idx + 1) as u32, // Use sequence number as UID for now
                    size,
                    deleted: false,
                });
            }
        }

        Ok(())
    }

    /// Apply deletions to the mailbox
    async fn apply_deletions(&self) -> anyhow::Result<()> {
        let deleted_ids: Vec<MessageId> = self
            .messages
            .iter()
            .filter(|m| m.deleted)
            .map(|m| m.message_id)
            .collect();

        if deleted_ids.is_empty() {
            return Ok(());
        }

        let message_store = self.storage_backend.message_store();
        message_store.delete_messages(&deleted_ids).await?;

        info!("Deleted {} messages", deleted_ids.len());
        Ok(())
    }

    /// Write a response to the client
    async fn write_response<W: AsyncWriteExt + Unpin>(
        &self,
        writer: &mut W,
        response: &Pop3Response,
    ) -> anyhow::Result<()> {
        let wire = response.to_wire();
        writer.write_all(wire.as_bytes()).await?;
        writer.flush().await?;
        debug!("Sent: {}", wire.trim_end());
        Ok(())
    }
}

/// Convert a Mail object to RFC822 wire format
///
/// For `MessageBody::Large`, the body bytes are read asynchronously before
/// formatting.  On read failure the body section is left empty and a warning
/// is logged.
async fn mail_to_wire(mail: &Mail) -> String {
    let message = mail.message();
    let headers = message.headers();
    let body = message.body();

    let mut result = String::new();

    // Add headers
    for (name, values) in headers.iter() {
        for value in values {
            result.push_str(name);
            result.push_str(": ");
            result.push_str(value);
            result.push_str("\r\n");
        }
    }

    // Blank line between headers and body
    result.push_str("\r\n");

    // Add body
    match body {
        rusmes_proto::MessageBody::Small(bytes) => {
            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                result.push_str(&text);
            } else {
                // Binary data, just append as-is
                result.push_str(&String::from_utf8_lossy(bytes));
            }
        }
        rusmes_proto::MessageBody::Large(large) => match large.read_to_bytes().await {
            Ok(bytes) => {
                result.push_str(&String::from_utf8_lossy(&bytes));
            }
            Err(e) => {
                warn!("Failed to read large message body for POP3 RETR: {e}");
            }
        },
    }

    result
}

/// Compute APOP MD5 digest
///
/// The digest is the hex-encoded MD5 hash of: timestamp + secret
fn compute_apop_digest(timestamp: &str, secret: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(secret.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Constant-time string comparison to prevent timing attacks
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut result = 0u8;

    for i in 0..a_bytes.len() {
        result |= a_bytes[i] ^ b_bytes[i];
    }

    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_apop_digest() {
        // Test vector from RFC 1939 Section 7 (example)
        // MD5(<1896.697170875@dbc.mtview.ca.us>tanstaaf)
        let timestamp = "<1896.697170875@dbc.mtview.ca.us>";
        let secret = "tanstaaf";
        // Expected digest computed from the timestamp + secret
        let expected = "7f4438180c8e9db500ca0b88225b554d";

        let digest = compute_apop_digest(timestamp, secret);
        assert_eq!(digest, expected);
    }

    #[test]
    fn test_compute_apop_digest_different_inputs() {
        let timestamp1 = "<12345.67890@example.com>";
        let secret1 = "password123";
        let digest1 = compute_apop_digest(timestamp1, secret1);

        // Different timestamp should produce different digest
        let timestamp2 = "<12345.67891@example.com>";
        let digest2 = compute_apop_digest(timestamp2, secret1);
        assert_ne!(digest1, digest2);

        // Different secret should produce different digest
        let digest3 = compute_apop_digest(timestamp1, "password456");
        assert_ne!(digest1, digest3);
    }

    #[test]
    fn test_constant_time_compare_equal() {
        assert!(constant_time_compare("hello", "hello"));
        assert!(constant_time_compare("", ""));
        assert!(constant_time_compare("abc123", "abc123"));
    }

    #[test]
    fn test_constant_time_compare_not_equal() {
        assert!(!constant_time_compare("hello", "world"));
        assert!(!constant_time_compare("abc", "abd"));
        assert!(!constant_time_compare("hello", "hello!"));
        assert!(!constant_time_compare("", "a"));
    }

    #[test]
    fn test_constant_time_compare_case_sensitive() {
        assert!(!constant_time_compare("Hello", "hello"));
        assert!(!constant_time_compare("ABC", "abc"));
    }

    #[test]
    fn test_apop_digest_is_lowercase_hex() {
        let timestamp = "<test@example.com>";
        let secret = "secret";
        let digest = compute_apop_digest(timestamp, secret);

        // Should be 32 characters (128-bit MD5 hash in hex)
        assert_eq!(digest.len(), 32);

        // Should only contain lowercase hex digits
        assert!(digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn test_pop3_state_equality() {
        assert_eq!(Pop3State::Authorization, Pop3State::Authorization);
        assert_eq!(Pop3State::Transaction, Pop3State::Transaction);
        assert_eq!(Pop3State::Update, Pop3State::Update);
        assert_ne!(Pop3State::Authorization, Pop3State::Transaction);
    }

    #[test]
    fn test_pop3_config_default() {
        let config = Pop3Config::default();
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.greeting, "POP3 server ready");
        assert_eq!(config.timeout_seconds, 600);
        assert!(!config.enable_stls);
    }

    #[test]
    fn test_pop3_config_with_stls() {
        let config = Pop3Config {
            enable_stls: true,
            ..Pop3Config::default()
        };
        assert!(config.enable_stls);
    }

    // NOTE: The following tests are commented out because they require
    // MemoryAuthBackend and MemoryStorageBackend which don't exist yet.
    // These will be uncommented when those test backends are implemented.

    /*
    #[tokio::test]
    async fn test_stls_only_in_authorization_state() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        let config = Pop3Config {
            hostname: "test.example.com".to_string(),
            greeting: "Test server".to_string(),
            timeout_seconds: 600,
            enable_stls: true,
        };

        let addr: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend = Arc::new(rusmes_storage::MemoryStorageBackend::new());

        let mut session = Pop3Session::new(addr, config.clone(), auth_backend, storage_backend);

        // Should work in Authorization state
        assert_eq!(session.state, Pop3State::Authorization);
        let response = session.handle_stls().await;
        assert_eq!(response.status(), crate::response::Pop3Status::Ok);
        assert!(response.message().contains("Begin TLS"));

        // Prepare session for Transaction state
        let addr2: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend2 = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend2 = Arc::new(rusmes_storage::MemoryStorageBackend::new());
        let mut session2 = Pop3Session::new(addr2, config, auth_backend2, storage_backend2);
        session2.state = Pop3State::Transaction;

        // Should fail in Transaction state
        let response2 = session2.handle_stls().await;
        assert_eq!(response2.status(), crate::response::Pop3Status::Err);
        assert!(response2.message().contains("not valid"));
    }

    #[tokio::test]
    async fn test_stls_requires_enable_stls_config() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        let config = Pop3Config {
            hostname: "test.example.com".to_string(),
            greeting: "Test server".to_string(),
            timeout_seconds: 600,
            enable_stls: false,
        };

        let addr: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend = Arc::new(rusmes_storage::MemoryStorageBackend::new());

        let mut session = Pop3Session::new(addr, config, auth_backend, storage_backend);

        // Should fail when STLS is disabled
        let response = session.handle_stls().await;
        assert_eq!(response.status(), crate::response::Pop3Status::Err);
        assert!(response.message().contains("not available"));
    }

    #[tokio::test]
    async fn test_stls_resets_session_state() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        let config = Pop3Config {
            hostname: "test.example.com".to_string(),
            greeting: "Test server".to_string(),
            timeout_seconds: 600,
            enable_stls: true,
        };

        let addr: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend = Arc::new(rusmes_storage::MemoryStorageBackend::new());

        let mut session = Pop3Session::new(addr, config, auth_backend, storage_backend);

        // Set a username
        session.username = Some(Username::new("testuser").unwrap());
        assert!(session.username.is_some());

        // Issue STLS
        let response = session.handle_stls().await;
        assert_eq!(response.status(), crate::response::Pop3Status::Ok);

        // Username should be cleared
        assert!(session.username.is_none());
        // State should remain Authorization
        assert_eq!(session.state, Pop3State::Authorization);
    }

    #[tokio::test]
    async fn test_capa_advertises_stls_in_authorization_state() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        let config = Pop3Config {
            hostname: "test.example.com".to_string(),
            greeting: "Test server".to_string(),
            timeout_seconds: 600,
            enable_stls: true,
        };

        let addr: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend = Arc::new(rusmes_storage::MemoryStorageBackend::new());

        let mut session = Pop3Session::new(addr, config, auth_backend, storage_backend);

        // In Authorization state, STLS should be advertised
        let response = session.handle_capa().await;
        assert_eq!(response.status(), crate::response::Pop3Status::Ok);
        let capabilities = response.multiline_data().unwrap();
        assert!(capabilities.iter().any(|c| c == "STLS"));
        assert!(capabilities.iter().any(|c| c == "USER"));
        assert!(capabilities.iter().any(|c| c == "TOP"));
        assert!(capabilities.iter().any(|c| c == "UIDL"));
    }

    #[tokio::test]
    async fn test_capa_does_not_advertise_stls_in_transaction_state() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        let config = Pop3Config {
            hostname: "test.example.com".to_string(),
            greeting: "Test server".to_string(),
            timeout_seconds: 600,
            enable_stls: true,
        };

        let addr: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend = Arc::new(rusmes_storage::MemoryStorageBackend::new());

        let mut session = Pop3Session::new(addr, config, auth_backend, storage_backend);
        session.state = Pop3State::Transaction;

        // In Transaction state, STLS should NOT be advertised
        let response = session.handle_capa().await;
        assert_eq!(response.status(), crate::response::Pop3Status::Ok);
        let capabilities = response.multiline_data().unwrap();
        assert!(!capabilities.iter().any(|c| c == "STLS"));
    }

    #[tokio::test]
    async fn test_capa_does_not_advertise_stls_when_disabled() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        let config = Pop3Config {
            hostname: "test.example.com".to_string(),
            greeting: "Test server".to_string(),
            timeout_seconds: 600,
            enable_stls: false,
        };

        let addr: SocketAddr = "127.0.0.1:110".parse().unwrap();
        let auth_backend = Arc::new(rusmes_auth::MemoryAuthBackend::new());
        let storage_backend = Arc::new(rusmes_storage::MemoryStorageBackend::new());

        let mut session = Pop3Session::new(addr, config, auth_backend, storage_backend);

        // When STLS is disabled, it should NOT be advertised
        let response = session.handle_capa().await;
        assert_eq!(response.status(), crate::response::Pop3Status::Ok);
        let capabilities = response.multiline_data().unwrap();
        assert!(!capabilities.iter().any(|c| c == "STLS"));
    }
    */
}
