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
use redisson::{AsyncRedissonClient, RedissonConfig, RedissonResult};

#[tokio::main]
async fn main() -> RedissonResult<()> {
    // 1. Create configuration
    let config = RedissonConfig::single_server("redis://127.0.0.1:6379");

    // 2. Create async client
    let client = AsyncRedissonClient::new(config).await?;

    // 3. Use async distributed data structures
    let bucket = client.get_bucket::<String>("async_bucket");
    bucket.set(&"Async Hello".to_string()).await?;
    let value = bucket.get().await?;
    println!("Async bucket value: {:?}", value);

    // 4. Use async distributed lock
    let lock = client.get_lock("async_lock");
    lock.lock().await?;

    // Execute protected async code
    println!("Async critical section accessed");

    lock.unlock().await?;

    client.shutdown().await?;
    Ok(())
}
