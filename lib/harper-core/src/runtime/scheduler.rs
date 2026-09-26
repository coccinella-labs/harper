// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Cooperative task scheduler built on top of `BinaryHeap`.
//!
//! This module supplies a deterministic priority queue that Harper can use
//! to coordinate background operations (command dispatch, MCP sessions,
//! telemetry flushes, etc.).

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

/// Represents the urgency of a task. Higher numbers mean higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskPriority(u8);

impl TaskPriority {
    pub const LOW: Self = Self(25);
    pub const NORMAL: Self = Self(50);
    pub const HIGH: Self = Self(75);
    pub const CRITICAL: Self = Self(100);

    pub fn new(level: u8) -> Self {
        Self(level)
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Public handle returned for ready tasks.
#[derive(Debug)]
pub struct ScheduledItem<T> {
    pub priority: TaskPriority,
    pub deadline: Instant,
    pub payload: T,
}

#[derive(Debug)]
pub struct TaskScheduler<T> {
    heap: BinaryHeap<HeapEntry<T>>,
    sequence: u64,
}

impl<T> TaskScheduler<T> {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            sequence: 0,
        }
    }

    pub fn schedule_at(&mut self, payload: T, deadline: Instant, priority: TaskPriority) {
        self.push_entry(payload, deadline, priority);
    }

    pub fn schedule_in(&mut self, payload: T, delay: Duration, priority: TaskPriority) {
        self.push_entry(payload, Instant::now() + delay, priority);
    }

    pub fn pop_ready(&mut self, now: Instant) -> Option<ScheduledItem<T>> {
        if matches!(self.heap.peek(), Some(head) if head.deadline <= now) {
            self.heap.pop().map(|entry| ScheduledItem {
                priority: entry.priority,
                deadline: entry.deadline,
                payload: entry.payload,
            })
        } else {
            None
        }
    }

    pub fn drain_ready(&mut self, now: Instant) -> Vec<ScheduledItem<T>> {
        let mut ready = Vec::new();
        while let Some(item) = self.pop_ready(now) {
            ready.push(item);
        }
        ready
    }

    pub fn peek_deadline(&self) -> Option<Instant> {
        self.heap.peek().map(|entry| entry.deadline)
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn clear(&mut self) {
        self.heap.clear();
    }

    fn push_entry(&mut self, payload: T, deadline: Instant, priority: TaskPriority) {
        self.sequence = self.sequence.wrapping_add(1);
        self.heap.push(HeapEntry {
            priority,
            deadline,
            payload,
            sequence: self.sequence,
        });
    }
}

impl<T> Default for TaskScheduler<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct HeapEntry<T> {
    priority: TaskPriority,
    deadline: Instant,
    payload: T,
    sequence: u64,
}

impl<T> PartialEq for HeapEntry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
            && self.deadline == other.deadline
            && self.sequence == other.sequence
    }
}

impl<T> Eq for HeapEntry<T> {}

impl<T> PartialOrd for HeapEntry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for HeapEntry<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.deadline.cmp(&self.deadline))
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_returns_highest_priority_then_deadline() {
        let mut scheduler = TaskScheduler::new();
        let base = Instant::now();
        scheduler.schedule_at("low", base + Duration::from_secs(5), TaskPriority::LOW);
        scheduler.schedule_at("high", base + Duration::from_secs(10), TaskPriority::HIGH);
        scheduler.schedule_at("urgent", base + Duration::from_secs(2), TaskPriority::HIGH);

        let ready = scheduler.pop_ready(base + Duration::from_secs(3));
        assert_eq!(ready.unwrap().payload, "urgent");
    }

    #[test]
    fn drain_ready_respects_deadlines() {
        let mut scheduler = TaskScheduler::new();
        let base = Instant::now();
        scheduler.schedule_at(1, base + Duration::from_millis(5), TaskPriority::LOW);
        scheduler.schedule_at(2, base + Duration::from_millis(10), TaskPriority::LOW);
        scheduler.schedule_at(3, base + Duration::from_millis(15), TaskPriority::LOW);

        let drained = scheduler.drain_ready(base + Duration::from_millis(12));
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].payload, 1);
        assert_eq!(drained[1].payload, 2);
    }

    #[test]
    fn schedule_in_uses_relative_deadline() {
        let mut scheduler = TaskScheduler::new();
        scheduler.schedule_in("soon", Duration::from_millis(5), TaskPriority::NORMAL);
        assert!(scheduler.peek_deadline().is_some());
        assert_eq!(scheduler.len(), 1);
    }
}
