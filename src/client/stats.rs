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
use std::time::{Duration, Instant};
use crate::{BatchStats};

#[derive(Debug, Clone)]
pub struct ClientStats {
    pub connection_stats: ConnectionStats,
    pub batch_stats: BatchStats,
    #[cfg(feature = "caching")]
    pub cache_stats: crate::CacheStats,
}


#[derive(Clone, Debug)]
pub struct ConnectionStats {
    pub total_connections_created: u64,
    pub total_connections_reused: u64,
    pub active_connections: u32,
    pub peak_connections: u32,
    pub total_wait_time_ms: f64,
    pub total_operations: u64,
    pub error_count: u64,
    pub last_reset: Instant,
}



impl ConnectionStats {
    pub fn new() -> Self {
        Self {
            total_connections_created: 0,
            total_connections_reused: 0,
            active_connections: 0,
            peak_connections: 0,
            total_wait_time_ms: 0.0,
            total_operations: 0,
            error_count: 0,
            last_reset: Instant::now(),
        }
    }

    pub fn record_connection(&mut self, created: bool, wait_time_ms: f64) {
        self.total_operations += 1;
        self.total_wait_time_ms += wait_time_ms;

        if created {
            self.total_connections_created += 1;
            self.active_connections += 1;
        } else {
            self.total_connections_reused += 1;
        }

        if self.active_connections > self.peak_connections {
            self.peak_connections = self.active_connections;
        }
    }

    pub fn record_success(&mut self, elapsed: Duration) {
        self.total_operations += 1;
        self.total_wait_time_ms += elapsed.as_millis() as f64;
        self.active_connections += 1;

        if self.active_connections > self.peak_connections {
            self.peak_connections = self.active_connections;
        }
    }

    pub fn record_failure(&mut self, elapsed: Duration) {
        self.total_operations += 1;
        self.error_count += 1;
        self.total_wait_time_ms += elapsed.as_millis() as f64;
    }
    
    pub fn record_error(&mut self) {
        self.error_count += 1;
    }

    pub fn record_release(&mut self) {
        self.active_connections = self.active_connections.saturating_sub(1);
    }

    pub fn record_connection_created(&mut self) {
        self.total_connections_created += 1;
        self.active_connections += 1;

        if self.active_connections > self.peak_connections {
            self.peak_connections = self.active_connections;
        }
    }

    pub fn record_connection_reused(&mut self) {
        self.total_connections_reused += 1;
    }

    // Getter methods
    pub fn total_connections_created(&self) -> u64 {
        self.total_connections_created
    }

    pub fn total_connections_reused(&self) -> u64 {
        self.total_connections_reused
    }

    pub fn active_connections(&self) -> u32 {
        self.active_connections
    }

    pub fn peak_connections(&self) -> u32 {
        self.peak_connections
    }

    pub fn total_wait_time_ms(&self) -> f64 {
        self.total_wait_time_ms
    }

    pub fn avg_wait_time_ms(&self) -> f64 {
        if self.total_operations > 0 {
            self.total_wait_time_ms / self.total_operations as f64
        } else {
            0.0
        }
    }

    pub fn total_operations(&self) -> u64 {
        self.total_operations
    }

    pub fn error_count(&self) -> u64 {
        self.error_count
    }

    pub fn error_rate(&self) -> f64 {
        if self.total_operations > 0 {
            self.error_count as f64 / self.total_operations as f64
        } else {
            0.0
        }
    }

    pub fn last_reset(&self) -> Instant {
        self.last_reset
    }

    pub fn reset(&mut self) {
        self.total_connections_created = 0;
        self.total_connections_reused = 0;
        self.active_connections = 0;
        self.peak_connections = 0;
        self.total_wait_time_ms = 0.0;
        self.total_operations = 0;
        self.error_count = 0;
        self.last_reset = Instant::now();
    }

    pub fn merge(&mut self, other: &ConnectionStats) {
        self.total_connections_created += other.total_connections_created;
        self.total_connections_reused += other.total_connections_reused;
        self.total_wait_time_ms += other.total_wait_time_ms;
        self.total_operations += other.total_operations;
        self.error_count += other.error_count;

        // 对于活跃连接和峰值连接，取最大值
        if other.active_connections > self.active_connections {
            self.active_connections = other.active_connections;
        }
        if other.peak_connections > self.peak_connections {
            self.peak_connections = other.peak_connections;
        }
    }
}


#[derive(Debug, Clone)]
pub struct DetailedConnectionStats {
    pub total_connections_created: u64,
    pub total_connections_reused: u64,
    pub active_connections: u32,
    pub peak_connections: u32,
    pub avg_wait_time_ms: f64,
    pub total_operations: u64,
    pub error_count: u64,
    pub error_rate: f64,
    pub last_reset: Instant,
}

impl DetailedConnectionStats {
    pub fn format(&self) -> String {
        format!(
            "ConnectionStats {{\n\
            \ttotal_connections_created: {},\n\
            \ttotal_connections_reused: {},\n\
            \tactive_connections: {},\n\
            \tpeak_connections: {},\n\
            \tavg_wait_time_ms: {:.2}ms,\n\
            \ttotal_operations: {},\n\
            \terror_count: {},\n\
            \terror_rate: {:.2}%,\n\
            \tlast_reset: {:?}\n\
            }}",
            self.total_connections_created,
            self.total_connections_reused,
            self.active_connections,
            self.peak_connections,
            self.avg_wait_time_ms,
            self.total_operations,
            self.error_count,
            self.error_rate * 100.0,
            self.last_reset.elapsed()
        )
    }
}