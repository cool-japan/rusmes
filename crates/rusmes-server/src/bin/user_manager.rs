//! User management utility for FileAuthBackend
//!
//! This utility helps manage users in the file-based authentication backend.
//! It provides commands to add, delete, and list users with bcrypt password hashing.
//!
//! Usage:
//!   user_manager add `<username>` `<password>` \[--file PATH\]
//!   user_manager delete `<username>` \[--file PATH\]
//!   user_manager list \[--file PATH\]
//!   user_manager change-password `<username>` `<new_password>` \[--file PATH\]
//!   user_manager verify `<username>` `<password>` \[--file PATH\]

use anyhow::{Context, Result};
use rusmes_auth::file::FileAuthBackend;
use rusmes_auth::AuthBackend;
use rusmes_proto::Username;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let command = &args[1];
    let default_file = "/tmp/rusmes/users.txt";

    match command.as_str() {
        "add" => {
            if args.len() < 4 {
                eprintln!("Error: 'add' requires username and password");
                eprintln!("Usage: {} add <username> <password> [--file PATH]", args[0]);
                std::process::exit(1);
            }

            let username_str = &args[2];
            let password = &args[3];
            let file_path = get_file_path(&args, 4, default_file);

            let username = Username::new(username_str.to_string()).context("Invalid username")?;

            let backend = FileAuthBackend::new(&file_path).await?;
            backend.create_user(&username, password).await?;

            println!(
                "User '{}' added successfully to {}",
                username_str, file_path
            );
        }
        "delete" => {
            if args.len() < 3 {
                eprintln!("Error: 'delete' requires username");
                eprintln!("Usage: {} delete <username> [--file PATH]", args[0]);
                std::process::exit(1);
            }

            let username_str = &args[2];
            let file_path = get_file_path(&args, 3, default_file);

            let username = Username::new(username_str.to_string()).context("Invalid username")?;

            let backend = FileAuthBackend::new(&file_path).await?;
            backend.delete_user(&username).await?;

            println!(
                "User '{}' deleted successfully from {}",
                username_str, file_path
            );
        }
        "list" => {
            let file_path = get_file_path(&args, 2, default_file);

            let backend = FileAuthBackend::new(&file_path).await?;
            let users = backend.list_users().await?;

            if users.is_empty() {
                println!("No users found in {}", file_path);
            } else {
                println!("Users in {}:", file_path);
                for user in users {
                    println!("  - {}", user.as_str());
                }
            }
        }
        "change-password" => {
            if args.len() < 4 {
                eprintln!("Error: 'change-password' requires username and new password");
                eprintln!(
                    "Usage: {} change-password <username> <new_password> [--file PATH]",
                    args[0]
                );
                std::process::exit(1);
            }

            let username_str = &args[2];
            let new_password = &args[3];
            let file_path = get_file_path(&args, 4, default_file);

            let username = Username::new(username_str.to_string()).context("Invalid username")?;

            let backend = FileAuthBackend::new(&file_path).await?;
            backend.change_password(&username, new_password).await?;

            println!(
                "Password changed successfully for user '{}' in {}",
                username_str, file_path
            );
        }
        "verify" => {
            if args.len() < 4 {
                eprintln!("Error: 'verify' requires username and password");
                eprintln!(
                    "Usage: {} verify <username> <password> [--file PATH]",
                    args[0]
                );
                std::process::exit(1);
            }

            let username_str = &args[2];
            let password = &args[3];
            let file_path = get_file_path(&args, 4, default_file);

            let username = Username::new(username_str.to_string()).context("Invalid username")?;

            let backend = FileAuthBackend::new(&file_path).await?;
            let result = backend.authenticate(&username, password).await?;

            if result {
                println!("Authentication successful for user '{}'", username_str);
                std::process::exit(0);
            } else {
                println!("Authentication failed for user '{}'", username_str);
                std::process::exit(1);
            }
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        _ => {
            eprintln!("Error: Unknown command '{}'", command);
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn get_file_path(args: &[String], start_idx: usize, default: &str) -> String {
    if args.len() > start_idx && args[start_idx] == "--file" {
        if args.len() > start_idx + 1 {
            args[start_idx + 1].clone()
        } else {
            eprintln!("Error: --file requires a path argument");
            std::process::exit(1);
        }
    } else {
        default.to_string()
    }
}

fn print_usage() {
    println!("User Management Utility for RusMES FileAuthBackend");
    println!();
    println!("USAGE:");
    println!("  user_manager <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("  add <username> <password>           Add a new user");
    println!("  delete <username>                   Delete a user");
    println!("  list                                List all users");
    println!("  change-password <username> <pass>   Change a user's password");
    println!("  verify <username> <password>        Verify user credentials");
    println!("  help                                Show this help message");
    println!();
    println!("OPTIONS:");
    println!("  --file <PATH>                       Path to password file");
    println!("                                      (default: /tmp/rusmes/users.txt)");
    println!();
    println!("EXAMPLES:");
    println!("  # Add a user");
    println!("  user_manager add testuser testpass");
    println!();
    println!("  # Add a user to custom file");
    println!("  user_manager add admin secretpass --file /etc/rusmes/users.txt");
    println!();
    println!("  # List all users");
    println!("  user_manager list");
    println!();
    println!("  # Change password");
    println!("  user_manager change-password testuser newpass");
    println!();
    println!("  # Verify credentials");
    println!("  user_manager verify testuser testpass");
    println!();
    println!("  # Delete a user");
    println!("  user_manager delete testuser");
}
