// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_core::core::error::HarperResult;
use harper_core::core::io_traits::{Input, Output};
use harper_core::memory::session_service::SessionService;
use harper_core::memory::storage;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
struct MockInput {
    lines: Vec<String>,
    current: Mutex<usize>,
}

impl MockInput {
    #[allow(dead_code)]
    fn new(lines: Vec<String>) -> Self {
        Self {
            lines,
            current: Mutex::new(0),
        }
    }
}

impl Input for MockInput {
    fn read_line(&self) -> HarperResult<String> {
        let mut idx = self.current.lock().unwrap();
        if *idx < self.lines.len() {
            let line = self.lines[*idx].clone();
            *idx += 1;
            Ok(line)
        } else {
            Ok("exit".to_string())
        }
    }
}

#[allow(dead_code)]
struct MockOutput {
    buffer: Arc<Mutex<Vec<String>>>,
}

impl MockOutput {
    #[allow(dead_code)]
    fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Output for MockOutput {
    fn print(&self, text: &str) -> HarperResult<()> {
        self.buffer.lock().unwrap().push(text.to_string());
        Ok(())
    }

    fn println(&self, text: &str) -> HarperResult<()> {
        self.buffer.lock().unwrap().push(format!("{text}\n"));
        Ok(())
    }

    fn flush(&self) -> HarperResult<()> {
        Ok(())
    }
}

#[test]
fn test_session_listing() {
    let conn = Connection::open_in_memory().unwrap();
    storage::init_db(&conn).unwrap();

    storage::save_session(&conn, "session-1").unwrap();
    storage::save_session(&conn, "session-2").unwrap();

    let service = SessionService::new(&conn);
    let sessions = service.list_sessions_data().unwrap();

    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|s| s.id == "session-1"));
    assert!(sessions.iter().any(|s| s.id == "session-2"));
}

#[test]
fn test_session_deletion() {
    let conn = Connection::open_in_memory().unwrap();
    storage::init_db(&conn).unwrap();

    storage::save_session(&conn, "session-to-delete").unwrap();

    storage::delete_session(&conn, "session-to-delete").unwrap();

    let service = SessionService::new(&conn);
    let sessions = service.list_sessions_data().unwrap();
    assert_eq!(sessions.len(), 0);
}
