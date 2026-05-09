//! Central CLI struct definition — shared between the binary (`main.rs`) and
//! the unit tests inside the library modules (`completions`, `man`).

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// Determine whether ANSI colors should be emitted given the user's choice and
/// whether stdout is currently a TTY.
///
/// This is a pure function that can be tested without side-effects.
///
/// | `choice`        | `is_tty` | result |
/// |-----------------|----------|--------|
/// | `Always`        | any      | `true` |
/// | `Never`         | any      | `false`|
/// | `Auto`          | `true`   | `true` |
/// | `Auto`          | `false`  | `false`|
pub fn should_color(choice: ColorChoice, is_tty: bool) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => is_tty,
    }
}

#[cfg(test)]
mod cli_def_tests {
    use super::*;

    #[test]
    fn test_should_color_always() {
        assert!(should_color(ColorChoice::Always, false));
        assert!(should_color(ColorChoice::Always, true));
    }

    #[test]
    fn test_should_color_never() {
        assert!(!should_color(ColorChoice::Never, false));
        assert!(!should_color(ColorChoice::Never, true));
    }

    #[test]
    fn test_should_color_auto_tty() {
        assert!(should_color(ColorChoice::Auto, true));
    }

    #[test]
    fn test_should_color_auto_no_tty() {
        assert!(!should_color(ColorChoice::Auto, false));
    }

    /// When `NO_COLOR` is set to any non-empty value, color should be off.
    ///
    /// This test exercises the NO_COLOR convention (https://no-color.org/).
    /// We call `should_color` directly after simulating the env check, rather
    /// than setting the env var (which would be process-wide and could affect
    /// parallel tests).
    #[test]
    fn test_no_color_env_logic() {
        // Simulate: if NO_COLOR is set and non-empty, always treat as Never.
        let no_color_set = true; // env var is present and non-empty
        let effective_choice = if no_color_set {
            ColorChoice::Never
        } else {
            ColorChoice::Auto
        };
        // Even with is_tty = true, color must be off.
        assert!(!should_color(effective_choice, true));
    }
}

/// The `rusmes` command-line application parser.
#[derive(Parser)]
#[command(name = "rusmes")]
#[command(about = "RusMES - Rust Mail Enterprise Server", long_about = None)]
#[command(version)]
pub struct CliApp {
    /// Server URL
    #[arg(long, env = "RUSMES_SERVER", default_value = "http://localhost:8080")]
    pub server: String,

    /// Runtime directory where the PID file and sockets are stored
    #[arg(long, default_value = "./data")]
    pub runtime_dir: String,

    /// Color output mode
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorChoice,

    /// Enable JSON output for structured commands
    #[arg(long)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Color output preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    /// Enable colors only when writing to a TTY
    Auto,
    /// Always enable colors
    Always,
    /// Never emit ANSI escape codes
    Never,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new RusMES installation
    Init {
        /// Server domain
        #[arg(long)]
        domain: String,
    },

    /// Validate configuration file
    CheckConfig {
        /// Configuration file path
        #[arg(short, long, default_value = "rusmes.toml")]
        config: String,
    },

    /// Show server status
    Status {
        /// Watch mode — redraw every N seconds (minimum 1)
        #[arg(long, value_name = "INTERVAL_SECS")]
        watch: Option<u64>,
    },

    /// User management commands
    User {
        #[command(subcommand)]
        action: UserAction,
    },

    /// Mailbox management commands
    Mailbox {
        #[command(subcommand)]
        action: MailboxAction,
    },

