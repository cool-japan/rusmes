//! Priority queue system for mail delivery
//!
//! This module provides a multi-level priority queue system for mail delivery,
//! allowing mails to be processed based on their priority level.
//!
//! # Priority Levels
//!
//! - **High**: Urgent mail (priority 3)
//! - **Normal**: Default for most mail (priority 2)
//! - **Low**: Non-urgent mail (priority 1)
//! - **Bulk**: Newsletters, marketing (priority 0)
//!
//! # Features
//!
//! - Priority-based scheduling (highest priority first)
//! - Per-priority statistics tracking
//! - Priority inheritance for retries
//! - Configurable priority assignment rules:
//!   - Per sender email
//!   - Per recipient email
//!   - Per domain
//! - Automatic priority boosting after N failed attempts
//!
//! # Example: Basic Priority Queue
//!
//! ```
//! use rusmes_core::queue::priority::{Priority, PriorityQueue};
//! use rusmes_proto::MailId;
//!
//! let mut queue = PriorityQueue::<String>::with_default_config();
//!
//! // Enqueue items with different priorities
//! queue.enqueue(MailId::new(), "bulk mail".to_string(), Priority::Bulk);
//! queue.enqueue(MailId::new(), "urgent mail".to_string(), Priority::High);
//! queue.enqueue(MailId::new(), "normal mail".to_string(), Priority::Normal);
//!
//! // Dequeue returns highest priority first (High, Normal, Bulk)
//! let (mail_id, item, priority) = queue.dequeue().unwrap();
//! assert_eq!(item, "urgent mail");
//! assert_eq!(priority, Priority::High);
//! ```
//!
//! # Example: Priority Configuration
//!
//! ```
//! use rusmes_core::queue::priority::{Priority, PriorityConfig};
//! use rusmes_proto::{Mail, MimeMessage, MessageBody, HeaderMap};
//! use bytes::Bytes;
//!
//! let mut config = PriorityConfig::new();
//!
//! // VIP sender always gets high priority
//! config.add_sender_priority("vip@example.com", Priority::High);
//!
//! // Important domain gets high priority
//! config.add_domain_priority("important.com", Priority::High);
//!
//! // Bulk domain gets low priority
//! config.add_domain_priority("marketing.com", Priority::Bulk);
//!
//! // Enable priority boost after 3 failed attempts
//! config.boost_after_attempts = Some(3);
//! config.boost_amount = 1;
//!
//! // Calculate priority for a mail
//! let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("test")));
//! let mail = Mail::new(
//!     Some("vip@example.com".parse().unwrap()),
//!     vec!["user@example.com".parse().unwrap()],
//!     message,
//!     None,
//!     None,
//! );
//!
//! let priority = config.calculate_priority(&mail, 0);
//! assert_eq!(priority, Priority::High);
//! ```
//!
//! # Example: Integration with MailQueue
//!
//! ```no_run
//! use rusmes_core::{MailQueue, PriorityConfig, Priority};
//! use rusmes_proto::{Mail, MimeMessage, MessageBody, HeaderMap};
//! use bytes::Bytes;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create priority configuration
//!     let mut priority_config = PriorityConfig::new();
//!     priority_config.add_domain_priority("urgent.com", Priority::High);
//!
//!     // Create queue with priority support
//!     let queue = MailQueue::new_with_priority_config(priority_config);
//!
//!     // Enqueue mail (priority calculated automatically)
//!     let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("test")));
//!     let mail = Mail::new(
//!         Some("sender@example.com".parse().unwrap()),
//!         vec!["user@urgent.com".parse().unwrap()],
//!         message,
//!         None,
//!         None,
//!     );
//!     queue.enqueue(mail).await.unwrap();
//!
//!     // Get ready mails (sorted by priority)
//!     let ready_mails = queue.get_ready_for_retry(10);
//!
//!     // Get statistics by priority
//!     let stats = queue.stats_by_priority();
//!     for (priority, stat) in stats {
//!         println!("{}: {} total, {} ready", priority, stat.total, stat.ready);
//!     }
//! }
//! ```

use rusmes_proto::{Mail, MailId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

/// Mail priority levels
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord, Default,
)]
pub enum Priority {
    /// Highest priority - urgent mail
    High = 3,
    /// Normal priority - default for most mail
    #[default]
    Normal = 2,
    /// Low priority - non-urgent mail
    Low = 1,
    /// Bulk mail - newsletters, marketing
    Bulk = 0,
}

