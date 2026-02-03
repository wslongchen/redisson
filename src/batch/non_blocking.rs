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

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use tokio::sync::{Mutex, RwLock, Notify, Semaphore};
use tokio::time;
use redis::{Value};
use lru::LruCache;
use std::num::NonZeroUsize;
use tracing::{info, error};

use crate::errors::RedissonError;
use crate::connection::AsyncRedisConnectionManager;
use crate::{BatchConfig, BatchGroup, BatchPriority, BatchResult, BatchStats, CommandBuilder, RedissonResult};
// ================ Asynchronous batch processors ================
/// Asynchronous batch processors
pub struct AsyncBatchProcessor {
    // Connection management
    connection_manager: Arc<AsyncRedisConnectionManager>,

    // CONFIGURATION
    config: BatchConfig,

    // Pending queue
    pending_batches: Arc<Mutex<VecDeque<BatchGroup>>>,

    // Statistical information
    stats: Arc<RwLock<BatchStats>>,

    // caching
    #[cfg(feature = "caching")]
    cache: Option<Arc<RwLock<LruCache<String, crate::CachedValue<BatchResult>>>>>,

    // Close flag
    is_closed: tokio::sync::watch::Sender<bool>,

    // Refresh notifications
    flush_notify: Arc<Notify>,

    // Concurrency control
    concurrent_semaphore: Arc<Semaphore>,

    // Refresh task handles in the background
    flusher_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl AsyncBatchProcessor {
    /// Create a new asynchronous batch processor
    pub async fn new(
        connection_manager: Arc<AsyncRedisConnectionManager>,
        config: BatchConfig,
    ) -> RedissonResult<Self> {
        let (tx, _) = tokio::sync::watch::channel(false);

        let processor = Self {
            connection_manager,
            config: config.clone(),
            pending_batches: Arc::new(Mutex::new(VecDeque::new())),
            stats: Arc::new(RwLock::new(BatchStats::new())),
            #[cfg(feature = "caching")]
            cache: None,
            is_closed: tx,
            flush_notify: Arc::new(Notify::new()),
            concurrent_semaphore: Arc::new(Semaphore::new(config.max_concurrent_batches)),
            flusher_handle: Arc::new(Mutex::new(None)),
        };

        let mut processor = processor;

        // Initializing the cache
        #[cfg(feature = "caching")]
        if processor.config.enable_cache {
            processor.cache = Some(Arc::new(RwLock::new(LruCache::new(
                NonZeroUsize::new(processor.config.cache_size).unwrap()
            ))));
        }

        // Start the background refresh task
        processor.start_background_flusher().await?;

        Ok(processor)
    }

    pub fn get_batch_config(&self) -> BatchConfig {
        self.config.clone()
    }
    
    /// Perform batch processing asynchronously (without returning results)
    pub async fn exec_batch(&self, commands: Vec<Box<dyn CommandBuilder>>) -> RedissonResult<()> {
        self.execute_batch_internal(commands, false).await.map(|_| ())
    }

    /// Asynchronous query batching (returning results)
    pub async fn query_batch(&self, commands: Vec<Box<dyn CommandBuilder>>) -> RedissonResult<Vec<BatchResult>> {
        self.execute_batch_internal(commands, true).await
    }

    /// Internal execution method
    async fn execute_batch_internal(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        needs_result: bool,
    ) -> RedissonResult<Vec<BatchResult>> {
        // Check if it is closed
        if *self.is_closed.borrow() {
            return Err(RedissonError::PoolError("The asynchronous batch processor has been turned off".to_string()));
        }

        let start = Instant::now();

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.total_batches += 1;
            stats.total_commands += commands.len() as u64;
        }

        // Checking the cache
        #[cfg(feature = "caching")]
        if self.config.enable_cache && needs_result && self.is_read_only_batch(&commands).await {
            if let Some(cached_results) = self.get_cached_results(&commands).await {
                let mut stats = self.stats.write().await;
                stats.cache_hits += 1;
                self.record_stats(start, commands.len(), true, true).await;
                return Ok(cached_results);
            }
            let mut stats = self.stats.write().await;
            stats.cache_misses += 1;
        }

        // Obtaining concurrency permissions
        let _permit = self.concurrent_semaphore.acquire().await
            .map_err(|e| RedissonError::PoolError(format!("Failed to obtain concurrent permission: {}", e)))?;

        // Updating concurrency statistics
        {
            let mut stats = self.stats.write().await;
            stats.concurrent_batches += 1;
        }

        // Performing batch processing
        let result = if self.config.enable_pipeline && commands.len() <= self.config.max_batch_size {
            if needs_result {
                self.query_with_pipeline(&commands).await
            } else {
                self.exec_with_pipeline(&commands).await.map(|_| Vec::new())
            }
        } else {
            if needs_result {
                self.query_in_chunks(&commands).await
            } else {
                self.exec_in_chunks(&commands).await.map(|_| Vec::new())
            }
        };

        // Updating concurrency statistics
        {
            let mut stats = self.stats.write().await;
            stats.concurrent_batches -= 1;
        }

        // Record statistics
        let is_success = result.is_ok();
        self.record_stats(start, commands.len(), is_success, false).await;

        // Updating the cache
        #[cfg(feature = "caching")]
        if self.config.enable_cache && needs_result && is_success {
            if let Ok(results) = &result {
                self.update_cache(&commands, results).await;
            }
        }

        result
    }

