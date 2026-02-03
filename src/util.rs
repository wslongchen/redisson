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
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::thread;
use std::time::Duration;
use rand::Rng;
use rand::distributions::Alphanumeric;
use redis::FromRedisValue;
use uuid::Uuid;

pub trait RedisMapExt {
    fn get_i64(&self, key: &str) -> i64;
    fn get_u64(&self, key: &str) -> u64;
    fn get_string(&self, key: &str) -> String;
}

impl RedisMapExt for HashMap<String, redis::Value> {
    fn get_i64(&self, key: &str) -> i64 {
        self.get(key)
            .and_then(|v| i64::from_redis_value(v.clone()).ok())
            .unwrap_or(0)
    }

    fn get_u64(&self, key: &str) -> u64 {
        self.get_i64(key) as u64
    }

    fn get_string(&self, key: &str) -> String {
        self.get(key)
            .and_then(|v| String::from_redis_value(v.clone()).ok())
            .unwrap_or_else(|| "0-0".to_string())
    }
}

pub(crate) fn get_random_string(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

pub(crate) fn get_lock_id() -> String {
    Uuid::new_v4().to_string()
}

pub(crate) fn num_milliseconds(duration: &Duration) -> u64 {
    duration.as_millis() as u64
}

pub(crate) fn calculate_drift(ttl: Duration, drift_factor: f64) -> Duration {
    let drift_ms = (ttl.as_millis() as f64 * drift_factor).ceil() as u64;
    Duration::from_millis(drift_ms)
}

pub(crate) fn calculate_quorum(n: usize) -> usize {
    (n as f64 / 2.0).floor() as usize + 1
}

pub(crate) fn jitter_delay(base_delay: Duration, jitter_ms: u64) -> Duration {
    let mut rng = rand::thread_rng();
    let jitter = rng.gen_range(0..=jitter_ms) as i64;
    if rng.gen_bool(0.5) {
        base_delay + Duration::from_millis(jitter as u64)
    } else {
        base_delay - Duration::from_millis(jitter as u64).min(base_delay)
    }
}

// Helper function to get the thread ID
pub(crate) fn thread_id_to_u64() -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    thread::current().id().hash(&mut hasher);
    hasher.finish()
}