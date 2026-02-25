//! SMTP session state machine and handler

use crate::command::SmtpCommand;
use crate::parser::parse_command;
use crate::response::SmtpResponse;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_auth::AuthBackend;
use rusmes_core::{MailProcessorRouter, RateLimiter};
use rusmes_proto::{MailAddress, Username};
use rusmes_storage::StorageBackend;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

/// SMTP session state
#[derive(Debug, Clone, PartialEq)]
pub enum SmtpState {
    /// Initial state before connection
    Initial,
    /// Connected, waiting for HELO/EHLO
    Connected,
    /// Authenticated (if AUTH required)
    Authenticated,
    /// In mail transaction (after MAIL FROM)
    MailTransaction,
    /// Receiving message data (after DATA command)
    Data,
    /// Quit command received
    Quit,
}

/// SMTP transaction state
#[derive(Debug, Clone)]
pub struct SmtpTransaction {
    sender: Option<MailAddress>,
    recipients: Vec<MailAddress>,
    helo_name: Option<String>,
    message_size: usize,
    /// Declared message size from MAIL FROM SIZE parameter
    declared_size: Option<usize>,
    /// BODY parameter value (7BIT, 8BITMIME, BINARYMIME)
    body_type: Option<String>,
    /// SMTPUTF8 flag
    smtputf8: bool,
    /// BDAT state for CHUNKING extension (RFC 3030)
    bdat_state: Option<crate::BdatState>,
    /// Message data received via DATA command
    message_data: Vec<u8>,
}

impl SmtpTransaction {
    fn new() -> Self {
        Self {
            sender: None,
            recipients: Vec::new(),
            helo_name: None,
            message_size: 0,
            declared_size: None,
            body_type: None,
            smtputf8: false,
            bdat_state: None,
            message_data: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.sender = None;
        self.recipients.clear();
        self.message_size = 0;
        self.declared_size = None;
        self.body_type = None;
        self.smtputf8 = false;
        self.bdat_state = None;
        self.message_data.clear();
    }

    fn is_valid(&self) -> bool {
        self.sender.is_some() && !self.recipients.is_empty()
    }
}

/// SMTP session configuration
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub hostname: String,
    pub max_message_size: usize,
    pub require_auth: bool,
    pub enable_starttls: bool,
    pub check_recipient_exists: bool,
    pub reject_unknown_recipients: bool,
    /// CIDR networks allowed to relay mail (e.g., "192.168.0.0/16")
    pub relay_networks: Vec<String>,
    /// Local domains that this server accepts mail for
    pub local_domains: Vec<String>,
    /// Total connection timeout (max session duration)
    pub connection_timeout: Duration,
    /// Idle timeout (time between commands)
    pub idle_timeout: Duration,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            hostname: "localhost".to_string(),
            max_message_size: 10 * 1024 * 1024, // 10MB
            require_auth: false,
            enable_starttls: false,
            check_recipient_exists: true,
            reject_unknown_recipients: true,
            relay_networks: vec!["127.0.0.0/8".to_string()],
            local_domains: vec!["localhost".to_string()],
            connection_timeout: Duration::from_secs(3600), // 1 hour
            idle_timeout: Duration::from_secs(300),        // 5 minutes
        }
    }
}

/// Cache entry for recipient validation
#[derive(Debug, Clone)]
struct RecipientCacheEntry {
    exists: bool,
    cached_at: Instant,
}

/// SCRAM-SHA-256 authentication state
#[derive(Debug, Clone)]
struct ScramState {
    client_first_bare: String,
    server_first: String,
    nonce: String,
    username: String,
}

/// SMTP session handler
pub struct SmtpSession {
    remote_addr: SocketAddr,
    state: SmtpState,
    transaction: SmtpTransaction,
    config: SmtpConfig,
    authenticated: bool,
    #[allow(dead_code)]
    username: Option<String>,
    #[allow(dead_code)]
    relaying_allowed: bool,
    #[allow(dead_code)]
    processor_router: Arc<MailProcessorRouter>,
    auth_backend: Arc<dyn AuthBackend>,
    rate_limiter: Arc<RateLimiter>,
    storage_backend: Arc<dyn StorageBackend>,
    recipient_cache: Arc<RwLock<HashMap<String, RecipientCacheEntry>>>,
    /// CRAM-MD5 challenge for ongoing authentication
    cram_md5_challenge: Option<String>,
    /// SCRAM-SHA-256 authentication state
    scram_state: Option<ScramState>,
}

/// SMTP session with stream
pub struct SmtpSessionHandler {
    session: SmtpSession,
    stream: TcpStream,
    /// Support for PIPELINING - buffer of commands to process
    #[allow(dead_code)]
    pipelined_commands: Vec<String>,
    /// Connection start time
    #[allow(dead_code)]
    connection_started: Instant,
    /// Last command received time
    #[allow(dead_code)]
    last_command: Instant,
}

