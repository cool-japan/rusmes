//! Throughput benchmark - Messages per second
//!
//! Target: >50,000 msg/sec

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::time::Duration;

// Mock email message for benchmarking
fn generate_email(size_kb: usize) -> String {
    let body = "A".repeat(size_kb * 1024);
    format!(
        "From: sender@example.com\r\n\
         To: recipient@example.com\r\n\
         Subject: Test Message\r\n\
         Content-Type: text/plain\r\n\
         \r\n\
         {}",
        body
    )
}

// Simulate SMTP message ingestion
fn ingest_message(email: &str) -> bool {
    // Simulate basic parsing and validation
    email.contains("From:") && email.contains("To:")
}

// Simulate IMAP message fetch
fn fetch_message(email: &str) -> usize {
    email.len()
}

// Simulate queue processing
fn process_queue_message(email: &str) -> bool {
    !email.is_empty()
}

fn benchmark_smtp_ingest_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("smtp_ingest_rate");
    group.throughput(Throughput::Elements(1));

    for size in [1, 10, 100, 1000].iter() {
        let email = generate_email(*size);

        group.bench_with_input(
            BenchmarkId::new("smtp_ingest", format!("{}KB", size)),
            size,
            |b, _| {
                b.iter(|| {
                    black_box(ingest_message(black_box(&email)));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_imap_fetch_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("imap_fetch_rate");
    group.throughput(Throughput::Elements(1));

    for size in [1, 10, 100, 1000].iter() {
        let email = generate_email(*size);

        group.bench_with_input(
            BenchmarkId::new("imap_fetch", format!("{}KB", size)),
            size,
            |b, _| {
                b.iter(|| {
                    black_box(fetch_message(black_box(&email)));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_queue_processing_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("queue_processing");
    group.throughput(Throughput::Elements(100));

    let emails: Vec<String> = (0..100).map(|_| generate_email(10)).collect();

    group.bench_function("process_100_messages", |b| {
        b.iter(|| {
            for email in &emails {
                black_box(process_queue_message(black_box(email)));
            }
        })
    });

    group.finish();
}

fn benchmark_batch_ingest(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_ingest");

    for batch_size in [10, 100, 1000, 10000].iter() {
        group.throughput(Throughput::Elements(*batch_size as u64));

        let emails: Vec<String> = (0..*batch_size).map(|_| generate_email(10)).collect();

        group.bench_with_input(BenchmarkId::new("batch", batch_size), batch_size, |b, _| {
            b.iter(|| {
                for email in &emails {
                    black_box(ingest_message(black_box(email)));
                }
            })
        });
    }

    group.finish();
}

fn benchmark_message_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_sizes");

    // Test various message sizes: 1KB, 10KB, 100KB, 1MB, 10MB
    for size in [1, 10, 100, 1000, 10000].iter() {
        group.throughput(Throughput::Bytes((*size * 1024) as u64));

        let email = generate_email(*size);

        group.bench_with_input(
            BenchmarkId::new("size", format!("{}KB", size)),
            size,
            |b, _| {
                b.iter(|| {
                    black_box(ingest_message(black_box(&email)));
                })
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = throughput_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_smtp_ingest_rate,
        benchmark_imap_fetch_rate,
        benchmark_queue_processing_rate,
        benchmark_batch_ingest,
        benchmark_message_sizes
}

criterion_main!(throughput_benches);
