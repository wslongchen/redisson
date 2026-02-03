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

use std::mem;
pub use blocking::*;
#[cfg(feature = "async")]
pub use non_blocking::*;



use std::time::{Duration, Instant};
use serde::Serialize;


/// Cache values
pub struct CachedValue<T> {
    pub value: T,
    pub expiry: Instant,
    pub created: Instant,
    pub hits: u64,
    pub size_bytes: usize,
    pub last_accessed: Instant,
    pub metadata: Option<serde_json::Value>,
}

impl<T> CachedValue<T> {

    #[inline]
    pub fn new(value: T, ttl: Duration, size_bytes: usize) -> Self {
        let now = Instant::now();
        Self {
            value,
            expiry: now + ttl,
            created: now,
            hits: 0,
            size_bytes,
            last_accessed: now,
            metadata: None,
        }
    }

    #[inline]
    pub fn new_with_metadata(value: T, ttl: Duration, size_bytes: usize, metadata: serde_json::Value) -> Self {
        let mut cached = Self::new(value, ttl, size_bytes);
        cached.metadata = Some(metadata);
        cached
    }

    #[inline]
    pub fn is_expired(&self) -> bool {
        Instant::now() > self.expiry
    }

    #[inline]
    pub fn remaining_ttl(&self) -> Option<Duration> {
        let now = Instant::now();
        if self.expiry > now {
            Some(self.expiry.duration_since(now))
        } else {
            None
        }
    }


    #[inline]
    pub fn age(&self) -> Duration {
        Instant::now().duration_since(self.created)
    }

    #[inline]
    pub fn record_access(&mut self) {
        self.hits += 1;
        self.last_accessed = Instant::now();
    }

    #[inline]
    pub fn reset_hits(&mut self) {
        self.hits = 0;
    }

    #[inline]
    pub fn update_ttl(&mut self, ttl: Duration) {
        self.expiry = Instant::now() + ttl;
    }

    #[inline]
    pub fn update_value(&mut self, value: T, size_bytes: usize) {
        self.value = value;
        self.size_bytes = size_bytes;
        self.last_accessed = Instant::now();
    }
}


#[derive(Clone, Debug)]
pub struct CacheEntryStats {
    pub total_entries: usize,
    pub active_entries: usize,
    pub expired_entries: usize,
    pub total_hits: u64,
    pub total_size_bytes: usize,
    pub avg_hits_per_entry: f64,
    pub name: String,
}

impl CacheEntryStats {
    pub fn new(name: String) -> Self {
        Self {
            total_entries: 0,
            active_entries: 0,
            expired_entries: 0,
            total_hits: 0,
            total_size_bytes: 0,
            avg_hits_per_entry: 0.0,
            name,
        }
    }

    #[inline]
    pub fn hit_rate(&self) -> f64 {
        if self.total_entries > 0 {
            self.total_hits as f64 / self.total_entries as f64
        } else {
            0.0
        }
    }

    #[inline]
    pub fn memory_efficiency(&self) -> f64 {
        if self.total_size_bytes > 0 {
            self.active_entries as f64 / self.total_size_bytes as f64
        } else {
            0.0
        }
    }

