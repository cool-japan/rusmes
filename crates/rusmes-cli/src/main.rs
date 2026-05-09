//! RusMES CLI tool

use std::io::IsTerminal;
use std::process;

use clap::Parser;

use rusmes_cli::cli_def::{
    should_color, BackupAction, CliApp, Commands, MailboxAction, QueueAction, RestoreAction,
    UserAction,
};
use rusmes_cli::client::Client;
use rusmes_cli::commands;
use rusmes_storage::backends::filesystem::FilesystemBackend;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = CliApp::parse();

    // Honour NO_COLOR env-var (https://no-color.org/) before our own flag.
    // If NO_COLOR is set to any non-empty value, force color off regardless of
    // the --color flag, matching common tool conventions.
    let color_enabled = if std::env::var("NO_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        false
    } else {
        should_color(cli.color, std::io::stdout().is_terminal())
    };
    colored::control::set_override(color_enabled);

    let json = cli.json;
    let runtime_dir = cli.runtime_dir.clone();

    let result: anyhow::Result<()> = match cli.command {
        Commands::Init { domain } => commands::init::run(&domain),

        Commands::CheckConfig { config } => commands::check_config::run(&config),

        Commands::Status { watch } => {
            if let Some(interval) = watch {
                let rt_dir = runtime_dir.clone();
                commands::watch::run_watch_secs(interval, move || {
                    let rt = rt_dir.clone();
                    async move { commands::status::render(&rt, json) }
                })
                .await
            } else {
                commands::status::run(&runtime_dir, json)
            }
        }

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
                MailboxAction::Repair { mailbox, vacuum } => {
                    let data_dir = std::path::PathBuf::from(&cli.runtime_dir).join("mailboxes");
                    let backend = FilesystemBackend::new(&data_dir);
                    commands::mailbox::repair(&backend, mailbox.as_deref(), vacuum, json).await
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
                    if json {
                        let value = serde_json::to_string_pretty(&stats)?;
                        println!("{}", value);
                    } else {
                        stats.print();
                        migrator.print_report();
                    }
                    Ok(())
                }
                Err(e) => {
                    if !json {
                        eprintln!("Migration failed: {}", e);
                        migrator.print_report();
                    } else {
                        let err = serde_json::json!({ "error": e.to_string() });
                        eprintln!("{}", serde_json::to_string_pretty(&err)?);
                    }
                    Err(e)
                }
            }
        }

        Commands::Completions { shell } => {
            use clap::CommandFactory;
            let mut cmd = CliApp::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }

        Commands::Man => {
            use clap::CommandFactory;
            use clap_mangen::Man;
            use std::io::Write;
            let cmd = CliApp::command();
            let man = Man::new(cmd);
            let mut stdout = std::io::stdout();
            man.render(&mut stdout)?;
            stdout.flush()?;
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
    Ok(())
}
