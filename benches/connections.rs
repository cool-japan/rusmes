//! Concurrent connections benchmark
//!
//! Target: 10,000+ concurrent IMAP connections with <100MB overhead

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Simulated connection pool
struct ConnectionPool {
    active_connections: Arc<AtomicUsize>,
    max_connections: usize,
    memory_per_connection: usize, // bytes
}

impl ConnectionPool {
    fn new(max_connections: usize) -> Self {
        Self {
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
            memory_per_connection: 10240, // 10KB per connection target
        }
    }

    fn acquire(&self) -> Option<Connection> {
        let current = self.active_connections.fetch_add(1, Ordering::SeqCst);
        if current < self.max_connections {
            Some(Connection {
                pool: self.active_connections.clone(),
                buffer: vec![0u8; self.memory_per_connection],
            })
        } else {
            self.active_connections.fetch_sub(1, Ordering::SeqCst);
            None
        }
    }

    fn active_count(&self) -> usize {
        self.active_connections.load(Ordering::SeqCst)
    }

    fn memory_usage(&self) -> usize {
        self.active_count() * self.memory_per_connection
    }
}

/// Simulated connection
struct Connection {
    pool: Arc<AtomicUsize>,
    #[allow(dead_code)]
    buffer: Vec<u8>, // Connection buffer
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.pool.fetch_sub(1, Ordering::SeqCst);
    }
}

fn benchmark_connection_establishment(c: &mut Criterion) {
    let mut group = c.benchmark_group("connection_establishment");

    for max_conn in [100, 1000, 10000].iter() {
        let pool = ConnectionPool::new(*max_conn);

        group.bench_with_input(BenchmarkId::new("establish", max_conn), max_conn, |b, _| {
            b.iter(|| {
                let conn = pool.acquire();
                black_box(conn);
            })
        });
    }

    group.finish();
}

fn benchmark_connection_pool_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("connection_pool_scaling");

    for max_conn in [10, 100, 1000, 10000].iter() {
        group.throughput(Throughput::Elements(*max_conn as u64));

        let pool = Arc::new(ConnectionPool::new(*max_conn));

        group.bench_with_input(BenchmarkId::new("pool", max_conn), max_conn, |b, &count| {
            b.iter(|| {
                let mut connections = Vec::new();
                for _ in 0..count {
                    if let Some(conn) = pool.acquire() {
                        connections.push(conn);
                    }
                }
                black_box(connections);
            })
        });
    }

    group.finish();
}

fn benchmark_concurrent_acquire_release(c: &mut Criterion) {
    let pool = Arc::new(ConnectionPool::new(10000));

    c.bench_function("concurrent_acquire_release", |b| {
        b.iter(|| {
            let pool = pool.clone();
            if let Some(conn) = pool.acquire() {
                black_box(&conn);
                // Connection drops here
            }
        })
    });
}

fn benchmark_memory_per_connection(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_per_connection");

    for conn_count in [100, 1000, 10000].iter() {
        let pool = ConnectionPool::new(*conn_count);

        group.bench_with_input(
            BenchmarkId::new("memory", conn_count),
            conn_count,
            |b, &count| {
                b.iter(|| {
                    let mut connections = Vec::new();
                    for _ in 0..count {
                        if let Some(conn) = pool.acquire() {
                            connections.push(conn);
                        }
                    }
                    let memory = pool.memory_usage();
                    black_box((connections, memory));
                })
            },
        );
    }

    group.finish();
}

fn benchmark_connection_cleanup(c: &mut Criterion) {
    let mut group = c.benchmark_group("connection_cleanup");

    for count in [100, 1000, 10000].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        let pool = ConnectionPool::new(*count);

        group.bench_with_input(BenchmarkId::new("cleanup", count), count, |b, &count| {
            b.iter(|| {
                let mut connections = Vec::new();
                for _ in 0..count {
                    if let Some(conn) = pool.acquire() {
                        connections.push(conn);
                    }
                }
                // Drop all connections
                connections.clear();
                black_box(pool.active_count());
            })
        });
    }

    group.finish();
}

criterion_group! {
    name = connection_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_connection_establishment,
        benchmark_connection_pool_scaling,
        benchmark_concurrent_acquire_release,
        benchmark_memory_per_connection,
        benchmark_connection_cleanup
}

criterion_main!(connection_benches);
