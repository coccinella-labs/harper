// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use harper_workspace::*;
use rusqlite::Connection;
use std::hint::black_box;
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;

#[allow(dead_code)]
fn setup_database(size: usize) -> (Connection, String) {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();
    init_db(&conn).unwrap();

    // Pre-populate with test data
    for i in 0..size {
        save_message(
            &conn,
            "bench-session",
            if i % 2 == 0 { "user" } else { "assistant" },
            &format!("Benchmark message {}: {}", i, "x".repeat(100)),
        )
        .unwrap();
    }

    (conn, temp_file.path().to_str().unwrap().to_string())
}

#[allow(dead_code, clippy::cast_sign_loss)]
fn save_messages_benchmark(c: &mut Criterion) {
    let sizes = [1, 10, 100, 1000];

    for &size in &sizes {
        let mut group = c.benchmark_group(format!("save_message_{size}"));

        group.throughput(Throughput::Elements(size as u64));

        group.bench_function("sequential", |b| {
            b.iter_batched(
                || {
                    let (conn, _) = setup_database(0);
                    conn
                },
                |conn| {
                    for i in 0..size {
                        save_message(
                            &conn,
                            "bench-session",
                            if i % 2 == 0 { "user" } else { "assistant" },
                            &format!("Message {i}"),
                        )
                        .unwrap();
                    }
                },
                BatchSize::PerIteration,
            );
        });

        group.finish();
    }
}

#[allow(dead_code)]
fn load_history_benchmark(c: &mut Criterion) {
    let sizes = [1, 10, 100, 1000, 10_000];

    for &size in &sizes {
        let mut group = c.benchmark_group(format!("load_history_{size}"));
        group.sample_size(10);

        let (conn, _) = setup_database(size);

        group.bench_function("load_all", |b| {
            b.iter(|| {
                let history = load_history(&conn, "bench-session").unwrap();
                assert_eq!(history.len(), size);
                black_box(history);
            });
        });

        group.bench_function("load_paginated", |b| {
            b.iter(|| {
                let page_size = 100;
                let pages = size.div_ceil(page_size);

                for page in 0..pages {
                    let offset = page * page_size;
                    let limit = page_size.min(size - offset);

                    let history = load_history(&conn, "bench-session").unwrap();
                    let page_data = history
                        .into_iter()
                        .skip(offset)
                        .take(limit)
                        .collect::<Vec<_>>();
                    assert!(page_data.len() <= page_size);
                    black_box(page_data);
                }
            });
        });

        group.finish();
    }
}

#[allow(dead_code)]
fn concurrent_access_benchmark(c: &mut Criterion) {
    let num_threads = num_cpus::get();
    let messages_per_thread = 100;

    c.bench_function("concurrent_writes", |b| {
        b.iter_batched(
            || {
                let temp_file = NamedTempFile::new().unwrap();
                let db_path = temp_file.path().to_path_buf();
                let conn = Connection::open(&db_path).unwrap();
                init_db(&conn).unwrap();
                (db_path, temp_file)
            },
            |(db_path, _temp_file)| {
                let handles: Vec<_> = (0..num_threads)
                    .map(|i| {
                        let db_path = db_path.clone();
                        thread::spawn(move || {
                            let conn = Connection::open(&db_path).unwrap();
                            let session_id = format!("session-{i}");
                            save_session(&conn, &session_id).unwrap();

                            for j in 0..messages_per_thread {
                                save_message(
                                    &conn,
                                    &session_id,
                                    if j % 2 == 0 { "user" } else { "assistant" },
                                    &format!("Message {j} from thread {i}"),
                                )
                                .unwrap();
                            }
                        })
                    })
                    .collect();

                for handle in handles {
                    handle.join().unwrap();
                }
            },
            BatchSize::PerIteration,
        );
    });
}

#[allow(dead_code)]
fn large_message_benchmark(c: &mut Criterion) {
    let sizes = [1_000, 10_000, 100_000];

    for &size in &sizes {
        let mut group = c.benchmark_group(format!("large_message_{size}"));

        group.throughput(Throughput::Elements(1));
        group.sample_size(10);

        group.bench_function("save_large_message", |b| {
            b.iter_batched(
                || {
                    let temp_file = NamedTempFile::new().unwrap();
                    let conn = Connection::open(temp_file.path()).unwrap();
                    init_db(&conn).unwrap();
                    save_session(&conn, "bench-session").unwrap();
                    (conn, temp_file)
                },
                |(conn, _temp_file)| {
                    let large_content = "x".repeat(size);
                    save_message(&conn, "bench-session", "user", &large_content).unwrap();
                },
                BatchSize::PerIteration,
            );
        });

        group.finish();
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(10);
    targets =
        save_messages_benchmark,
        load_history_benchmark,
        concurrent_access_benchmark,
        large_message_benchmark
);

criterion_main!(benches);
