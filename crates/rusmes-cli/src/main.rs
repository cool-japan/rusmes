//! RusMES CLI tool

use clap::{Parser, Subcommand};
use std::process;

use rusmes_cli::client::Client;
use rusmes_cli::commands;

#[derive(Parser)]
#[command(name = "rusmes")]
#[command(about = "RusMES - Rust Mail Enterprise Server", long_about = None)]
#[command(version)]
struct Cli {
    /// Server URL
    #[arg(long, env = "RUSMES_SERVER", default_value = "http://localhost:8080")]
    server: String,

    /// Output format (text or json)
    #[arg(long, value_enum, default_value = "text")]
    output: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Commands {
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
    Status,

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
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum UserAction {
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

#[derive(Subcommand)]
enum MailboxAction {
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

    /// Repair mailbox
    Repair {
        /// User email
        user: String,
        /// Mailbox name
        #[arg(long)]
        name: String,
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

#[derive(Subcommand)]
enum QueueAction {
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

#[derive(Subcommand)]
enum BackupAction {
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

#[derive(Subcommand)]
enum RestoreAction {
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

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum BackupFormat {
    TarGz,
    Binary,
}

impl From<BackupFormat> for commands::backup::BackupFormat {
    fn from(f: BackupFormat) -> Self {
        match f {
            BackupFormat::TarGz => commands::backup::BackupFormat::TarGz,
            BackupFormat::Binary => commands::backup::BackupFormat::Binary,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CompressionType {
    None,
    Gzip,
    Zstd,
}

impl From<CompressionType> for commands::backup::CompressionType {
    fn from(c: CompressionType) -> Self {
        match c {
            CompressionType::None => commands::backup::CompressionType::None,
            CompressionType::Gzip => commands::backup::CompressionType::Gzip,
            CompressionType::Zstd => commands::backup::CompressionType::Zstd,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let json = matches!(cli.output, OutputFormat::Json);

    let result: anyhow::Result<()> = match cli.command {
        Commands::Init { domain } => commands::init::run(&domain),

        Commands::CheckConfig { config } => commands::check_config::run(&config),

        Commands::Status => commands::status::run(),

        Commands::User { action } => {
            let client = Client::new(&cli.server)
                .map_err(|e| anyhow::anyhow!("Failed to connect to server: {}", e))?;
            match action {
                UserAction::Add {
                    email,
                    password,
                    quota,
                } => commands::user::add(&client, &email, &password, quota, json).await,
                UserAction::List => commands::user::list(&client, json).await,
                UserAction::Delete { email, force } => {
                    commands::user::delete(&client, &email, force, json).await
                }
                UserAction::Passwd { email, password } => {
                    commands::user::passwd(&client, &email, &password, json).await
                }
                UserAction::Show { email } => commands::user::show(&client, &email, json).await,
                UserAction::SetQuota { email, quota } => {
                    commands::user::set_quota(&client, &email, quota, json).await
                }
                UserAction::Enable { email } => commands::user::enable(&client, &email, json).await,
                UserAction::Disable { email } => {
                    commands::user::disable(&client, &email, json).await
                }
            }
        }

        Commands::Mailbox { action } => {
            let client = Client::new(&cli.server)
                .map_err(|e| anyhow::anyhow!("Failed to connect to server: {}", e))?;
            match action {
                MailboxAction::List { user } => commands::mailbox::list(&client, &user, json).await,
                MailboxAction::Create { user, name } => {
                    commands::mailbox::create(&client, &user, &name, json).await
                }
                MailboxAction::Delete { user, name, force } => {
                    commands::mailbox::delete(&client, &user, &name, force, json).await
                }
                MailboxAction::Rename {
                    user,
                    old_name,
                    new_name,
                } => commands::mailbox::rename(&client, &user, &old_name, &new_name, json).await,
                MailboxAction::Repair { user, name } => {
                    commands::mailbox::repair(&client, &user, &name, json).await
                }
                MailboxAction::Subscribe { user, name } => {
                    commands::mailbox::subscribe(&client, &user, &name, json).await
                }
                MailboxAction::Unsubscribe { user, name } => {
                    commands::mailbox::unsubscribe(&client, &user, &name, json).await
                }
                MailboxAction::Show { user, name } => {
                    commands::mailbox::show(&client, &user, &name, json).await
                }
            }
        }

        Commands::Queue { action } => {
            let client = Client::new(&cli.server)
                .map_err(|e| anyhow::anyhow!("Failed to connect to server: {}", e))?;
            match action {
                QueueAction::List { filter } => {
                    commands::queue::list(&client, json, filter.as_deref()).await
                }
                QueueAction::Flush => commands::queue::flush(&client, json).await,
                QueueAction::Inspect { message_id } => {
                    commands::queue::inspect(&client, &message_id, json).await
                }
                QueueAction::Delete { message_id } => {
                    commands::queue::delete(&client, &message_id, json).await
                }
                QueueAction::Retry { message_id } => {
                    commands::queue::retry(&client, &message_id, json).await
                }
                QueueAction::Purge => commands::queue::purge(&client, json).await,
                QueueAction::Stats => commands::queue::stats(&client, json).await,
            }
        }

        Commands::Backup { action } => {
            let client = Client::new(&cli.server)
                .map_err(|e| anyhow::anyhow!("Failed to connect to server: {}", e))?;
            match action {
                BackupAction::Full {
                    output,
                    format,
                    compression,
                    encrypt,
                } => {
                    commands::backup::full(
                        &client,
                        &output,
                        format.into(),
                        compression.into(),
                        encrypt,
                        None,  // password_file
                        false, // verify
                        json,
                    )
                    .await
                }
                BackupAction::Incremental {
                    output,
                    base,
                    format,
                    compression,
                    encrypt,
                } => {
                    commands::backup::incremental(
                        &client,
                        &output,
                        &base,
                        format.into(),
                        compression.into(),
                        encrypt,
                        None,  // password_file
                        false, // verify
                        json,
                    )
                    .await
                }
                BackupAction::List => commands::backup::list_backups(&client, json).await,
                BackupAction::Verify { backup, key } => {
                    commands::backup::verify(&client, &backup, key.as_deref(), json).await
                }
                BackupAction::UploadS3 {
                    backup,
                    bucket,
                    region,
                    access_key,
                    secret_key,
                } => {
                    commands::backup::upload_s3(
                        &backup,
                        &bucket,
                        &region,
                        None, // endpoint
                        &access_key,
                        &secret_key,
                        None, // prefix
                        json,
                    )
                    .await
                }
            }
        }

        Commands::Restore { action } => {
            let client = Client::new(&cli.server)?;
            match action {
                RestoreAction::Restore {
                    backup,
                    key,
                    point_in_time,
                    dry_run,
                } => {
                    commands::restore::restore(
                        &client,
                        &backup,
                        key.as_deref(),
                        None, // password_file
                        point_in_time.as_deref(),
                        dry_run,
                        false, // verify
                        json,
                    )
                    .await
                }
                RestoreAction::User {
                    backup,
                    user,
                    key,
                    dry_run,
                } => {
                    commands::restore::restore_user(
                        &client,
                        &backup,
                        &user,
                        key.as_deref(),
                        None, // password_file
                        dry_run,
                        false, // verify
                        json,
                    )
                    .await
                }
                RestoreAction::FromS3 {
                    s3_url,
                    bucket,
                    region,
                    access_key,
                    secret_key,
                    key,
                } => {
                    commands::restore::restore_from_s3(
                        &client,
                        &s3_url,
                        &bucket,
                        &region,
                        &access_key,
                        &secret_key,
                        key.as_deref(),
                        json,
                    )
                    .await
                }
                RestoreAction::History => commands::restore::history(&client, json).await,
                RestoreAction::Show { restore_id } => {
                    commands::restore::show_restore(&client, &restore_id, json).await
                }
            }
        }

        Commands::Migrate {
            from,
            to,
            source_config,
            dest_config,
            batch_size,
            parallel,
            verify,
            dry_run,
            resume,
        } => {
            use commands::migrate::{BackendType, MigrationConfig, StorageMigrator};

            #[allow(clippy::too_many_arguments)]
            async fn run_migration(
                from: String,
                to: String,
                source_config: Option<String>,
                dest_config: Option<String>,
                batch_size: usize,
                parallel: usize,
                verify: bool,
                dry_run: bool,
                resume: bool,
            ) -> anyhow::Result<()> {
                let source_type: BackendType = from.parse()?;
                let dest_type: BackendType = to.parse()?;

                let default_source_config = match source_type {
                    BackendType::Filesystem => "/var/lib/rusmes/mail".to_string(),
                    BackendType::Postgres => "postgresql://localhost/rusmes".to_string(),
                    BackendType::Amaters => "http://localhost:8081".to_string(),
                };

                let default_dest_config = match dest_type {
                    BackendType::Filesystem => "/var/lib/rusmes/mail_new".to_string(),
                    BackendType::Postgres => "postgresql://localhost/rusmes_new".to_string(),
                    BackendType::Amaters => "http://localhost:8082".to_string(),
                };

                let config = MigrationConfig {
                    source_type,
                    source_config: source_config.unwrap_or(default_source_config),
                    dest_type,
                    dest_config: dest_config.unwrap_or(default_dest_config),
                    batch_size,
                    parallel,
                    verify,
                    dry_run,
                    resume,
                };

                let mut migrator = StorageMigrator::new(config);

                match migrator.migrate().await {
                    Ok(stats) => {
                        stats.print();
                        migrator.print_report();
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("Migration failed: {}", e);
                        migrator.print_report();
                        Err(e)
                    }
                }
            }

            run_migration(
                from,
                to,
                source_config,
                dest_config,
                batch_size,
                parallel,
                verify,
                dry_run,
                resume,
            )
            .await
        }

        Commands::Completions { shell } => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
    Ok(())
}
