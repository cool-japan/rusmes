//! Authentication benchmarks
//!
//! Benchmarks for password hashing, verification, and various auth methods

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

/// Simple in-memory auth backend
struct MemoryAuthBackend {
    users: HashMap<String, String>, // username -> password_hash
}

impl MemoryAuthBackend {
    fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    fn add_user(&mut self, username: String, password_hash: String) {
        self.users.insert(username, password_hash);
    }

    fn verify(&self, username: &str, password: &str) -> bool {
        if let Some(stored_hash) = self.users.get(username) {
            // Simulate password verification
            simple_hash(password) == *stored_hash
        } else {
            false
        }
    }
}

/// Simple hash function for benchmarking (NOT secure, just for testing)
fn simple_hash(password: &str) -> String {
    format!("hash_{}", password)
}

/// Simulate bcrypt hashing (expensive)
fn simulate_bcrypt_hash(password: &str, cost: u32) -> String {
    let mut hash = password.to_string();
    for _ in 0..cost {
        hash = simple_hash(&hash);
    }
    hash
}

/// Simulate bcrypt verification
fn simulate_bcrypt_verify(password: &str, hash: &str, cost: u32) -> bool {
    simulate_bcrypt_hash(password, cost) == hash
}

/// Simulate LDAP bind operation
fn simulate_ldap_bind(username: &str, password: &str) -> bool {
    // Simulate network delay and verification
    !username.is_empty() && !password.is_empty()
}

/// Simulate SQL query for auth
fn simulate_sql_auth(username: &str, password: &str) -> bool {
    // Simulate database query and verification
    !username.is_empty() && !password.is_empty()
}

/// Simulate OAuth2 token validation
fn simulate_oauth2_validate(token: &str) -> bool {
    // Simulate token validation
    token.len() > 10
}

fn benchmark_bcrypt_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcrypt_hashing");

    for cost in [4, 8, 10, 12].iter() {
        group.bench_with_input(BenchmarkId::new("cost", cost), cost, |b, &cost| {
            b.iter(|| {
                black_box(simulate_bcrypt_hash(black_box("password123"), cost));
            })
        });
    }

    group.finish();
}

fn benchmark_bcrypt_verification(c: &mut Criterion) {
    let mut group = c.benchmark_group("bcrypt_verification");

    for cost in [4, 8, 10, 12].iter() {
        let hash = simulate_bcrypt_hash("password123", *cost);

        group.bench_with_input(BenchmarkId::new("cost", cost), cost, |b, &cost| {
            b.iter(|| {
                black_box(simulate_bcrypt_verify(
                    black_box("password123"),
                    black_box(&hash),
                    cost,
                ));
            })
        });
    }

    group.finish();
}

fn benchmark_memory_backend(c: &mut Criterion) {
    let mut backend = MemoryAuthBackend::new();

    // Add some users
    for i in 0..1000 {
        let username = format!("user{}", i);
        let hash = simple_hash(&format!("password{}", i));
        backend.add_user(username, hash);
    }

    c.bench_function("memory_auth_verify", |b| {
        b.iter(|| {
            black_box(backend.verify(black_box("user500"), black_box("password500")));
        })
    });
}

fn benchmark_ldap_simulation(c: &mut Criterion) {
    c.bench_function("ldap_bind", |b| {
        b.iter(|| {
            black_box(simulate_ldap_bind(
                black_box("user@example.com"),
                black_box("password123"),
            ));
        })
    });
}

fn benchmark_sql_simulation(c: &mut Criterion) {
    c.bench_function("sql_auth", |b| {
        b.iter(|| {
            black_box(simulate_sql_auth(
                black_box("user@example.com"),
                black_box("password123"),
            ));
        })
    });
}

fn benchmark_oauth2_simulation(c: &mut Criterion) {
    let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";

    c.bench_function("oauth2_validate", |b| {
        b.iter(|| {
            black_box(simulate_oauth2_validate(black_box(token)));
        })
    });
}

fn benchmark_concurrent_auth(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_auth");

    let mut backend = MemoryAuthBackend::new();
    for i in 0..1000 {
        let username = format!("user{}", i);
        let hash = simple_hash(&format!("password{}", i));
        backend.add_user(username, hash);
    }

    for concurrent in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent", concurrent),
            concurrent,
            |b, &count| {
                b.iter(|| {
                    for i in 0..count {
                        let username = format!("user{}", i % 1000);
                        let password = format!("password{}", i % 1000);
                        black_box(backend.verify(&username, &password));
                    }
                })
            },
        );
    }

    group.finish();
}

fn benchmark_auth_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_cache");

    // Simulate cache hit vs miss
    let mut cache: HashMap<String, bool> = HashMap::new();

    group.bench_function("cache_hit", |b| {
        cache.insert("user@example.com".to_string(), true);
        b.iter(|| {
            black_box(cache.get(black_box("user@example.com")));
        })
    });

    group.bench_function("cache_miss", |b| {
        b.iter(|| {
            black_box(cache.get(black_box("nonexistent@example.com")));
        })
    });

    group.finish();
}

criterion_group! {
    name = auth_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        benchmark_bcrypt_hashing,
        benchmark_bcrypt_verification,
        benchmark_memory_backend,
        benchmark_ldap_simulation,
        benchmark_sql_simulation,
        benchmark_oauth2_simulation,
        benchmark_concurrent_auth,
        benchmark_auth_cache
}

criterion_main!(auth_benches);
