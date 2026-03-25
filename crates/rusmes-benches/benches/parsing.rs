//! Protocol parsing benchmarks
//!
//! Benchmarks for SMTP, IMAP, MIME, and JSON parsing

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::time::Duration;

// SMTP command parsing simulation
fn parse_smtp_command(input: &str) -> Option<(&str, Vec<&str>)> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    Some((parts[0], parts[1..].to_vec()))
}

// IMAP command parsing simulation
fn parse_imap_command(input: &str) -> Option<(String, String, Vec<String>)> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let tag = parts[0].to_string();
    let command = parts[1].to_string();
    let args = parts[2..].iter().map(|s| s.to_string()).collect();
    Some((tag, command, args))
}

// Email address parsing simulation
fn parse_email_address(input: &str) -> Option<(String, String)> {
    if let Some(at_pos) = input.find('@') {
        let local = &input[..at_pos];
        let domain = &input[at_pos + 1..];
        Some((local.to_string(), domain.to_string()))
    } else {
        None
    }
}

// Header parsing simulation
fn parse_header(input: &str) -> Option<(String, String)> {
    if let Some(colon_pos) = input.find(':') {
        let name = input[..colon_pos].trim().to_string();
        let value = input[colon_pos + 1..].trim().to_string();
        Some((name, value))
    } else {
        None
    }
}

// MIME boundary detection
fn detect_mime_boundary(input: &str, boundary: &str) -> Vec<usize> {
    let boundary_str = format!("--{}", boundary);
    input.match_indices(&boundary_str).map(|(i, _)| i).collect()
}

// JSON parsing simulation
fn parse_json_simple(input: &str) -> bool {
    input.starts_with('{') && input.ends_with('}')
}

fn benchmark_smtp_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("smtp_parsing");

    let commands = vec![
        "HELO example.com",
        "EHLO mail.example.com",
        "MAIL FROM:<user@example.com>",
        "RCPT TO:<recipient@example.com>",
        "DATA",
        "QUIT",
        "RSET",
        "NOOP",
        "AUTH PLAIN dGVzdA==",
        "STARTTLS",
    ];

    for cmd in &commands {
        let cmd_type = cmd.split_whitespace().next().unwrap();
        group.bench_with_input(BenchmarkId::new("command", cmd_type), cmd, |b, &cmd| {
            b.iter(|| {
                black_box(parse_smtp_command(black_box(cmd)));
            })
        });
    }

    group.finish();
}

fn benchmark_imap_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("imap_parsing");

    let commands = vec![
        "A001 LOGIN user password",
        "A002 SELECT INBOX",
        "A003 FETCH 1:10 (FLAGS BODY[])",
        "A004 SEARCH ALL",
        "A005 STORE 1 +FLAGS (\\Seen)",
        "A006 EXPUNGE",
        "A007 LOGOUT",
    ];

    for cmd in &commands {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let cmd_type = if parts.len() > 1 { parts[1] } else { "UNKNOWN" };

        group.bench_with_input(BenchmarkId::new("command", cmd_type), cmd, |b, &cmd| {
            b.iter(|| {
                black_box(parse_imap_command(black_box(cmd)));
            })
        });
    }

    group.finish();
}

fn benchmark_imap_literals(c: &mut Criterion) {
    let mut group = c.benchmark_group("imap_literals");

    for size in [100, 1000, 10000, 100000].iter() {
        let literal_data = "A".repeat(*size);
        let command = format!("A001 APPEND INBOX {{{}}} {}", size, literal_data);

        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::new("literal", size), &command, |b, cmd| {
            b.iter(|| {
                black_box(parse_imap_command(black_box(cmd)));
            })
        });
    }

    group.finish();
}

fn benchmark_email_address_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("email_address");

    let addresses = vec![
        "user@example.com",
        "user.name@example.com",
        "user+tag@example.com",
        "user@subdomain.example.com",
        "very.long.email.address@very.long.domain.example.com",
    ];

    for addr in &addresses {
        group.bench_with_input(BenchmarkId::new("parse", addr), addr, |b, &addr| {
            b.iter(|| {
                black_box(parse_email_address(black_box(addr)));
            })
        });
    }

    group.finish();
}

fn benchmark_header_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("header_parsing");

    let headers = vec![
        "From: sender@example.com",
        "To: recipient@example.com",
        "Subject: Test message",
        "Date: Thu, 15 Feb 2024 10:00:00 +0000",
        "Content-Type: text/plain; charset=utf-8",
        "Message-ID: <unique-id@example.com>",
    ];

    for header in &headers {
        let header_name = header.split(':').next().unwrap();
        group.bench_with_input(
            BenchmarkId::new("header", header_name),
            header,
            |b, &header| {
                b.iter(|| {
                    black_box(parse_header(black_box(header)));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_mime_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("mime_parsing");

    for parts in [2, 5, 10, 20].iter() {
        let boundary = "boundary123";
        let mut message = String::new();

        for i in 0..*parts {
            message.push_str(&format!("--{}\r\n", boundary));
            message.push_str("Content-Type: text/plain\r\n\r\n");
            message.push_str(&format!("Part {} content\r\n", i));
        }
        message.push_str(&format!("--{}--\r\n", boundary));

        group.bench_with_input(BenchmarkId::new("parts", parts), &message, |b, msg| {
            b.iter(|| {
                black_box(detect_mime_boundary(black_box(msg), boundary));
            })
        });
    }

    group.finish();
}

fn benchmark_json_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_parsing");

    let jmap_requests = [
        r#"{"using":["urn:ietf:params:jmap:core"],"methodCalls":[]}"#,
        r#"{"using":["urn:ietf:params:jmap:core","urn:ietf:params:jmap:mail"],"methodCalls":[["Email/query",{"accountId":"u1","filter":{},"sort":[{"property":"receivedAt","isAscending":false}]},"c1"]]}"#,
        r#"{"using":["urn:ietf:params:jmap:core","urn:ietf:params:jmap:mail"],"methodCalls":[["Email/get",{"accountId":"u1","ids":["1","2","3"],"properties":["id","subject","from","to","receivedAt"]},"c1"]]}"#,
    ];

    for (i, json) in jmap_requests.iter().enumerate() {
        group.bench_with_input(BenchmarkId::new("jmap", i), json, |b, &json| {
            b.iter(|| {
                black_box(parse_json_simple(black_box(json)));
            })
        });
    }

    group.finish();
}

criterion_group! {
    name = parsing_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_smtp_parsing,
        benchmark_imap_parsing,
        benchmark_imap_literals,
        benchmark_email_address_parsing,
        benchmark_header_parsing,
        benchmark_mime_parsing,
        benchmark_json_parsing
}

criterion_main!(parsing_benches);
