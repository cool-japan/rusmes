//! Search performance benchmark

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use std::hint::black_box;

struct SimpleSearchIndex {
    data: HashMap<String, Vec<String>>,
}

impl SimpleSearchIndex {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    fn index_message(&mut self, id: String, content: &str) {
        let words: Vec<String> = content
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .collect();

        for word in words {
            self.data.entry(word).or_default().push(id.clone());
        }
    }

    fn search(&self, query: &str) -> Vec<String> {
        self.data
            .get(&query.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }
}

fn benchmark_indexing(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_indexing");

    for count in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            b.iter(|| {
                let mut index = SimpleSearchIndex::new();
                for i in 0..count {
                    let content = format!("test message {} with some content", i);
                    index.index_message(format!("msg_{}", i), &content);
                }
                black_box(index);
            })
        });
    }

    group.finish();
}

fn benchmark_search(c: &mut Criterion) {
    let mut index = SimpleSearchIndex::new();
    for i in 0..10000 {
        let content = format!("test message {} with some content", i);
        index.index_message(format!("msg_{}", i), &content);
    }

    c.bench_function("search_query", |b| {
        b.iter(|| {
            let results = index.search(black_box("test"));
            black_box(results);
        })
    });
}

criterion_group!(search_benches, benchmark_indexing, benchmark_search);
criterion_main!(search_benches);
