//! Search performance benchmark
//!
//! Target: <50ms p95 search query latency

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

/// Simple inverted index for benchmarking
struct SearchIndex {
    data: HashMap<String, Vec<usize>>,
    documents: Vec<String>,
}

impl SearchIndex {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
            documents: Vec::new(),
        }
    }

    fn index_message(&mut self, content: String) {
        let id = self.documents.len();
        self.documents.push(content.clone());

        let words: Vec<String> = content
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .collect();

        for word in words {
            self.data.entry(word).or_default().push(id);
        }
    }

    fn search(&self, query: &str) -> Vec<usize> {
        let terms: Vec<&str> = query.split_whitespace().collect();

        if terms.is_empty() {
            return Vec::new();
        }

        // Simple AND search
        let mut results: Option<Vec<usize>> = None;

        for term in terms {
            if let Some(doc_ids) = self.data.get(&term.to_lowercase()) {
                match &mut results {
                    None => results = Some(doc_ids.clone()),
                    Some(existing) => {
                        existing.retain(|id| doc_ids.contains(id));
                    }
                }
            } else {
                return Vec::new();
            }
        }

        results.unwrap_or_default()
    }

    fn search_or(&self, query: &str) -> Vec<usize> {
        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut results = Vec::new();

        for term in terms {
            if let Some(doc_ids) = self.data.get(&term.to_lowercase()) {
                for id in doc_ids {
                    if !results.contains(id) {
                        results.push(*id);
                    }
                }
            }
        }

        results
    }
}

fn generate_message(id: usize) -> String {
    format!(
        "From: sender{}@example.com\n\
         To: recipient@example.com\n\
         Subject: Test message number {}\n\
         \n\
         This is a test message with some content and keywords like urgent, \
         important, meeting, deadline, project, update.",
        id, id
    )
}

fn benchmark_index_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_indexing");

    for count in [100, 1000, 10000, 100000].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(BenchmarkId::new("build", count), count, |b, &count| {
            b.iter(|| {
                let mut index = SearchIndex::new();
                for i in 0..count {
                    let content = generate_message(i);
                    index.index_message(content);
                }
                black_box(index);
            })
        });
    }

    group.finish();
}

fn benchmark_simple_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("simple_search");

    for count in [1000, 10000, 100000].iter() {
        let mut index = SearchIndex::new();
        for i in 0..*count {
            let content = generate_message(i);
            index.index_message(content);
        }

        group.bench_with_input(BenchmarkId::new("subject", count), count, |b, _| {
            b.iter(|| {
                let results = index.search(black_box("subject"));
                black_box(results);
            })
        });
    }

    group.finish();
}

fn benchmark_complex_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_search");

    let mut index = SearchIndex::new();
    for i in 0..10000 {
        let content = generate_message(i);
        index.index_message(content);
    }

    // AND search
    group.bench_function("and_search", |b| {
        b.iter(|| {
            let results = index.search(black_box("urgent meeting"));
            black_box(results);
        })
    });

    // OR search
    group.bench_function("or_search", |b| {
        b.iter(|| {
            let results = index.search_or(black_box("urgent important deadline"));
            black_box(results);
        })
    });

    // Complex multi-term
    group.bench_function("multi_term", |b| {
        b.iter(|| {
            let results = index.search(black_box("urgent meeting project"));
            black_box(results);
        })
    });

    group.finish();
}

fn benchmark_search_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_scaling");

    for index_size in [1000, 10000, 100000].iter() {
        let mut index = SearchIndex::new();
        for i in 0..*index_size {
            let content = generate_message(i);
            index.index_message(content);
        }

        group.bench_with_input(BenchmarkId::new("scale", index_size), index_size, |b, _| {
            b.iter(|| {
                let results = index.search(black_box("meeting"));
                black_box(results);
            })
        });
    }

    group.finish();
}

fn benchmark_index_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_update");

    let mut index = SearchIndex::new();
    for i in 0..10000 {
        let content = generate_message(i);
        index.index_message(content);
    }

    group.bench_function("add_message", |b| {
        let mut counter = 10000;
        b.iter(|| {
            let content = generate_message(counter);
            index.index_message(content);
            counter += 1;
        })
    });

    group.finish();
}

criterion_group! {
    name = search_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_index_build,
        benchmark_simple_search,
        benchmark_complex_search,
        benchmark_search_scaling,
        benchmark_index_update
}

criterion_main!(search_benches);
