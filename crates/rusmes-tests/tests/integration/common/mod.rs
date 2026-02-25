//! Common test utilities and helpers for integration tests

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Test server instance
pub struct TestServer {
    process: Arc<Mutex<Option<Child>>>,
    config_path: PathBuf,
    data_dir: PathBuf,
    smtp_port: u16,
    imap_port: u16,
    pop3_port: u16,
    jmap_port: u16,
}

impl TestServer {
    /// Create a new test server with random ports
    pub fn new() -> std::io::Result<Self> {
        let temp_dir = std::env::temp_dir().join(format!("rusmes_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir)?;

        let data_dir = temp_dir.join("data");
        std::fs::create_dir_all(&data_dir)?;

        let config_path = temp_dir.join("config.toml");

        // Use random available ports
        let smtp_port = Self::find_free_port()?;
        let imap_port = Self::find_free_port()?;
        let pop3_port = Self::find_free_port()?;
        let jmap_port = Self::find_free_port()?;

        // Generate test config
        let config = format!(
            r#"
[server]
hostname = "localhost"
data_dir = "{}"

[smtp]
bind = "127.0.0.1:{}"
enabled = true

[imap]
bind = "127.0.0.1:{}"
enabled = true

[pop3]
bind = "127.0.0.1:{}"
enabled = true

[jmap]
bind = "127.0.0.1:{}"
enabled = true

[auth]
backend = "file"
file_path = "{}/users.txt"

[storage]
backend = "filesystem"
path = "{}/mail"
"#,
            data_dir.display(),
            smtp_port,
            imap_port,
            pop3_port,
            jmap_port,
            data_dir.display(),
            data_dir.display()
        );

        std::fs::write(&config_path, config)?;

        // Create test users file
        let users_file = data_dir.join("users.txt");
        std::fs::write(&users_file, "testuser:testpass\nadmin:admin123\n")?;

        Ok(Self {
            process: Arc::new(Mutex::new(None)),
            config_path,
            data_dir,
            smtp_port,
            imap_port,
            pop3_port,
            jmap_port,
        })
    }

    fn find_free_port() -> std::io::Result<u16> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        Ok(port)
    }

    /// Start the server
    pub async fn start(&self) -> std::io::Result<()> {
        let mut cmd = Command::new("cargo");
        cmd.args(["run", "--bin", "rusmes-server", "--"])
            .arg("--config")
            .arg(&self.config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut process = cmd.spawn()?;
        let _ = process.stdout.take();
        let _ = process.stderr.take();

        *self.process.lock().await = Some(process);

        // Wait for server to be ready
        self.wait_ready().await?;

        Ok(())
    }

    /// Wait for server to be ready (all ports listening)
    async fn wait_ready(&self) -> std::io::Result<()> {
        let max_attempts = 30;
        let delay = Duration::from_millis(100);

        for _ in 0..max_attempts {
            if self.check_all_ports().await {
                return Ok(());
            }
            sleep(delay).await;
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Server failed to start",
        ))
    }

    async fn check_all_ports(&self) -> bool {
        self.check_port(self.smtp_port).await
            && self.check_port(self.imap_port).await
            && self.check_port(self.pop3_port).await
            && self.check_port(self.jmap_port).await
    }

    async fn check_port(&self, port: u16) -> bool {
        TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
    }

    /// Stop the server
    pub async fn stop(&self) -> std::io::Result<()> {
        let mut process = self.process.lock().await;
        if let Some(mut p) = process.take() {
            let _ = p.kill();
            let _ = p.wait();
        }
        Ok(())
    }

    /// Get SMTP port
    #[allow(dead_code)]
    pub fn smtp_port(&self) -> u16 {
        self.smtp_port
    }

    /// Get IMAP port
    pub fn imap_port(&self) -> u16 {
        self.imap_port
    }

    /// Get POP3 port
    #[allow(dead_code)]
    pub fn pop3_port(&self) -> u16 {
        self.pop3_port
    }

    /// Get JMAP port
    #[allow(dead_code)]
    pub fn jmap_port(&self) -> u16 {
        self.jmap_port
    }

    /// Clean up test data
    pub fn cleanup(&self) -> std::io::Result<()> {
        if let Some(parent) = self.data_dir.parent() {
            std::fs::remove_dir_all(parent)?;
        }
        Ok(())
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.try_lock().ok().and_then(|mut p| p.take()) {
            let _ = process.kill();
        }
        let _ = self.cleanup();
    }
}

/// SMTP test client
#[allow(dead_code)]
pub struct SmtpClient {
    stream: BufReader<TcpStream>,
}

#[allow(dead_code)]
impl SmtpClient {
    pub async fn connect(addr: SocketAddr) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let mut client = Self {
            stream: BufReader::new(stream),
        };

