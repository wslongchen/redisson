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
mod local;
mod redis;

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
pub use local::*;
pub use redis::*;
use crate::{AsyncRedisConnectionManager, RedissonResult};

#[async_trait]
pub trait AsyncCache<K, V> {
    async fn get(&self, key: &K) -> RedissonResult<Option<V>>;
    async fn set(&self, key: K, value: V) -> RedissonResult<()>;
    async fn remove(&self, key: &K) -> RedissonResult<bool>;
    async fn clear(&self) -> RedissonResult<()>;
    async fn refresh(&self, key: &K) -> RedissonResult<bool>;
}





/// Asynchronous cache factory
pub struct AsyncCacheFactory {
    connection_manager: Arc<AsyncRedisConnectionManager>,
    local_cache_manager: Arc<AsyncLocalCacheManager<String, serde_json::Value>>,
}

impl AsyncCacheFactory {
    pub async fn new(connection_manager: Arc<AsyncRedisConnectionManager>) -> Self {
        let local_cache_manager = Arc::new(AsyncLocalCacheManager::new(
            Duration::from_secs(300), // 默认5分钟TTL
            1000, // 默认最大1000个条目
        ));

        Self {
            connection_manager,
            local_cache_manager,
        }
    }

    pub async fn create_cache<K, V>(
        &self,
        cache_name: &str,
        ttl: Duration,
        max_size: usize,
    ) -> Arc<dyn AsyncCache<K, V>>
    where
        K: Eq + Hash + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let cache = AsyncRedisIntegratedCache::new(
            self.connection_manager.clone(),
            cache_name,
            ttl,
            max_size,
        );

        Arc::new(cache)
    }

    pub async fn get_local_cache(&self, name: &str) -> Arc<AsyncLocalCache<String, serde_json::Value>> {
        self.local_cache_manager.get_or_create_cache(name).await
    }
}