    #[inline]
    pub fn eviction_rate(&self, previous_stats: &CacheEntryStats) -> f64 {
        if previous_stats.total_entries > 0 {
            (self.expired_entries as f64 - previous_stats.expired_entries as f64)
                / previous_stats.total_entries as f64
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct CacheStats {
    pub total_hits: u64,
    pub total_misses: u64,
    pub total_evictions: u64,
    pub total_entries: usize,
    pub memory_usage_bytes: u64,
    pub avg_hit_rate: f64,
    pub last_cleanup: Option<Instant>,
    pub last_cleanup_duration: Duration,
    pub last_cleanup_count: usize,
    pub creation_time: Instant,
    pub peak_memory_usage: u64,
    pub peak_total_entries: usize,
    pub cache_operations: CacheOperations,
    pub recent_hit_rates: Vec<f64>,
    pub max_recent_samples: usize,
}

#[derive(Clone, Debug)]
pub struct CacheOperations {
    pub get_operations: u64,
    pub set_operations: u64,
    pub remove_operations: u64,
    pub clear_operations: u64,
    pub refresh_operations: u64,
}

impl Default for CacheOperations {
    fn default() -> Self {
        Self {
            get_operations: 0,
            set_operations: 0,
            remove_operations: 0,
            clear_operations: 0,
            refresh_operations: 0,
        }
    }
}

impl CacheStats {
    pub fn new() -> Self {
        Self {
            total_hits: 0,
            total_misses: 0,
            total_evictions: 0,
            total_entries: 0,
            memory_usage_bytes: 0,
            avg_hit_rate: 1.0,
            last_cleanup: None,
            last_cleanup_duration: Duration::ZERO,
            last_cleanup_count: 0,
            creation_time: Instant::now(),
            peak_memory_usage: 0,
            peak_total_entries: 0,
            cache_operations: CacheOperations::default(),
            recent_hit_rates: Vec::with_capacity(100),
            max_recent_samples: 100,
        }
    }

    pub fn with_max_samples(mut self, max_samples: usize) -> Self {
        self.max_recent_samples = max_samples.max(1);
        self.recent_hit_rates = Vec::with_capacity(self.max_recent_samples);
        self
    }

    #[inline]
    pub fn record_hit(&mut self) {
        self.total_hits += 1;
        self.cache_operations.get_operations += 1;
        self.update_hit_rate();
    }

    #[inline]
    pub fn record_hits(&mut self, count: usize) {
        self.total_hits += count as u64;
        self.cache_operations.get_operations += count as u64;
        self.update_hit_rate();
    }

    #[inline]
    pub fn record_miss(&mut self) {
        self.total_misses += 1;
        self.cache_operations.get_operations += 1;
        self.update_hit_rate();
    }

    #[inline]
    pub fn record_misses(&mut self, count: usize) {
        self.total_misses += count as u64;
        self.cache_operations.get_operations += count as u64;
        self.update_hit_rate();
    }

    #[inline]
    pub fn record_eviction(&mut self, count: usize) {
        self.total_evictions += count as u64;
    }

    #[inline]
    pub fn record_set(&mut self) {
        self.cache_operations.set_operations += 1;
    }

    #[inline]
    pub fn record_remove(&mut self) {
        self.cache_operations.remove_operations += 1;
    }

    #[inline]
    pub fn record_clear(&mut self) {
        self.cache_operations.clear_operations += 1;
    }

    #[inline]
    pub fn record_refresh(&mut self) {
        self.cache_operations.refresh_operations += 1;
    }

    #[inline]
    pub fn update_entries_count(&mut self, count: usize) {
        self.total_entries = count;
        if count > self.peak_total_entries {
            self.peak_total_entries = count;
        }
    }

    #[inline]
    pub fn update_memory_usage(&mut self, usage: u64) {
        self.memory_usage_bytes = usage;
        if usage > self.peak_memory_usage {
            self.peak_memory_usage = usage;
        }
    }

    pub fn update_hit_rate(&mut self) {
        let total = self.total_hits + self.total_misses;
        if total > 0 {
            let hit_rate = self.total_hits as f64 / total as f64;
            self.avg_hit_rate = hit_rate;

            // The most recent death rate was recorded for trend analysis
            self.recent_hit_rates.push(hit_rate);
            if self.recent_hit_rates.len() > self.max_recent_samples {
                self.recent_hit_rates.remove(0);
            }
        }
    }

    #[inline]
    pub fn total_operations(&self) -> u64 {
        self.total_hits + self.total_misses
    }

    #[inline]
    pub fn hit_rate(&self) -> f64 {
        let total = self.total_hits + self.total_misses;
        if total > 0 {
            self.total_hits as f64 / total as f64
        } else {
            0.0
        }
    }

    #[inline]
    pub fn miss_rate(&self) -> f64 {
        let total = self.total_hits + self.total_misses;
        if total > 0 {
            self.total_misses as f64 / total as f64
        } else {
            0.0
        }
    }

    #[inline]
    pub fn eviction_rate(&self) -> f64 {
        if self.total_entries > 0 {
            self.total_evictions as f64 / self.total_entries as f64
        } else {
            0.0
        }
    }

    #[inline]
    pub fn uptime(&self) -> Duration {
        Instant::now().duration_since(self.creation_time)
    }

    pub fn recent_hit_rate_trend(&self) -> (f64, f64) {
        if self.recent_hit_rates.len() < 2 {
            return (0.0, 0.0);
        }

        let samples = self.recent_hit_rates.len();
        let first_half = &self.recent_hit_rates[0..samples/2];
        let second_half = &self.recent_hit_rates[samples/2..];

        let avg_first = first_half.iter().sum::<f64>() / first_half.len() as f64;
        let avg_second = second_half.iter().sum::<f64>() / second_half.len() as f64;

        (avg_first, avg_second)
    }

    pub fn to_metrics_string(&self) -> String {
        format!(
            "Cache Stats:\n\
             Uptime: {:.2}s\n\
             Total Operations: {}\n\
             Hit Rate: {:.2}%\n\
             Miss Rate: {:.2}%\n\
             Eviction Rate: {:.2}%\n\
             Total Entries: {}\n\
             Peak Entries: {}\n\
             Memory Usage: {} bytes\n\
             Peak Memory: {} bytes\n\
             Recent Trend: {:.2}% -> {:.2}%",
            self.uptime().as_secs_f64(),
            self.total_operations(),
            self.hit_rate() * 100.0,
            self.miss_rate() * 100.0,
            self.eviction_rate() * 100.0,
            self.total_entries,
            self.peak_total_entries,
            self.memory_usage_bytes,
            self.peak_memory_usage,
            self.recent_hit_rate_trend().0 * 100.0,
            self.recent_hit_rate_trend().1 * 100.0,
        )
    }

    pub fn reset(&mut self) {
        let creation_time = self.creation_time;
        let max_recent_samples = self.max_recent_samples;

        *self = Self::new();
        self.creation_time = creation_time;
        self.max_recent_samples = max_recent_samples;
    }
}



pub fn estimate_size<V: Serialize>(value: &V) -> usize {
    // First try serializing to JSON to estimate the size
    if let Ok(json) = serde_json::to_string(value) {
        // Returns the length of the JSON string + some overhead
        json.len() + mem::size_of::<String>()
    } else {
        // If serialization fails, use the size of the type as a conservative estimate
        mem::size_of_val(value)
    }
}

