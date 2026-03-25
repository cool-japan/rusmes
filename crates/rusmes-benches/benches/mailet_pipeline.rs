//! Mailet pipeline latency benchmark

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

struct Message {
    #[allow(dead_code)]
    from: String,
    #[allow(dead_code)]
    to: String,
    body: String,
}

trait Mailet {
    fn process(&self, msg: &mut Message);
}

struct AddHeaderMailet;
impl Mailet for AddHeaderMailet {
    fn process(&self, msg: &mut Message) {
        msg.body = format!("X-Processed: true\r\n{}", msg.body);
    }
}

struct SpamCheckMailet;
impl Mailet for SpamCheckMailet {
    fn process(&self, msg: &mut Message) {
        if msg.body.contains("spam") {
            msg.body = format!("X-Spam: detected\r\n{}", msg.body);
        }
    }
}

struct VirusScanMailet;
impl Mailet for VirusScanMailet {
    fn process(&self, _msg: &mut Message) {
        // Simulate virus scan
    }
}

fn benchmark_single_mailet(c: &mut Criterion) {
    let mailet = AddHeaderMailet;
    let mut msg = Message {
        from: "sender@example.com".to_string(),
        to: "recipient@example.com".to_string(),
        body: "Test message".to_string(),
    };

    c.bench_function("single_mailet", |b| {
        b.iter(|| {
            mailet.process(black_box(&mut msg));
        })
    });
}

fn benchmark_pipeline(c: &mut Criterion) {
    let mailets: Vec<Box<dyn Mailet>> = vec![
        Box::new(AddHeaderMailet),
        Box::new(SpamCheckMailet),
        Box::new(VirusScanMailet),
    ];

    c.bench_function("full_pipeline", |b| {
        b.iter(|| {
            let mut msg = Message {
                from: "sender@example.com".to_string(),
                to: "recipient@example.com".to_string(),
                body: "Test message".to_string(),
            };

            for mailet in &mailets {
                mailet.process(&mut msg);
            }

            black_box(msg);
        })
    });
}

criterion_group!(
    pipeline_benches,
    benchmark_single_mailet,
    benchmark_pipeline
);
criterion_main!(pipeline_benches);
