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

use async_trait::async_trait;
use lru::LruCache;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time;

use crate::{estimate_size, AsyncCache, CacheEntryStats, CacheStats, CachedValue, RedissonResult};


/// Local cache instance - asynchronous version
pub struct AsyncLocalCache<K, V> {
    cache: Arc<RwLock<LruCache<K, CachedValue<V>>>>,
    pub ttl: Duration,
    stats: Arc<RwLock<CacheStats>>,
    name: String,
}

impl<K: Eq + Hash + Clone, V: Clone + Serialize + DeserializeOwned> AsyncLocalCache<K, V> {
    pub fn new(name: String, ttl: Duration, max_size: usize, stats: Arc<RwLock<CacheStats>>) -> Self {
        let capacity = NonZeroUsize::new(max_size.max(1)).unwrap();
        Self {
            cache: Arc::new(RwLock::new(lru::LruCache::new(capacity))),
            ttl,
            stats,
            name,
        }
    }

    pub async fn len(&self) -> usize {
        self.cache.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.cache.read().await.is_empty()
    }

    pub async fn get_ttl(&self, key: &K) -> Option<Duration> {
        self.cache
            .read().await
            .peek(key)
            .and_then(|cached| cached.remaining_ttl())
    }

    pub async fn get_stats(&self) -> CacheEntryStats {
        let cache = self.cache.read().await;
        let now = Instant::now();

        let mut total_hits = 0u64;
        let mut total_size = 0usize;
        let mut active_count = 0usize;

        for cached in cache.iter() {
            total_hits += cached.1.hits;
            total_size += cached.1.size_bytes;

            if cached.1.expiry > now {
                active_count += 1;
            }
        }

        let len = cache.len();
        CacheEntryStats {
            total_entries: len,
            active_entries: active_count,
            expired_entries: len - active_count,
            total_hits,
            total_size_bytes: total_size,
            avg_hits_per_entry: if len > 0 {
                total_hits as f64 / len as f64
            } else {
                0.0
            },
            name: self.name.clone(),
        }
    }

    pub async fn cleanup(&self) -> usize {
        let mut cache = self.cache.write().await;
        let now = Instant::now();
        let initial_len = cache.len();

        // A temporary vector is used to collect expired keys and avoid modifying the cache in iterations
        let expired_keys: Vec<K> = cache
            .iter()
            .filter(|(_, v)| v.expiry <= now)
            .map(|(k, _)| k.clone())
            .collect();

        let evicted_count = expired_keys.len();

        for key in expired_keys {
            cache.pop(&key);
        }

        if evicted_count > 0 {
            let mut stats = self.stats.write().await;
            stats.record_eviction(evicted_count);
            stats.total_entries = cache.len();
        }

        initial_len - cache.len()
    }

    pub async fn contains(&self, key: &K) -> bool {
        self.cache
            .read().await
            .peek(key)
            .map_or(false, |cached| !cached.is_expired())
    }

    pub async fn get_map(&self, keys: &[K]) -> HashMap<K, V> {
        let mut result = HashMap::with_capacity(keys.len());
        let mut cache = self.cache.write().await;
        let now = Instant::now();

        for key in keys {
            if let Some(cached) = cache.get_mut(key) {
                if cached.expiry > now {
                    cached.hits += 1;
                    result.insert(key.clone(), cached.value.clone());
                } else {
                    cache.pop(key);
                }
            }
        }

        let hit_count = result.len();
        let miss_count = keys.len() - hit_count;

        {
            let mut stats = self.stats.write().await;
            stats.record_hits(hit_count);
            stats.record_misses(miss_count);
        }

        result
    }

    pub async fn set_multi(&self, items: impl IntoIterator<Item = (K, V)>) -> usize {
        let mut cache = self.cache.write().await;
        let mut evicted_count = 0;

        for (key, value) in items {
            let size_bytes = estimate_size(&value);
            let cached_value = CachedValue {
                value,
                expiry: Instant::now() + self.ttl,
                created: Instant::now(),
                hits: 0,
                size_bytes,
                last_accessed: Instant::now(),
                metadata: None,
            };

            if cache.put(key, cached_value).is_some() {
                evicted_count += 1;
            }
        }

        if evicted_count > 0 {
            let mut stats = self.stats.write().await;
            stats.record_eviction(evicted_count);
            stats.total_entries = cache.len();
        }

        evicted_count
    }

    pub async fn set_with_ttl(&self, key: K, value: V, ttl: Duration) {
        let size_bytes = estimate_size(&value);

        let cached_value = CachedValue {
            value,
            expiry: Instant::now() + ttl,
            created: Instant::now(),
            hits: 0,
            size_bytes,
            last_accessed: Instant::now(),
            metadata: None,
        };

        let mut cache = self.cache.write().await;
        let evicted = cache.put(key, cached_value);

        if evicted.is_some() {
            let mut stats = self.stats.write().await;
            stats.record_eviction(1);
        }
    }

    pub async fn clear(&self) -> RedissonResult<()> {
        let mut cache = self.cache.write().await;
        let evicted_count = cache.len();
        cache.clear();

        let mut stats = self.stats.write().await;
        stats.record_eviction(evicted_count);
        stats.total_entries = 0;
        Ok(())
    }

    
    pub async fn refresh_ttl(&self, key: &K) -> bool {
        let mut cache = self.cache.write().await;

        if let Some(cached) = cache.get_mut(key) {
            if cached.expiry > Instant::now() {
                cached.expiry = Instant::now() + self.ttl;
                return true;
            }
        }

        false
    }
}

