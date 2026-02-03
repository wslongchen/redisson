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

mod blocking;
#[cfg(feature = "async")]
mod non_blocking;
mod command_builder;
mod command;

pub use blocking::*;
pub use command::*;
pub use command_builder::*;
#[cfg(feature = "async")]
pub use non_blocking::*;


use crate::{BackoffStrategyConfig, RedissonError, RedissonResult};
use redis::{from_redis_value, Value};
use serde::de::DeserializeOwned;
use std::fmt::{Debug, Formatter};
use std::time::{Duration, Instant};


// ================ Priority enumeration ================
/// Batch priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BatchPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Execution mode
#[derive(Debug, Clone, Copy)]
pub enum ExecutionMode {
    Sync,
    Async,
}


impl Default for BatchPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Batch statistics
#[derive(Clone, Debug)]
pub struct BatchStats {
    pub total_batches: u64,
    pub total_commands: u64,
    pub total_executions: u64,
    pub total_success: u64,
    pub total_failures: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub avg_cached_command_count: u64,
    pub total_cached_items: u64,
    pub cache_size_bytes: u64,
    pub avg_batch_size: f64,
    pub avg_execution_time_ms: f64,
    pub queue_size: usize,
    pub last_flush: Option<Instant>,
    pub concurrent_batches: usize,
}

impl BatchStats {
    pub fn new() -> Self {
        Self {
            total_batches: 0,
            total_commands: 0,
            total_executions: 0,
            total_success: 0,
            total_failures: 0,
            cache_hits: 0,
            cache_misses: 0,
            avg_cached_command_count: 0,
            total_cached_items: 0,
            cache_size_bytes: 0,
            avg_batch_size: 0.0,
            avg_execution_time_ms: 0.0,
            concurrent_batches: 0,
            queue_size: 0,
            last_flush: None,
        }
    }
}

/// Batch group
pub struct BatchGroup {
    commands: Vec<Box<dyn CommandBuilder>>,
    created_at: Instant,
    priority: BatchPriority,
    callback: Option<Box<dyn FnOnce(RedissonResult<Vec<BatchResult>>) + Send>>,
}

impl Debug for BatchGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BatchGroup{{commands: {}, priority: {:?}, created_at: {:?}}}",
            self.commands.len(),
            self.priority,
            self.created_at
        )
    }
}


// ================ Fallback strategy ================

/// Backoff strategy
#[derive(Clone)]
enum BackoffStrategy {
    Linear(Duration),
    Exponential(Duration),
    Fixed(Duration),
}

impl BackoffStrategy {
    
    fn new(strategy: &BackoffStrategyConfig, backoff: Duration) -> Self {
        match strategy {
            BackoffStrategyConfig::Linear => BackoffStrategy::Linear(backoff),
            BackoffStrategyConfig::Exponential => BackoffStrategy::Exponential(backoff),
            BackoffStrategyConfig::Fixed => BackoffStrategy::Fixed(backoff),
        }
    }
    
    fn calculate_delay(&self, retry_count: u32) -> Duration {
        match self {
            BackoffStrategy::Linear(base) => *base * retry_count,
            BackoffStrategy::Exponential(base) => {
                let factor = 2u32.pow(retry_count.saturating_sub(1));
                *base * factor
            }
            BackoffStrategy::Fixed(base) => *base,
        }
    }
}



/// Batch results
#[derive(Debug, Clone)]
pub enum BatchResult {
    /// String results
    String(String),
    /// Integer results
    Integer(i64),
    /// Floating-point results
    Float(f64),
    /// Boolean result
    Bool(bool),
    /// Array results
    Array(Vec<BatchResult>),
    /// Empty result
    Nil,
    /// Status result (such as OK)
    Status(String),
    /// Raw Redis value
    Raw(Value),
    /// False results
    Error(String),
}

