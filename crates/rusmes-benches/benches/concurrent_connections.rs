//! Concurrent connections benchmark

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct ConnectionPool {
    active_connections: Arc<AtomicUsize>,
    max_connections: usize,
}

impl ConnectionPool {
    fn new(max_connections: usize) -> Self {
        Self {
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
        }
    }

    fn acquire(&self) -> Option<Connection> {
        let current = self.active_connections.fetch_add(1, Ordering::SeqCst);
        if current < self.max_connections {
            Some(Connection {
                pool: self.active_connections.clone(),
            })
        } else {
            self.active_connections.fetch_sub(1, Ordering::SeqCst);
            None
        }
    }

    #[allow(dead_code)]
    fn active_count(&self) -> usize {
        self.active_connections.load(Ordering::SeqCst)
    }
}

struct Connection {
    pool: Arc<AtomicUsize>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.pool.fetch_sub(1, Ordering::SeqCst);
    }
}

fn benchmark_connection_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("connection_pool");

    for max_conn in [10, 100, 1000].iter() {
        let pool = ConnectionPool::new(*max_conn);

        group.bench_with_input(BenchmarkId::from_parameter(max_conn), max_conn, |b, _| {
            b.iter(|| {
                let _conn = pool.acquire();
                black_box(&pool);
            })
        });
    }

    group.finish();
}

fn benchmark_concurrent_acquire(c: &mut Criterion) {
    let pool = Arc::new(ConnectionPool::new(1000));

    c.bench_function("concurrent_acquire", |b| {
        b.iter(|| {
            let pool = pool.clone();
            if let Some(_conn) = pool.acquire() {
                black_box(&pool);
            }
        })
    });
}

criterion_group!(
    connection_benches,
    benchmark_connection_pool,
    benchmark_concurrent_acquire
);
criterion_main!(connection_benches);
