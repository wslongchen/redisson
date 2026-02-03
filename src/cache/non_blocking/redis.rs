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
use crate::{AsyncCache, AsyncLocalCache, AsyncRedisConnectionManager, CacheStats, RedissonError, RedissonResult};
use async_trait::async_trait;
use redis::AsyncCommands;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

/// Caching integrated with Redis - asynchronous version
pub struct AsyncRedisIntegratedCache<K, V> {
    connection_manager: Arc<AsyncRedisConnectionManager>,
    local_cache: Arc<AsyncLocalCache<K, V>>,
    cache_key_prefix: String,
    read_through: bool,
    write_through: bool,
    use_local_cache: bool,
}


impl<K: Eq + Hash + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
    V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static> AsyncRedisIntegratedCache<K, V> {

    pub fn new(
        connection_manager: Arc<AsyncRedisConnectionManager>,
        cache_name: &str,
        ttl: Duration,
        max_size: usize,
    ) -> Self {
        let stats = Arc::new(tokio::sync::RwLock::new(CacheStats::new()));

        let local_cache = AsyncLocalCache::new(
            cache_name.to_string(),
            ttl,
            max_size,
            stats,
        );

        Self {
            connection_manager,
            local_cache: Arc::new(local_cache),
            cache_key_prefix: format!("cache:{}", cache_name),
            read_through: true,
            write_through: true,
            use_local_cache: true,
        }
    }

    #[inline]
    pub fn with_read_through(mut self, enabled: bool) -> Self {
        self.read_through = enabled;
        self
    }

    #[inline]
    pub fn with_write_through(mut self, enabled: bool) -> Self {
        self.write_through = enabled;
        self
    }

    #[inline]
    pub fn with_local_cache(mut self, enabled: bool) -> Self {
        self.use_local_cache = enabled;
        self
    }


    fn build_redis_key(&self, key: &K) -> RedissonResult<String> {
        let key_json = serde_json::to_string(key)
            .map_err(|e| crate::errors::RedissonError::SerializationError(e.to_string()))?;

        Ok(format!("{}:{}", self.cache_key_prefix, key_json))
    }


    #[inline]
    pub fn get_local_cache(&self) -> &Arc<AsyncLocalCache<K, V>> {
        &self.local_cache
    }

    // Batch acquisition
    pub async fn get_map(&self, keys: &[K]) -> RedissonResult<HashMap<K, V>> {
        let mut result = HashMap::with_capacity(keys.len());
        let mut missing_keys = Vec::new();

        // 1. Fetched from the local cache
        if self.use_local_cache {
            let local_results = self.local_cache.get_map(keys).await;

            for (key, value) in local_results {
                result.insert(key, value);
            }

            // Collect the missed keys
            for key in keys {
                if !result.contains_key(key) {
                    missing_keys.push(key.clone());
                }
            }
        } else {
            missing_keys.extend_from_slice(keys);
        }

        // 2. Retrieve the missing key from Redis
        if self.read_through && !missing_keys.is_empty() {
            let mut conn = self.connection_manager.get_connection().await?;
            let mut pipe = redis::pipe();

            for key in &missing_keys {
                let redis_key = self.build_redis_key(key)?;
                pipe.get(&redis_key);
            }

            let redis_results: Vec<Option<String>> = pipe.query_async(&mut conn).await?;

            for (i, redis_result) in redis_results.into_iter().enumerate() {
                if let Some(json) = redis_result {
                    if let Ok(value) = serde_json::from_str::<V>(&json) {
                        let key = missing_keys[i].clone();

                        // Updating the local cache
                        if self.use_local_cache {
                            let _ = self.local_cache.set(key.clone(), value.clone());
                        }

                        result.insert(key, value);
                    }
                }
            }
        }

        Ok(result)
    }


    // Batch setup
    pub async fn set_multi(&self, items: impl IntoIterator<Item = (K, V)>) -> RedissonResult<()> {
        let items: Vec<(K, V)> = items.into_iter().collect();

        // 1. Updating the local cache
        if self.use_local_cache {
            self.local_cache.set_multi(items.iter().cloned()).await;
        }

        // 2. Updating Redis
        if self.write_through && !items.is_empty() {
            let mut conn = self.connection_manager.get_connection().await?;
            let mut pipe = redis::pipe();

            for (key, value) in items {
                let redis_key = self.build_redis_key(&key)?;
                let value_json = serde_json::to_string(&value)
                    .map_err(|e| crate::errors::RedissonError::SerializationError(e.to_string()))?;

                let ttl_secs = self.local_cache.ttl.as_secs();
                pipe.set_ex(&redis_key, value_json, ttl_secs);
            }

            pipe.query_async::<()>(&mut conn).await?;
        }

        Ok(())
    }

    // Check the Redis connection
    pub async fn check_redis_connection(&self) -> RedissonResult<bool> {
        match self.connection_manager.get_connection().await {
            Ok(mut conn) => {
                let result: Result<String, _> = redis::cmd("PING").query_async(&mut conn).await;
                Ok(result.is_ok())
            }
            Err(_) => Ok(false),
        }
    }
    
}