impl SmtpSessionHandler {
    /// Create a new SMTP session handler
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stream: TcpStream,
        remote_addr: SocketAddr,
        config: SmtpConfig,
        processor_router: Arc<MailProcessorRouter>,
        auth_backend: Arc<dyn AuthBackend>,
        rate_limiter: Arc<RateLimiter>,
        storage_backend: Arc<dyn StorageBackend>,
    ) -> Self {
        let now = Instant::now();
        Self {
            session: SmtpSession {
                remote_addr,
                state: SmtpState::Connected,
                transaction: SmtpTransaction::new(),
                config,
                authenticated: false,
                username: None,
                relaying_allowed: false,
                processor_router,
                auth_backend,
                rate_limiter,
                storage_backend,
                recipient_cache: Arc::new(RwLock::new(HashMap::new())),
                cram_md5_challenge: None,
                scram_state: None,
            },
            stream,
            pipelined_commands: Vec::new(),
            connection_started: now,
            last_command: now,
        }
    }

    /// Handle the SMTP session
    pub async fn handle(mut self) -> anyhow::Result<()> {
        let (read_half, write_half) = tokio::io::split(self.stream);
        let mut reader = BufReader::new(read_half);
        let mut writer = BufWriter::new(write_half);

        // Send greeting
        Self::write_response_to(
            &mut writer,
            SmtpResponse::service_ready(&self.session.config.hostname),
            &self.session.remote_addr,
        )
        .await?;

        let mut line = String::new();

        loop {
            // Check total connection timeout
            if self.connection_started.elapsed() > self.session.config.connection_timeout {
                tracing::info!(
                    "Connection timeout exceeded for {}",
                    self.session.remote_addr
                );
                Self::write_response_to(
                    &mut writer,
                    SmtpResponse::new(421, "4.4.2 Connection timeout - closing connection"),
                    &self.session.remote_addr,
                )
                .await?;
                break;
            }

            line.clear();

            // Read command with idle timeout
            let n = match tokio::time::timeout(
                self.session.config.idle_timeout,
                reader.read_line(&mut line),
            )
            .await
            {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    tracing::error!("Read error from {}: {}", self.session.remote_addr, e);
                    break;
                }
                Err(_) => {
                    // Idle timeout
                    tracing::info!("Idle timeout for {}", self.session.remote_addr);
                    Self::write_response_to(
                        &mut writer,
                        SmtpResponse::new(421, "4.4.2 Idle timeout - closing connection"),
                        &self.session.remote_addr,
                    )
                    .await?;
                    break;
                }
            };

            if n == 0 {
                break; // EOF
            }

            // Update last command time
            self.last_command = Instant::now();

            let line_trimmed = line.trim();
            tracing::debug!(
                "SMTP command from {}: {}",
                self.session.remote_addr,
                line_trimmed
            );

            // Check if we're waiting for CRAM-MD5 response
            if self.session.cram_md5_challenge.is_some() {
                let response = match self.session.handle_cram_md5_response(line_trimmed).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        tracing::error!("Error handling CRAM-MD5 response: {}", e);
                        SmtpResponse::new(535, "5.7.8 Authentication credentials invalid")
                    }
                };
                Self::write_response_to(&mut writer, response, &self.session.remote_addr).await?;
                continue;
            }

            // Check if we're waiting for SCRAM-SHA-256 response
            if self.session.scram_state.is_some() {
                let response = match self.session.handle_scram_client_final(line_trimmed).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        tracing::error!("Error handling SCRAM-SHA-256 client-final: {}", e);
                        self.session.scram_state = None;
                        SmtpResponse::new(535, "5.7.8 Authentication credentials invalid")
                    }
                };
                Self::write_response_to(&mut writer, response, &self.session.remote_addr).await?;
                continue;
            }

            // Parse command
            let command = match parse_command(line_trimmed) {
                Ok(cmd) => cmd,
                Err(e) => {
                    tracing::warn!("Failed to parse command: {}", e);
                    Self::write_response_to(
                        &mut writer,
                        SmtpResponse::syntax_error("Command not recognized"),
                        &self.session.remote_addr,
                    )
                    .await?;
                    continue;
                }
            };

            // Handle command
            let response = match self.session.handle_command(command.clone()).await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::error!("Error handling command: {}", e);
                    SmtpResponse::local_error("Internal server error")
                }
            };

            Self::write_response_to(&mut writer, response, &self.session.remote_addr).await?;

            // Check if we should close the connection
            if self.session.state == SmtpState::Quit {
                break;
            }

            // For PIPELINING: DATA command requires special handling
            // After DATA is accepted, we must stop processing pipelined commands
            // and read the message data
            if matches!(command, SmtpCommand::Data) && self.session.state == SmtpState::Data {
                // Read message data (until .<CRLF>)
                let remote_addr = self.session.remote_addr;
                if let Err(e) = Self::handle_data_input(
                    &mut self.session,
                    &mut reader,
                    &mut writer,
                    &remote_addr,
                )
                .await
                {
                    tracing::error!("Error reading message data: {}", e);
                    Self::write_response_to(
                        &mut writer,
                        SmtpResponse::local_error("Error reading message data"),
                        &remote_addr,
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    /// Handle DATA input (message body)
    async fn handle_data_input<R, W>(
        session: &mut SmtpSession,
        reader: &mut R,
        writer: &mut W,
        remote_addr: &SocketAddr,
    ) -> anyhow::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        let mut message_data = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("Unexpected EOF during DATA"));
            }

            // Check for end of message (.<CRLF>)
            if line.trim() == "." {
                break;
            }

            // Remove transparency (leading dot)
            let line_to_add = if line.starts_with("..") {
                &line[1..]
            } else {
                &line
            };

            message_data.extend_from_slice(line_to_add.as_bytes());
        }

        // Check size limit
        let message_size = message_data.len();
        if message_size > session.config.max_message_size {
            Self::write_response_to(
                writer,
                SmtpResponse::storage_exceeded(format!(
                    "Message size {} exceeds maximum {}",
                    message_size, session.config.max_message_size
                )),
                remote_addr,
            )
            .await?;
            session.transaction.reset();
            session.state = SmtpState::Authenticated;
            return Ok(());
        }

        // Check declared size if provided
        if let Some(declared_size) = session.transaction.declared_size {
            // Allow some tolerance (10%) for encoding differences
            let max_allowed = declared_size + (declared_size / 10);
            if message_size > max_allowed {
                Self::write_response_to(
                    writer,
                    SmtpResponse::storage_exceeded(format!(
                        "Message size {} exceeds declared size {}",
                        message_size, declared_size
                    )),
                    remote_addr,
                )
                .await?;
                session.transaction.reset();
                session.state = SmtpState::Authenticated;
                return Ok(());
            }
        }

        // Message accepted
        session.transaction.message_size = message_size;
        session.transaction.message_data = message_data;

        let sender_display = session
            .transaction
            .sender
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        tracing::info!(
            "Accepted message from {} ({} bytes) with {} recipient(s)",
            sender_display,
            message_size,
            session.transaction.recipients.len()
        );

        // Send immediate response to client
        Self::write_response_to(
            writer,
            SmtpResponse::ok("Message accepted for delivery"),
            remote_addr,
        )
        .await?;

        // Process the message asynchronously
        tracing::info!("About to spawn async message processing task");
        let sender = session.transaction.sender.clone();
        let recipients = session.transaction.recipients.clone();
        let message_data = session.transaction.message_data.clone();
        let router = session.processor_router.clone();

        tracing::info!(
            "Spawning task for {} recipients, {} bytes",
            recipients.len(),
            message_data.len()
        );

        tokio::spawn(async move {
            tracing::info!("Inside spawned task - starting processing");
            if let Err(e) = SmtpSessionHandler::process_accepted_message(
                sender,
                recipients,
                message_data,
                router,
            )
            .await
            {
                tracing::error!("Failed to process message: {}", e);
            }
        });

        // Reset transaction state
        session.transaction.reset();
        session.state = SmtpState::Authenticated;

        Ok(())
    }

    /// Write a response to a writer
    async fn write_response_to<W: AsyncWriteExt + Unpin>(
        writer: &mut W,
        response: SmtpResponse,
        remote_addr: &SocketAddr,
    ) -> anyhow::Result<()> {
        let formatted = response.format();
        tracing::debug!("SMTP response to {}: {}", remote_addr, formatted.trim());
        writer.write_all(formatted.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Process an accepted message through the mail processor pipeline
    async fn process_accepted_message(
        sender: Option<rusmes_proto::MailAddress>,
        recipients: Vec<rusmes_proto::MailAddress>,
        message_data: Vec<u8>,
        router: Arc<rusmes_core::MailProcessorRouter>,
    ) -> anyhow::Result<()> {
        use bytes::Bytes;
        use rusmes_proto::{HeaderMap, Mail, MessageBody, MimeMessage};

        tracing::info!(
            "Starting message processing for {} recipients",
            recipients.len()
        );

        // Parse message headers and body
        let (headers, body_offset) = rusmes_proto::mime::parse_headers(&message_data)?;

        let mut header_map = HeaderMap::new();
        for (name, value) in headers {
            header_map.insert(name, value);
        }

        let body = if body_offset < message_data.len() {
            Bytes::from(message_data[body_offset..].to_vec())
        } else {
            Bytes::new()
        };

        let message = MimeMessage::new(header_map, MessageBody::Small(body));
        let mail = Mail::new(sender, recipients, message, None, None);

        tracing::info!("Processing mail {} through pipeline", mail.id());

        // Process through the router
        router.route(mail).await?;

        tracing::info!("Mail processing completed");
        Ok(())
    }
}

impl SmtpSession {
    /// Handle a single SMTP command
    async fn handle_command(&mut self, command: SmtpCommand) -> anyhow::Result<SmtpResponse> {
        match command {
            SmtpCommand::Helo(domain) => self.handle_helo(domain).await,
            SmtpCommand::Ehlo(domain) => self.handle_ehlo(domain).await,
            SmtpCommand::Mail { from, params } => self.handle_mail(from, params).await,
            SmtpCommand::Rcpt { to, params } => self.handle_rcpt(to, params).await,
            SmtpCommand::Data => self.handle_data().await,
            SmtpCommand::Bdat { chunk_size, last } => self.handle_bdat(chunk_size, last).await,
            SmtpCommand::Rset => self.handle_rset().await,
            SmtpCommand::Noop => Ok(SmtpResponse::ok_simple()),
            SmtpCommand::Quit => self.handle_quit().await,
            SmtpCommand::StartTls => self.handle_starttls().await,
            SmtpCommand::Auth {
                mechanism,
                initial_response,
            } => self.handle_auth(mechanism, initial_response).await,
            _ => Ok(SmtpResponse::not_implemented("Command not implemented")),
        }
    }

    /// Handle HELO command
    async fn handle_helo(&mut self, domain: String) -> anyhow::Result<SmtpResponse> {
        if self.state != SmtpState::Connected && self.state != SmtpState::Authenticated {
            return Ok(SmtpResponse::bad_sequence("Out of sequence"));
        }

        self.transaction.helo_name = Some(domain);
        self.state = SmtpState::Authenticated;

        Ok(SmtpResponse::ok(format!(
            "{} Hello {}",
            self.config.hostname,
            self.remote_addr.ip()
        )))
    }

    /// Handle EHLO command
    async fn handle_ehlo(&mut self, domain: String) -> anyhow::Result<SmtpResponse> {
        if self.state != SmtpState::Connected && self.state != SmtpState::Authenticated {
            return Ok(SmtpResponse::bad_sequence("Out of sequence"));
        }

        self.transaction.helo_name = Some(domain);
        self.state = SmtpState::Authenticated;

        let mut extensions = vec![
            format!("SIZE {}", self.config.max_message_size),
            "8BITMIME".to_string(),
            "SMTPUTF8".to_string(),
            "PIPELINING".to_string(),
            "CHUNKING".to_string(), // RFC 3030 - BDAT support
        ];

        if self.config.enable_starttls {
            extensions.push("STARTTLS".to_string());
        }

        if self.config.require_auth {
            extensions.push("AUTH PLAIN LOGIN CRAM-MD5 SCRAM-SHA-256".to_string());
        }

        Ok(SmtpResponse::ehlo(&self.config.hostname, extensions))
    }

    /// Handle MAIL FROM command
    async fn handle_mail(
        &mut self,
        from: MailAddress,
        params: Vec<crate::command::MailParam>,
    ) -> anyhow::Result<SmtpResponse> {
        if self.state != SmtpState::Authenticated {
            return Ok(SmtpResponse::bad_sequence("Must send HELO/EHLO first"));
        }

        if self.config.require_auth && !self.authenticated {
            return Ok(SmtpResponse::bad_sequence("Authentication required"));
        }

        // Check message rate limit
        let ip = self.remote_addr.ip();
        if !self.rate_limiter.allow_message(ip).await {
            tracing::warn!("Message rate limit exceeded for {} from {}", from, ip);
            return Ok(SmtpResponse::mailbox_unavailable(
                "Rate limit exceeded, please try again later",
            ));
        }

        // Process ESMTP parameters
        for param in params {
            match param.keyword.to_uppercase().as_str() {
                "SIZE" => {
                    // RFC 1870 - SIZE extension
                    if let Some(size_str) = param.value {
                        match size_str.parse::<usize>() {
                            Ok(size) => {
                                if size > self.config.max_message_size {
                                    return Ok(SmtpResponse::storage_exceeded(format!(
                                        "Message size {} exceeds maximum {}",
                                        size, self.config.max_message_size
                                    )));
                                }
                                self.transaction.declared_size = Some(size);
                            }
                            Err(_) => {
                                return Ok(SmtpResponse::parameter_error("Invalid SIZE parameter"));
                            }
                        }
                    } else {
                        return Ok(SmtpResponse::parameter_error(
                            "SIZE parameter requires a value",
                        ));
                    }
                }
                "BODY" => {
                    // RFC 6152 - 8BITMIME extension
                    if let Some(body_value) = param.value {
                        let body_upper = body_value.to_uppercase();
                        match body_upper.as_str() {
                            "7BIT" | "8BITMIME" => {
                                self.transaction.body_type = Some(body_upper);
                            }
                            _ => {
                                return Ok(SmtpResponse::parameter_not_implemented(format!(
                                    "Unsupported BODY type: {}",
                                    body_value
                                )));
                            }
                        }
                    } else {
                        return Ok(SmtpResponse::parameter_error(
                            "BODY parameter requires a value",
                        ));
                    }
                }
                "SMTPUTF8" => {
                    // RFC 6531 - SMTPUTF8 extension
                    // This parameter has no value
                    if param.value.is_none() {
                        self.transaction.smtputf8 = true;
                    } else {
                        return Ok(SmtpResponse::parameter_error(
                            "SMTPUTF8 parameter must not have a value",
                        ));
                    }
                }
                _ => {
                    // Unknown parameter - ignore per RFC 5321
                    tracing::debug!("Unknown MAIL parameter: {}", param.keyword);
                }
            }
        }

        self.transaction.sender = Some(from.clone());
        self.state = SmtpState::MailTransaction;

        Ok(SmtpResponse::ok(format!("Sender {} OK", from)))
    }

    /// Handle RCPT TO command
    async fn handle_rcpt(
        &mut self,
        to: MailAddress,
        params: Vec<crate::command::MailParam>,
    ) -> anyhow::Result<SmtpResponse> {
        if self.state != SmtpState::MailTransaction {
            return Ok(SmtpResponse::bad_sequence("Must send MAIL FROM first"));
        }

        // Process ESMTP parameters (for future extensions like DSN)
        for param in params {
            // Unknown parameter - ignore per RFC 5321
            tracing::debug!("Unknown RCPT parameter: {}", param.keyword);
        }

        // Check relay authorization
        if !self.is_relay_allowed(&to) {
            return Ok(SmtpResponse::new(550, "5.7.1 Relaying denied"));
        }

        // Validate recipient if configured
        if self.config.check_recipient_exists {
            // Skip validation for relay-authorized senders
            if !self.authenticated && !self.relaying_allowed {
                match self.validate_recipient(&to).await {
                    Ok(true) => {
                        // Recipient exists, continue
                    }
                    Ok(false) => {
                        if self.config.reject_unknown_recipients {
                            tracing::warn!("Rejecting unknown recipient: {}", to);
                            return Ok(SmtpResponse::new(
                                550,
                                format!("5.1.1 User unknown: {}", to),
                            ));
                        } else {
                            // Accept but log
                            tracing::info!(
                                "Accepting unknown recipient (rejection disabled): {}",
                                to
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error validating recipient {}: {}", to, e);
                        // On error, fail open to avoid blocking legitimate mail
                        tracing::warn!("Accepting recipient {} due to validation error", to);
                    }
                }
            }
        }

        // Add recipient
        self.transaction.recipients.push(to.clone());

        Ok(SmtpResponse::ok(format!("Recipient {} OK", to)))
    }

    /// Handle DATA command
    async fn handle_data(&mut self) -> anyhow::Result<SmtpResponse> {
        if self.state != SmtpState::MailTransaction {
            return Ok(SmtpResponse::bad_sequence("Must send RCPT TO first"));
        }

        if !self.transaction.is_valid() {
            return Ok(SmtpResponse::bad_sequence("Need at least one recipient"));
        }

        self.state = SmtpState::Data;
        Ok(SmtpResponse::start_data())
    }

    /// Handle BDAT command (RFC 3030 CHUNKING)
    ///
    /// This method only validates the command and prepares for chunk reception.
    /// Actual chunk data reading must be done by the caller after receiving this response.
    async fn handle_bdat(&mut self, chunk_size: usize, last: bool) -> anyhow::Result<SmtpResponse> {
        // BDAT can only be used after MAIL FROM and RCPT TO
        if self.state != SmtpState::MailTransaction {
            return Ok(SmtpResponse::bad_sequence(
                "Must send MAIL FROM and RCPT TO first",
            ));
        }

        if !self.transaction.is_valid() {
            return Ok(SmtpResponse::bad_sequence(
                "Need sender and at least one recipient",
            ));
        }

        // Initialize BDAT state if not already present
        if self.transaction.bdat_state.is_none() {
            self.transaction.bdat_state = Some(crate::BdatState::new(self.config.max_message_size));
        }

        // Note: The actual chunk data reading happens outside this method
        // The caller must read exactly chunk_size bytes and call add_chunk on bdat_state

        // Check if this would exceed size limits (preliminary check)
        // Safety: we just initialized bdat_state above if it was None
        let bdat_state = match self.transaction.bdat_state.as_ref() {
            Some(state) => state,
            None => {
                return Err(anyhow::anyhow!(
                    "Internal error: bdat_state not initialized"
                ))
            }
        };
        if bdat_state.total_size() + chunk_size > self.config.max_message_size {
            return Ok(SmtpResponse::storage_exceeded(format!(
                "Message size {} exceeds maximum {}",
                bdat_state.total_size() + chunk_size,
                self.config.max_message_size
            )));
        }

        // If this is the LAST chunk and message will be complete, log it
        if last {
            let sender_display = self
                .transaction
                .sender
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            tracing::info!(
                "BDAT LAST chunk ({} bytes) from {} with {} recipient(s)",
                chunk_size,
                sender_display,
                self.transaction.recipients.len()
            );
        }

        // Return success - caller must now read chunk_size bytes
        Ok(SmtpResponse::ok(format!("{} octets received", chunk_size)))
    }

    /// Handle RSET command
    async fn handle_rset(&mut self) -> anyhow::Result<SmtpResponse> {
        self.transaction.reset();
        self.state = SmtpState::Authenticated;
        Ok(SmtpResponse::ok_simple())
    }

    /// Handle QUIT command
    async fn handle_quit(&mut self) -> anyhow::Result<SmtpResponse> {
        self.state = SmtpState::Quit;
        Ok(SmtpResponse::closing())
    }

    /// Handle STARTTLS command
    async fn handle_starttls(&mut self) -> anyhow::Result<SmtpResponse> {
        if !self.config.enable_starttls {
            return Ok(SmtpResponse::not_implemented("STARTTLS not available"));
        }

        // In a real implementation, we would upgrade to TLS here
        Ok(SmtpResponse::new(220, "Ready to start TLS"))
    }

    /// Handle AUTH command
    async fn handle_auth(
        &mut self,
        mechanism: String,
        initial_response: Option<String>,
    ) -> anyhow::Result<SmtpResponse> {
        if !self.config.require_auth {
            return Ok(SmtpResponse::not_implemented("AUTH not available"));
        }

        match mechanism.to_uppercase().as_str() {
            "CRAM-MD5" => self.handle_auth_cram_md5().await,
            "SCRAM-SHA-256" => self.handle_auth_scram_sha256(initial_response).await,
            "PLAIN" => {
                if let Some(response) = initial_response {
                    self.handle_auth_plain(response).await
                } else {
                    // Request credentials
                    Ok(SmtpResponse::new(334, ""))
                }
            }
            "LOGIN" => {
                // LOGIN authentication requires multi-step exchange
                // Send "334 VXNlcm5hbWU6" (Username: in base64)
                Ok(SmtpResponse::new(334, "VXNlcm5hbWU6"))
            }
            _ => Ok(SmtpResponse::parameter_not_implemented(
                "Authentication mechanism not supported",
            )),
        }
    }

    /// Handle PLAIN authentication
    async fn handle_auth_plain(&mut self, response: String) -> anyhow::Result<SmtpResponse> {
        // Parse credentials
        let (username, password) = match crate::auth::parse_plain_auth(&response) {
            Ok(creds) => creds,
            Err(e) => {
                tracing::warn!("Failed to parse PLAIN auth: {}", e);
                return Ok(SmtpResponse::new(535, "5.7.8 Authentication failed"));
            }
        };

        // Create Username object
        let username_obj = match rusmes_proto::Username::new(username.clone()) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("Invalid username '{}': {}", username, e);
                return Ok(SmtpResponse::new(535, "5.7.8 Authentication failed"));
            }
        };

        // Authenticate with backend
        match self
            .auth_backend
            .authenticate(&username_obj, &password)
            .await
        {
            Ok(true) => {
                self.authenticated = true;
                self.username = Some(username.clone());
                tracing::info!("User '{}' authenticated successfully (PLAIN)", username);
                Ok(SmtpResponse::new(235, "2.7.0 Authentication successful"))
            }
            Ok(false) => {
                tracing::warn!("Authentication failed for user '{}'", username);
                Ok(SmtpResponse::new(535, "5.7.8 Authentication failed"))
            }
            Err(e) => {
                tracing::error!("Authentication error for user '{}': {}", username, e);
                Ok(SmtpResponse::new(535, "5.7.8 Authentication failed"))
            }
        }
    }

    /// Handle CRAM-MD5 authentication - send challenge
    async fn handle_auth_cram_md5(&mut self) -> anyhow::Result<SmtpResponse> {
        // Generate challenge
        let challenge = crate::auth::generate_cram_md5_challenge(&self.config.hostname)?;

        // Store challenge for verification
        self.cram_md5_challenge = Some(challenge.clone());

        // Encode and send challenge
        let encoded = crate::auth::encode_challenge(&challenge);
        Ok(SmtpResponse::new(334, encoded))
    }

    /// Handle CRAM-MD5 response
    async fn handle_cram_md5_response(
        &mut self,
        response_line: &str,
    ) -> anyhow::Result<SmtpResponse> {
        // Get the challenge (must be set)
        let challenge = self
            .cram_md5_challenge
            .take()
            .ok_or_else(|| anyhow::anyhow!("No CRAM-MD5 challenge pending"))?;

        // Check for SASL abort
        if response_line == "*" {
            tracing::info!("CRAM-MD5 authentication aborted by client");
            return Ok(SmtpResponse::new(501, "5.7.0 Authentication aborted"));
        }

        // Decode response
        let decoded = crate::auth::decode_response(response_line)?;

        // Parse username and HMAC
        let (username, client_hmac) = crate::auth::parse_cram_md5_response(&decoded)?;

        // IMPORTANT: CRAM-MD5 requires plaintext passwords or password-equivalent secrets
        // The current AuthBackend uses bcrypt, which is one-way hashing
        // For CRAM-MD5 to work, we would need:
        // 1. A separate plaintext password store (security risk)
        // 2. A password-equivalent secret store
        // 3. A different authentication backend
        //
        // For now, we'll try to authenticate but it will fail with bcrypt
        // This is documented limitation - CRAM-MD5 is not compatible with secure password storage

        tracing::warn!(
            "CRAM-MD5 authentication attempted for user '{}', but cannot verify HMAC with bcrypt-hashed passwords",
            username
        );

        // We cannot compute the expected HMAC without the plaintext password
        // The proper solution would be to store CRAM-MD5 secrets separately
        // or use a more modern authentication mechanism like SCRAM

        // For demonstration purposes, we log the authentication attempt
        tracing::info!(
            "CRAM-MD5 authentication for user '{}' from {} - challenge: {}, client_hmac: {}",
            username,
            self.remote_addr,
            challenge,
            client_hmac
        );

        // Check if user exists
        let username_obj = rusmes_proto::Username::new(username.to_string())
            .map_err(|e| anyhow::anyhow!("Invalid username: {}", e))?;

        let user_exists = self.auth_backend.verify_identity(&username_obj).await?;

        if !user_exists {
            tracing::warn!(
                "CRAM-MD5 authentication failed: user '{}' does not exist",
                username
            );
            return Ok(SmtpResponse::new(
                535,
                "5.7.8 Authentication credentials invalid",
            ));
        }

        // Since we cannot verify HMAC with bcrypt, reject the authentication
        // In a real implementation with plaintext or reversible password storage,
        // we would:
        // 1. Get password from auth backend
        // 2. Compute expected HMAC: compute_cram_md5_hmac(password, challenge)
        // 3. Compare with client_hmac (constant-time comparison)

        tracing::warn!(
            "CRAM-MD5 authentication rejected: mechanism requires plaintext password storage"
        );

        Ok(SmtpResponse::new(
            535,
            "5.7.8 Authentication credentials invalid",
        ))
    }

    /// Check if relay is allowed for the given recipient
    ///
    /// Returns `true` if:
    /// - User is authenticated, OR
    /// - Client IP is in relay_networks (CIDR notation), OR
    /// - Recipient domain is a local domain
    fn is_relay_allowed(&self, recipient: &MailAddress) -> bool {
        // Allow if authenticated
        if self.authenticated {
            tracing::debug!(
                "Relay allowed for {} from {}: authenticated user",
                recipient,
                self.remote_addr.ip()
            );
            return true;
        }

        // Allow if client IP is in relay_networks
        if crate::is_ip_in_networks(self.remote_addr.ip(), &self.config.relay_networks) {
            tracing::debug!(
                "Relay allowed for {} from {}: client IP in relay_networks",
                recipient,
                self.remote_addr.ip()
            );
            return true;
        }

        // Allow if recipient is local domain
        let recipient_domain = recipient.domain().as_str();
        for local_domain in &self.config.local_domains {
            if recipient_domain.eq_ignore_ascii_case(local_domain) {
                tracing::debug!(
                    "Relay allowed for {} from {}: local domain",
                    recipient,
                    self.remote_addr.ip()
                );
                return true;
            }
        }

        // Deny relay
        tracing::warn!(
            "Relay denied for {} from {}: not authenticated, not in relay_networks, not local domain",
            recipient,
            self.remote_addr.ip()
        );
        false
    }

    /// Validate recipient against storage backend with caching
    async fn validate_recipient(&self, recipient: &MailAddress) -> anyhow::Result<bool> {
        // Cache TTL: 5 minutes
        const CACHE_TTL: Duration = Duration::from_secs(300);

        let cache_key = recipient.as_string();

        // Check cache first
        {
            let cache = self.recipient_cache.read().await;
            if let Some(entry) = cache.get(&cache_key) {
                if entry.cached_at.elapsed() < CACHE_TTL {
                    tracing::debug!("Recipient validation cache hit for {}", recipient);
                    return Ok(entry.exists);
                }
            }
        }

        // Cache miss or expired, query storage backend
        tracing::debug!(
            "Recipient validation cache miss for {}, querying storage",
            recipient
        );

        // Extract username from mail address
        let username = Username::new(recipient.local_part())?;

        // Query storage backend for mailboxes
        let mailbox_store = self.storage_backend.mailbox_store();
        let mailboxes = mailbox_store.list_mailboxes(&username).await?;

        let exists = !mailboxes.is_empty();

        // Update cache
        {
            let mut cache = self.recipient_cache.write().await;
            cache.insert(
                cache_key,
                RecipientCacheEntry {
                    exists,
                    cached_at: Instant::now(),
                },
            );
        }

        Ok(exists)
    }

    /// Handle SCRAM-SHA-256 authentication - initial client-first message
    async fn handle_auth_scram_sha256(
        &mut self,
        initial_response: Option<String>,
    ) -> anyhow::Result<SmtpResponse> {
        // SCRAM-SHA-256 requires client-first message
        // If not provided, send 334 continuation to request it
        let client_first_encoded = match initial_response {
            Some(resp) => resp,
            None => {
                // Send initial continuation to request client-first
                return Ok(SmtpResponse::new(334, ""));
            }
        };

        // Decode client-first message
        let client_first_decoded = BASE64
            .decode(client_first_encoded.trim())
            .map_err(|e| anyhow::anyhow!("Failed to decode client-first: {}", e))?;
        let client_first_str = String::from_utf8(client_first_decoded)
            .map_err(|e| anyhow::anyhow!("Failed to decode UTF-8: {}", e))?;

        // Parse client-first message
        let (username, client_nonce, client_first_bare) =
            crate::auth::parse_scram_client_first(&client_first_str)?;

        // Generate server nonce and combine with client nonce
        let server_nonce = crate::auth::generate_scram_server_nonce()?;
        let nonce = format!("{}{}", client_nonce, server_nonce);

        // IMPORTANT: SCRAM-SHA-256 requires different credential storage than bcrypt
        // The current AuthBackend uses bcrypt which is incompatible with SCRAM
        // For SCRAM to work, we need:
        // 1. Salt and iteration count for PBKDF2
        // 2. StoredKey and ServerKey derived from password
        //
        // For now, we'll use placeholder values and document this limitation
        tracing::warn!(
            "SCRAM-SHA-256 authentication attempted for user '{}', but AuthBackend does not support SCRAM credentials",
            username
        );

        // Placeholder: Use fixed salt and iteration count
        // In production, these should be stored per-user in the auth backend
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt)
            .map_err(|e| anyhow::anyhow!("RNG failure generating SCRAM salt: {}", e))?;
        let iterations = 4096u32;

        // Build server-first message
        let server_first = format!("r={},s={},i={}", nonce, BASE64.encode(salt), iterations);

        // Store state for next round
        self.scram_state = Some(ScramState {
            client_first_bare: client_first_bare.clone(),
            server_first: server_first.clone(),
            nonce: nonce.clone(),
            username: username.clone(),
        });

        // Send server-first as base64-encoded 334 response
        let server_first_encoded = BASE64.encode(server_first.as_bytes());
        Ok(SmtpResponse::new(334, server_first_encoded))
    }

    /// Handle SCRAM-SHA-256 client-final message
    async fn handle_scram_client_final(
        &mut self,
        client_final_line: &str,
    ) -> anyhow::Result<SmtpResponse> {
        // Get stored state
        let state = self
            .scram_state
            .take()
            .ok_or_else(|| anyhow::anyhow!("No SCRAM state"))?;

        // Check for SASL abort
        if client_final_line == "*" {
            tracing::info!("SCRAM-SHA-256 authentication aborted by client");
            return Ok(SmtpResponse::new(501, "5.7.0 Authentication aborted"));
        }

        // Decode client-final message
        let client_final_decoded = BASE64
            .decode(client_final_line.trim())
            .map_err(|e| anyhow::anyhow!("Failed to decode client-final: {}", e))?;
        let client_final_str = String::from_utf8(client_final_decoded)
            .map_err(|e| anyhow::anyhow!("Failed to decode UTF-8: {}", e))?;

        // Parse client-final message
        let (_channel_binding, nonce, proof, client_final_without_proof) =
            crate::auth::parse_scram_client_final(&client_final_str)?;

        // Verify nonce matches
        if nonce != state.nonce {
            tracing::warn!(
                "SCRAM-SHA-256 nonce mismatch for user '{}': expected '{}', got '{}'",
                state.username,
                state.nonce,
                nonce
            );
            return Ok(SmtpResponse::new(
                535,
                "5.7.8 Authentication credentials invalid",
            ));
        }

        // Check if user exists
        let username_obj = rusmes_proto::Username::new(state.username.clone())
            .map_err(|e| anyhow::anyhow!("Invalid username: {}", e))?;

        let user_exists = self.auth_backend.verify_identity(&username_obj).await?;

        if !user_exists {
            tracing::warn!(
                "SCRAM-SHA-256 authentication failed: user '{}' does not exist",
                state.username
            );
            return Ok(SmtpResponse::new(
                535,
                "5.7.8 Authentication credentials invalid",
            ));
        }

        // IMPORTANT: Since AuthBackend doesn't support SCRAM credentials,
        // we cannot verify the client proof properly.
        //
        // In a real implementation with SCRAM support, we would:
        // 1. Get stored_key and server_key from auth backend
        // 2. Construct auth_message from client_first_bare, server_first, client_final_without_proof
        // 3. Verify client proof using verify_scram_client_proof()
        // 4. Compute server signature using compute_scram_server_signature()
        //
        // For now, we reject all SCRAM authentication attempts with a clear error message

        tracing::warn!(
            "SCRAM-SHA-256 authentication rejected for user '{}': AuthBackend does not support SCRAM credential storage",
            state.username
        );

        // Log the authentication attempt for debugging
        tracing::debug!(
            "SCRAM-SHA-256 auth attempt - client_first_bare: {}, server_first: {}, client_final_without_proof: {}, proof: {}",
            state.client_first_bare,
            state.server_first,
            client_final_without_proof,
            proof
        );

        Ok(SmtpResponse::new(
            535,
            "5.7.8 Authentication credentials invalid - SCRAM-SHA-256 requires separate credential storage",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_validity() {
        let mut tx = SmtpTransaction::new();
        assert!(!tx.is_valid());

        tx.sender = Some(
            "sender@example.com"
                .parse()
                .expect("valid email address literal"),
        );
        assert!(!tx.is_valid());

        tx.recipients.push(
            "rcpt@example.com"
                .parse()
                .expect("valid email address literal"),
        );
        assert!(tx.is_valid());

        tx.reset();
        assert!(!tx.is_valid());
    }

    #[test]
    fn test_smtp_config_default() {
        let config = SmtpConfig::default();
        assert_eq!(config.hostname, "localhost");
        assert_eq!(config.max_message_size, 10 * 1024 * 1024);
        assert!(!config.require_auth);
        assert!(!config.enable_starttls);
    }
}