    /// Pipelining (without returning a result)
    async fn exec_with_pipeline(&self, commands: &[Box<dyn CommandBuilder>]) -> RedissonResult<()> {
        let mut conn = self.connection_manager.get_connection().await?;

        let mut pipeline = redis::Pipeline::new();
        for cmd in commands {
            let redis_cmd = cmd.build();
            pipeline.add_command(redis_cmd);
        }

        let results: Vec<Value> = pipeline.query_async(&mut conn).await
            .map_err(RedissonError::RedisError)?;

        // Checking for errors
        for result in results {
            if let Err(err) = result.extract_error() {
                return Err(RedissonError::RedisError(err));
            }
        }

        Ok(())
    }

    /// Query with pipe (returns results)
    async fn query_with_pipeline(&self, commands: &[Box<dyn CommandBuilder>]) -> RedissonResult<Vec<BatchResult>> {
        let mut conn = self.connection_manager.get_connection().await?;

        let mut pipeline = redis::Pipeline::new();
        for cmd in commands {
            let redis_cmd = cmd.build();
            pipeline.add_command(redis_cmd);
        }

        let results: Vec<Value> = pipeline.query_async(&mut conn).await
            .map_err(RedissonError::RedisError)?;

        self.convert_results(results).await
    }

    /// Chunked execution (no result returned)
    async fn exec_in_chunks(&self, commands: &[Box<dyn CommandBuilder>]) -> RedissonResult<()> {
        let mut conn = self.connection_manager.get_connection().await?;

        for chunk in commands.chunks(self.config.max_batch_size) {
            let mut pipeline = redis::Pipeline::new();

            for cmd in chunk {
                let redis_cmd = cmd.build();
                pipeline.add_command(redis_cmd);
            }

            let results: Vec<Value> = pipeline.query_async(&mut conn).await
                .map_err(RedissonError::RedisError)?;

            // Checking for errors
            for result in results {
                if let Err(err) = result.extract_error() {
                    return Err(RedissonError::RedisError(err));
                }
                
            }
        }
        Ok(())
    }


