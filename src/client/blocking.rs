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


use crate::client::stats::ClientStats;
use crate::{BatchProcessor, RAtomicLong, RBatch, RBitSet, RBlockingQueue, RBloomFilter, RBucket, RCountDownLatch, RDelayedQueue, RFairLock, RGeo, RKeys, RList, RLock, RMap, RMultiLock, RRateLimiter, RReadWriteLock, RRedLock, RSemaphore, RSet, RSortedSet, RStream, RTopic, RedissonConfig, RedissonResult, SyncRedisConnectionManager, SyncTransactionBuilder, SyncTransactionContext};
#[cfg(feature = "caching")]
use crate::{LocalCacheManager, RedisIntegratedCache};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

// Synchronous client
pub struct RedissonClient {
    config: RedissonConfig,
    connection_manager: Arc<SyncRedisConnectionManager>,
    batch_processor: Arc<BatchProcessor>,
    #[cfg(feature = "caching")]
    cache_manager: Arc<LocalCacheManager<String, String>>,
}

impl RedissonClient {
    pub fn new(config: RedissonConfig) -> RedissonResult<Self> {
        // Create a connection manager
        let connection_manager = Arc::new(SyncRedisConnectionManager::new(&config)?);

        // Create the batch optimizer
        let batch_optimizer = BatchProcessor::new(connection_manager.clone(), config.batch_config.clone().unwrap_or_default())?;

        Ok(Self {
            config,
            connection_manager,
            batch_processor: Arc::new(batch_optimizer),
            #[cfg(feature = "caching")]
            cache_manager: Arc::new(LocalCacheManager::new(
                Duration::from_secs(300),
                1000,
            )),
        })
    }
    
    pub fn get_connection_manager(&self) -> &Arc<SyncRedisConnectionManager> {
        &self.connection_manager
    }

    pub fn get_batch_processor(&self) -> &Arc<BatchProcessor> {
        &self.batch_processor
    }
    
