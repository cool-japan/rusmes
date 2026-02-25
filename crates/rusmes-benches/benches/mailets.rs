//! Mailet pipeline latency benchmark
//!
//! Target: <50ms avg latency for complete pipeline

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::time::Duration;

/// Message structure
#[derive(Clone)]
struct Message {
    from: String,
    #[allow(dead_code)]
    to: Vec<String>,
    subject: String,
    body: String,
    headers: Vec<(String, String)>,
}

impl Message {
    fn new() -> Self {
        Self {
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            subject: "Test message".to_string(),
            body: "This is a test message body".to_string(),
            headers: Vec::new(),
        }
    }

    fn add_header(&mut self, name: String, value: String) {
        self.headers.push((name, value));
    }

    fn body_size(&self) -> usize {
        self.body.len()
    }
}

/// Mailet trait
trait Mailet {
    fn process(&self, msg: &mut Message) -> Result<(), String>;
}

/// DKIM verification mailet
struct DkimMailet;
impl Mailet for DkimMailet {
    fn process(&self, msg: &mut Message) -> Result<(), String> {
        // Simulate DKIM verification
        let signature_valid = msg.from.contains("@");
        if signature_valid {
            msg.add_header(
                "DKIM-Signature".to_string(),
                "v=1; a=rsa-sha256; d=example.com".to_string(),
            );
        }
        Ok(())
    }
}

/// SPF verification mailet
struct SpfMailet;
impl Mailet for SpfMailet {
    fn process(&self, msg: &mut Message) -> Result<(), String> {
        // Simulate SPF check
        msg.add_header("Received-SPF".to_string(), "pass".to_string());
        Ok(())
    }
}

/// DMARC verification mailet
struct DmarcMailet;
impl Mailet for DmarcMailet {
    fn process(&self, msg: &mut Message) -> Result<(), String> {
        // Simulate DMARC check
        msg.add_header("DMARC-Status".to_string(), "pass".to_string());
        Ok(())
    }
}

/// ClamAV scanning mailet
struct ClamAvMailet;
impl Mailet for ClamAvMailet {
    fn process(&self, msg: &mut Message) -> Result<(), String> {
        // Simulate virus scan
        let size = msg.body_size();
        if size > 0 {
            msg.add_header("X-Virus-Scanned".to_string(), "ClamAV".to_string());
        }
        Ok(())
    }
}

/// SpamAssassin checking mailet
struct SpamAssassinMailet;
impl Mailet for SpamAssassinMailet {
    fn process(&self, msg: &mut Message) -> Result<(), String> {
        // Simulate spam checking
        let spam_score = if msg.subject.contains("FREE") {
            5.0
        } else {
            0.5
        };
        msg.add_header("X-Spam-Score".to_string(), format!("{:.1}", spam_score));
        Ok(())
    }
}

/// Sieve script execution mailet
struct SieveMailet;
impl Mailet for SieveMailet {
    fn process(&self, msg: &mut Message) -> Result<(), String> {
        // Simulate Sieve script execution
        if msg.subject.contains("urgent") {
            msg.add_header("X-Sieve-Action".to_string(), "fileinto INBOX".to_string());
        }
        Ok(())
    }
}

fn benchmark_single_mailet(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_mailet");

    let msg = Message::new();

    group.bench_function("dkim", |b| {
        let mailet = DkimMailet;
        b.iter(|| {
            let mut m = msg.clone();
            mailet.process(&mut m).unwrap();
            black_box(());
        })
    });

    group.bench_function("spf", |b| {
        let mailet = SpfMailet;
        b.iter(|| {
            let mut m = msg.clone();
            mailet.process(&mut m).unwrap();
            black_box(());
        })
    });

    group.bench_function("dmarc", |b| {
        let mailet = DmarcMailet;
        b.iter(|| {
            let mut m = msg.clone();
            mailet.process(&mut m).unwrap();
            black_box(());
        })
    });

    group.bench_function("clamav", |b| {
        let mailet = ClamAvMailet;
        b.iter(|| {
            let mut m = msg.clone();
            mailet.process(&mut m).unwrap();
            black_box(());
        })
    });

    group.bench_function("spamassassin", |b| {
        let mailet = SpamAssassinMailet;
        b.iter(|| {
            let mut m = msg.clone();
            mailet.process(&mut m).unwrap();
            black_box(());
        })
    });

    group.bench_function("sieve", |b| {
        let mailet = SieveMailet;
        b.iter(|| {
            let mut m = msg.clone();
            mailet.process(&mut m).unwrap();
            black_box(());
        })
    });

    group.finish();
}

fn benchmark_full_pipeline(c: &mut Criterion) {
    let mailets: Vec<Box<dyn Mailet>> = vec![
        Box::new(DkimMailet),
        Box::new(SpfMailet),
        Box::new(DmarcMailet),
        Box::new(ClamAvMailet),
        Box::new(SpamAssassinMailet),
        Box::new(SieveMailet),
    ];

    c.bench_function("full_pipeline", |b| {
        b.iter(|| {
            let mut msg = Message::new();

            for mailet in &mailets {
                mailet.process(&mut msg).unwrap();
            }

            black_box(msg);
        })
    });
}

fn benchmark_pipeline_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_throughput");

    let mailets: Vec<Box<dyn Mailet>> = vec![
        Box::new(DkimMailet),
        Box::new(SpfMailet),
        Box::new(DmarcMailet),
        Box::new(ClamAvMailet),
        Box::new(SpamAssassinMailet),
        Box::new(SieveMailet),
    ];

    for count in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        let messages: Vec<Message> = (0..*count).map(|_| Message::new()).collect();

        group.bench_with_input(BenchmarkId::new("throughput", count), count, |b, _| {
            b.iter(|| {
                for mut msg in messages.clone() {
                    for mailet in &mailets {
                        mailet.process(&mut msg).unwrap();
                    }
                }
            })
        });
    }

    group.finish();
}

fn benchmark_message_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_size_impact");

    let mailets: Vec<Box<dyn Mailet>> = vec![
        Box::new(DkimMailet),
        Box::new(SpfMailet),
        Box::new(DmarcMailet),
        Box::new(ClamAvMailet),
        Box::new(SpamAssassinMailet),
        Box::new(SieveMailet),
    ];

    for size_kb in [1, 10, 100, 1000].iter() {
        let mut msg = Message::new();
        msg.body = "A".repeat(size_kb * 1024);

        group.bench_with_input(
            BenchmarkId::new("size", format!("{}KB", size_kb)),
            size_kb,
            |b, _| {
                b.iter(|| {
                    let mut m = msg.clone();
                    for mailet in &mailets {
                        mailet.process(&mut m).unwrap();
                    }
                    black_box(m);
                })
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = mailet_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_single_mailet,
        benchmark_full_pipeline,
        benchmark_pipeline_throughput,
        benchmark_message_size_impact
}

criterion_main!(mailet_benches);
