// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Cache-aware primitives used by Harper's memory subsystem.
//!
//! These helpers provide an ergonomic way to store frequently accessed data on
//! 64-byte boundaries (a common CPU cache-line size) so read/write workloads do
//! not suffer from false sharing or misaligned loads.

use std::io::{self, Write};
use std::ops::{Deref, DerefMut};

pub const CACHE_LINE_BYTES: usize = 64;

#[repr(align(64))]
#[derive(Debug, Clone, Copy)]
struct CacheLine {
    bytes: [u8; CACHE_LINE_BYTES],
}

impl CacheLine {
    const fn new() -> Self {
        Self {
            bytes: [0; CACHE_LINE_BYTES],
        }
    }

    fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.bytes.as_mut_ptr()
    }
}

impl Default for CacheLine {
    fn default() -> Self {
        Self::new()
    }
}

/// Wraps any type so its address is aligned to a cache line boundary.
#[repr(align(64))]
#[derive(Debug)]
pub struct CacheAligned<T> {
    inner: T,
}

impl<T> CacheAligned<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn as_ptr(&self) -> *const T {
        &self.inner
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        &mut self.inner
    }
}

impl<T: Clone> Clone for CacheAligned<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Copy> Copy for CacheAligned<T> {}

impl<T: Default> Default for CacheAligned<T> {
    fn default() -> Self {
        Self {
            inner: T::default(),
        }
    }
}

impl<T> Deref for CacheAligned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for CacheAligned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// A byte buffer whose start address is always cache-line aligned.
#[derive(Debug, Clone)]
pub struct CacheAlignedBuffer {
    lines: Vec<CacheLine>,
    len: usize,
}

impl CacheAlignedBuffer {
    pub fn new(len: usize) -> Self {
        let mut buffer = Self {
            lines: Vec::new(),
            len,
        };
        buffer.ensure_capacity(len.max(1));
        buffer
    }

    pub fn with_capacity(bytes: usize) -> Self {
        let mut buffer = Self {
            lines: Vec::new(),
            len: 0,
        };
        buffer.ensure_capacity(bytes.max(1));
        buffer
    }

    pub fn from_slice(data: &[u8]) -> Self {
        let mut buffer = Self::new(data.len());
        buffer.as_mut_slice().copy_from_slice(data);
        buffer
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.lines.len() * CACHE_LINE_BYTES
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// # Safety
    ///
    /// `lines` always holds at least one 64-byte `CacheLine` (all constructors
    /// reserve via `ensure_capacity`), so `lines[0]` plus `len` bytes stays
    /// inside the allocation. Callers hold `&self`, so no `&mut` reallocation
    /// can invalidate the slice while it is alive.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.raw_ptr(), self.len) }
    }

    /// # Safety
    ///
    /// Same allocation guarantee as `as_slice`, plus exclusive `&mut self`
    /// access, so no live shared slice can alias this mutable one.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.raw_mut_ptr(), self.len) }
    }

    pub fn resize(&mut self, new_len: usize) {
        let old_len = self.len;
        self.ensure_capacity(new_len.max(1));
        self.len = new_len;
        if new_len > old_len {
            self.as_mut_slice()[old_len..new_len].fill(0);
        }
    }

    pub fn fill(&mut self, value: u8) {
        self.as_mut_slice().fill(value);
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.resize(bytes.len());
        self.as_mut_slice().copy_from_slice(bytes);
    }

    pub fn append_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let start = self.len;
        self.resize(self.len + bytes.len());
        self.as_mut_slice()[start..start + bytes.len()].copy_from_slice(bytes);
    }

    fn ensure_capacity(&mut self, target_len: usize) {
        let needed_lines = target_len.div_ceil(CACHE_LINE_BYTES);
        if needed_lines > self.lines.len() {
            self.lines.resize_with(needed_lines, CacheLine::default);
        }
        if self.len > target_len {
            self.len = target_len;
        }
    }

    fn raw_ptr(&self) -> *const u8 {
        debug_assert!(
            !self.lines.is_empty(),
            "CacheAlignedBuffer must reserve at least one line"
        );
        self.lines[0].as_ptr()
    }

    fn raw_mut_ptr(&mut self) -> *mut u8 {
        debug_assert!(
            !self.lines.is_empty(),
            "CacheAlignedBuffer must reserve at least one line"
        );
        self.lines[0].as_mut_ptr()
    }
}

impl Write for CacheAlignedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.append_bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_aligned_wrapper_aligns_memory() {
        let wrapped = CacheAligned::new([0u8; 32]);
        let addr = wrapped.as_ptr() as usize;
        assert_eq!(addr % CACHE_LINE_BYTES, 0);
    }

    #[test]
    fn buffer_resizes_and_retains_alignment() {
        let mut buffer = CacheAlignedBuffer::new(16);
        let ptr = buffer.as_slice().as_ptr() as usize;
        assert_eq!(ptr % CACHE_LINE_BYTES, 0);

        buffer.resize(256);
        assert_eq!(buffer.len(), 256);
        assert!(buffer.capacity() >= 256);
        let new_ptr = buffer.as_slice().as_ptr() as usize;
        assert_eq!(new_ptr % CACHE_LINE_BYTES, 0);
    }

    #[test]
    fn write_and_read_round_trip() {
        let payload = b"hello cache";
        let mut buffer = CacheAlignedBuffer::with_capacity(512);
        buffer.write_bytes(payload);
        assert_eq!(buffer.as_slice(), payload);
    }

    #[test]
    fn append_via_write_trait_accumulates_bytes() {
        use std::io::Write as _;
        let mut buffer = CacheAlignedBuffer::with_capacity(64);
        write!(&mut buffer, "abc").unwrap();
        write!(&mut buffer, "123").unwrap();
        assert_eq!(buffer.as_slice(), b"abc123");
    }
}