#[async_trait]
impl<K, V> AsyncCache<K, V> for AsyncRedisIntegratedCache<K, V>
where
    K: Eq + Hash + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
    V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    async fn get(&self, key: &K) -> RedissonResult<Option<V>> {
        // 1. Try local caching
        if self.use_local_cache {
            if let Some(value) = self.local_cache.get(key).await? {
                return Ok(Some(value));
            }
        }

        if !self.read_through {
            return Ok(None);
        }

        // 2. Read from Redis
        let redis_key = self.build_redis_key(key)?;
        let mut conn = self.connection_manager.get_connection().await?;

        let value_json: Option<String> = conn.get(&redis_key).await?;

        if let Some(json) = value_json {
            let value: V = serde_json::from_str(&json)
                .map_err(|e| crate::errors::RedissonError::DeserializationError(e.to_string()))?;

            // 3. Updating the local cache
            if self.use_local_cache {
                self.local_cache.set(key.clone(), value.clone()).await?;
            }

            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    async fn set(&self, key: K, value: V) -> RedissonResult<()> {
        // 1. Updating the local cache
        if self.use_local_cache {
            self.local_cache.set(key.clone(), value.clone()).await?;
        }

        if !self.write_through {
            return Ok(());
        }

        // 2. Updating Redis
        let redis_key = self.build_redis_key(&key)?;
        let value_json = serde_json::to_string(&value)
            .map_err(|e| RedissonError::SerializationError(e.to_string()))?;

        let mut conn = self.connection_manager.get_connection().await?;

        // Set an expiration time (using a locally cached TTL)
        let ttl_secs = self.local_cache.ttl.as_secs() as i64;

        redis::pipe()
            .atomic()
            .set(&redis_key, &value_json)
            .expire(&redis_key, ttl_secs)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| RedissonError::RedisError(e))?;

        Ok(())
    }

    async fn remove(&self, key: &K) -> RedissonResult<bool> {
        // 1. Clearing the local cache
        if self.use_local_cache {
            self.local_cache.remove(key).await?;
        }

        if !self.write_through {
            return Ok(true);
        }

        // 2. Clear Redis
        let redis_key = self.build_redis_key(key)?;
        let mut conn = self.connection_manager.get_connection().await?;

        let deleted: i32 = conn.del(&redis_key).await?;
        Ok(deleted > 0)
    }

    async fn clear(&self) -> RedissonResult<()> {
        // 1. Clearing the local cache
        if self.use_local_cache {
            self.local_cache.clear().await?;
        }

        if !self.write_through {
            return Ok(());
        }

        // 2. Clear all related keys from Redis
        let pattern = format!("{}:*", self.cache_key_prefix);
        let mut conn = self.connection_manager.get_connection().await?;

        let keys: Vec<String> = conn.keys(&pattern).await?;

        if !keys.is_empty() {
            conn.del::<_, ()>(keys).await?;
        }

        Ok(())
    }

    async fn refresh(&self, key: &K) -> RedissonResult<bool> {
        // Flush from Redis to the local cache
        let redis_key = self.build_redis_key(key)?;
        let mut conn = self.connection_manager.get_connection().await?;

        let value_json: Option<String> = conn.get(&redis_key).await?;

        if let Some(json) = value_json {
            let value: V = serde_json::from_str(&json)
                .map_err(|e| RedissonError::DeserializationError(e.to_string()))?;

            if self.use_local_cache {
                self.local_cache.set(key.clone(), value).await?;
            }
            Ok(true)
        } else {
            if self.use_local_cache {
                self.local_cache.remove(key).await?;
            }
            Ok(false)
        }
    }
}
