//! MODSEQ (Modification Sequence) tracking for IMAP CONDSTORE
//!
//! This module implements modification sequence numbers (MODSEQ) for tracking
//! message and mailbox changes, as required by RFC 7162 (CONDSTORE/QRESYNC).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Modification sequence number
///
/// MODSEQ is a 64-bit unsigned integer that increases monotonically
/// for each change to a mailbox or message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModSeq(pub u64);

impl ModSeq {
    /// Create a new MODSEQ with value 0 (invalid/initial state)
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Create a new MODSEQ with value 1 (first valid MODSEQ)
    pub const fn one() -> Self {
        Self(1)
    }

    /// Create a new MODSEQ from a u64 value
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Get the u64 value
    pub const fn value(&self) -> u64 {
        self.0
    }

    /// Check if this is a valid MODSEQ (non-zero)
    pub const fn is_valid(&self) -> bool {
        self.0 > 0
    }

    /// Increment and return the new value
    pub fn increment(&mut self) -> Self {
        self.0 += 1;
        *self
    }
}

impl std::fmt::Display for ModSeq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for ModSeq {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<ModSeq> for u64 {
    fn from(modseq: ModSeq) -> u64 {
        modseq.0
    }
}

/// MODSEQ generator for creating unique modification sequence numbers
///
/// This is a thread-safe atomic counter that ensures MODSEQs are
/// monotonically increasing across the entire server.
#[derive(Debug, Clone)]
pub struct ModSeqGenerator {
    counter: Arc<AtomicU64>,
}

impl ModSeqGenerator {
    /// Create a new MODSEQ generator starting from 1
    pub fn new() -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Create a new MODSEQ generator starting from a specific value
    pub fn with_start_value(start: u64) -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(start.max(1))),
        }
    }

    /// Generate the next MODSEQ value
    pub fn next(&self) -> ModSeq {
        let value = self.counter.fetch_add(1, Ordering::SeqCst);
        ModSeq(value)
    }

    /// Get the current MODSEQ value without incrementing
    pub fn current(&self) -> ModSeq {
        let value = self.counter.load(Ordering::SeqCst);
        ModSeq(value.saturating_sub(1).max(1))
    }

    /// Get the highest MODSEQ value (next - 1)
    pub fn highest(&self) -> ModSeq {
        self.current()
    }
}

impl Default for ModSeqGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Message metadata with MODSEQ tracking
#[derive(Debug, Clone)]
pub struct MessageModSeq {
    /// Message UID
    pub uid: u32,
    /// Current MODSEQ value for this message
    pub modseq: ModSeq,
}

impl MessageModSeq {
    /// Create new message metadata
    pub fn new(uid: u32, modseq: ModSeq) -> Self {
        Self { uid, modseq }
    }
}

/// Mailbox metadata with MODSEQ tracking
#[derive(Debug, Clone)]
pub struct MailboxModSeq {
    /// Mailbox name
    pub name: String,
    /// Highest MODSEQ in this mailbox
    pub highestmodseq: ModSeq,
    /// UIDVALIDITY value
    pub uidvalidity: u32,
    /// Next UID to be assigned
    pub uidnext: u32,
}

impl MailboxModSeq {
    /// Create new mailbox metadata
    pub fn new(name: String, uidvalidity: u32, uidnext: u32) -> Self {
        Self {
            name,
            highestmodseq: ModSeq::one(),
            uidvalidity,
            uidnext,
        }
    }

