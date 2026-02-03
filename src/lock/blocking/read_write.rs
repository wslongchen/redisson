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
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use redis::Commands;
use crate::lock::LocalLockState;
use crate::{scripts, thread_id_to_u64, LockInfo, LockWatchdog, RedissonError, RedissonResult, SyncRedisConnectionManager};

// === RReadWriteLock (read/write lock) ===
pub struct RReadWriteLock {
    connection_manager: Arc<SyncRedisConnectionManager>,
    name: String,
    lease_time: Duration,
}

impl RReadWriteLock {
    pub fn new(
        connection_manager: Arc<SyncRedisConnectionManager>,
        name: String,
        lease_time: Duration,
    ) -> Self {
        Self {
            connection_manager,
            name,
            lease_time,
        }
    }

    /// Acquire the read lock object
    pub fn read_lock(&self) -> RReadLock {
        RReadLock::new(
            self.connection_manager.clone(),
            self.name.clone(),
            self.lease_time,
        )
    }

    /// Obtain the write lock object
    pub fn write_lock(&self) -> RWriteLock {
        RWriteLock::new(
            self.connection_manager.clone(),
            self.name.clone(),
            self.lease_time,
        )
    }

    /// Force unlock read/write lock (ignore reentry count)
    pub fn force_unlock(&self) -> RedissonResult<bool> {
        let mut conn = self.connection_manager.get_connection()?;
        let read_lock_name = format!("{}:read", self.name);
        let write_lock_name = format!("{}:write", self.name);

        // Remove read and write locks
        let deleted_read: i32 = conn.del(&read_lock_name)?;
        let deleted_write: i32 = conn.del(&write_lock_name)?;

        Ok(deleted_read > 0 || deleted_write > 0)
    }

    /// Check that the read lock is held
    pub fn is_read_locked(&self) -> RedissonResult<bool> {
        let mut conn = self.connection_manager.get_connection()?;
        let read_lock_name = format!("{}:read", self.name);
        let exists: i32 = conn.exists(&read_lock_name)?;
        Ok(exists > 0)
    }

    /// Checks whether the write lock is held
    pub fn is_write_locked(&self) -> RedissonResult<bool> {
        let mut conn = self.connection_manager.get_connection()?;
        let write_lock_name = format!("{}:write", self.name);
        let exists: i32 = conn.exists(&write_lock_name)?;
        Ok(exists > 0)
    }

    /// The number of readers that acquire the lock
    pub fn get_read_lock_count(&self) -> RedissonResult<u32> {
        let mut conn = self.connection_manager.get_connection()?;
        let read_lock_name = format!("{}:read", self.name);

        let count: i32 = scripts::GET_READER_COUNT_SCRIPT
            .key(&read_lock_name)
            .invoke(&mut conn)?;

        Ok(count.max(0) as u32)
    }
}

// === RReadLock (Read lock) ===
pub struct RReadLock {
    connection_manager: Arc<SyncRedisConnectionManager>,
    name: String,
    lease_time: Duration,
    local_locks: Arc<Mutex<HashMap<String, LocalLockState>>>,
    watchdog: Arc<Mutex<Option<LockWatchdog>>>,
}

impl RReadLock {
    fn new(
        connection_manager: Arc<SyncRedisConnectionManager>,
        name: String,
        lease_time: Duration,
    ) -> Self {
        Self {
            connection_manager,
            name,
            lease_time,
            local_locks: Arc::new(Mutex::new(HashMap::new())),
            watchdog: Arc::new(Mutex::new(None)),
        }
    }

    /// Lock acquisition (blocking)
    pub fn lock(&self) -> RedissonResult<()> {
        self.lock_with_lease_time(self.lease_time)
    }

    /// Lock with lease duration
    pub fn lock_with_lease_time(&self, lease_time: Duration) -> RedissonResult<()> {
        let lock_info = self.try_acquire_with_retry(lease_time, None)?;

        // Start the watchdog (if the lease is 0, it needs to be automatically renewed)
        if lease_time.as_secs() == 0 {
            self.start_watchdog(lock_info.value.clone());
        }

        Ok(())
    }

    /// Attempt to acquire lock (non-blocking)
    pub fn try_lock(&self) -> RedissonResult<bool> {
        self.try_lock_with_timeout(Duration::from_secs(0))
    }

