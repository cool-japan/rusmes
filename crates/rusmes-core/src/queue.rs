//! Mail queue management with retry logic and priority support

pub mod core;
pub mod priority;

// Re-export core types
pub use core::{
    FilesystemQueueStore, MailQueue, QueueEntry, QueueEntryData, QueueStats, QueueStore,
};

// Re-export priority types
pub use priority::{Priority, PriorityConfig, PriorityQueue, PriorityStats};