        // Read greeting
        client.read_response().await?;

        Ok(client)
    }

    pub async fn ehlo(&mut self, hostname: &str) -> std::io::Result<String> {
        self.send_command(&format!("EHLO {}\r\n", hostname)).await?;
        self.read_response().await
    }

    pub async fn mail_from(&mut self, from: &str) -> std::io::Result<String> {
        self.send_command(&format!("MAIL FROM:<{}>\r\n", from))
            .await?;
        self.read_response().await
    }

    pub async fn rcpt_to(&mut self, to: &str) -> std::io::Result<String> {
        self.send_command(&format!("RCPT TO:<{}>\r\n", to)).await?;
        self.read_response().await
    }

    #[allow(dead_code)]
    pub async fn data(&mut self, content: &str) -> std::io::Result<String> {
        self.send_command("DATA\r\n").await?;
        self.read_response().await?;

        self.send_command(&format!("{}\r\n.\r\n", content)).await?;
        self.read_response().await
    }

    pub async fn quit(&mut self) -> std::io::Result<String> {
        self.send_command("QUIT\r\n").await?;
        self.read_response().await
    }

    async fn send_command(&mut self, cmd: &str) -> std::io::Result<()> {
        self.stream.get_mut().write_all(cmd.as_bytes()).await
    }

    async fn read_response(&mut self) -> std::io::Result<String> {
        let mut response = String::new();
        self.stream.read_line(&mut response).await?;
        Ok(response)
    }
}

/// IMAP test client
#[allow(dead_code)]
pub struct ImapClient {
    stream: BufReader<TcpStream>,
    tag_counter: u32,
}

impl ImapClient {
    #[allow(dead_code)]
    pub async fn connect(addr: SocketAddr) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let mut client = Self {
            stream: BufReader::new(stream),
            tag_counter: 0,
        };

        // Read greeting
        client.read_response().await?;

        Ok(client)
    }

    #[allow(dead_code)]
    pub async fn login(&mut self, username: &str, password: &str) -> std::io::Result<String> {
        self.send_command(&format!("LOGIN {} {}\r\n", username, password))
            .await
    }

    #[allow(dead_code)]
    pub async fn select(&mut self, mailbox: &str) -> std::io::Result<String> {
        self.send_command(&format!("SELECT {}\r\n", mailbox)).await
    }

    #[allow(dead_code)]
    pub async fn fetch(&mut self, sequence: &str, items: &str) -> std::io::Result<String> {
        self.send_command(&format!("FETCH {} {}\r\n", sequence, items))
            .await
    }

    #[allow(dead_code)]
    pub async fn logout(&mut self) -> std::io::Result<String> {
        self.send_command("LOGOUT\r\n").await
    }

    async fn send_command(&mut self, cmd: &str) -> std::io::Result<String> {
        self.tag_counter += 1;
        let tagged_cmd = format!("A{:04} {}", self.tag_counter, cmd);
        self.stream
            .get_mut()
            .write_all(tagged_cmd.as_bytes())
            .await?;
        self.read_response().await
    }

    async fn read_response(&mut self) -> std::io::Result<String> {
        let mut response = String::new();
        loop {
            let mut line = String::new();
            self.stream.read_line(&mut line).await?;
            response.push_str(&line);

            if line.starts_with(&format!("A{:04} ", self.tag_counter))
                || line.starts_with("* OK")
                || line.starts_with("* BYE")
            {
                break;
            }
        }
        Ok(response)
    }
}

/// POP3 test client
#[allow(dead_code)]
pub struct Pop3Client {
    stream: BufReader<TcpStream>,
}

