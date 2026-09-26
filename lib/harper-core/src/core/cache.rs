// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Simple in-memory cache for API responses
//!
//! This module provides basic caching functionality to reduce API calls
//! and improve performance for repeated requests.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// Cache entry with expiration time
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    value: T,
    expires_at: Instant,
}

impl<T> CacheEntry<T> {
    fn new(value: T, ttl: Duration) -> Self {
        Self {
            value,
            expires_at: Instant::now() + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }
}

/// Simple TTL-based cache
#[derive(Debug)]
pub struct Cache<K, V> {
    entries: HashMap<K, CacheEntry<V>>,
    default_ttl: Duration,
}

impl<K, V> Cache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new cache with default TTL
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            default_ttl,
        }
    }

    /// Get a value from the cache
    pub fn get(&self, key: &K) -> Option<&V> {
        if let Some(entry) = self.entries.get(key) {
            if !entry.is_expired() {
                return Some(&entry.value);
            }
        }
        None
    }

    /// Insert a value into the cache with default TTL
    pub fn insert(&mut self, key: K, value: V) {
        self.insert_with_ttl(key, value, self.default_ttl);
    }

    /// Insert a value into the cache with custom TTL
    pub fn insert_with_ttl(&mut self, key: K, value: V, ttl: Duration) {
        let entry = CacheEntry::new(value, ttl);
        self.entries.insert(key, entry);
    }

    /// Remove expired entries from the cache
    #[allow(dead_code)]
    pub fn cleanup(&mut self) {
        self.entries.retain(|_, entry| !entry.is_expired());
    }

    /// Clear all entries from the cache
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get the number of entries in the cache
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Cache key for API requests
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiCacheKey {
    pub provider: String,
    pub model: String,
    pub messages_hash: u64,
}

impl ApiCacheKey {
    /// Create a new API cache key
    pub fn new(provider: &str, model: &str, messages: &[super::Message]) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for msg in messages {
            msg.role.hash(&mut hasher);
            msg.content.hash(&mut hasher);
        }

        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            messages_hash: hasher.finish(),
        }
    }
}

/// Global API response cache
pub type ApiResponseCache = Cache<ApiCacheKey, String>;

/// Create a new API response cache with default TTL
pub fn new_api_cache() -> ApiResponseCache {
    Cache::new(crate::core::constants::cache::API_RESPONSE_TTL)
}