impl Priority {
    /// Get all priority levels in order (highest first)
    pub fn all() -> &'static [Priority] {
        &[
            Priority::High,
            Priority::Normal,
            Priority::Low,
            Priority::Bulk,
        ]
    }

    /// Get priority as numeric value
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Create priority from numeric value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            3 => Some(Priority::High),
            2 => Some(Priority::Normal),
            1 => Some(Priority::Low),
            0 => Some(Priority::Bulk),
            _ => None,
        }
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::High => write!(f, "high"),
            Priority::Normal => write!(f, "normal"),
            Priority::Low => write!(f, "low"),
            Priority::Bulk => write!(f, "bulk"),
        }
    }
}

impl std::str::FromStr for Priority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "high" => Ok(Priority::High),
            "normal" => Ok(Priority::Normal),
            "low" => Ok(Priority::Low),
            "bulk" => Ok(Priority::Bulk),
            _ => Err(format!("Invalid priority: {}", s)),
        }
    }
}

/// Configuration for priority assignment rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityConfig {
    /// Default priority if no rules match
    pub default_priority: Priority,

    /// Sender-based priority rules (email -> priority)
    pub sender_priorities: HashMap<String, Priority>,

    /// Recipient-based priority rules (email -> priority)
    pub recipient_priorities: HashMap<String, Priority>,

    /// Domain-based priority rules (domain -> priority)
    pub domain_priorities: HashMap<String, Priority>,

    /// Enable priority inheritance for retries
    pub inherit_priority_on_retry: bool,

    /// Boost priority after N failed attempts
    pub boost_after_attempts: Option<u32>,

    /// Priority boost amount (e.g., Low -> Normal)
    pub boost_amount: u8,
}

impl Default for PriorityConfig {
    fn default() -> Self {
        Self {
            default_priority: Priority::Normal,
            sender_priorities: HashMap::new(),
            recipient_priorities: HashMap::new(),
            domain_priorities: HashMap::new(),
            inherit_priority_on_retry: true,
            boost_after_attempts: Some(3),
            boost_amount: 1,
        }
    }
}

impl PriorityConfig {
    /// Create a new priority configuration with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Add sender priority rule
    pub fn add_sender_priority(&mut self, sender: impl Into<String>, priority: Priority) {
        self.sender_priorities.insert(sender.into(), priority);
    }

    /// Add recipient priority rule
    pub fn add_recipient_priority(&mut self, recipient: impl Into<String>, priority: Priority) {
        self.recipient_priorities.insert(recipient.into(), priority);
    }

    /// Add domain priority rule
    pub fn add_domain_priority(&mut self, domain: impl Into<String>, priority: Priority) {
        self.domain_priorities.insert(domain.into(), priority);
    }

    /// Calculate priority for a mail based on configured rules
    pub fn calculate_priority(&self, mail: &Mail, current_attempts: u32) -> Priority {
        // Check if priority is already set as an attribute
        if let Some(attr) = mail.get_attribute("priority") {
            if let Some(priority_str) = attr.as_str() {
                if let Ok(priority) = priority_str.parse::<Priority>() {
                    return self.apply_boost(priority, current_attempts);
                }
            }
        }

        // Check sender-based rules
        if let Some(sender) = mail.sender() {
            let sender_email = sender.as_string();
            if let Some(&priority) = self.sender_priorities.get(&sender_email) {
                return self.apply_boost(priority, current_attempts);
            }
        }

        // Check recipient-based rules (use highest priority if multiple recipients)
        let mut max_priority = None;
        for recipient in mail.recipients() {
            let recipient_email = recipient.as_string();
            if let Some(&priority) = self.recipient_priorities.get(&recipient_email) {
                max_priority = Some(max_priority.map_or(priority, |p: Priority| p.max(priority)));
            }

            // Check domain-based rules
            let domain = recipient.domain().as_str();
            if let Some(&priority) = self.domain_priorities.get(domain) {
                max_priority = Some(max_priority.map_or(priority, |p: Priority| p.max(priority)));
            }
        }

        if let Some(priority) = max_priority {
            return self.apply_boost(priority, current_attempts);
        }

        // Use default priority
        self.apply_boost(self.default_priority, current_attempts)
    }

    /// Apply priority boost based on retry attempts
    fn apply_boost(&self, priority: Priority, current_attempts: u32) -> Priority {
        if let Some(boost_after) = self.boost_after_attempts {
            if current_attempts >= boost_after {
                let current_value = priority.as_u8();
                let boosted_value = current_value.saturating_add(self.boost_amount);
                return Priority::from_u8(boosted_value.min(3)).unwrap_or(Priority::High);
            }
        }
        priority
    }
}

/// Statistics for a single priority level
#[derive(Debug, Clone, Default)]
pub struct PriorityStats {
    /// Total mails in this priority queue
    pub total: usize,
    /// Mails ready for delivery
    pub ready: usize,
    /// Mails waiting for retry
    pub delayed: usize,
    /// Total mails enqueued (lifetime)
    pub enqueued_total: u64,
    /// Total mails delivered (lifetime)
    pub delivered_total: u64,
}

