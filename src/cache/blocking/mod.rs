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


pub use local::*;
pub use redis::*;

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use serde::de::DeserializeOwned;
use serde::Serialize;
use crate::{CacheStats, RedissonResult, SyncRedisConnectionManager};

pub trait Cache<K, V> {
    fn get(&self, key: &K) -> RedissonResult<Option<V>>;
    fn set(&self, key: K, value: V) -> RedissonResult<()>;

    fn set_with_ttl(&self, key: K, value: V, ttl: Duration) -> RedissonResult<()>;
    fn remove(&self, key: &K) -> RedissonResult<bool>;
    fn clear(&self) -> RedissonResult<()>;
    fn refresh(&self, key: &K) -> RedissonResult<bool>;
}

/// Cache factory
pub struct CacheFactory {
    connection_manager: Arc<SyncRedisConnectionManager>,
    local_cache_manager: Arc<LocalCacheManager<String, serde_json::Value>>,
}

impl CacheFactory {
    pub fn new(connection_manager: Arc<SyncRedisConnectionManager>) -> Self {
        Self::with_defaults(connection_manager, Duration::from_secs(300), 1000)
    }

    pub fn with_defaults(
        connection_manager: Arc<SyncRedisConnectionManager>,
        default_ttl: Duration,
        default_max_size: usize,
    ) -> Self {
        let local_cache_manager = Arc::new(LocalCacheManager::new(
            default_ttl,
            default_max_size,
        ));

        Self {
            connection_manager,
            local_cache_manager,
        }
    }

    pub fn create_cache<K, V>(
        &self,
        cache_name: &str,
        ttl: Duration,
        max_size: usize,
    ) -> Arc<dyn Cache<K, V>>
    where
        K: Eq + Hash + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let cache = RedisIntegratedCache::new(
            self.connection_manager.clone(),
            cache_name,
            ttl,
            max_size,
        );

        Arc::new(cache)
    }

    pub fn create_cache_with_options<K, V>(
        &self,
        cache_name: &str,
        ttl: Duration,
        max_size: usize,
        read_through: bool,
        write_through: bool,
        use_local_cache: bool,
    ) -> Arc<dyn Cache<K, V>>
    where
        K: Eq + Hash + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let cache = RedisIntegratedCache::new(
            self.connection_manager.clone(),
            cache_name,
            ttl,
            max_size,
        )
            .with_read_through(read_through)
            .with_write_through(write_through)
            .with_local_cache(use_local_cache);

        Arc::new(cache)
    }

    pub fn get_local_cache(&self, name: &str) -> Arc<LocalCache<String, serde_json::Value>> {
        self.local_cache_manager.get_or_create_cache(name)
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.local_cache_manager.get_stats()
    }
}