    /// Update the highest MODSEQ if the given value is higher
    pub fn update_modseq(&mut self, modseq: ModSeq) {
        if modseq > self.highestmodseq {
            self.highestmodseq = modseq;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_modseq_creation() {
        let modseq = ModSeq::zero();
        assert_eq!(modseq.value(), 0);
        assert!(!modseq.is_valid());

        let modseq = ModSeq::one();
        assert_eq!(modseq.value(), 1);
        assert!(modseq.is_valid());

        let modseq = ModSeq::new(42);
        assert_eq!(modseq.value(), 42);
        assert!(modseq.is_valid());
    }

    #[test]
    fn test_modseq_increment() {
        let mut modseq = ModSeq::one();
        assert_eq!(modseq.value(), 1);

        let new_modseq = modseq.increment();
        assert_eq!(new_modseq.value(), 2);
        assert_eq!(modseq.value(), 2);
    }

    #[test]
    fn test_modseq_increment_multiple() {
        let mut modseq = ModSeq::zero();
        for i in 1..=10 {
            let result = modseq.increment();
            assert_eq!(result.value(), i);
            assert_eq!(modseq.value(), i);
        }
    }

    #[test]
    fn test_modseq_ordering() {
        let modseq1 = ModSeq::new(10);
        let modseq2 = ModSeq::new(20);

        assert!(modseq1 < modseq2);
        assert!(modseq2 > modseq1);
        assert_eq!(modseq1, modseq1.clone());
    }

    #[test]
    fn test_modseq_ordering_edge_cases() {
        let zero = ModSeq::zero();
        let one = ModSeq::one();
        let max = ModSeq::new(u64::MAX);

        assert!(zero < one);
        assert!(one < max);
        assert!(zero < max);
    }

    #[test]
    fn test_modseq_equality() {
        let m1 = ModSeq::new(100);
        let m2 = ModSeq::new(100);
        let m3 = ModSeq::new(200);

        assert_eq!(m1, m2);
        assert_ne!(m1, m3);
    }

    #[test]
    fn test_modseq_hash() {
        let mut set = HashSet::new();
        set.insert(ModSeq::new(1));
        set.insert(ModSeq::new(2));
        set.insert(ModSeq::new(1)); // Duplicate

        assert_eq!(set.len(), 2);
        assert!(set.contains(&ModSeq::new(1)));
        assert!(set.contains(&ModSeq::new(2)));
    }

    #[test]
    fn test_modseq_generator() {
        let gen = ModSeqGenerator::new();

        let modseq1 = gen.next();
        assert_eq!(modseq1.value(), 1);

        let modseq2 = gen.next();
        assert_eq!(modseq2.value(), 2);

        let modseq3 = gen.next();
        assert_eq!(modseq3.value(), 3);
    }

    #[test]
    fn test_modseq_generator_with_start() {
        let gen = ModSeqGenerator::with_start_value(100);

        let modseq1 = gen.next();
        assert_eq!(modseq1.value(), 100);

        let modseq2 = gen.next();
        assert_eq!(modseq2.value(), 101);
    }

    #[test]
    fn test_modseq_generator_with_zero_start() {
        // Zero should be converted to 1
        let gen = ModSeqGenerator::with_start_value(0);
        let modseq = gen.next();
        assert_eq!(modseq.value(), 1);
    }

    #[test]
    fn test_modseq_generator_current() {
        let gen = ModSeqGenerator::new();

        let _m1 = gen.next();
        let _m2 = gen.next();
        let _m3 = gen.next();

        let current = gen.current();
        assert_eq!(current.value(), 3);

        let highest = gen.highest();
        assert_eq!(highest, current);
    }

    #[test]
    fn test_modseq_generator_current_before_first_next() {
        let gen = ModSeqGenerator::new();
        let current = gen.current();
        assert_eq!(current.value(), 1);
    }

    #[test]
    fn test_modseq_generator_clone() {
        let gen1 = ModSeqGenerator::new();
        let _m1 = gen1.next();
        let _m2 = gen1.next();

        // Cloning shares the same Arc<AtomicU64>, so both generators use the same counter
        let gen2 = gen1.clone();
        let m3 = gen2.next();
        let m4 = gen1.next();

        assert_eq!(m3.value(), 3);
        assert_eq!(m4.value(), 4); // gen1 continues from where gen2 left off
    }

    #[test]
    fn test_modseq_generator_thread_safety() {
        use std::thread;

        let gen = ModSeqGenerator::new();
        let gen1 = gen.clone();
        let gen2 = gen.clone();

        let handle1 = thread::spawn(move || {
            let mut values = Vec::new();
            for _ in 0..10 {
                values.push(gen1.next().value());
            }
            values
        });

        let handle2 = thread::spawn(move || {
            let mut values = Vec::new();
            for _ in 0..10 {
                values.push(gen2.next().value());
            }
            values
        });

        let values1 = handle1.join().unwrap();
        let values2 = handle2.join().unwrap();

        // All values should be unique
        let mut all_values: Vec<_> = values1.into_iter().chain(values2).collect();
        all_values.sort_unstable();
        all_values.dedup();
        assert_eq!(all_values.len(), 20);
    }

    #[test]
    fn test_message_modseq() {
        let msg = MessageModSeq::new(42, ModSeq::new(100));
        assert_eq!(msg.uid, 42);
        assert_eq!(msg.modseq.value(), 100);
    }

    #[test]
    fn test_message_modseq_clone() {
        let msg1 = MessageModSeq::new(42, ModSeq::new(100));
        let msg2 = msg1.clone();
        assert_eq!(msg1.uid, msg2.uid);
        assert_eq!(msg1.modseq, msg2.modseq);
    }

    #[test]
    fn test_mailbox_modseq_new() {
        let mbox = MailboxModSeq::new("INBOX".to_string(), 123, 456);
        assert_eq!(mbox.name, "INBOX");
        assert_eq!(mbox.highestmodseq, ModSeq::one());
        assert_eq!(mbox.uidvalidity, 123);
        assert_eq!(mbox.uidnext, 456);
    }

    #[test]
    fn test_mailbox_modseq_update() {
        let mut mbox = MailboxModSeq::new("INBOX".to_string(), 1, 1);
        assert_eq!(mbox.highestmodseq.value(), 1);

        mbox.update_modseq(ModSeq::new(10));
        assert_eq!(mbox.highestmodseq.value(), 10);

        // Should not decrease
        mbox.update_modseq(ModSeq::new(5));
        assert_eq!(mbox.highestmodseq.value(), 10);

        // Should increase
        mbox.update_modseq(ModSeq::new(15));
        assert_eq!(mbox.highestmodseq.value(), 15);
    }

    #[test]
    fn test_mailbox_modseq_update_same_value() {
        let mut mbox = MailboxModSeq::new("INBOX".to_string(), 1, 1);
        mbox.update_modseq(ModSeq::new(10));
        assert_eq!(mbox.highestmodseq.value(), 10);

        // Updating with same value should not change anything
        mbox.update_modseq(ModSeq::new(10));
        assert_eq!(mbox.highestmodseq.value(), 10);
    }

    #[test]
    fn test_mailbox_modseq_clone() {
        let mbox1 = MailboxModSeq::new("INBOX".to_string(), 123, 456);
        let mbox2 = mbox1.clone();
        assert_eq!(mbox1.name, mbox2.name);
        assert_eq!(mbox1.highestmodseq, mbox2.highestmodseq);
        assert_eq!(mbox1.uidvalidity, mbox2.uidvalidity);
        assert_eq!(mbox1.uidnext, mbox2.uidnext);
    }

    #[test]
    fn test_modseq_from_u64() {
        let modseq: ModSeq = 42u64.into();
        assert_eq!(modseq.value(), 42);

        let value: u64 = modseq.into();
        assert_eq!(value, 42);
    }

    #[test]
    fn test_modseq_from_u64_max() {
        let modseq: ModSeq = u64::MAX.into();
        assert_eq!(modseq.value(), u64::MAX);
    }

    #[test]
    fn test_modseq_display() {
        let modseq = ModSeq::new(12345);
        assert_eq!(modseq.to_string(), "12345");
    }

    #[test]
    fn test_modseq_display_zero() {
        let modseq = ModSeq::zero();
        assert_eq!(modseq.to_string(), "0");
    }

    #[test]
    fn test_modseq_display_large() {
        let modseq = ModSeq::new(u64::MAX);
        assert_eq!(modseq.to_string(), u64::MAX.to_string());
    }

    #[test]
    fn test_modseq_generator_default() {
        let gen = ModSeqGenerator::default();
        let modseq = gen.next();
        assert_eq!(modseq.value(), 1);
    }

    #[test]
    fn test_modseq_const_functions() {
        const ZERO: ModSeq = ModSeq::zero();
        const ONE: ModSeq = ModSeq::one();
        const CUSTOM: ModSeq = ModSeq::new(42);

        assert_eq!(ZERO.value(), 0);
        assert_eq!(ONE.value(), 1);
        assert_eq!(CUSTOM.value(), 42);
    }
}