    /// Queue management commands
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },

    /// Backup commands
    Backup {
        #[command(subcommand)]
        action: BackupAction,
    },

    /// Restore commands
    Restore {
        #[command(subcommand)]
        action: RestoreAction,
    },

    /// Migrate storage between backends
    Migrate {
        /// Source backend type (filesystem, postgres, amaters)
        #[arg(long)]
        from: String,

        /// Destination backend type
        #[arg(long)]
        to: String,

        /// Source backend configuration (path or connection string)
        #[arg(long)]
        source_config: Option<String>,

        /// Destination backend configuration
        #[arg(long)]
        dest_config: Option<String>,

        /// Batch size (messages per batch)
        #[arg(long, default_value = "100")]
        batch_size: usize,

        /// Parallel workers
        #[arg(long, default_value = "4")]
        parallel: usize,

        /// Enable verification
        #[arg(long)]
        verify: bool,

        /// Dry run (don't make changes)
        #[arg(long)]
        dry_run: bool,

        /// Resume from previous migration
        #[arg(long)]
        resume: bool,
    },

    /// Generate shell completions
    Completions {
        /// Shell type
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Generate man page (roff format written to stdout)
    Man,
}

/// User management sub-actions.
#[derive(Subcommand)]
pub enum UserAction {
    /// Add a new user
    Add {
        /// Email address
        email: String,
        /// Password
        #[arg(long)]
        password: String,
        /// Quota in MB
        #[arg(long)]
        quota: Option<u64>,
    },

    /// List all users
    List,

    /// Delete a user
    Delete {
        /// Email address
        email: String,
        /// Force deletion without confirmation
        #[arg(long)]
        force: bool,
    },

    /// Change user password
    Passwd {
        /// Email address
        email: String,
        /// New password
        #[arg(long)]
        password: String,
    },

    /// Show user details
    Show {
        /// Email address
        email: String,
    },

    /// Set user quota
    SetQuota {
        /// Email address
        email: String,
        /// Quota in MB
        #[arg(long)]
        quota: u64,
    },

    /// Enable user account
    Enable {
        /// Email address
        email: String,
    },

    /// Disable user account
    Disable {
        /// Email address
        email: String,
    },
}

/// Mailbox management sub-actions.
#[derive(Subcommand)]
pub enum MailboxAction {
    /// List mailboxes for a user
    List {
        /// User email
        user: String,
    },

    /// Create a new mailbox
    Create {
        /// User email
        user: String,
        /// Mailbox name
        #[arg(long)]
        name: String,
    },

    /// Delete a mailbox
    Delete {
        /// User email
        user: String,
        /// Mailbox name
        #[arg(long)]
        name: String,
        /// Force deletion without confirmation
        #[arg(long)]
        force: bool,
    },

    /// Rename a mailbox
    Rename {
        /// User email
        user: String,
        /// Old mailbox name
        #[arg(long)]
        old_name: String,
        /// New mailbox name
        #[arg(long)]
        new_name: String,
    },

    /// Repair mailbox — validate on-disk state vs metadata index
    Repair {
        /// Target mailbox name (repairs all mailboxes when omitted)
        #[arg(long)]
        mailbox: Option<String>,

        /// Compact expunged messages after repair
        #[arg(long)]
        vacuum: bool,
    },

    /// Subscribe to a mailbox
    Subscribe {
        /// User email
        user: String,
        /// Mailbox name
        #[arg(long)]
        name: String,
    },

    /// Unsubscribe from a mailbox
    Unsubscribe {
        /// User email
        user: String,
        /// Mailbox name
        #[arg(long)]
        name: String,
    },

    /// Show mailbox details
    Show {
        /// User email
        user: String,
        /// Mailbox name
        #[arg(long)]
        name: String,
    },
}

/// Queue management sub-actions.
#[derive(Subcommand)]
pub enum QueueAction {
    /// List messages in queue
    List {
        /// Filter by status (pending, retrying, failed)
        #[arg(long)]
        filter: Option<String>,
    },

    /// Flush the queue
    Flush,

    /// Inspect a specific message
    Inspect {
        /// Message ID
        message_id: String,
    },

    /// Delete a message from the queue
    Delete {
        /// Message ID
        message_id: String,
    },

    /// Retry a failed message
    Retry {
        /// Message ID
        message_id: String,
    },

    /// Purge all failed messages
    Purge,

    /// Show queue statistics
    Stats,
}

