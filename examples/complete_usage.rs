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
use std::time::Duration;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use tokio::runtime::Runtime;
use redisson::{AsyncRedissonClient, BatchPriority, BatchResult, Cache, RedissonClient, RedissonConfig, RedissonResult, SetCommand};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Order {
    id: String,
    customer_id: String,
    amount: f64,
    items: Vec<OrderItem>,
    status: OrderStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OrderItem {
    product_id: String,
    quantity: i32,
    price: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
enum OrderStatus {
    Pending,
    Processing,
    Shipped,
    Delivered,
    Cancelled,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UserSession {
    user_id: String,
    session_token: String,
    created_at: u64,
    last_activity: u64,
    ip_address: String,
}

fn main() -> RedissonResult<()> {
    println!("🚀 Redisson Rust - Complete Optimized Example");
    println!("=============================================\n");

    // 1. Create configuration
    let config = RedissonConfig::single_server("redis://127.0.0.1:6379")
        .with_pool_size(20)
        .with_connection_timeout(Duration::from_secs(5))
        .with_response_timeout(Duration::from_secs(3))
        .with_lock_expire_time(Duration::from_secs(30))
        .with_watchdog_timeout(Duration::from_secs(10))
        .with_retry_count(3)
        .with_drift_factor(0.01)
        .with_backup_pool_count(2);

    // 2. Create client
    let client = RedissonClient::new(config)?;
    println!("✅ Client created successfully\n");

    // 3. Demonstrate Stream functionality
    println!("📡 Redis Stream Demo");
    stream_demo(&client)?;

    // 4. Demonstrate batch operation optimization
    println!("\n📚 Batch Operation Optimization Demo");
    batch_optimization_demo(&client)?;

    // 5. Demonstrate local cache integration
    println!("\n💾 Local Cache Integration Demo");
    cache_integration_demo(&client)?;

    // 6. Demonstrate performance statistics
    println!("\n📊 Performance Statistics");
    show_stats(&client)?;

    // 7. Asynchronous example
    println!("\n⚡ Asynchronous Operations Demo");
    async_demo()?;

    println!("\n🎉 All demos completed!");

    client.shutdown()?;
    println!("🔌 Client closed");

    Ok(())
}

fn stream_demo(client: &RedissonClient) -> RedissonResult<()> {
    println!("  1. Creating order stream...");
    let order_stream = client.get_stream::<Order>("orders:stream");

    // Create consumer group
    order_stream.create_group("order-processors", "0")?;
    println!("     ✅ Created consumer group: order-processors");

    // Add order messages
    println!("  2. Publishing order messages...");

    let order1 = Order {
        id: "ORD-001".to_string(),
        customer_id: "CUST-001".to_string(),
        amount: 299.99,
        items: vec![
            OrderItem {
                product_id: "PROD-001".to_string(),
                quantity: 2,
                price: 149.99,
            },
        ],
        status: OrderStatus::Pending,
    };

    let mut fields1 = HashMap::new();
    fields1.insert("order".to_string(), order1.clone());

    let message_id1 = order_stream.add_auto_id(&fields1)?;
    println!("     📨 Order 1 message ID: {}", message_id1);

    let order2 = Order {
        id: "ORD-002".to_string(),
        customer_id: "CUST-002".to_string(),
        amount: 599.99,
        items: vec![
            OrderItem {
                product_id: "PROD-002".to_string(),
                quantity: 1,
                price: 599.99,
            },
        ],
        status: OrderStatus::Pending,
    };

    let mut fields2 = HashMap::new();
    fields2.insert("order".to_string(), order2.clone());

    let message_id2 = order_stream.add_auto_id(&fields2)?;
    println!("     📨 Order 2 message ID: {}", message_id2);

    // Read messages
    println!("  3. Consuming order messages...");
    let messages = order_stream.read_group("order-processors", "consumer-1", Some(10), None, false)?;

    println!("     📥 Received {} messages", messages.len());

    for (i, message) in messages.iter().enumerate() {
        if let Some(order_field) = message.fields.get("order") {
            println!("     🛒 Message {}: Order ID: {}, Amount: ${}",
                     i + 1, order_field.id, order_field.amount);

            // Acknowledge message processing
            order_stream.ack("order-processors", &[message.id.clone()])?;
        }
    }

    // Get stream information
    println!("  4. Getting stream information...");
    let info = order_stream.info()?;
    println!("     📊 Stream length: {}", info.length);
    println!("     👥 Number of consumer groups: {}", info.groups);

    // Set max length and trim
    println!("  5. Trimming stream...");
    let trimmed = order_stream.trim(1000, true)?;
    println!("     ✂️  Trimmed {} messages", trimmed);

    Ok(())
}

fn batch_optimization_demo(client: &RedissonClient) -> RedissonResult<()> {
    println!("  1. Creating batch operation...");
    let mut batch = &mut client.create_batch();

    // Add multiple operations
    let start = std::time::Instant::now();

    for i in 1..=100 {
        let key = format!("batch:user:{}", i);
        let value = format!("User {}", i);
        batch = batch.set(&key, &value);

        if i % 10 == 0 {
            batch = batch.get::<String>(key.to_string());
        }

        if i % 20 == 0 {
            batch = batch.expire(key, 3600);
        }
    }

    println!("     📋 Added 100 SET operations, 10 GET operations, and 5 EXPIRE operations");

    // Execute batch operation
    println!("  2. Executing batch operation...");
    let results = batch.execute()?.unwrap_or_default();
    let duration = start.elapsed();

    println!("     ⚡ Batch execution duration: {:?}", duration);
    println!("     📊 Number of results returned: {}", results.len());

    // Analyze results
    let set_count = results.iter()
        .filter(|r| !matches!(r, BatchResult::Error(_)))
        .count();

    let get_count = results.iter()
        .filter(|r| !matches!(r, BatchResult::Error(_)))
        .count();

    println!("     ✅ SET operations successful: {}", set_count);
    println!("     ✅ GET results: {}", get_count);

    // Demonstrate priority batch operations
    println!("  3. Priority batch operations...");
    let batch_optimizer = client.get_batch_processor();

    // Add high priority operations
    for i in 1..=5 {
        let key = format!("priority:high:{}", i);
        let value = format!("High Priority {}", i);

        batch_optimizer.exec_batch(
            vec![Box::new(SetCommand::new(key, value).with_ttl(Duration::from_secs(300)))]
        )?;
    }

    println!("     🚀 Added 5 high priority operations");

    // Add normal priority operations
    for i in 1..=20 {
        let key = format!("priority:normal:{}", i);
        let value = format!("Normal Priority {}", i);

        batch_optimizer.exec_batch(
            vec![Box::new(SetCommand::new(key, value))]
        )?;
    }

    println!("     📄 Added 20 normal priority operations");

    // Flush immediately
    batch_optimizer.flush()?;
    println!("     🔄 Batch flush completed");

    Ok(())
}

fn cache_integration_demo(client: &RedissonClient) -> RedissonResult<()> {
    println!("  1. Creating integrated cache...");
    let user_cache = client.get_cache::<String, UserSession>("user_sessions");

    println!("     💾 Cache created successfully (read-through/write-through mode)");

    // Create session data
    println!("  2. Setting cache data...");
    let session = UserSession {
        user_id: "user123".to_string(),
        session_token: "abc123def456".to_string(),
        created_at: 1234567890,
        last_activity: 1234567990,
        ip_address: "192.168.1.100".to_string(),
    };

    user_cache.set("user123".to_string(), session.clone())?;
    println!("     💾 Session data cached");

    // Read data (should hit local cache)
    println!("  3. Reading cached data...");
    let start = std::time::Instant::now();

    let cached_session = user_cache.get(&"user123".to_string())?;
    let first_duration = start.elapsed();

    if let Some(session) = cached_session {
        println!("     ✅ Cache hit: User {}", session.user_id);
        println!("     ⚡ First read duration: {:?}", first_duration);
    }

    // Read again (should be faster)
    println!("  4. Reading again (local cache)...");
    let start = std::time::Instant::now();

    let cached_session2 = user_cache.get(&"user123".to_string())?;
    let second_duration = start.elapsed();

    println!("     ⚡ Second read duration: {:?}", second_duration);
    println!("     🚀 Performance improvement: {:.1}x",
             first_duration.as_nanos() as f64 / second_duration.as_nanos() as f64);

    // Get cache statistics
    println!("  5. Cache statistics...");
    let cache_stats = user_cache.get_local_cache().get_stats();
    let client_stats = client.get_stats();

    println!("     📊 Local cache hit rate: {:.1}%",
             client_stats.cache_stats.avg_hit_rate * 100.0);
    println!("     💿 Local cache entries: {}", cache_stats.total_entries);
    println!("     🔥 Local cache hits: {}", cache_stats.total_hits);

    // Clear cache
    println!("  6. Clearing cache...");
    user_cache.clear()?;
    println!("     🧹 Cache cleared");

    Ok(())
}

fn show_stats(client: &RedissonClient) -> RedissonResult<()> {
    let stats = client.get_stats();

    println!("  Connection Pool Statistics:");
    println!("    📈 Total connections created: {}", stats.connection_stats.total_connections_created);
    println!("    🔄 Connection reuse rate: {:.1}%",
             stats.connection_stats.total_connections_reused as f64 /
                 stats.connection_stats.total_operations as f64 * 100.0);
    println!("    ⚡ Average wait time: {:.2}ms", stats.connection_stats.total_wait_time_ms);
    println!("    📊 Peak connections: {}", stats.connection_stats.peak_connections);

    println!("\n  Batch Operation Statistics:");
    println!("    📦 Total batches: {}", stats.batch_stats.total_batches);
    println!("    📝 Total commands: {}", stats.batch_stats.total_commands);
    println!("    📏 Average batch size: {:.1}", stats.batch_stats.avg_batch_size);
    println!("    ⏱️  Average execution time: {:.2}ms", stats.batch_stats.avg_execution_time_ms);

    println!("\n  Cache Statistics:");
    println!("    🎯 Total hits: {}", stats.cache_stats.total_hits);
    println!("    ❌ Total misses: {}", stats.cache_stats.total_misses);
    println!("    📈 Hit rate: {:.1}%", stats.cache_stats.avg_hit_rate * 100.0);
    println!("    🗑️  Total evictions: {}", stats.cache_stats.total_evictions);

    Ok(())
}

fn async_demo() -> RedissonResult<()> {
    let rt = Runtime::new().unwrap();

    rt.block_on(async {
        println!("  1. Creating async client...");

        let config = RedissonConfig::single_server("redis://127.0.0.1:6379");
        let client = AsyncRedissonClient::new(config).await.unwrap();

        println!("     ✅ Async client created successfully");

        println!("  2. Async lock operations...");
        let lock = client.get_lock("async:demo:lock");

        lock.lock().await.unwrap();
        println!("     🔒 Async lock acquired successfully");

        // Simulate async operation
        tokio::time::sleep(Duration::from_millis(100)).await;

        lock.unlock().await.unwrap();
        println!("     🔓 Async lock released successfully");

        println!("  3. Async data operations...");
        let bucket = client.get_bucket::<String>("async:demo:data");

        bucket.set(&"Hello Async".to_string()).await.unwrap();
        println!("     💾 Data set successfully");

        let value = bucket.get().await.unwrap();
        println!("     📖 Read data: {:?}", value);

        client.shutdown().await.unwrap();
        println!("     🔌 Async client closed");
    });

    Ok(())
}

