//! Storage operations benchmarks
//!
//! Benchmarks for message append, retrieval, and other storage operations

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

/// Simple in-memory storage backend for benchmarking
#[derive(Clone)]
struct Message {
    #[allow(dead_code)]
    id: usize,
    mailbox_id: usize,
    flags: Vec<String>,
    body: Vec<u8>,
}

struct MemoryStorage {
    messages: HashMap<usize, Message>,
    mailboxes: HashMap<usize, Vec<usize>>, // mailbox_id -> message_ids
    next_id: usize,
}

impl MemoryStorage {
    fn new() -> Self {
        Self {
            messages: HashMap::new(),
            mailboxes: HashMap::new(),
            next_id: 1,
        }
    }

    fn append_message(&mut self, mailbox_id: usize, body: Vec<u8>) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let msg = Message {
            id,
            mailbox_id,
            flags: Vec::new(),
            body,
        };

        self.messages.insert(id, msg);
        self.mailboxes.entry(mailbox_id).or_default().push(id);

        id
    }

    fn get_message(&self, id: usize) -> Option<&Message> {
        self.messages.get(&id)
    }

    fn update_flags(&mut self, id: usize, flags: Vec<String>) {
        if let Some(msg) = self.messages.get_mut(&id) {
            msg.flags = flags;
        }
    }

    fn copy_message(&mut self, id: usize, dest_mailbox: usize) -> Option<usize> {
        if let Some(msg) = self.messages.get(&id).cloned() {
            let new_id = self.next_id;
            self.next_id += 1;

            let new_msg = Message {
                id: new_id,
                mailbox_id: dest_mailbox,
                flags: msg.flags.clone(),
                body: msg.body.clone(),
            };

            self.messages.insert(new_id, new_msg);
            self.mailboxes.entry(dest_mailbox).or_default().push(new_id);

            Some(new_id)
        } else {
            None
        }
    }

    fn delete_message(&mut self, id: usize) {
        if let Some(msg) = self.messages.remove(&id) {
            if let Some(mailbox) = self.mailboxes.get_mut(&msg.mailbox_id) {
                mailbox.retain(|&mid| mid != id);
            }
        }
    }

    fn list_mailbox(&self, mailbox_id: usize) -> Vec<usize> {
        self.mailboxes.get(&mailbox_id).cloned().unwrap_or_default()
    }
}

fn generate_message_body(size_kb: usize) -> Vec<u8> {
    vec![b'A'; size_kb * 1024]
}

fn benchmark_message_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_append");

    // Test various message sizes: 1KB, 10KB, 100KB, 1MB, 10MB
    for size in [1, 10, 100, 1000, 10000].iter() {
        group.throughput(Throughput::Bytes((*size * 1024) as u64));

        let body = generate_message_body(*size);

        group.bench_with_input(
            BenchmarkId::new("append", format!("{}KB", size)),
            size,
            |b, _| {
                let mut storage = MemoryStorage::new();
                b.iter(|| {
                    black_box(storage.append_message(1, body.clone()));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_message_retrieval(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_retrieval");

    for size in [1, 10, 100, 1000, 10000].iter() {
        let mut storage = MemoryStorage::new();
        let body = generate_message_body(*size);
        let id = storage.append_message(1, body);

        group.throughput(Throughput::Bytes((*size * 1024) as u64));

        group.bench_with_input(
            BenchmarkId::new("retrieve", format!("{}KB", size)),
            size,
            |b, _| {
                b.iter(|| {
                    black_box(storage.get_message(id));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_flag_updates(c: &mut Criterion) {
    let mut group = c.benchmark_group("flag_updates");

    let mut storage = MemoryStorage::new();
    let body = generate_message_body(10);
    let id = storage.append_message(1, body);

    let flags = vec![
        "\\Seen".to_string(),
        "\\Answered".to_string(),
        "\\Flagged".to_string(),
    ];

    group.bench_function("update_flags", |b| {
        b.iter(|| {
            storage.update_flags(id, flags.clone());
        })
    });

    group.finish();
}

fn benchmark_message_copy(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_copy");

    for size in [1, 10, 100, 1000].iter() {
        let mut storage = MemoryStorage::new();
        let body = generate_message_body(*size);
        let id = storage.append_message(1, body);

        group.throughput(Throughput::Bytes((*size * 1024) as u64));

        group.bench_with_input(
            BenchmarkId::new("copy", format!("{}KB", size)),
            size,
            |b, _| {
                b.iter(|| {
                    black_box(storage.copy_message(id, 2));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_message_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_delete");

    for count in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(BenchmarkId::new("delete", count), count, |b, &count| {
            b.iter(|| {
                let mut storage = MemoryStorage::new();
                let body = generate_message_body(10);

                // Add messages
                let ids: Vec<usize> = (0..count)
                    .map(|_| storage.append_message(1, body.clone()))
                    .collect();

                // Delete them
                for id in ids {
                    storage.delete_message(id);
                }
            })
        });
    }

    group.finish();
}

fn benchmark_mailbox_listing(c: &mut Criterion) {
    let mut group = c.benchmark_group("mailbox_listing");

    for count in [10, 100, 1000, 10000].iter() {
        let mut storage = MemoryStorage::new();
        let body = generate_message_body(10);

        for _ in 0..*count {
            storage.append_message(1, body.clone());
        }

        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(BenchmarkId::new("list", count), count, |b, _| {
            b.iter(|| {
                black_box(storage.list_mailbox(1));
            })
        });
    }

    group.finish();
}

fn benchmark_batch_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_operations");

    for count in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        let body = generate_message_body(10);

        group.bench_with_input(
            BenchmarkId::new("batch_append", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let mut storage = MemoryStorage::new();
                    for _ in 0..count {
                        black_box(storage.append_message(1, body.clone()));
                    }
                })
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = storage_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_message_append,
        benchmark_message_retrieval,
        benchmark_flag_updates,
        benchmark_message_copy,
        benchmark_message_delete,
        benchmark_mailbox_listing,
        benchmark_batch_operations
}

criterion_main!(storage_benches);
