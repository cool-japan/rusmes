//! Throughput benchmark - Messages per second

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use std::time::Duration;

fn parse_email(email: &str) -> usize {
    // Simulate email parsing
    email.lines().count()
}

fn process_email(email: &str) -> bool {
    // Simulate email processing
    !email.is_empty()
}

fn benchmark_single_message(c: &mut Criterion) {
    let email =
        "From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\n\r\nBody";

    c.bench_function("parse_single_email", |b| {
        b.iter(|| parse_email(black_box(email)))
    });

    c.bench_function("process_single_email", |b| {
        b.iter(|| process_email(black_box(email)))
    });
}

fn benchmark_message_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_batch");

    for size in [10, 100, 1000].iter() {
        let emails: Vec<String> = (0..*size)
            .map(|i| format!("From: sender{}@example.com\r\nTo: recipient@example.com\r\nSubject: Test {}\r\n\r\nBody", i, i))
            .collect();

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                for email in &emails {
                    black_box(parse_email(email));
                }
            })
        });
    }

    group.finish();
}

fn benchmark_message_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_sizes");

    for size in [1_024, 10_240, 102_400].iter() {
        let body = "A".repeat(*size);
        let email = format!(
            "From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\n\r\n{}",
            body
        );

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| parse_email(black_box(&email)))
        });
    }

    group.finish();
}

criterion_group! {
    name = throughput_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(10));
    targets = benchmark_single_message, benchmark_message_batch, benchmark_message_sizes
}

criterion_main!(throughput_benches);
