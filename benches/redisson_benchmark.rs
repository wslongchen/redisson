/*
 *
 *  *
 *  *      Copyright (c) 2018-2025, SnackCloud All rights reserved.
 *  *
 *  *   Redistribution and use in source and binary forms, with or without
 *  *   modification, are permitted provided that the following conditions are met:
 *  *
 *  *   Redistributions of source code must retain the above copyright notice,
 *  *   this list of conditions and the following disclaimer.
 *  *   Redistributions in binary form must reproduce the above copyright
 *  *   notice, this list of conditions and the following disclaimer in the
 *  *   documentation and/or other materials provided with the distribution.
 *  *   Neither the name of the www.snackcloud.cn developer nor the names of its
 *  *   contributors may be used to endorse or promote products derived from
 *  *   this software without specific prior written permission.
 *  *   Author: SnackCloud
 *  *
 *
 */

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Duration;
use std::sync::Arc;
use std::thread;
use criterion::async_executor::FuturesExecutor;
use serde::{Serialize, Deserialize};
use redisson::{BatchConfig, Cache, RedissonClient, RedissonConfig};

#[derive(Serialize, Deserialize, Clone)]
struct BenchmarkData {
    id: u64,
    name: String,
    value: f64,
    tags: Vec<String>,
    metadata: std::collections::HashMap<String, String>,
}

impl Default for BenchmarkData {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Test".to_string(),
            value: 123.456,
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            metadata: std::collections::HashMap::from([
                ("key1".to_string(), "value1".to_string()),
                ("key2".to_string(), "value2".to_string()),
            ]),
        }
    }
}

fn create_benchmark_client() -> RedissonClient {
    let config = RedissonConfig::single_server("redis://172.16.8.16:6379")
        .with_pool_size(10)
        .with_connection_timeout(Duration::from_millis(500))
        .with_response_timeout(Duration::from_millis(500))
        .with_idle_timeout(Duration::from_secs(5))
        .with_batch_config(BatchConfig::default()
            .with_max_batch_size(50)
            .with_pipeline(true));

    RedissonClient::new(config).unwrap()
}

fn bench_lock(c: &mut Criterion) {
    let client = create_benchmark_client();

    c.bench_function("lock_unlock", |b| {
        b.iter(|| {
            let lock = client.get_lock("bench:lock");
            lock.lock().unwrap();
            lock.unlock().unwrap();
        });
    });
}

fn bench_stream(c: &mut Criterion) {
    let client = create_benchmark_client();
    let stream = client.get_stream::<BenchmarkData>("bench:stream");

    c.bench_function("stream_add_auto_id", |b| {
        b.iter(|| {
            let mut fields = std::collections::HashMap::new();
            let data = BenchmarkData::default();
            fields.insert("data".to_string(), data);
            stream.add_auto_id(&fields).unwrap();
        });
    });
}

fn bench_batch(c: &mut Criterion) {
    let client = create_benchmark_client();

    c.bench_function("batch_100_sets", |b| {
        b.iter(|| {
            let mut batch = client.create_batch();
            for i in 0..100 {
                batch.set(&format!("batch:{}", i), &"value".to_string());
            }
            batch.execute().unwrap();
        });
    });
}

fn bench_cache(c: &mut Criterion) {
    let client = create_benchmark_client();
    let cache = client.get_cache::<String, BenchmarkData>("bench:cache");
    let data = BenchmarkData::default();

    c.bench_function("cache_set_get", |b| {
        b.iter(|| {
            cache.set("test_key".to_string(), data.clone()).unwrap();
            cache.get(&"test_key".to_string()).unwrap();
        });
    });
}

