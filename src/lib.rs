#![allow(unused_imports,unreachable_patterns,dead_code,missing_docs, incomplete_features,unused_variables)]
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
//! 
//! <p align="center">
//!   <img src="https://img.icons8.com/color/96/000000/redis.png" alt="Redis Logo" width="96" height="96">
//! </p>
//! 
//! <p align="center">
//!   <strong>A Redis-based distributed synchronization and data structures library for Rust</strong>
//! </p>
//! 
//! ## 🎯 Features
//! 
//! - **🔒 Distributed Locks**: Reentrant locks, fair locks, read-write locks, and RedLock algorithm with automatic renewal
//! - **📊 Rich Data Structures**: Distributed maps, lists, sets, sorted sets, buckets, and streams
//! - **⚡ Dual Runtime**: Full support for both synchronous and asynchronous operations
//! - **🔄 Synchronization Primitives**: Semaphores, rate limiters, countdown latches, and atomic counters
//! - **📈 High Performance**: Connection pooling, command pipelining, and batch operations
//! - **💾 Local Cache Integration**: Read-through/write-through caching with local cache
//! - **🔧 Comprehensive Configuration**: Flexible configuration for various Redis deployment modes
//! - **🎯 Type Safety**: Full Rust type system support with compile-time checking
//! - **🛡️ Production Ready**: Automatic reconnection, timeout handling, and comprehensive error management
//! - **📡 Advanced Features**: Redis Stream support, delayed queues, and publish/subscribe messaging
//! 
//! ## 📦 Installation
//! 
//! Add this to your `Cargo.toml`:
//! 
//! ### Basic Installation
//! ```toml
//! [dependencies]
//! redisson = "0.1"
//! ```
//!!
//!! ```rust
//! use redisson::{RedissonClient, RedissonConfig};
//! use std::time::Duration;
//! 
//! fn main() -> redisson::RedissonResult<()> {
//!     //! Create configuration
//!     let config = RedissonConfig::single_server("redis://!127.0.0.1:6379")
//!         .with_pool_size(10)
//!         .with_connection_timeout(Duration::from_secs(5));
//! 
//!     //! Create client
//!     let client = RedissonClient::new(config)?;
//! 
//!     //! Use distributed lock
//!     let lock = client.get_lock("my-resource");
//!     lock.lock()?;
//!     println!("Critical section accessed");
//!     lock.unlock()?;
//! 
//!     //! Use distributed data structures
//!     let bucket = client.get_bucket::<String>("my-bucket");
//!     bucket.set(&"Hello World".to_string())?;
//!     let value: Option<String> = bucket.get()?;
//!     println!("Bucket value: {:?}", value);
//! 
//!     //! Use distributed map
//!     let map = client.get_map::<String, i32>("my-map");
//!     map.put(&"key1".to_string(), &42)?;
//! 
//!     client.shutdown()?;
//!     Ok(())
//! }
//! ```

mod config;
mod errors;
mod util;
mod objects;
mod client;
mod batch;
mod lock;
mod scripts;
mod connection;
#[cfg(feature = "caching")]
mod cache;
mod network_latency;
mod transaction;

#[cfg(feature = "caching")]
pub use cache::*;
pub use transaction::*;
pub use network_latency::*;
pub use connection::*;
pub use config::*;
pub use errors::*;
pub use util::*;
pub use objects::*;
pub use client::*;
pub use batch::*;
pub use lock::*;
pub use scripts::*;