#[async_trait]
impl<K: Eq + Hash + Clone + Send + Sync, V: Clone + Serialize + DeserializeOwned + Send + Sync> AsyncCache<K, V> for AsyncLocalCache<K, V> {
    async fn get(&self, key: &K) -> RedissonResult<Option<V>> {
        let mut cache = self.cache.write().await;

        match cache.get_mut(key) {
            Some(cached) if !cached.is_expired() => {
                // Cache hit
                cached.hits += 1;
                self.stats.write().await.record_hit();
                Ok(Some(cached.value.clone()))
            }
            Some(_) => {
                // 已过期
                cache.pop(key);
                self.stats.write().await.record_miss();
                Ok(None)
            }
            None => {
                self.stats.write().await.record_miss();
                Ok(None)
            }
        }
    }

    async fn set(&self, key: K, value: V) -> RedissonResult<()> {
        let size_bytes = estimate_size(&value);

        let cached_value = CachedValue {
            value,
            expiry: Instant::now() + self.ttl,
            created: Instant::now(),
            hits: 0,
            size_bytes,
            last_accessed: Instant::now(),
            metadata: None,
        };

        let mut cache = self.cache.write().await;
        if cache.put(key, cached_value).is_some() {
            let mut stats = self.stats.write().await;
            stats.record_eviction(1);
            stats.total_entries = cache.len();
        }
        Ok(())
    }

    async fn remove(&self, key: &K) -> RedissonResult<bool> {
        let mut cache = self.cache.write().await;
        Ok(cache.pop(key).map(|cached| cached.value).is_some())
    }

    async fn clear(&self) -> RedissonResult<()> {
        let mut cache = self.cache.write().await;
        let evicted_count = cache.len();
        cache.clear();

        let mut stats = self.stats.write().await;
        stats.record_eviction(evicted_count);
        stats.total_entries = 0;
        Ok(())
    }

    async fn refresh(&self, key: &K) -> RedissonResult<bool> {
        let mut cache = self.cache.write().await;

        if let Some(cached) = cache.get_mut(key) {
            if cached.expiry > Instant::now() {
                cached.expiry = Instant::now() + self.ttl;
                return Ok(true)
            }
        }

        Ok(false)
    }
}

impl<K: Eq + Hash + Clone, V: Clone + Serialize + DeserializeOwned> Clone for AsyncLocalCacheManager<K, V> {
    fn clone(&self) -> Self {
        Self {
            caches: self.caches.clone(),
            default_ttl: self.default_ttl,
            default_max_size: self.default_max_size,
            stats: self.stats.clone(),
            cleanup_interval: self.cleanup_interval,
        }
    }
}

/// Local cache Manager - Asynchronous version
pub struct AsyncLocalCacheManager<K, V> {
    caches: Arc<RwLock<HashMap<String, Arc<AsyncLocalCache<K, V>>>>>,
    default_ttl: Duration,
    default_max_size: usize,
    stats: Arc<RwLock<CacheStats>>,
    cleanup_interval: Duration,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static, V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static> AsyncLocalCacheManager<K, V> {
    pub fn new(default_ttl: Duration, default_max_size: usize) -> Self {
        let manager = Self {
            caches: Arc::new(RwLock::new(HashMap::new())),
            default_ttl,
            default_max_size,
            stats: Arc::new(RwLock::new(CacheStats::new())),
            cleanup_interval: Duration::from_secs(60),
        };

        // Start a background cleanup task
        manager.start_cleanup_task();

        manager
    }

    pub fn with_cleanup_interval(mut self, interval: Duration) -> Self {
        self.cleanup_interval = interval;
        self
    }

    pub async fn get_or_create_cache(&self, name: &str) -> Arc<AsyncLocalCache<K, V>> {
        {
            let caches = self.caches.read().await;
            if let Some(cache) = caches.get(name) {
                return cache.clone();
            }
        }

        let mut caches = self.caches.write().await;

        // Double checking
        if let Some(cache) = caches.get(name) {
            return cache.clone();
        }

        let cache = Arc::new(AsyncLocalCache::<K, V>::new(
            name.to_string(),
            self.default_ttl,
            self.default_max_size,
            self.stats.clone(),
        ));

        caches.insert(name.to_string(), cache.clone());

        cache
    }

    pub async fn remove_cache(&self, name: &str) -> bool {
        self.caches.write().await.remove(name).is_some()
    }

    pub async fn clear_all(&self) {
        self.caches.write().await.clear();
    }

    pub async fn get_stats(&self) -> CacheStats {
        self.stats.read().await.clone()
    }

    fn start_cleanup_task(&self) {
        let manager = Arc::new(self.clone());

        tokio::spawn(async move {
            let interval = manager.cleanup_interval;

            loop {
                time::sleep(interval).await;
                manager.cleanup_expired().await;
            }
        });
    }

    async fn cleanup_expired(&self) {
        let caches = self.caches.read().await;

        for cache in caches.values() {
            cache.cleanup().await;
        }

        {
            let mut stats = self.stats.write().await;
            stats.last_cleanup = Some(Instant::now());
        }
    }
}