    // Distributed data structure acquisition
    pub fn get_bucket<V>(&self, name: &str) -> RBucket<V>
    where
        V: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        RBucket::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_map<K, V>(&self, name: &str) -> RMap<K, V>
    where
        K: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + Eq + std::hash::Hash + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        RMap::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_list<V: for<'de> DeserializeOwned + Serialize + Send + Sync + 'static>(&self, name: &str) -> RList<V> {
        RList::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_set<V: serde::Serialize + serde::de::DeserializeOwned + Eq + std::hash::Hash>(&self, name: &str) -> RSet<V> {
        RSet::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_sorted_set<V: serde::Serialize + serde::de::DeserializeOwned + Ord>(&self, name: &str) -> RSortedSet<V> {
        RSortedSet::new(self.connection_manager.clone(), name.to_string())
    }

    // Advanced data structures
    pub fn get_atomic_long(&self, name: &str) -> RAtomicLong {
        RAtomicLong::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_semaphore(&self, name: &str, max_permits: usize) -> RSemaphore {
        RSemaphore::new(self.connection_manager.clone(), name.to_string(), max_permits)
    }

    pub fn get_rate_limiter(&self, name: &str, rate: f64, capacity: f64) -> RRateLimiter {
        RRateLimiter::new(self.connection_manager.clone(), name.to_string(), rate, capacity)
    }
    pub fn get_bloom_filter<V: AsRef<[u8]>>(&self, name: &str, expected_insertions: usize, false_positive_rate: f64) -> RBloomFilter<V> {
        RBloomFilter::new(self.connection_manager.clone(), name.to_string(), expected_insertions, false_positive_rate)
    }

    pub fn get_count_down_latch(&self, name: &str, count: i32) -> RCountDownLatch {
        RCountDownLatch::new(self.connection_manager.clone(), name.to_string(), count)
    }

    pub fn get_bit_set(&self, name: &str) -> RBitSet {
        RBitSet::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_geo<V>(&self, name: &str) -> RGeo<V>
    where
        V: Serialize + DeserializeOwned + std::default::Default,
    {
        RGeo::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_topic<V>(&self, name: &str) -> RTopic<V> 
    where V: Serialize + DeserializeOwned + Send + Sync + 'static + Clone {
        RTopic::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_blocking_queue<V>(&self, name: &str) -> RBlockingQueue<V>
    where
        V: serde::Serialize + serde::de::DeserializeOwned + 'static,
    {
        RBlockingQueue::new(self.connection_manager.clone(), name.to_string())
    }
    
    
    pub fn get_delayed_queue<V>(&self, name: &str) -> RDelayedQueue<V>
    where
        V: serde::Serialize + serde::de::DeserializeOwned + 'static,
    {
        RDelayedQueue::new(self.connection_manager.clone(), name.to_string())
    }
    
    
    // Distributed lock acquisition
    pub fn get_lock(&self, name: &str) -> RLock {
        RLock::new(self.connection_manager.clone(), name.to_string(), self.config.watchdog_timeout)
    }

    pub fn get_fair_lock(&self, name: &str) -> RFairLock {
        RFairLock::new(self.connection_manager.clone(), name.to_string(), self.config.watchdog_timeout)
    }

    pub fn get_multi_lock(&self, names: Vec<String>) -> RMultiLock {
        let locks = names.iter()
            .map(|name| self.get_lock(name))
            .collect();
        RMultiLock::new(locks)
    }

    pub fn get_red_lock(&self, names: String) -> RRedLock {
        RRedLock::new(vec![self.connection_manager.clone()], names, self.config.watchdog_timeout)
    }

    pub fn get_read_write_lock(&self, name: &str, lease_time: Duration) -> RReadWriteLock {
        RReadWriteLock::new(self.connection_manager.clone(), name.to_string(), lease_time)
    }
    
    pub fn execute_transaction(&self, transaction_func: impl FnOnce(&mut SyncTransactionContext) -> RedissonResult<()>) -> RedissonResult<()> {
        let mut context = SyncTransactionBuilder::new(self.connection_manager.clone()).build();
        transaction_func(&mut context)?;
        Ok(())
    }
    
    // Adding Stream support
    pub fn get_stream<V>(&self, name: &str) -> RStream<V>
    where
        V: serde::Serialize + serde::de::DeserializeOwned + Clone + 'static,
    {
        RStream::new(self.connection_manager.clone(), name.to_string())
    }

    // Adding batch operations
    pub fn create_batch(&self) -> RBatch {
        RBatch::new(self.batch_processor.clone())
    }
    // Utility methods
    pub fn get_keys(&self) -> RKeys {
        RKeys::new(self.connection_manager.clone())
    }


    // Adding caching support
    #[cfg(feature = "caching")]
    pub fn get_cache<K, V>(&self, name: &str) -> RedisIntegratedCache<K, V>
    where
        K: std::cmp::Eq + std::hash::Hash + Clone + serde::Serialize + serde::de::DeserializeOwned + 'static + std::fmt::Debug,
        V: Clone + serde::Serialize + serde::de::DeserializeOwned + 'static,
    {
        RedisIntegratedCache::new(
            self.connection_manager.clone(),
            name,
            Duration::from_secs(300),
            1000,
        )
    }


    // 获取统计信息
    pub fn get_stats(&self) -> ClientStats {
        ClientStats {
            connection_stats: self.connection_manager.get_stats(),
            batch_stats: self.batch_processor.get_stats(),
            #[cfg(feature = "caching")]
            cache_stats: self.cache_manager.get_stats(),
        }
    }

    pub fn shutdown(&self) -> RedissonResult<()> {
        // Stop the batch optimizer background thread
        self.batch_processor.close()?;
        Ok(())
    }
}


impl Clone for RedissonClient {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            connection_manager: self.connection_manager.clone(),
            batch_processor: self.batch_processor.clone(),
            #[cfg(feature = "caching")]
            cache_manager: self.cache_manager.clone(),
        }
    }
}
