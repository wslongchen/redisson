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
use std::collections::{VecDeque};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc};
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::{Mutex, RwLock};
use redis::{Value};
use tracing::{error, info, warn};
use crate::errors::{RedissonError, RedissonResult};
use crate::{BatchConfig, BatchGroup, BatchPriority, BatchResult, BatchStats, SyncRedisConnectionManager};
#[cfg(feature = "caching")]
use crate::{CachedValue};
use redis::{ConnectionLike, Pipeline};
use crate::batch::{BackoffStrategy};
use crate::batch::command_builder::CommandBuilder;

// ================ Batch processor ================
/// Master batch processor
pub struct BatchProcessor {
    // Connection management
    connection_manager: Arc<SyncRedisConnectionManager>,

    // Batch configuration
    config: BatchConfig,

    // Pending queue
    pending_batches: Arc<Mutex<VecDeque<BatchGroup>>>,

    // Statistical information
    stats: Arc<RwLock<BatchStats>>,

    // CACHING
    #[cfg(feature = "caching")]
    cache: Option<Arc<RwLock<LruCache<String, CachedValue<BatchResult>>>>>,

    // Close flag
    is_closed: Arc<AtomicBool>,

    // Thread handles are refreshed in the background
    flusher_handle: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl BatchProcessor {
    /// Create a new batch processor
    pub fn new(
        connection_manager: Arc<SyncRedisConnectionManager>,
        config: BatchConfig,
    ) -> RedissonResult<Self> {
        let processor = Self {
            connection_manager,
            config: config.clone(),
            pending_batches: Arc::new(Mutex::new(VecDeque::new())),
            stats: Arc::new(RwLock::new(BatchStats::new())),
            #[cfg(feature = "caching")]
            cache: None,
            is_closed: Arc::new(AtomicBool::new(false)),
            flusher_handle: Arc::new(Mutex::new(None)),
        };

        // Initializing the cache
        let mut processor = processor;
        #[cfg(feature = "caching")]
        if processor.config.enable_cache {
            processor.cache = Some(Arc::new(RwLock::new(LruCache::new(
                NonZeroUsize::new(processor.config.cache_size).unwrap()
            ))));
        }

        // Start the background refresh thread
        if processor.config.enable_background_flush {
            processor.start_background_flusher()?;
        }

        Ok(processor)
    }

    /// Performs batch processing (returns no results, high performance)
    pub fn exec_batch(&self, commands: Vec<Box<dyn CommandBuilder>>) -> RedissonResult<()> {
        self.execute_batch_internal(commands,false).map(|_| ())
    }

    /// Query batching (returning results)
    pub fn query_batch(&self, commands: Vec<Box<dyn CommandBuilder>>) -> RedissonResult<Vec<BatchResult>> {
        self.execute_batch_internal(commands, true)
    }

    /// Perform batch processing asynchronously (without returning results)
    pub fn exec_batch_async<F>(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        callback: F,
    ) -> RedissonResult<()>
    where
        F: FnOnce(RedissonResult<()>) + Send + 'static,
    {
        self.add_to_queue(commands, BatchPriority::Normal, Some(Box::new(move |result| {
            callback(result.map(|_| ()));
        })))
    }

    /// Asynchronous query batching (returning results)
    pub fn query_batch_async<F>(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        callback: F,
    ) -> RedissonResult<()>
    where
        F: FnOnce(RedissonResult<Vec<BatchResult>>) + Send + 'static,
    {
        self.add_to_queue(commands, BatchPriority::Normal, Some(Box::new(callback)))
    }

    /// Execution with retry
    pub fn execute_batch_with_retry(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        needs_result: bool,
        max_retries: u32,
    ) -> RedissonResult<Option<Vec<BatchResult>>> {
        let mut retry_count = 0;
        let backoff_strategy = BackoffStrategy::new(&self.config.backoff_strategy.clone().unwrap_or_default(),Duration::from_millis(self.config.initial_backoff_ms));

        loop {
            match self.execute_batch_internal(commands.clone(), needs_result) {
                Ok(results) => {
                    if needs_result {
                        return Ok(Some(results));
                    } else {
                        return Ok(None);
                    }
                }
                Err(err) => {
                    if retry_count < max_retries {
                        retry_count += 1;
                        let delay = backoff_strategy.calculate_delay(retry_count)
                            .min(Duration::from_millis(self.config.max_backoff_ms));
                        std::thread::sleep(delay);
                        continue;
                    } else {
                        return Err(err);
                    }
                }
            }
        }
    }

    /// Internal execution method
    fn execute_batch_internal(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        needs_result: bool,
    ) -> RedissonResult<Vec<BatchResult>> {
        if self.is_closed.load(Ordering::Acquire) {
            return Err(RedissonError::PoolError("The batch processor has been shut down".to_string()));
        }

        let start = Instant::now();

        // Update statistics
        {
            let mut stats = self.stats.write();
            stats.total_batches += 1;
            stats.total_commands += commands.len() as u64;
        }

        // Check cache (if enabled and results needed)
        #[cfg(feature = "caching")]
        if self.config.enable_cache && needs_result && self.is_read_only_batch(&commands) {
            if let Some(cached_results) = self.get_cached_results(&commands) {
                {
                    let mut stats = self.stats.write();
                    stats.cache_hits += 1;
                }
                self.record_stats(start, commands.len(), true, true);
                return Ok(cached_results);
            }
            {
                let mut stats = self.stats.write();
                stats.cache_misses += 1;
            }
        }

        // Getting connections
        let mut conn = self.connection_manager.get_connection()?;

        let result = if self.config.enable_pipeline && commands.len() <= self.config.max_batch_size {
            // Executing with a pipeline
            if needs_result {
                self.query_with_pipeline(&mut conn, &commands)
            } else {
                self.exec_with_pipeline(&mut conn, &commands).map(|_| Vec::new())
            }
        } else {
            // Chunking execution
            if needs_result {
                self.query_in_chunks(&mut conn, &commands)
            } else {
                self.exec_in_chunks(&mut conn, &commands).map(|_| Vec::new())
            }
        };

        // Record statistics
        let is_success = result.is_ok();
        self.record_stats(start, commands.len(), is_success, false);

        // Update cache (if enabled)
        #[cfg(feature = "caching")]
        if self.config.enable_cache && needs_result && is_success {
            if let Ok(results) = &result {
                self.update_cache(&commands, results);
            }
        }

        result
    }

    /// Determine whether it is a read-only batch (cacheable)
    fn is_read_only_batch(&self, commands: &[Box<dyn CommandBuilder>]) -> bool {
        commands.iter().all(|cmd| cmd.needs_result())
    }

    /// Get the result from the cache
    #[cfg(feature = "caching")]
    fn get_cached_results(&self, commands: &[Box<dyn CommandBuilder>]) -> Option<Vec<BatchResult>> {
        if let Some(cache) = &self.cache {
            let mut cache = cache.write();
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
    fn update_cache(&self, commands: &[Box<dyn CommandBuilder>], results: &[BatchResult]) {
        if let Some(cache) = &self.cache {
            let mut cache = cache.write();
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

            cache.put(cache_key, CachedValue {
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
    fn record_stats(&self, start: Instant, command_count: usize, success: bool, cache_hit: bool) {
        let elapsed = start.elapsed();

        let mut stats = self.stats.write();
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

        // Update the average batch size
        stats.avg_batch_size = (stats.avg_batch_size * (stats.total_batches as f64 - 1.0)
            + command_count as f64) / stats.total_batches as f64;

        // Update the average execution time
        stats.avg_execution_time_ms = (stats.avg_execution_time_ms * (stats.total_executions as f64 - 1.0)
            + elapsed.as_millis() as f64) / stats.total_executions as f64;
    }

    /// Pipelining (without returning a result)
    fn exec_with_pipeline(
        &self,
        conn: &mut dyn ConnectionLike,
        commands: &[Box<dyn CommandBuilder>],
    ) -> RedissonResult<()> {
        let mut pipeline = Pipeline::new();

        for cmd in commands {
            let redis_cmd = cmd.build();
            pipeline.add_command(redis_cmd);
        }

        pipeline.query(conn).map(|_:()| ()).map_err(RedissonError::RedisError)
    }

    /// Query with pipe (returns results)
    fn query_with_pipeline(
        &self,
        conn: &mut dyn ConnectionLike,
        commands: &[Box<dyn CommandBuilder>],
    ) -> RedissonResult<Vec<BatchResult>> {
        let mut pipeline = Pipeline::new();

        for cmd in commands {
            let redis_cmd = cmd.build();
            pipeline.add_command(redis_cmd);
        }

        let results: Vec<Value> = pipeline.query(conn)
            .map_err(RedissonError::RedisError)?;

        self.convert_results(results)
    }

    /// Chunked execution (no result returned)
    fn exec_in_chunks(
        &self,
        conn: &mut dyn ConnectionLike,
        commands: &[Box<dyn CommandBuilder>],
    ) -> RedissonResult<()> {
        for chunk in commands.chunks(self.config.max_batch_size) {
            let mut pipeline = Pipeline::new();

            for cmd in chunk {
                let redis_cmd = cmd.build();
                pipeline.add_command(redis_cmd);
            }

            pipeline.query(conn).map(|_:Value| ()).map_err(RedissonError::RedisError)?;
        }

        Ok(())
    }

    /// Chunk query (return results)
    fn query_in_chunks(
        &self,
        conn: &mut dyn ConnectionLike,
        commands: &[Box<dyn CommandBuilder>],
    ) -> RedissonResult<Vec<BatchResult>> {
        let mut all_results = Vec::new();

        for chunk in commands.chunks(self.config.max_batch_size) {
            let mut pipeline = Pipeline::new();

            for cmd in chunk {
                let redis_cmd = cmd.build();
                pipeline.add_command(redis_cmd);
            }

            let results: Vec<Value> = pipeline.query(conn)
                .map_err(RedissonError::RedisError)?;

            let converted = self.convert_results(results)?;
            all_results.extend(converted);
        }

        Ok(all_results)
    }

    /// Conversion result
    fn convert_results(&self, values: Vec<Value>) -> RedissonResult<Vec<BatchResult>> {
        let mut results = Vec::with_capacity(values.len());

        for value in values {
            match BatchResult::from_redis_value(value) {
                Ok(result) => results.push(result),
                Err(e) => results.push(BatchResult::Error(e.to_string())),
            }
        }

        Ok(results)
    }

    /// Add commands to the queue
    fn add_to_queue(
        &self,
        commands: Vec<Box<dyn CommandBuilder>>,
        priority: BatchPriority,
        callback: Option<Box<dyn FnOnce(RedissonResult<Vec<BatchResult>>) + Send>>,
    ) -> RedissonResult<()> {
        if self.is_closed.load(Ordering::Acquire) {
            return Err(RedissonError::PoolError("The batch processor has been shut down".to_string()));
        }

        let mut queue = self.pending_batches.lock();

        // Check if the queue is full
        if queue.len() >= self.config.max_queue_size {
            return Err(RedissonError::PoolError("The batch queue is full".to_string()));
        }

        // Creating batch groups
        let batch_group = BatchGroup {
            commands,
            created_at: Instant::now(),
            priority,
            callback,
        };

        // Depending on the configuration, decide whether to sort by priority or not
        if self.config.enable_priority {
            // Insert in the appropriate place according to priority
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

        // Update queue statistics
        {
            let mut stats = self.stats.write();
            stats.queue_size = queue.len();
        }

        Ok(())
    }

    /// Batches in the execution queue
    pub fn flush(&self) -> RedissonResult<()> {
        if self.is_closed.load(Ordering::Acquire) {
            return Err(RedissonError::PoolError("The batch processor has been shut down".to_string()));
        }

        let batches_to_execute = {
            let mut queue = self.pending_batches.lock();
            let now = Instant::now();
            let mut batches = Vec::new();

            // Collect batches that need to be executed
            while let Some(batch) = queue.pop_front() {
                // Check whether the batch should be executed (Max size reached or time out)
                let should_execute = batch.commands.len() >= self.config.max_batch_size
                    || now.duration_since(batch.created_at) >= self.config.flush_interval;

                if should_execute {
                    batches.push(batch);

                    // Limit the number of batches executed at a time
                    if batches.len() >= 10 {
                        break;
                    }
                } else {
                    // Put back in queue
                    queue.push_front(batch);
                    break;
                }
            }

            // Update queue statistics
            {
                let mut stats = self.stats.write();
                stats.queue_size = queue.len();
                stats.last_flush = Some(Instant::now());
            }

            batches
        };

        // Execution batch
        self.execute_batches(batches_to_execute)
    }

    /// Executing multiple batches
    fn execute_batches(&self, batches: Vec<BatchGroup>) -> RedissonResult<()> {
        if batches.is_empty() {
            return Ok(());
        }

        if self.config.enable_async {
            // Asynchronous execution
            let processor = self.clone();

            std::thread::spawn(move || {
                for batch in batches {
                    let result = processor.execute_batch_internal(
                        batch.commands,
                        batch.callback.is_some(),
                    );

                    // Executing the callback
                    if let Some(callback) = batch.callback {
                        callback(result);
                    }
                }
            });

            Ok(())
        } else {
            // Synchronous execution
            for batch in batches {
                let result = self.execute_batch_internal(
                    batch.commands,
                    batch.callback.is_some(),
                );

                // Executing the callback
                if let Some(callback) = batch.callback {
                    callback(result);
                }
            }

            Ok(())
        }
    }

    /// Start the background refresh thread
    fn start_background_flusher(&mut self) -> RedissonResult<()> {
        let stop_signal = self.is_closed.clone();
        let pending_batches = self.pending_batches.clone();
        let config = self.config.clone();
        let processor = self.clone();

        let handle = std::thread::Builder::new()
            .name("batch-processor-flusher".to_string())
            .spawn(move || {
                Self::background_flusher_worker(
                    stop_signal,
                    pending_batches,
                    config.flush_interval,
                    processor,
                );
            })
            .map_err(|e| RedissonError::ThreadError(e.to_string()))?;

        // 存储线程句柄
        let mut handle_guard = self.flusher_handle.lock();
        *handle_guard = Some(handle);

        Ok(())
    }

    /// Background flush thread worker function
    fn background_flusher_worker(
        stop_signal: Arc<AtomicBool>,
        pending_batches: Arc<Mutex<VecDeque<BatchGroup>>>,
        flush_interval: Duration,
        processor: BatchProcessor,
    ) {
        let mut last_flush_time = Instant::now();

        while !stop_signal.load(Ordering::Acquire) {
            let now = Instant::now();
            let time_since_last_flush = now.duration_since(last_flush_time);

            // Check if a refresh is required
            let should_flush = {
                if let Some(queue) = pending_batches.try_lock() {
                    !queue.is_empty() && time_since_last_flush >= flush_interval
                } else {
                    // Mutex poisoning, exit thread
                    error!("Batch queue mutex poisoning");
                    break;
                }
            };

            if should_flush {
                // Perform a refresh
                if let Err(e) = processor.flush() {
                    error!("Background refresh failed: {}", e);
                }
                last_flush_time = Instant::now();
            } else {
                // Wait for the next refresh cycle
                let remaining_wait = flush_interval.checked_sub(time_since_last_flush)
                    .unwrap_or(Duration::from_millis(100));

                std::thread::sleep(remaining_wait);
            }
        }

        info!("The batch processor background refresh thread has stopped");
    }

    /// Triggers an immediate refresh
    pub fn trigger_flush(&self) -> RedissonResult<()>  {
        if self.is_closed.load(Ordering::Acquire) {
            return Err(RedissonError::InvalidOperation("The batch processor has been shut down".to_string()));
        }

        // The refresh is performed in a new thread without blocking the caller
        let processor = self.clone();
        std::thread::spawn(move || {
            match processor.flush() {
                Ok(_) => info!("The asynchronous refresh performed successfully"),
                Err(e) => error!("Asynchronous refresh failed to execute: {}", e),
            }
        });
        Ok(())
    }

    /// Getting statistics
    pub fn get_stats(&self) -> BatchStats {
        self.stats.read().clone()
    }

    pub fn get_batch_config(&self) -> &BatchConfig {
        &self.config
    }

    /// Shutting down the batch processor
    pub fn close(&self) -> RedissonResult<()> {
        // Setting the close flag
        self.is_closed.store(true, Ordering::Release);

        // Wait for the background thread to finish
        let mut handle_opt = self.flusher_handle.lock();
        if let Some(handle) = handle_opt.take() {
            if let Err(e) = handle.join() {
                error!("An error occurred while waiting for the background refresh thread to end: {:?}", e);
            }
        }

        // Empty the queue and execute the callback
        let mut queue = self.pending_batches.lock();
        let mut callbacks = Vec::new();

        while let Some(batch) = queue.pop_front() {
            if let Some(callback) = batch.callback {
                callbacks.push(callback);
            }
        }

        // Execute callbacks (to avoid executing user code inside the lock)
        drop(queue);

        for callback in callbacks {
            callback(Err(RedissonError::PoolError("The batch processor has been shut down".to_string())));
        }

        info!("The batch processor has been shut down");
        Ok(())
    }

    /// Check if it is closed
    pub fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::Acquire)
    }

    /// Graceful shutdown (wait for all tasks to complete)
    pub fn graceful_close(&self, timeout: Duration) -> bool {
        info!("Start shutting down the batch processor gracefully");

        let start = Instant::now();
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 10;

        // The loop tries to empty the queue
        while attempts < MAX_ATTEMPTS {
            attempts += 1;

            // Triggers the final refresh
            let _ = self.flush();

            // Check if the queue is empty
            let is_empty = {
                if let Some(queue) = self.pending_batches.try_lock() {
                    queue.is_empty()
                } else {
                    false
                }
            };

            if is_empty {
                info!("The queue has been emptied and started to close");
                let _ = self.close();
                return true;
            }

            // Check for a timeout
            if start.elapsed() >= timeout {
                warn!("Gracefully close timeouts and force closings");
                let _ = self.close();
                return false;
            }

            // Wait a while and try again
            let wait_time = Duration::from_millis(100 * attempts as u64);
            std::thread::sleep(wait_time);
        }

        warn!("Force close when maximum number of retries is reached");
        let _ = self.close();
        false
    }

    /// Check if there are any tasks left to process
    pub fn has_pending_tasks(&self) -> bool {
        if let Some(queue) = self.pending_batches.try_lock() {
            !queue.is_empty()
        } else {
            false
        }
    }

    /// Gets the number of pending tasks
    pub fn pending_task_count(&self) -> usize {
        if let Some(queue) = self.pending_batches.try_lock() {
            queue.len()
        } else {
            0
        }
    }
}

impl Clone for BatchProcessor {
    fn clone(&self) -> Self {
        Self {
            connection_manager: self.connection_manager.clone(),
            config: self.config.clone(),
            pending_batches: self.pending_batches.clone(),
            stats: self.stats.clone(),
            #[cfg(feature = "caching")]
            cache: self.cache.clone(),
            is_closed: Arc::new(AtomicBool::new(false)),
            flusher_handle: Arc::new(Mutex::new(None)),
        }
    }
}

impl Drop for BatchProcessor {
    fn drop(&mut self) {
        if !self.is_closed.load(Ordering::Acquire) {
            warn!("The batch processor is not shut down properly and is shutting down automatically");
            let _ = self.close();
        }
    }
}