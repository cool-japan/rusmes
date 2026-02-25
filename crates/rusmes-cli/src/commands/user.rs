//! User management commands

use anyhow::Result;
use colored::*;
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

use crate::client::Client;

#[derive(Debug, Serialize, Deserialize, Tabled)]
pub struct UserInfo {
    pub email: String,
    pub created_at: String,
    pub quota_used: u64,
    pub quota_limit: u64,
    pub enabled: bool,
}

/// Add a new user
pub async fn add(
    client: &Client,
    email: &str,
    password: &str,
    quota: Option<u64>,
    json: bool,
) -> Result<()> {
    #[derive(Serialize)]
    struct AddUserRequest {
        email: String,
        password: String,
        quota: Option<u64>,
    }

    #[derive(Deserialize, Serialize)]
    struct AddUserResponse {
        success: bool,
    }

    let request = AddUserRequest {
        email: email.to_string(),
        password: password.to_string(),
        quota,
    };

    let response: AddUserResponse = client.post("/api/users", &request).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ User {} created successfully", email)
                .green()
                .bold()
        );
        if let Some(q) = quota {
            println!("  Quota: {} MB", q / (1024 * 1024));
        }
    }

    Ok(())
}

/// List all users
pub async fn list(client: &Client, json: bool) -> Result<()> {
    let users: Vec<UserInfo> = client.get("/api/users").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&users)?);
    } else {
        if users.is_empty() {
            println!("{}", "No users found".yellow());
            return Ok(());
        }

        let table = Table::new(&users).to_string();
        println!("{}", table);
        println!("\n{} total users", users.len().to_string().bold());
    }

    Ok(())
}

/// Delete a user
pub async fn delete(client: &Client, email: &str, force: bool, json: bool) -> Result<()> {
    if !force && !json {
        println!("{}", format!("Delete user {}?", email).yellow());
        println!("This will delete all mailboxes and messages for this user.");
        println!("Use --force to skip this confirmation.");

        use std::io::{self, Write};
        print!("Continue? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    #[derive(Deserialize, Serialize)]
    struct DeleteResponse {
        success: bool,
    }

    let response: DeleteResponse = client.delete(&format!("/api/users/{}", email)).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ User {} deleted successfully", email)
                .green()
                .bold()
        );
    }

    Ok(())
}

/// Change user password
pub async fn passwd(client: &Client, email: &str, password: &str, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct PasswdRequest {
        password: String,
    }

    #[derive(Deserialize, Serialize)]
    struct PasswdResponse {
        success: bool,
    }

    let request = PasswdRequest {
        password: password.to_string(),
    };

    let response: PasswdResponse = client
        .put(&format!("/api/users/{}/password", email), &request)
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Password changed for {}", email).green().bold()
        );
    }

    Ok(())
}

/// Show user details
pub async fn show(client: &Client, email: &str, json: bool) -> Result<()> {
    let user: UserInfo = client.get(&format!("/api/users/{}", email)).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&user)?);
    } else {
        println!("{}", format!("User: {}", email).bold());
        println!("  Created: {}", user.created_at);
        println!(
            "  Status: {}",
            if user.enabled {
                "Enabled".green()
            } else {
                "Disabled".red()
            }
        );
        println!(
            "  Quota: {} / {} MB ({:.1}%)",
            user.quota_used / (1024 * 1024),
            user.quota_limit / (1024 * 1024),
            (user.quota_used as f64 / user.quota_limit as f64) * 100.0
        );
    }

    Ok(())
}

/// Set user quota
pub async fn set_quota(client: &Client, email: &str, quota_mb: u64, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct QuotaRequest {
        quota: u64,
    }

    #[derive(Deserialize, Serialize)]
    struct QuotaResponse {
        success: bool,
    }

    let request = QuotaRequest {
        quota: quota_mb * 1024 * 1024,
    };

    let response: QuotaResponse = client
        .put(&format!("/api/users/{}/quota", email), &request)
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}",
            format!("✓ Quota set to {} MB for {}", quota_mb, email)
                .green()
                .bold()
        );
    }

    Ok(())
}

/// Enable a user account
pub async fn enable(client: &Client, email: &str, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct EnableRequest {
        enabled: bool,
    }

    #[derive(Deserialize, Serialize)]
    struct EnableResponse {
        success: bool,
    }

    let request = EnableRequest { enabled: true };

    let response: EnableResponse = client
        .put(&format!("/api/users/{}/status", email), &request)
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", format!("✓ User {} enabled", email).green().bold());
    }

    Ok(())
}

/// Disable a user account
pub async fn disable(client: &Client, email: &str, json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct EnableRequest {
        enabled: bool,
    }

    #[derive(Deserialize, Serialize)]
    struct DisableResponse {
        success: bool,
    }

    let request = EnableRequest { enabled: false };

    let response: DisableResponse = client
        .put(&format!("/api/users/{}/status", email), &request)
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", format!("✓ User {} disabled", email).yellow().bold());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_info_serialization() {
        let user = UserInfo {
            email: "test@example.com".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            quota_used: 1024 * 1024,
            quota_limit: 100 * 1024 * 1024,
            enabled: true,
        };

        let json = serde_json::to_string(&user).unwrap();
        assert!(json.contains("test@example.com"));
    }

    #[test]
    fn test_quota_calculation() {
        let quota_mb = 100u64;
        let quota_bytes = quota_mb * 1024 * 1024;
        assert_eq!(quota_bytes, 104_857_600);
    }

    #[test]
    fn test_user_info_disabled() {
        let user = UserInfo {
            email: "disabled@example.com".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            quota_used: 0,
            quota_limit: 100 * 1024 * 1024,
            enabled: false,
        };

        assert!(!user.enabled);
        assert_eq!(user.quota_used, 0);
    }

    #[test]
    fn test_quota_percentage_calculation() {
        let user = UserInfo {
            email: "test@example.com".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            quota_used: 50 * 1024 * 1024,
            quota_limit: 100 * 1024 * 1024,
            enabled: true,
        };

        let percentage = (user.quota_used as f64 / user.quota_limit as f64) * 100.0;
        assert_eq!(percentage, 50.0);
    }

    #[test]
    fn test_quota_over_limit() {
        let user = UserInfo {
            email: "test@example.com".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            quota_used: 150 * 1024 * 1024,
            quota_limit: 100 * 1024 * 1024,
            enabled: true,
        };

        let percentage = (user.quota_used as f64 / user.quota_limit as f64) * 100.0;
        assert!(percentage > 100.0);
    }

    #[test]
    fn test_user_info_deserialization() {
        let json = r#"{
            "email": "test@example.com",
            "created_at": "2024-01-01T00:00:00Z",
            "quota_used": 1048576,
            "quota_limit": 104857600,
            "enabled": true
        }"#;

        let user: UserInfo = serde_json::from_str(json).unwrap();
        assert_eq!(user.email, "test@example.com");
        assert_eq!(user.quota_used, 1048576);
        assert!(user.enabled);
    }

    #[test]
    fn test_quota_zero() {
        let quota_mb = 0u64;
        let quota_bytes = quota_mb * 1024 * 1024;
        assert_eq!(quota_bytes, 0);
    }

    #[test]
    fn test_quota_large_value() {
        let quota_mb = 10_000u64; // 10GB
        let quota_bytes = quota_mb * 1024 * 1024;
        assert_eq!(quota_bytes, 10_485_760_000);
    }
}