    /// Chunk query (return results)
    async fn query_in_chunks(&self, commands: &[Box<dyn CommandBuilder>]) -> RedissonResult<Vec<BatchResult>> {
        let mut conn = self.connection_manager.get_connection().await?;
        let mut all_results = Vec::new();

        for chunk in commands.chunks(self.config.max_batch_size) {
            let mut pipeline = redis::Pipeline::new();

            for cmd in chunk {
                let redis_cmd = cmd.build();
                pipeline.add_command(redis_cmd);
            }

            let results: Vec<Value> = pipeline.query_async(&mut conn).await
                .map_err(RedissonError::RedisError)?;

            let converted = self.convert_results(results).await?;
            all_results.extend(converted);
        }

        Ok(all_results)
    }

    /// Conversion result
    async fn convert_results(&self, values: Vec<Value>) -> RedissonResult<Vec<BatchResult>> {
        let mut results = Vec::with_capacity(values.len());

        for value in values {
            match BatchResult::from_redis_value(value) {
                Ok(result) => results.push(result),
                Err(e) => results.push(BatchResult::Error(e.to_string())),
            }
        }

        Ok(results)
    }

    /// Determines whether the batch is read-only
    async fn is_read_only_batch(&self, commands: &[Box<dyn CommandBuilder>]) -> bool {
        commands.iter().all(|cmd| cmd.needs_result())
    }

    /// Get the result from the cache
    #[cfg(feature = "caching")]
    async fn get_cached_results(&self, commands: &[Box<dyn CommandBuilder>]) -> Option<Vec<BatchResult>> {
        if let Some(cache) = &self.cache {
            let mut cache = cache.write().await;
            let mut cache_key_parts = Vec::new();

            for cmd in commands {
                let keys = cmd.keys();
                if keys.is_empty() {
                    return None;
                }
                cache_key_parts.extend(keys);
            }

            // Generate cache keys
            let cache_key = cache_key_parts.join("|");

            if let Some(cached_value) = cache.get(&cache_key) {
                if !cached_value.is_expired() {
                    // To simplify things, we should actually store the serialization of the entire batch
                    return None;
                }
            }

            None
        } else {
            None
        }
    }

    /// Updating the cache
    #[cfg(feature = "caching")]
    async fn update_cache(&self, commands: &[Box<dyn CommandBuilder>], results: &[BatchResult]) {
        if let Some(cache) = &self.cache {
            let mut cache = cache.write().await;
            let now = Instant::now();
            let expires_at = now + self.config.cache_ttl;

            // Generate cache keys
            let mut cache_key_parts = Vec::new();
            for cmd in commands {
                cache_key_parts.extend(cmd.keys());
            }
            let cache_key = cache_key_parts.join("|");

            // Serializing the result (simplifies processing)
            let size_bytes = std::mem::size_of_val(results);

            cache.put(cache_key, crate::CachedValue {
                // This is where you store the serialized result to make things easier
                value: BatchResult::Nil,
                expiry: expires_at,
                created: now,
                hits: 0,
                size_bytes,
                last_accessed: Instant::now(),
                metadata: None,
            });
        }
    }

    /// Logging statistics
    async fn record_stats(&self, start: Instant, command_count: usize, success: bool, cache_hit: bool) {
        let elapsed = start.elapsed();

        let mut stats = self.stats.write().await;
        stats.total_executions += 1;

        if success {
            stats.total_success += 1;
        } else {
            stats.total_failures += 1;
        }

        if cache_hit {
            stats.cache_hits += 1;
        } else {
            stats.cache_misses += 1;
        }

        stats.avg_batch_size = (stats.avg_batch_size * (stats.total_batches as f64 - 1.0)
            + command_count as f64) / stats.total_batches as f64;

        stats.avg_execution_time_ms = (stats.avg_execution_time_ms * (stats.total_executions as f64 - 1.0)
            + elapsed.as_millis() as f64) / stats.total_executions as f64;
    }