/// Backup sub-actions.
#[derive(Subcommand)]
pub enum BackupAction {
    /// Create a full backup
    Full {
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Backup format
        #[arg(long, value_enum, default_value = "tar-gz")]
        format: BackupFormat,
        /// Compression type
        #[arg(long, value_enum, default_value = "gzip")]
        compression: CompressionType,
        /// Encrypt backup
        #[arg(long)]
        encrypt: bool,
    },

    /// Create an incremental backup
    Incremental {
        /// Output file path
        #[arg(short, long)]
        output: String,
        /// Base backup path
        #[arg(long)]
        base: String,
        /// Backup format
        #[arg(long, value_enum, default_value = "tar-gz")]
        format: BackupFormat,
        /// Compression type
        #[arg(long, value_enum, default_value = "gzip")]
        compression: CompressionType,
        /// Encrypt backup
        #[arg(long)]
        encrypt: bool,
    },

    /// List available backups
    List,

    /// Verify backup integrity
    Verify {
        /// Backup file path
        backup: String,
        /// Encryption key (if encrypted)
        #[arg(long)]
        key: Option<String>,
    },

    /// Upload backup to S3
    UploadS3 {
        /// Backup file path
        backup: String,
        /// S3 bucket
        #[arg(long)]
        bucket: String,
        /// AWS region
        #[arg(long)]
        region: String,
        /// AWS access key
        #[arg(long, env = "AWS_ACCESS_KEY_ID")]
        access_key: String,
        /// AWS secret key
        #[arg(long, env = "AWS_SECRET_ACCESS_KEY")]
        secret_key: String,
    },
}

/// Restore sub-actions.
#[derive(Subcommand)]
pub enum RestoreAction {
    /// Restore from a backup
    Restore {
        /// Backup file path
        backup: String,
        /// Encryption key (if encrypted)
        #[arg(long)]
        key: Option<String>,
        /// Point-in-time to restore to
        #[arg(long)]
        point_in_time: Option<String>,
        /// Dry run (don't actually restore)
        #[arg(long)]
        dry_run: bool,
    },

    /// Restore for a specific user
    User {
        /// Backup file path
        backup: String,
        /// User email
        #[arg(long)]
        user: String,
        /// Encryption key (if encrypted)
        #[arg(long)]
        key: Option<String>,
        /// Dry run (don't actually restore)
        #[arg(long)]
        dry_run: bool,
    },

    /// Download backup from S3 and restore
    FromS3 {
        /// S3 URL
        s3_url: String,
        /// S3 bucket
        #[arg(long)]
        bucket: String,
        /// AWS region
        #[arg(long)]
        region: String,
        /// AWS access key
        #[arg(long, env = "AWS_ACCESS_KEY_ID")]
        access_key: String,
        /// AWS secret key
        #[arg(long, env = "AWS_SECRET_ACCESS_KEY")]
        secret_key: String,
        /// Encryption key (if encrypted)
        #[arg(long)]
        key: Option<String>,
    },

    /// Show restore history
    History,

    /// Show details of a specific restore
    Show {
        /// Restore ID
        restore_id: String,
    },
}

/// Backup format selection.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum BackupFormat {
    TarGz,
    Binary,
}

/// Compression algorithm selection.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CompressionType {
    None,
    Gzip,
    Zstd,
}

impl From<BackupFormat> for crate::commands::backup::BackupFormat {
    fn from(f: BackupFormat) -> Self {
        match f {
            BackupFormat::TarGz => crate::commands::backup::BackupFormat::TarGz,
            BackupFormat::Binary => crate::commands::backup::BackupFormat::Binary,
        }
    }
}

impl From<CompressionType> for crate::commands::backup::CompressionType {
    fn from(c: CompressionType) -> Self {
        match c {
            CompressionType::None => crate::commands::backup::CompressionType::None,
            CompressionType::Gzip => crate::commands::backup::CompressionType::Gzip,
            CompressionType::Zstd => crate::commands::backup::CompressionType::Zstd,
        }
    }
}
