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
use redisson::{RedissonClient, RedissonConfig, RedissonResult};
use std::time::Duration;

fn main() -> RedissonResult<()> {
    // 1. Create configuration
    let config = RedissonConfig::single_server("redis://127.0.0.1:6379")
        .with_pool_size(20)
        .with_watchdog_timeout(Duration::from_secs(30));

    // 2. Create synchronous client
    let client = RedissonClient::new(config)?;

    // 3. Use distributed data structures
    let bucket = client.get_bucket::<String>("my_bucket");
    bucket.set(&"Hello World".to_string())?;
    let value: Option<String> = bucket.get()?;
    println!("Bucket value: {:?}", value);

    // 4. Use distributed lock
    let lock = client.get_lock("my_lock");
    lock.lock()?;

    // Execute protected code
    println!("Critical section accessed");

    lock.unlock()?;

    // 5. Use Map
    let map = client.get_map::<String, i32>("my_map");
    map.put(&"key1".to_string(), &42)?;
    let map_value = map.get(&"key1".to_string())?;
    println!("Map value: {:?}", map_value);

    client.shutdown()?;
    Ok(())
}

// examples/async_usage.rs