    /// An attempt to acquire a lock with a timeout
    pub fn try_lock_with_timeout(&self, wait_time: Duration) -> RedissonResult<bool> {
        match self.try_acquire_with_retry(self.lease_time, Some(wait_time)) {
            Ok(lock_info) => {
                // Start the watchdog
                if self.lease_time.as_secs() == 0 {
                    self.start_watchdog(lock_info.value.clone());
                }
                Ok(true)
            }
            Err(RedissonError::TimeoutError) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Release a lock
    pub fn unlock(&self) -> RedissonResult<bool> {
        let mut local_locks = self.local_locks.lock();

        // Checks if the lock is held locally
        if let Some(state) = local_locks.get_mut(&self.name) {
            // Check the reentry count
            if state.lock_count > 1 {
                state.lock_count -= 1;
                return Ok(true);
            }

            // Acquire connection and release Redis lock
            let mut conn = self.connection_manager.get_connection()?;

            let released: i32 = scripts::READ_UNLOCK_SCRIPT
                .key(&self.name)
                .arg(&state.lock_value)
                .invoke(&mut conn)?;

            // Stop the watchdog
            if let Some(mut watchdog) = self.watchdog.lock().take() {
                watchdog.stop();
            }

            if released > 0 {
                local_locks.remove(&self.name);
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    /// Attempt to acquire lock (with retry)
    fn try_acquire_with_retry(
        &self,
        lease_time: Duration,
        wait_time: Option<Duration>,
    ) -> RedissonResult<LockInfo> {
        let start_time = Instant::now();
        let lock_info = LockInfo::new(format!("{}:read", self.name), lease_time);
        let thread_id = thread_id_to_u64();

        loop {
            // Check for local reentry
            {
                let mut local_locks = self.local_locks.lock();
                if let Some(state) = local_locks.get_mut(&self.name) {
                    // Check for reentry of the same thread
                    if state.lock_value == lock_info.value {
                        state.lock_count += 1;
                        state.last_renew_time = Instant::now();
                        return Ok(lock_info);
                    }
                }
            }

            // Attempt to acquire a Redis lock
            let mut conn = self.connection_manager.get_connection()?;

            let acquired: i32 = scripts::READ_LOCK_SCRIPT
                .key(&self.name)
                .arg(&lock_info.value)
                .arg(thread_id)
                .arg(lease_time.as_millis() as i64)
                .invoke(&mut conn)?;

            if acquired > 0 {
                // Updating local state
                let mut local_locks = self.local_locks.lock();
                local_locks.insert(
                    self.name.clone(),
                    LocalLockState {
                        lock_count: 1,
                        lock_value: lock_info.value.clone(),
                        last_renew_time: Instant::now(),
                    },
                );
                return Ok(lock_info);
            }

            // Check for a timeout
            if let Some(wait_time) = wait_time {
                if start_time.elapsed() >= wait_time {
                    return Err(RedissonError::TimeoutError);
                }
                thread::sleep(Duration::from_millis(100));
            } else {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// Start the watchdog
    fn start_watchdog(&self, lock_value: String) {
        let connection_manager = self.connection_manager.clone();
        let name = self.name.clone();
        let lease_time = self.lease_time;

        let renew_func = move || {
            match connection_manager.get_connection() {
                Ok(mut conn) => {
                    match scripts::READ_RENEW_SCRIPT
                        .key(&name)
                        .arg(&lock_value)
                        .arg(lease_time.as_millis() as i64)
                        .invoke::<u64>(&mut conn)
                    {
                        Ok(result) => result == 1,
                        Err(_) => false,
                    }
                }
                Err(_) => false,
            }
        };

        let mut watchdog_guard = self.watchdog.lock();
        let mut watchdog = LockWatchdog::new();
        watchdog.start(Duration::from_secs(10), renew_func);
        *watchdog_guard = Some(watchdog);
    }

    /// Checks whether the current thread holds the lock
    pub fn is_held_by_current_thread(&self) -> bool {
        let local_locks = self.local_locks.lock();
        local_locks.get(&self.name).is_some()
    }

    /// Time remaining to acquire the lock
    pub fn remaining_time(&self) -> RedissonResult<Duration> {
        let mut conn = self.connection_manager.get_connection()?;
        let lock_name = format!("{}:read", self.name);
        let ttl_ms: i64 = conn.pttl(&lock_name)?;

        if ttl_ms > 0 {
            Ok(Duration::from_millis(ttl_ms as u64))
        } else {
            Ok(Duration::from_secs(0))
        }
    }
}

// === RWriteLock (Write lock) ===
pub struct RWriteLock {
    connection_manager: Arc<SyncRedisConnectionManager>,
    name: String,
    lease_time: Duration,
    local_locks: Arc<Mutex<HashMap<String, LocalLockState>>>,
    watchdog: Arc<Mutex<Option<LockWatchdog>>>,
}

impl RWriteLock {
    fn new(
        connection_manager: Arc<SyncRedisConnectionManager>,
        name: String,
        lease_time: Duration,
    ) -> Self {
        Self {
            connection_manager,
            name,
            lease_time,
            local_locks: Arc::new(Mutex::new(HashMap::new())),
            watchdog: Arc::new(Mutex::new(None)),
        }
    }

    /// Lock acquisition (blocking)
    pub fn lock(&self) -> RedissonResult<()> {
        self.lock_with_lease_time(self.lease_time)
    }

    /// Lock with lease duration
    pub fn lock_with_lease_time(&self, lease_time: Duration) -> RedissonResult<()> {
        let lock_info = self.try_acquire_with_retry(lease_time, None)?;

        // Start the watchdog (if the lease is 0, it needs to be automatically renewed)
        if lease_time.as_secs() == 0 {
            self.start_watchdog(lock_info.value.clone());
        }

        Ok(())
    }

    /// Attempt to acquire lock (non-blocking)
    pub fn try_lock(&self) -> RedissonResult<bool> {
        self.try_lock_with_timeout(Duration::from_secs(0))
    }

    /// An attempt to acquire a lock with a timeout
    pub fn try_lock_with_timeout(&self, wait_time: Duration) -> RedissonResult<bool> {
        match self.try_acquire_with_retry(self.lease_time, Some(wait_time)) {
            Ok(lock_info) => {
                // Start the watchdog
                if self.lease_time.as_secs() == 0 {
                    self.start_watchdog(lock_info.value.clone());
                }
                Ok(true)
            }
            Err(RedissonError::TimeoutError) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Release a lock
    pub fn unlock(&self) -> RedissonResult<bool> {
        let mut local_locks = self.local_locks.lock();

        // Checks if the lock is held locally
        if let Some(state) = local_locks.get_mut(&self.name) {
            // Check the reentry count
            if state.lock_count > 1 {
                state.lock_count -= 1;
                return Ok(true);
            }

            // Acquire connection and release Redis lock
            let mut conn = self.connection_manager.get_connection()?;

            let released: i32 = scripts::WRITE_UNLOCK_SCRIPT
                .key(&self.name)
                .arg(&state.lock_value)
                .invoke(&mut conn)?;

            // Stop the watchdog
            if let Some(mut watchdog) = self.watchdog.lock().take() {
                watchdog.stop();
            }

            if released > 0 {
                local_locks.remove(&self.name);
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    /// Attempt to acquire lock (with retry)
    fn try_acquire_with_retry(
        &self,
        lease_time: Duration,
        wait_time: Option<Duration>,
    ) -> RedissonResult<LockInfo> {
        let start_time = Instant::now();
        let lock_info = LockInfo::new(format!("{}:write", self.name), lease_time);
        let thread_id = thread_id_to_u64();

        loop {
            // Check for local reentry
            {
                let mut local_locks = self.local_locks.lock();
                if let Some(state) = local_locks.get_mut(&self.name) {
                    // Check for reentry of the same thread
                    if state.lock_value == lock_info.value {
                        state.lock_count += 1;
                        state.last_renew_time = Instant::now();
                        return Ok(lock_info);
                    }
                }
            }

            // Attempt to acquire a Redis lock
            let mut conn = self.connection_manager.get_connection()?;

            let acquired: i32 = scripts::WRITE_LOCK_SCRIPT
                .key(&self.name)
                .arg(&lock_info.value)
                .arg(thread_id)
                .arg(lease_time.as_millis() as i64)
                .invoke(&mut conn)?;

            if acquired > 0 {
                // Updating local state
                let mut local_locks = self.local_locks.lock();
                local_locks.insert(
                    self.name.clone(),
                    LocalLockState {
                        lock_count: 1,
                        lock_value: lock_info.value.clone(),
                        last_renew_time: Instant::now(),
                    },
                );
                return Ok(lock_info);
            }

            // Check for a timeout
            if let Some(wait_time) = wait_time {
                if start_time.elapsed() >= wait_time {
                    return Err(RedissonError::TimeoutError);
                }
                thread::sleep(Duration::from_millis(100));
            } else {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// Start the watchdog
    fn start_watchdog(&self, lock_value: String) {
        let connection_manager = self.connection_manager.clone();
        let name = self.name.clone();
        let lease_time = self.lease_time;

        let renew_func = move || {
            match connection_manager.get_connection() {
                Ok(mut conn) => {
                    match scripts::WRITE_RENEW_SCRIPT
                        .key(&name)
                        .arg(&lock_value)
                        .arg(lease_time.as_millis() as i64)
                        .invoke::<u64>(&mut conn)
                    {
                        Ok(result) => result == 1,
                        Err(_) => false,
                    }
                }
                Err(_) => false,
            }
        };

        let mut watchdog_guard = self.watchdog.lock();
        let mut watchdog = LockWatchdog::new();
        watchdog.start(Duration::from_secs(10), renew_func);
        *watchdog_guard = Some(watchdog);
    }

    /// Checks whether the current thread holds the lock
    pub fn is_held_by_current_thread(&self) -> bool {
        let local_locks = self.local_locks.lock();
        local_locks.get(&self.name).is_some()
    }

    /// Time remaining to acquire the lock
    pub fn remaining_time(&self) -> RedissonResult<Duration> {
        let mut conn = self.connection_manager.get_connection()?;
        let lock_name = format!("{}:write", self.name);
        let ttl_ms: i64 = conn.pttl(&lock_name)?;

        if ttl_ms > 0 {
            Ok(Duration::from_millis(ttl_ms as u64))
        } else {
            Ok(Duration::from_secs(0))
        }
    }
}

impl Clone for RReadWriteLock {
    fn clone(&self) -> Self {
        Self {
            connection_manager: self.connection_manager.clone(),
            name: self.name.clone(),
            lease_time: self.lease_time,
        }
    }
}