impl BatchResult {
    /// Converting from Redis values
    pub fn from_redis_value(value: Value) -> RedissonResult<Self> {
        match value {
            Value::Nil => Ok(BatchResult::Nil),
            Value::Int(i) => Ok(BatchResult::Integer(i)),
            Value::BulkString(data) => {
                match String::from_utf8(data) {
                    Ok(s) => Ok(BatchResult::String(s)),
                    Err(e) => Err(RedissonError::DeserializationError(e.to_string())),
                }
            }
            Value::SimpleString(s) => Ok(BatchResult::String(s)),
            Value::Okay => Ok(BatchResult::Status("OK".to_string())),
            Value::Array(arr) => {
                let mut inner_results = Vec::with_capacity(arr.len());
                for item in arr {
                    inner_results.push(BatchResult::from_redis_value(item)?);
                }
                Ok(BatchResult::Array(inner_results))
            }
            Value::Map(map) => {
                let mut result_map = Vec::new();
                for (key, value) in map {
                    let key_result = BatchResult::from_redis_value(key)?;
                    let value_result = BatchResult::from_redis_value(value)?;
                    result_map.push((key_result, value_result));
                }
                Ok(BatchResult::Array(
                    result_map.into_iter()
                        .flat_map(|(k, v)| vec![k, v])
                        .collect()
                ))
            }
            Value::Set(set) => {
                let mut results = Vec::with_capacity(set.len());
                for item in set {
                    results.push(BatchResult::from_redis_value(item)?);
                }
                Ok(BatchResult::Array(results))
            }
            Value::Push{ data, .. } => {
                let mut results = Vec::with_capacity(data.len());
                for item in data {
                    results.push(BatchResult::from_redis_value(item)?);
                }
                Ok(BatchResult::Array(results))
            }
            Value::Attribute { data, attributes } => {
                let mut results = Vec::new();
                results.push(BatchResult::from_redis_value(*data)?);
                for (key, value) in attributes {
                    results.push(BatchResult::Raw(key));
                    results.push(BatchResult::from_redis_value(value)?);
                }
                Ok(BatchResult::Array(results))
            }
            _ => Ok(BatchResult::Raw(value))
        }
    }

    /// Convert to a concrete type
    pub fn as_value<T: DeserializeOwned + redis::FromRedisValue>(&self) -> RedissonResult<T> {
        match self {
            BatchResult::String(s) => serde_json::from_str(s)
                .map_err(|e| RedissonError::DeserializationError(e.to_string())),
            BatchResult::Raw(value) => from_redis_value(value.clone())
                .map_err(|e| RedissonError::DeserializationError(e.to_string())),
            _ => Err(RedissonError::DeserializationError(
                format!("This type of result cannot be converted: {:?}", self)
            )),
        }
    }

    /// Convert to string
    pub fn as_string(&self) -> RedissonResult<String> {
        match self {
            BatchResult::String(s) => Ok(s.clone()),
            BatchResult::Status(s) => Ok(s.clone()),
            BatchResult::Raw(Value::SimpleString(s)) => Ok(s.clone()),
            BatchResult::Raw(Value::BulkString(data)) =>
                String::from_utf8(data.clone())
                    .map_err(|e| RedissonError::DeserializationError(e.to_string())),
            _ => Err(RedissonError::DeserializationError(
                "Not a string type".to_string()
            )),
        }
    }

    /// Converting to integer
    pub fn as_i64(&self) -> RedissonResult<i64> {
        match self {
            BatchResult::Integer(i) => Ok(*i),
            BatchResult::Raw(Value::Int(i)) => Ok(*i),
            _ => Err(RedissonError::DeserializationError(
                "Not an integer type".to_string()
            )),
        }
    }

    /// Convert to Boolean
    pub fn as_bool(&self) -> RedissonResult<bool> {
        match self {
            BatchResult::Bool(b) => Ok(*b),
            BatchResult::Integer(1) => Ok(true),
            BatchResult::Integer(0) => Ok(false),
            BatchResult::String(s) => match s.as_str() {
                "1" | "true" | "TRUE" | "yes" | "YES" => Ok(true),
                "0" | "false" | "FALSE" | "no" | "NO" => Ok(false),
                _ => Err(RedissonError::DeserializationError(
                    format!("Unable to convert a string to a Boolean: {}", s)
                )),
            },
            _ => Err(RedissonError::DeserializationError(
                "Not a Boolean type".to_string()
            )),
        }
    }

    /// Check that commands succeed
    pub fn is_success(&self) -> bool {
        match self {
            BatchResult::Status(s) if s == "OK" => true,
            BatchResult::Integer(1) => true, // Usually indicates success.
            BatchResult::Bool(true) => true,
            BatchResult::Nil => true, // For a read command, nil may indicate that the key does not exist
            BatchResult::Raw(Value::Okay) => true,
            BatchResult::Raw(Value::SimpleString(s)) if s == "OK" => true,
            BatchResult::Raw(Value::Int(1)) => true,
            _ => false,
        }
    }
}