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
use crate::{AsyncBatchProcessor, AsyncRAtomicLong, AsyncRBatch, AsyncRBitSet, AsyncRBlockingQueue, AsyncRBloomFilter, AsyncRBucket, AsyncRCountDownLatch, AsyncRFairLock, AsyncRGeo, AsyncRKeys, AsyncRList, AsyncRLock, AsyncRMap, AsyncRMultiLock, AsyncRRateLimiter, AsyncRReadWriteLock, AsyncRRedLock, AsyncRSemaphore, AsyncRSet, AsyncRSortedSet, AsyncRStream, AsyncRTopic, AsyncRedisConnectionManager, AsyncTransactionBuilder, AsyncTransactionContext, RedissonConfig, RedissonResult};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

// Asynchronous client
pub struct AsyncRedissonClient {
    config: RedissonConfig,
    connection_manager: Arc<AsyncRedisConnectionManager>,
    batch_processor: Arc<AsyncBatchProcessor>,
    #[cfg(feature = "caching")]
    cache_manager: Arc<crate::AsyncLocalCacheManager<String, String>>,
}

impl AsyncRedissonClient {
    pub async fn new(config: RedissonConfig) -> RedissonResult<Self> {
        // Create a connection manager
        let connection_manager = Arc::new(AsyncRedisConnectionManager::new(&config).await?);

        // Create the batch optimizer
        let batch_optimizer = AsyncBatchProcessor::new(connection_manager.clone(), config.batch_config.clone().unwrap_or_default()).await?;
        Ok(Self {
            config,
            connection_manager,
            batch_processor: Arc::new(batch_optimizer),
            #[cfg(feature = "caching")]
            cache_manager: Arc::new(crate::AsyncLocalCacheManager::new(
                Duration::from_secs(300),
                1000,
            )),
        })
    }

    pub fn get_batch_processor(&self) -> &Arc<AsyncBatchProcessor> {
        &self.batch_processor
    }

    // Distributed data structure acquisition
    pub fn get_bucket<V>(&self, name: &str) -> AsyncRBucket<V>
    where
        V: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        AsyncRBucket::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_map<K, V>(&self, name: &str) -> AsyncRMap<K, V>
    where
        K: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + Eq + std::hash::Hash + 'static,
        V: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        AsyncRMap::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_list<V: for<'de> DeserializeOwned + Serialize + Send + Sync + 'static>(&self, name: &str) -> AsyncRList<V> {
        AsyncRList::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_set<V: Serialize + DeserializeOwned + Eq + std::hash::Hash + Send + Sync + 'static>(&self, name: &str) -> AsyncRSet<V> {
        AsyncRSet::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_sorted_set<V: Serialize + DeserializeOwned + Send + Sync + 'static>(&self, name: &str) -> AsyncRSortedSet<V> {
        AsyncRSortedSet::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_bloom_filter<V: AsRef<[u8]> + Send + Sync + 'static>(&self, name: &str, expected_insertions: usize, false_positive_rate: f64) -> AsyncRBloomFilter<V> {
        AsyncRBloomFilter::new(self.connection_manager.clone(), name.to_string(), expected_insertions, false_positive_rate)
    }

    // Advanced data structures
    pub fn get_atomic_long(&self, name: &str) -> AsyncRAtomicLong {
        AsyncRAtomicLong::new(self.connection_manager.clone(), name.to_string())
    }

    pub async fn get_semaphore(&self, name: &str, max_permits: usize) -> AsyncRSemaphore {
        AsyncRSemaphore::new(self.connection_manager.clone(), name.to_string(), max_permits).await
    }

    pub fn get_rate_limiter(&self, name: &str, rate: f64, capacity: f64) -> AsyncRRateLimiter {
        AsyncRRateLimiter::new(self.connection_manager.clone(), name.to_string(), rate, capacity)
    }