impl Pop3Client {
    #[allow(dead_code)]
    pub async fn connect(addr: SocketAddr) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let mut client = Self {
            stream: BufReader::new(stream),
        };

        // Read greeting
        client.read_response().await?;

        Ok(client)
    }

    #[allow(dead_code)]
    pub async fn user(&mut self, username: &str) -> std::io::Result<String> {
        self.send_command(&format!("USER {}\r\n", username)).await
    }

    #[allow(dead_code)]
    pub async fn pass(&mut self, password: &str) -> std::io::Result<String> {
        self.send_command(&format!("PASS {}\r\n", password)).await
    }

    #[allow(dead_code)]
    pub async fn stat(&mut self) -> std::io::Result<String> {
        self.send_command("STAT\r\n").await
    }

    #[allow(dead_code)]
    pub async fn list(&mut self) -> std::io::Result<String> {
        self.send_command("LIST\r\n").await
    }

    #[allow(dead_code)]
    pub async fn retr(&mut self, msg_num: u32) -> std::io::Result<String> {
        self.send_command(&format!("RETR {}\r\n", msg_num)).await
    }

    #[allow(dead_code)]
    pub async fn quit(&mut self) -> std::io::Result<String> {
        self.send_command("QUIT\r\n").await
    }

    async fn send_command(&mut self, cmd: &str) -> std::io::Result<String> {
        self.stream.get_mut().write_all(cmd.as_bytes()).await?;
        self.read_response().await
    }

    async fn read_response(&mut self) -> std::io::Result<String> {
        let mut response = String::new();
        self.stream.read_line(&mut response).await?;
        Ok(response)
    }
}

/// JMAP test client
#[allow(dead_code)]
pub struct JmapClient {
    base_url: String,
    client: reqwest::Client,
    username: String,
    password: String,
}

impl JmapClient {
    #[allow(dead_code)]
    pub fn new(base_url: String, username: String, password: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
            username,
            password,
        }
    }

    #[allow(dead_code)]
    pub async fn session(&self) -> Result<serde_json::Value, reqwest::Error> {
        self.client
            .get(format!("{}/.well-known/jmap", self.base_url))
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?
            .json()
            .await
    }

    #[allow(dead_code)]
    pub async fn request(
        &self,
        method_calls: serde_json::Value,
    ) -> Result<serde_json::Value, reqwest::Error> {
        let request = serde_json::json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            "methodCalls": method_calls
        });

        self.client
            .post(format!("{}/jmap", self.base_url))
            .basic_auth(&self.username, Some(&self.password))
            .json(&request)
            .send()
            .await?
            .json()
            .await
    }
}

/// Test message generator
pub struct MessageGenerator;

impl MessageGenerator {
    #[allow(dead_code)]
    pub fn simple_message(from: &str, to: &str, subject: &str, body: &str) -> String {
        format!(
            "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\n\r\n{}",
            from,
            to,
            subject,
            chrono::Utc::now().to_rfc2822(),
            body
        )
    }

    #[allow(dead_code)]
    pub fn multipart_message(from: &str, to: &str, subject: &str) -> String {
        format!(
            r#"From: {}
To: {}
Subject: {}
Date: {}
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="boundary123"

--boundary123
Content-Type: text/plain; charset=utf-8

Plain text part

--boundary123
Content-Type: text/html; charset=utf-8

<html><body>HTML part</body></html>

--boundary123--
"#,
            from,
            to,
            subject,
            chrono::Utc::now().to_rfc2822()
        )
    }

    #[allow(dead_code)]
    pub fn with_attachments(from: &str, to: &str, subject: &str) -> String {
        format!(
            r#"From: {}
To: {}
Subject: {}
Date: {}
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="attach123"

--attach123
Content-Type: text/plain

Message with attachment

--attach123
Content-Type: application/octet-stream; name="file.txt"
Content-Transfer-Encoding: base64
Content-Disposition: attachment; filename="file.txt"

VGVzdCBmaWxlIGNvbnRlbnQ=

--attach123--
"#,
            from,
            to,
            subject,
            chrono::Utc::now().to_rfc2822()
        )
    }
}
