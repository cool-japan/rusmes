//! Message generators for load testing

use crate::config::MessageContent;
use rand::RngExt;

/// Message generator
#[derive(Clone)]
pub struct MessageGenerator {
    min_size: usize,
    max_size: usize,
    #[allow(dead_code)]
    content_type: MessageContent,
}

impl MessageGenerator {
    /// Create a new message generator
    pub fn new(min_size: usize, max_size: usize) -> Self {
        Self {
            min_size,
            max_size,
            content_type: MessageContent::Random,
        }
    }

    /// Create a new message generator with content type
    pub fn with_content_type(
        min_size: usize,
        max_size: usize,
        content_type: MessageContent,
    ) -> Self {
        Self {
            min_size,
            max_size,
            content_type,
        }
    }

    /// Generate a random message
    pub fn generate(&self) -> String {
        let mut rng = rand::rng();
        let target_size = rng.random_range(self.min_size..=self.max_size);

        let from = format!("loadtest{}@example.com", rng.random::<u32>());
        let to = format!("user{}@example.com", rng.random::<u32>());
        let subject = format!("Load Test Message {}", rng.random::<u32>());

        // Build headers first to know their exact size
        let headers = format!(
            "From: {}\r\n\
             To: {}\r\n\
             Subject: {}\r\n\
             \r\n",
            from, to, subject
        );

        // Calculate body size to reach target total size
        let body_size = target_size.saturating_sub(headers.len()).max(1);
        let body = self.generate_body(body_size);

        format!("{}{}", headers, body)
    }

    /// Generate random body content
    fn generate_body(&self, size: usize) -> String {
        let mut rng = rand::rng();
        let words = vec![
            "Lorem",
            "ipsum",
            "dolor",
            "sit",
            "amet",
            "consectetur",
            "adipiscing",
            "elit",
            "sed",
            "do",
            "eiusmod",
            "tempor",
            "incididunt",
            "ut",
            "labore",
            "et",
            "dolore",
            "magna",
            "aliqua",
            "enim",
            "ad",
            "minim",
            "veniam",
            "quis",
        ];

        let mut body = String::with_capacity(size);
        while body.len() < size {
            let word = words[rng.random_range(0..words.len())];
            body.push_str(word);
            body.push(' ');
        }

        body.truncate(size);
        body
    }

    /// Generate a message with attachment
    pub fn generate_with_attachment(&self, attachment_size: usize) -> String {
        let mut rng = rand::rng();

        let from = format!("loadtest{}@example.com", rng.random::<u32>());
        let to = format!("user{}@example.com", rng.random::<u32>());
        let subject = format!("Load Test Message with Attachment {}", rng.random::<u32>());

        let boundary = "----=_NextPart_000_0001_01D0A1B2.C3D4E5F6";
        let attachment_data = "A".repeat(attachment_size);

        format!(
            "From: {}\r\n\
             To: {}\r\n\
             Subject: {}\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"{}\"\r\n\
             \r\n\
             --{}\r\n\
             Content-Type: text/plain; charset=\"utf-8\"\r\n\
             \r\n\
             This is a load test message with attachment.\r\n\
             \r\n\
             --{}\r\n\
             Content-Type: application/octet-stream\r\n\
             Content-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"test.dat\"\r\n\
             \r\n\
             {}\r\n\
             --{}--\r\n",
            from, to, subject, boundary, boundary, boundary, attachment_data, boundary
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_generation() {
        let generator = MessageGenerator::new(1024, 2048);
        let message = generator.generate();

        assert!(message.len() >= 1024);
        assert!(message.len() <= 2048);
        assert!(message.contains("From:"));
        assert!(message.contains("To:"));
        assert!(message.contains("Subject:"));
    }

    #[test]
    fn test_message_with_attachment() {
        let generator = MessageGenerator::new(1024, 2048);
        let message = generator.generate_with_attachment(100);

        assert!(message.contains("MIME-Version: 1.0"));
        assert!(message.contains("Content-Type: multipart/mixed"));
        assert!(message.contains("attachment"));
    }

    #[test]
    fn test_body_generation() {
        let generator = MessageGenerator::new(100, 200);
        let body = generator.generate_body(150);

        assert_eq!(body.len(), 150);
    }
}