    pub async fn get_count_down_latch(&self, name: &str, count: i32) -> AsyncRCountDownLatch {
        AsyncRCountDownLatch::new(self.connection_manager.clone(), name.to_string(), count).await
    }

    pub fn get_bit_set(&self, name: &str) -> AsyncRBitSet {
        AsyncRBitSet::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_geo<V>(&self, name: &str) -> AsyncRGeo<V>
    where
        V: Serialize + DeserializeOwned + Send + Sync + Default + 'static,
    {
        AsyncRGeo::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_topic<V>(&self, name: &str) -> AsyncRTopic<V>
    where V: Serialize + DeserializeOwned + Send + Sync + 'static + Clone {
        AsyncRTopic::new(self.connection_manager.clone(), name.to_string())
    }

    pub fn get_blocking_queue<V>(&self, name: &str) -> AsyncRBlockingQueue<V>
    where
        V: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        AsyncRBlockingQueue::new(self.connection_manager.clone(), name.to_string())
    }

    // Distributed lock acquisition
    pub fn get_lock(&self, name: &str) -> AsyncRLock {
        AsyncRLock::new(self.connection_manager.clone(), name.to_string(), self.config.watchdog_timeout)
    }

    pub fn get_fair_lock(&self, name: &str) -> AsyncRFairLock {
        AsyncRFairLock::new(self.connection_manager.clone(), name.to_string(), self.config.watchdog_timeout)
    }

    pub fn get_multi_lock(&self, names: Vec<String>) -> AsyncRMultiLock {
        let locks = names.iter()
            .map(|name| self.get_lock(name))
            .collect();
        AsyncRMultiLock::new(locks)
    }

    pub fn get_red_lock(&self, names: String) -> AsyncRRedLock {
        AsyncRRedLock::new(vec![self.connection_manager.clone()], names, self.config.watchdog_timeout)
    }

    pub fn get_read_write_lock(&self, name: &str, lease_time: Duration) -> AsyncRReadWriteLock {
        AsyncRReadWriteLock::new(self.connection_manager.clone(), name.to_string(), lease_time)
    }

    pub async fn execute_transaction(&self, transaction_func: impl FnOnce(&mut AsyncTransactionContext) -> RedissonResult<()>) -> RedissonResult<()> {
        let mut context = AsyncTransactionBuilder::new(self.connection_manager.clone()).build().await;
        transaction_func(&mut context)?;
        Ok(())
    }

    // Utility methods
    pub fn get_keys(&self) -> AsyncRKeys {
        AsyncRKeys::new(self.connection_manager.clone())
    }

    // Adding Stream support
    pub fn get_stream<V>(&self, name: &str) -> AsyncRStream<V>
    where
        V: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    {
        AsyncRStream::new(self.connection_manager.clone(), name.to_string())
    }

    // Adding batch operations
    pub fn create_batch(&self) -> AsyncRBatch {
        AsyncRBatch::new(self.batch_processor.clone())
    }

    // Adding caching support
    #[cfg(feature = "caching")]
    pub fn get_cache<K, V>(&self, name: &str) -> crate::AsyncRedisIntegratedCache<K, V>
    where
        K: Eq + Hash + Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static,
        V: Clone + Serialize + DeserializeOwned + Send + Sync + 'static
    {
        crate::AsyncRedisIntegratedCache::<K, V>::new(
            self.connection_manager.clone(),
            name,
            Duration::from_secs(300),
            1000,
        )
    }


    // Getting statistics
    pub async fn get_stats(&self) -> ClientStats {
        ClientStats {
            connection_stats: self.connection_manager.get_stats().await,
            batch_stats: self.batch_processor.get_stats().await,
            #[cfg(feature = "caching")]
            cache_stats: self.cache_manager.get_stats().await,
        }
    }

    pub async fn shutdown(&self) -> RedissonResult<()> {
        // Stop the batch optimizer background thread
        self.batch_processor.close().await?;
        Ok(())
    }
}


impl Clone for AsyncRedissonClient {
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
