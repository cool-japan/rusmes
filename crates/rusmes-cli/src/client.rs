//! HTTP client for communicating with RusMES server

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};

pub struct Client {
    base_url: String,
    client: reqwest::Client,
}

impl Client {
    /// Create a new client
    pub fn new(base_url: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            base_url: base_url.to_string(),
            client,
        })
    }

    /// GET request
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Request failed ({}): {}", status, text);
        }

        response
            .json()
            .await
            .context("Failed to parse JSON response")
    }

    /// POST request
    pub async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .context("Failed to send POST request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Request failed ({}): {}", status, text);
        }

        response
            .json()
            .await
            .context("Failed to parse JSON response")
    }

    /// PUT request
    pub async fn put<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .put(&url)
            .json(body)
            .send()
            .await
            .context("Failed to send PUT request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Request failed ({}): {}", status, text);
        }

        response
            .json()
            .await
            .context("Failed to parse JSON response")
    }

    /// DELETE request
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .context("Failed to send DELETE request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Request failed ({}): {}", status, text);
        }

        response
            .json()
            .await
            .context("Failed to parse JSON response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = Client::new("http://localhost:8080").unwrap();
        assert_eq!(client.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_url_formatting() {
        let client = Client::new("http://localhost:8080").unwrap();
        let url = format!("{}{}", client.base_url, "/api/users");
        assert_eq!(url, "http://localhost:8080/api/users");
    }

    #[test]
    fn test_client_with_https() {
        let client = Client::new("https://mail.example.com:8080").unwrap();
        assert_eq!(client.base_url, "https://mail.example.com:8080");
    }

    #[test]
    fn test_url_formatting_with_query() {
        let client = Client::new("http://localhost:8080").unwrap();
        let url = format!("{}{}", client.base_url, "/api/queue?status=pending");
        assert_eq!(url, "http://localhost:8080/api/queue?status=pending");
    }

    #[test]
    fn test_client_with_trailing_slash() {
        let client = Client::new("http://localhost:8080/").unwrap();
        assert_eq!(client.base_url, "http://localhost:8080/");
    }

    #[test]
    fn test_multiple_path_segments() {
        let client = Client::new("http://localhost:8080").unwrap();
        let url = format!(
            "{}{}",
            client.base_url, "/api/users/test@example.com/mailboxes"
        );
        assert_eq!(
            url,
            "http://localhost:8080/api/users/test@example.com/mailboxes"
        );
    }
}