/// Multi-level priority queue
pub struct PriorityQueue<T> {
    /// Separate queue for each priority level
    queues: HashMap<Priority, VecDeque<(MailId, T)>>,
    /// Priority configuration
    config: Arc<RwLock<PriorityConfig>>,
    /// Per-priority statistics
    stats: HashMap<Priority, Arc<RwLock<PriorityStats>>>,
}

impl<T> PriorityQueue<T> {
    /// Create a new priority queue
    pub fn new(config: PriorityConfig) -> Self {
        let mut queues = HashMap::new();
        let mut stats = HashMap::new();

        for &priority in Priority::all() {
            queues.insert(priority, VecDeque::new());
            stats.insert(priority, Arc::new(RwLock::new(PriorityStats::default())));
        }

        Self {
            queues,
            config: Arc::new(RwLock::new(config)),
            stats,
        }
    }

    /// Create with default configuration
    pub fn with_default_config() -> Self {
        Self::new(PriorityConfig::default())
    }

    /// Enqueue an item with a specific priority
    pub fn enqueue(&mut self, mail_id: MailId, item: T, priority: Priority) {
        if let Some(queue) = self.queues.get_mut(&priority) {
            queue.push_back((mail_id, item));
        }

        // Update statistics
        if let Some(stats) = self.stats.get(&priority) {
            if let Ok(mut stats) = stats.write() {
                stats.total += 1;
                stats.enqueued_total += 1;
            }
        }
    }

    /// Dequeue the next item based on priority (highest priority first)
    pub fn dequeue(&mut self) -> Option<(MailId, T, Priority)> {
        // Try queues in priority order (high to low)
        for &priority in Priority::all() {
            if let Some(queue) = self.queues.get_mut(&priority) {
                if let Some((mail_id, item)) = queue.pop_front() {
                    // Update statistics
                    if let Some(stats) = self.stats.get(&priority) {
                        if let Ok(mut stats) = stats.write() {
                            stats.total = stats.total.saturating_sub(1);
                        }
                    }
                    return Some((mail_id, item, priority));
                }
            }
        }
        None
    }

    /// Peek at the next item without removing it
    pub fn peek(&self) -> Option<(&MailId, &T, Priority)> {
        for &priority in Priority::all() {
            if let Some(queue) = self.queues.get(&priority) {
                if let Some((mail_id, item)) = queue.front() {
                    return Some((mail_id, item, priority));
                }
            }
        }
        None
    }

    /// Get the number of items in a specific priority queue
    pub fn len_for_priority(&self, priority: Priority) -> usize {
        self.queues.get(&priority).map_or(0, |q| q.len())
    }

    /// Get total number of items across all priorities
    pub fn len(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove a specific item by mail ID
    pub fn remove(&mut self, mail_id: &MailId) -> Option<(T, Priority)> {
        for &priority in Priority::all() {
            if let Some(queue) = self.queues.get_mut(&priority) {
                if let Some(pos) = queue.iter().position(|(id, _)| id == mail_id) {
                    if let Some((_, item)) = queue.remove(pos) {
                        // Update statistics
                        if let Some(stats) = self.stats.get(&priority) {
                            if let Ok(mut stats) = stats.write() {
                                stats.total = stats.total.saturating_sub(1);
                            }
                        }
                        return Some((item, priority));
                    }
                }
            }
        }
        None
    }

    /// Update priority configuration
    pub fn update_config(&self, config: PriorityConfig) {
        if let Ok(mut guard) = self.config.write() {
            *guard = config;
        }
    }

    /// Get current configuration
    pub fn get_config(&self) -> PriorityConfig {
        self.config.read().map(|g| g.clone()).unwrap_or_default()
    }

    /// Get statistics for a specific priority
    pub fn stats_for_priority(&self, priority: Priority) -> PriorityStats {
        self.stats
            .get(&priority)
            .and_then(|s| s.read().ok().map(|g| g.clone()))
            .unwrap_or_default()
    }

    /// Mark item as delivered (for statistics)
    pub fn mark_delivered(&self, priority: Priority) {
        if let Some(stats) = self.stats.get(&priority) {
            if let Ok(mut stats) = stats.write() {
                stats.delivered_total += 1;
            }
        }
    }

    /// Update ready/delayed counts for a priority
    pub fn update_ready_delayed_stats(&self, priority: Priority, ready: usize, delayed: usize) {
        if let Some(stats) = self.stats.get(&priority) {
            if let Ok(mut stats) = stats.write() {
                stats.ready = ready;
                stats.delayed = delayed;
            }
        }
    }

    /// Clear all queues
    pub fn clear(&mut self) {
        for queue in self.queues.values_mut() {
            queue.clear();
        }
        for stats in self.stats.values() {
            if let Ok(mut stats) = stats.write() {
                stats.total = 0;
                stats.ready = 0;
                stats.delayed = 0;
            }
        }
    }

    /// Get all items for a specific priority
    pub fn items_for_priority(&self, priority: Priority) -> Vec<&(MailId, T)> {
        self.queues
            .get(&priority)
            .map(|q| q.iter().collect())
            .unwrap_or_default()
    }
}

impl<T> Default for PriorityQueue<T> {
    fn default() -> Self {
        Self::with_default_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
        assert!(Priority::Low > Priority::Bulk);
    }