    /// Add command to queue (asynchronous callback)
    pub async fn add_to_queue(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        priority: BatchPriority,
        callback: impl FnOnce(RedissonResult<Vec<BatchResult>>) + Send + Sync + 'static,
    ) -> RedissonResult<()> {
        if *self.is_closed.borrow() {
            return Err(RedissonError::PoolError("The asynchronous batch processor has been turned off".to_string()));
        }

        let mut queue = self.pending_batches.lock().await;

        // Check if the queue is full
        if queue.len() >= self.config.max_queue_size {
            return Err(RedissonError::PoolError("The asynchronous batch queue is full".to_string()));
        }

        // Creating batch groups
        let batch_group = BatchGroup {
            commands,
            created_at: Instant::now(),
            priority,
            callback: Some(Box::new(callback)),
        };

        // Insert according to priority
        if self.config.enable_priority {
            let mut insert_pos = 0;
            for (i, existing) in queue.iter().enumerate() {
                if priority <= existing.priority {
                    insert_pos = i;
                    break;
                }
                insert_pos = i + 1;
            }
            queue.insert(insert_pos, batch_group);
        } else {
            queue.push_back(batch_group);
        }

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.queue_size = queue.len();
        }

        // Notify the refresh thread
        self.flush_notify.notify_one();