fn bench_concurrent_streams(c: &mut Criterion) {
    let client = Arc::new(create_benchmark_client());

    c.bench_function("concurrent_streams_10x10", |b| {
        b.iter(|| {
            let mut handles = Vec::new();

            for i in 0..10 {
                let client = client.clone();
                let handle = thread::spawn(move || {
                    let stream = client.get_stream::<String>(&format!("concurrent:stream:{}", i));

                    for j in 0..10 {
                        let mut fields = std::collections::HashMap::new();
                        fields.insert("message".to_string(), format!("Message {}", j));
                        stream.add_auto_id(&fields).unwrap();
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                handle.join().unwrap();
            }
        });
    });
}

fn bench_mixed_operations(c: &mut Criterion) {
    let client = create_benchmark_client();

    c.bench_function("mixed_operations", |b| {
        b.iter(|| {
            let lock = client.get_lock("mixed:lock");
            lock.lock().unwrap();

            let cache = client.get_cache::<String, String>("mixed:cache");
            cache.set("key".to_string(), "value".to_string()).unwrap();

            let stream = client.get_stream::<String>("mixed:stream");
            let mut fields = std::collections::HashMap::new();
            fields.insert("data".to_string(), "stream data".to_string());
            stream.add_auto_id(&fields).unwrap();

            let mut batch = client.create_batch();
            for i in 0..50 {
                batch.set(&format!("mixed:batch:{}", i), &"batch value".to_string());
            }
            batch.execute().unwrap();

            lock.unlock().unwrap();
        });
    });
}

fn bench_memory_efficiency(c: &mut Criterion) {
    let client = create_benchmark_client();

    c.bench_function("memory_efficiency_100_objects", |b| {
        let mut caches = Vec::new();
        for i in 0..100 {
            // Create outside benchmark loop
            caches.push(client.get_cache::<String, String>(&format!("cache:{}", i)));
        }

        b.iter(|| {
            for (i, cache) in caches.iter().enumerate() {
                // Use cached instance directly
                cache.set("test".to_string(), "value".to_string()).unwrap();
            }

            // Cleanup
            for cache in &caches {
                cache.clear().unwrap();
            }
        });
    });
}

// Comparative benchmark group
fn compare_benchmarks(c: &mut Criterion) {
    let client = create_benchmark_client();

    let mut group = c.benchmark_group("comparison");
    group.sample_size(10);  // Reduce sample size for speed

    group.bench_function("simple_lock", |b| {
        b.iter(|| {
            let lock = client.get_lock("compare:lock:simple");
            lock.lock().unwrap();
            lock.unlock().unwrap();
        });
    });

    group.bench_function("batch_operations", |b| {
        b.iter(|| {
            let mut batch = client.create_batch();
            for i in 0..100 {
                batch.set(&format!("compare:batch:{}", i), &"value".to_string());
            }
            batch.execute().unwrap();
        });
    });

    group.bench_function("cache_operations", |b| {
        b.iter(|| {
            let cache = client.get_cache::<String, String>("compare:cache");
            cache.set("key".to_string(), "value".to_string()).unwrap();
            cache.get(&"key".to_string()).unwrap();
        });
    });

    group.bench_function("stream_operations", |b| {
        b.iter(|| {
            let stream = client.get_stream::<String>("compare:stream");
            let mut fields = std::collections::HashMap::new();
            fields.insert("data".to_string(), "test".to_string());
            stream.add_auto_id(&fields).unwrap();
        });
    });

    group.finish();
}

// Parameterized benchmarks
fn parameterized_benchmarks(c: &mut Criterion) {
    let client = Arc::new(create_benchmark_client());

    let mut group = c.benchmark_group("parameterized");

    for batch_size in [10, 50, 100, 200].iter() {
        group.bench_with_input(
            BenchmarkId::new("batch_size", batch_size),
            batch_size,
            |b, &size| {
                b.iter(|| {
                    let mut batch = client.create_batch();
                    for i in 0..size {
                        batch.set(&format!("param:batch:{}:{}", size, i), &"value".to_string());
                    }
                    batch.execute().unwrap();
                });
            },
        );
    }

    for thread_count in [1, 2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_locks", thread_count),
            thread_count,
            |b, &threads| {
                b.iter(|| {
                    let mut handles = Vec::new();

                    for i in 0..threads {
                        let client = client.clone();
                        let handle = thread::spawn(move || {
                            let lock = client.get_lock(&format!("param:lock:{}", i));
                            lock.lock().unwrap();
                            lock.unlock().unwrap();
                        });
                        handles.push(handle);
                    }

                    for handle in handles {
                        handle.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

// Async benchmark tests (requires async executor)
fn bench_async_operations(c: &mut Criterion) {
    use redisson::AsyncRedissonClient;

    let config = RedissonConfig::single_server("redis://172.16.8.16:6379");

    c.bench_function("async_lock", |b| {
        b.to_async(FuturesExecutor).iter(|| async {
            let client = AsyncRedissonClient::new(config.clone()).await.unwrap();
            let lock = client.get_lock("async:bench:lock");
            lock.lock().await.unwrap();
            lock.unlock().await.unwrap();
        });
    });
}

// Define benchmark groups
criterion_group!(
    name = basic_benches;
    config = Criterion::default()
        .sample_size(20)          // Sample size
        .warm_up_time(Duration::from_secs(3))  // Warm-up time
        .measurement_time(Duration::from_secs(10)); // Measurement time
    targets = bench_lock, bench_stream, bench_batch, bench_cache, bench_mixed_operations
);

criterion_group!(
    name = concurrency_benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5));
    targets = bench_concurrent_streams, bench_memory_efficiency
);

criterion_group!(
    name = comparison_benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = compare_benchmarks, parameterized_benchmarks
);

criterion_group!(
    name = async_benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5));
    targets = bench_async_operations
);

// Run all benchmark groups
criterion_main!(
    basic_benches,
    concurrency_benches,
    comparison_benches,
    // async_benches,  // Optional
);