    #[test]
    fn test_priority_from_str() {
        assert_eq!("high".parse::<Priority>().unwrap(), Priority::High);
        assert_eq!("normal".parse::<Priority>().unwrap(), Priority::Normal);
        assert_eq!("low".parse::<Priority>().unwrap(), Priority::Low);
        assert_eq!("bulk".parse::<Priority>().unwrap(), Priority::Bulk);
        assert!("invalid".parse::<Priority>().is_err());
    }

    #[test]
    fn test_priority_queue_enqueue_dequeue() {
        let mut queue = PriorityQueue::<String>::with_default_config();

        let mail_id1 = MailId::new();
        let mail_id2 = MailId::new();
        let mail_id3 = MailId::new();

        queue.enqueue(mail_id1, "low priority".to_string(), Priority::Low);
        queue.enqueue(mail_id2, "high priority".to_string(), Priority::High);
        queue.enqueue(mail_id3, "normal priority".to_string(), Priority::Normal);

        // Should dequeue in priority order: High, Normal, Low
        let (_, item1, priority1) = queue.dequeue().unwrap();
        assert_eq!(item1, "high priority");
        assert_eq!(priority1, Priority::High);

        let (_, item2, priority2) = queue.dequeue().unwrap();
        assert_eq!(item2, "normal priority");
        assert_eq!(priority2, Priority::Normal);

        let (_, item3, priority3) = queue.dequeue().unwrap();
        assert_eq!(item3, "low priority");
        assert_eq!(priority3, Priority::Low);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_priority_queue_remove() {
        let mut queue = PriorityQueue::<String>::with_default_config();

        let mail_id = MailId::new();
        queue.enqueue(mail_id, "test".to_string(), Priority::Normal);

        assert_eq!(queue.len(), 1);
        let (item, priority) = queue.remove(&mail_id).unwrap();
        assert_eq!(item, "test");
        assert_eq!(priority, Priority::Normal);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_priority_config_sender_rule() {
        let mut config = PriorityConfig::new();
        config.add_sender_priority("vip@example.com", Priority::High);

        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("test")));

        let mail = Mail::new(
            Some("vip@example.com".parse().unwrap()),
            vec!["user@example.com".parse().unwrap()],
            message,
            None,
            None,
        );

        let priority = config.calculate_priority(&mail, 0);
        assert_eq!(priority, Priority::High);
    }

    #[test]
    fn test_priority_config_domain_rule() {
        let mut config = PriorityConfig::new();
        config.add_domain_priority("important.com", Priority::High);

        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("test")));

        let mail = Mail::new(
            Some("sender@example.com".parse().unwrap()),
            vec!["user@important.com".parse().unwrap()],
            message,
            None,
            None,
        );

        let priority = config.calculate_priority(&mail, 0);
        assert_eq!(priority, Priority::High);
    }

    #[test]
    fn test_priority_boost_on_retry() {
        let mut config = PriorityConfig::new();
        config.boost_after_attempts = Some(3);
        config.boost_amount = 1;

        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("test")));

        let mail = Mail::new(
            Some("sender@example.com".parse().unwrap()),
            vec!["user@example.com".parse().unwrap()],
            message,
            None,
            None,
        );

        // Before boost threshold
        let priority = config.calculate_priority(&mail, 2);
        assert_eq!(priority, Priority::Normal);

        // After boost threshold
        let priority = config.calculate_priority(&mail, 3);
        assert_eq!(priority, Priority::High);
    }

    #[test]
    fn test_priority_queue_stats() {
        let mut queue = PriorityQueue::<String>::with_default_config();

        let mail_id = MailId::new();
        queue.enqueue(mail_id, "test".to_string(), Priority::High);

        let stats = queue.stats_for_priority(Priority::High);
        assert_eq!(stats.total, 1);
        assert_eq!(stats.enqueued_total, 1);

        queue.mark_delivered(Priority::High);
        let stats = queue.stats_for_priority(Priority::High);
        assert_eq!(stats.delivered_total, 1);
    }
}