        Ok(())
    }

    /// Perform a refresh
    pub async fn flush(&self) -> RedissonResult<()> {
        if *self.is_closed.borrow() {
            return Err(RedissonError::PoolError("The asynchronous batch processor has been turned off".to_string()));
        }

        let batches_to_execute = {
            let mut queue = self.pending_batches.lock().await;
            let now = Instant::now();
            let mut batches = Vec::new();

            // Collect batches that need to be executed
            while let Some(batch) = queue.pop_front() {
                let should_execute = batch.commands.len() >= self.config.max_batch_size
                    || now.duration_since(batch.created_at) >= self.config.flush_interval;

                if should_execute {
                    batches.push(batch);

                    if batches.len() >= 10 {
                        break;
                    }
                } else {
                    queue.push_front(batch);
                    break;
                }
            }

            // Update statistics
            {
                let mut stats = self.stats.write().await;
                stats.queue_size = queue.len();
                stats.last_flush = Some(Instant::now());
            }

            batches
        };

        // Execution batch
        self.execute_batches(batches_to_execute).await
    }

    /// Executing multiple batches
    async fn execute_batches(&self, batches: Vec<BatchGroup>) -> RedissonResult<()> {
        if batches.is_empty() {
            return Ok(());
        }

        // Execute all batches concurrently
        let mut tasks = Vec::new();

        for batch in batches {
            let processor = self.clone();
            let task = tokio::spawn(async move {
                let result = processor.execute_batch_internal(
                    batch.commands,
                    batch.callback.is_some(),
                ).await;

                // Executing the callback
                if let Some(callback) = batch.callback {
                    callback(result);
                }
            });

            tasks.push(task);
        }

        // Wait for all tasks to complete
        for task in tasks {
            if let Err(e) = task.await {
                error!("Batch execution failed: {}", e);
            }
        }

        Ok(())
    }

    /// Start the background refresh task
    async fn start_background_flusher(&mut self) -> RedissonResult<()> {
        let stop_rx = self.is_closed.subscribe();
        let pending_batches = self.pending_batches.clone();
        let flush_notify = self.flush_notify.clone();
        let flush_interval = self.config.flush_interval;
        let processor = self.clone();

        let handle = tokio::spawn(async move {
            AsyncBatchProcessor::background_flusher_worker(
                stop_rx,
                pending_batches,
                flush_notify,
                flush_interval,
                processor,
            ).await;
        });

        // Store task handles
        let mut handle_guard = self.flusher_handle.lock().await;
        *handle_guard = Some(handle);

        Ok(())
    }

    /// Background refresh task worker function
    async fn background_flusher_worker(
        mut stop_rx: tokio::sync::watch::Receiver<bool>,
        pending_batches: Arc<Mutex<VecDeque<BatchGroup>>>,
        flush_notify: Arc<Notify>,
        flush_interval: Duration,
        processor: AsyncBatchProcessor,
    ) {
        let mut last_flush_time = Instant::now();

        loop {
            // Check if it stops
            if *stop_rx.borrow() {
                info!("The asynchronous batch processor background refresh task receives a stop signal");
                break;
            }

            let now = Instant::now();
            let time_since_last_flush = now.duration_since(last_flush_time);

            // Check if a refresh is required
            let should_flush = {
                let queue = pending_batches.lock().await;
                !queue.is_empty() && time_since_last_flush >= flush_interval
            };

            if should_flush {
                // Perform a refresh
                if let Err(e) = processor.flush().await {
                    error!("Asynchronous background refresh failed: {}", e);
                }
                last_flush_time = Instant::now();
            } else {
                // Wait for the next refresh cycle or notification
                let remaining_wait = flush_interval.checked_sub(time_since_last_flush)
                    .unwrap_or(Duration::from_millis(100));

                tokio::select! {
                    _ = time::sleep(remaining_wait) => {
                        // Timeout, continue loop
                    }
                    _ = flush_notify.notified() => {
                        // When notified, refresh immediately
                        info!("The asynchronous background refresh task is notified");
                        if let Err(e) = processor.flush().await {
                            error!("Asynchronous immediate refresh failed: {}", e);
                        }
                        last_flush_time = Instant::now();
                    }
                    _ = stop_rx.changed() => {
                        // Stop signal change
                        if *stop_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        }

        info!("The asynchronous batch processor background refresh task has stopped");
    }

    /// Triggers an immediate refresh
    pub async fn trigger_flush(&self) -> RedissonResult<()> {
        if *self.is_closed.borrow() {
            return Err(RedissonError::PoolError("The asynchronous batch processor has been turned off".to_string()));
        }

        info!("Triggers an asynchronous batch processor flush immediately");
        self.flush_notify.notify_one();

        // Perform a simultaneous refresh
        self.flush().await
    }

    /// Turn off the asynchronous batch processor
    pub async fn close(&self) -> RedissonResult<()> {
        // Send off signal
        self.is_closed.send(true)
            .map_err(|_| RedissonError::PoolError("Failed to send the close signal".to_string()))?;

        // Wait for the background task to finish
        let mut handle_opt = self.flusher_handle.lock().await;
        if let Some(handle) = handle_opt.take() {
            if let Err(e) = handle.await {
                error!("An error occurred while waiting for the asynchronous background refresh task to end: {}", e);
            }
        }

        // Clear the queue
        let mut queue = self.pending_batches.lock().await;
        let mut callbacks = Vec::new();

        while let Some(batch) = queue.pop_front() {
            if let Some(callback) = batch.callback {
                callbacks.push(callback);
            }
        }

        // Executing the callback
        for callback in callbacks {
            callback(Err(RedissonError::PoolError("The asynchronous batch processor has been turned off".to_string())));
        }

        info!("The asynchronous batch processor has been turned off");
        Ok(())
    }

    /// Getting statistics
    pub async fn get_stats(&self) -> BatchStats {
        self.stats.read().await.clone()
    }

    /// Check if it is closed
    pub fn is_closed(&self) -> bool {
        *self.is_closed.borrow()
    }
}

impl Clone for AsyncBatchProcessor {
    fn clone(&self) -> Self {
        Self {
            connection_manager: self.connection_manager.clone(),
            config: self.config.clone(),
            pending_batches: self.pending_batches.clone(),
            stats: self.stats.clone(),
            #[cfg(feature = "caching")]
            cache: self.cache.clone(),
            is_closed: self.is_closed.clone(),
            flush_notify: self.flush_notify.clone(),
            concurrent_semaphore: self.concurrent_semaphore.clone(),
            flusher_handle: self.flusher_handle.clone(),
        }
    }
}