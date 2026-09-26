// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Database storage operations for conversation persistence
//!
//! This module provides functions for storing and retrieving chat sessions
//! and messages using SQLite as the backend.

use crate::core::Message;
use crate::core::agents::ResolvedAgents;
use crate::core::error::HarperResult;
use crate::core::plan::{PlanRuntime, PlanState};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Create a new database connection
pub fn create_connection(path: &str) -> HarperResult<Connection> {
    Connection::open(path).map_err(|e| crate::core::error::HarperError::Database(e.to_string()))
}

/// Initialize the database schema
///
/// Creates the necessary tables for storing sessions and messages if they don't exist.
///
/// # Arguments
/// * `conn` - SQLite database connection
///
/// # Errors
/// Returns `HarperError::Database` if table creation fails
pub fn init_db(conn: &Connection) -> HarperResult<()> {
    // Enable WAL mode for better concurrent access
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    // Enable foreign key constraints
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    // Migration: Drop tables with old foreign key constraints
    for table_name in &["messages", "command_logs"] {
        let has_table: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table_name],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            > 0;

        if has_table {
            let has_fk = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_foreign_key_list(?1)",
                    [table_name],
                    |row| row.get::<_, i32>(0),
                )
                .unwrap_or(0)
                > 0;

            if has_fk {
                if *table_name == "messages" {
                    conn.execute("DROP TABLE IF EXISTS messages", [])?;
                } else if *table_name == "command_logs" {
                    conn.execute("DROP TABLE IF EXISTS command_logs", [])?;
                }
            }
        }
    }

    conn.execute(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT,
            title TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;
    if !column_exists(conn, "sessions", "user_id")? {
        conn.execute("ALTER TABLE sessions ADD COLUMN user_id TEXT", [])?;
    }
    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             session_id TEXT,
             role TEXT,
             content TEXT,
             created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS todos (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             description TEXT NOT NULL,
             created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS command_logs (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             session_id TEXT,
             command TEXT NOT NULL,
             source TEXT NOT NULL,
             requires_approval INTEGER NOT NULL,
             approved INTEGER NOT NULL,
             status TEXT NOT NULL,
             exit_code INTEGER,
             duration_ms INTEGER,
             stdout_preview TEXT,
             stderr_preview TEXT,
             error_message TEXT,
             created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_command_logs_session_id ON command_logs(session_id)",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pending_tools (
             id TEXT PRIMARY KEY,
             session_id TEXT,
             tool TEXT NOT NULL,
             args TEXT NOT NULL,
             created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_plans (
             session_id TEXT PRIMARY KEY,
             explanation TEXT,
             items_json TEXT NOT NULL,
             updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_plan_runtime (
             session_id TEXT PRIMARY KEY,
             runtime_json TEXT NOT NULL,
             updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_plan_events (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             session_id TEXT NOT NULL,
             created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_plan_events_session_id_id
         ON session_plan_events(session_id, id)",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_agents (
             session_id TEXT PRIMARY KEY,
             sources_json TEXT NOT NULL,
             updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_tool_dedup (
             session_id TEXT NOT NULL,
             dedupe_key TEXT NOT NULL,
             created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
             PRIMARY KEY (session_id, dedupe_key)
         )",
        [],
    )?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> HarperResult<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;

    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Save a message to the database
///
/// Stores a message in the database associated with a specific session.
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `session_id` - Unique identifier for the chat session
/// * `role` - Role of the message sender (user, assistant, system)
/// * `content` - The message content
///
/// # Errors
/// Returns `HarperError::Database` if the insert operation fails
pub fn save_message(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: &str,
) -> HarperResult<()> {
    conn.execute(
        "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
        params![session_id, role, content],
    )?;
    Ok(())
}

/// Save a session to the database
///
/// Creates a new session record in the database.
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `session_id` - Unique identifier for the chat session
///
/// # Errors
/// Returns `HarperError::Database` if the insert operation fails
pub fn save_session(conn: &Connection, session_id: &str) -> HarperResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id) VALUES (?1)",
        params![session_id],
    )?;
    Ok(())
}

pub fn save_session_for_user(
    conn: &Connection,
    session_id: &str,
    user_id: &str,
) -> HarperResult<bool> {
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, user_id) VALUES (?1, ?2)",
        params![session_id, user_id],
    )?;

    let updated = conn.execute(
        "UPDATE sessions
         SET user_id = ?2, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1 AND (user_id IS NULL OR user_id = ?2)",
        params![session_id, user_id],
    )?;

    Ok(updated > 0)
}

pub fn session_belongs_to_user(
    conn: &Connection,
    session_id: &str,
    user_id: &str,
) -> HarperResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND user_id = ?2",
        params![session_id, user_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn update_session_title(conn: &Connection, session_id: &str, title: &str) -> HarperResult<()> {
    conn.execute(
        "UPDATE sessions SET title = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![session_id, title],
    )?;
    Ok(())
}

/// Load conversation history for a session
///
/// Retrieves all messages for a given session from the database,
/// ordered by creation time.
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `session_id` - Unique identifier for the chat session
///
/// # Returns
/// A vector of `Message` structs representing the conversation history
///
/// # Errors
/// Returns `HarperError::Database` if the query fails
pub fn load_history(conn: &Connection, session_id: &str) -> HarperResult<Vec<Message>> {
    let mut stmt =
        conn.prepare("SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id ASC")?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok(Message {
            role: row.get(0)?,
            content: row.get(1)?,
        })
    })?;

    let mut messages = Vec::new();
    for message in rows {
        messages.push(message?);
    }
    Ok(messages)
}

pub fn save_plan_state(conn: &Connection, session_id: &str, plan: &PlanState) -> HarperResult<()> {
    let items_json = serde_json::to_string(&plan.items)
        .map_err(|e| crate::core::error::HarperError::Database(e.to_string()))?;
    conn.execute(
        "INSERT INTO session_plans (session_id, explanation, items_json, updated_at)
         VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
         ON CONFLICT(session_id) DO UPDATE SET
             explanation = excluded.explanation,
             items_json = excluded.items_json,
             updated_at = CURRENT_TIMESTAMP",
        params![session_id, plan.explanation, items_json],
    )?;
    save_plan_runtime(conn, session_id, plan.runtime.as_ref())?;
    let event_id = insert_plan_event(conn, session_id)?;
    crate::core::plan_events::notify(event_id, session_id, Some(plan.clone()));
    if let Some(db_key) = database_key(conn) {
        crate::core::plan_events::notify_cross_process(&db_key, event_id, session_id);
    }
    Ok(())
}

pub fn delete_plan_state(conn: &Connection, session_id: &str) -> HarperResult<()> {
    conn.execute(
        "DELETE FROM session_plans WHERE session_id = ?1",
        params![session_id],
    )?;
    conn.execute(
        "DELETE FROM session_plan_runtime WHERE session_id = ?1",
        params![session_id],
    )?;
    let event_id = insert_plan_event(conn, session_id)?;
    crate::core::plan_events::notify(event_id, session_id, None);
    if let Some(db_key) = database_key(conn) {
        crate::core::plan_events::notify_cross_process(&db_key, event_id, session_id);
    }
    Ok(())
}

fn database_key(conn: &Connection) -> Option<String> {
    let mut stmt = conn.prepare("PRAGMA database_list").ok()?;
    let mut rows = stmt.query([]).ok()?;
    while let Ok(Some(row)) = rows.next() {
        let name: String = row.get(1).ok()?;
        let path: String = row.get(2).ok()?;
        if name == "main" && !path.trim().is_empty() {
            return Some(path);
        }
    }
    None
}

fn insert_plan_event(conn: &Connection, session_id: &str) -> HarperResult<i64> {
    conn.execute(
        "INSERT INTO session_plan_events (session_id) VALUES (?1)",
        params![session_id],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn load_latest_plan_event_id(conn: &Connection, session_id: &str) -> HarperResult<Option<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM session_plan_events WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
    )?;
    let mut rows = stmt.query(params![session_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn load_plan_state(conn: &Connection, session_id: &str) -> HarperResult<Option<PlanState>> {
    let mut stmt = conn.prepare(
        "SELECT explanation, items_json, updated_at FROM session_plans WHERE session_id = ?1",
    )?;
    let mut rows = stmt.query(params![session_id])?;

    if let Some(row) = rows.next()? {
        let explanation = row.get(0)?;
        let items_json: String = row.get(1)?;
        let updated_at = row.get(2)?;
        let runtime = load_plan_runtime(conn, session_id)?;
        let items = serde_json::from_str(&items_json)
            .map_err(|e| crate::core::error::HarperError::Database(e.to_string()))?;
        Ok(Some(PlanState {
            explanation,
            items,
            runtime,
            updated_at: Some(updated_at),
        }))
    } else {
        Ok(None)
    }
}

pub fn save_plan_runtime(
    conn: &Connection,
    session_id: &str,
    runtime: Option<&PlanRuntime>,
) -> HarperResult<()> {
    if let Some(runtime) = runtime {
        let runtime_json = serde_json::to_string(runtime)
            .map_err(|e| crate::core::error::HarperError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO session_plan_runtime (session_id, runtime_json, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(session_id) DO UPDATE SET
                 runtime_json = excluded.runtime_json,
                 updated_at = CURRENT_TIMESTAMP",
            params![session_id, runtime_json],
        )?;
    } else {
        conn.execute(
            "DELETE FROM session_plan_runtime WHERE session_id = ?1",
            params![session_id],
        )?;
    }
    Ok(())
}

pub fn load_plan_runtime(conn: &Connection, session_id: &str) -> HarperResult<Option<PlanRuntime>> {
    let mut stmt =
        conn.prepare("SELECT runtime_json FROM session_plan_runtime WHERE session_id = ?1")?;
    let mut rows = stmt.query(params![session_id])?;
    if let Some(row) = rows.next()? {
        let runtime_json: String = row.get(0)?;
        let runtime = serde_json::from_str(&runtime_json)
            .map_err(|e| crate::core::error::HarperError::Database(e.to_string()))?;
        Ok(Some(runtime))
    } else {
        Ok(None)
    }
}

pub fn save_active_agents(
    conn: &Connection,
    session_id: &str,
    agents: Option<&ResolvedAgents>,
) -> HarperResult<()> {
    if let Some(agents) = agents {
        let sources_json = serde_json::to_string(agents)
            .map_err(|e| crate::core::error::HarperError::Database(e.to_string()))?;
        conn.execute(
            "INSERT INTO session_agents (session_id, sources_json, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(session_id) DO UPDATE SET
                 sources_json = excluded.sources_json,
                 updated_at = CURRENT_TIMESTAMP",
            params![session_id, sources_json],
        )?;
    } else {
        conn.execute(
            "DELETE FROM session_agents WHERE session_id = ?1",
            params![session_id],
        )?;
    }
    Ok(())
}

pub fn load_active_agents(
    conn: &Connection,
    session_id: &str,
) -> HarperResult<Option<ResolvedAgents>> {
    let mut stmt = conn.prepare("SELECT sources_json FROM session_agents WHERE session_id = ?1")?;
    let mut rows = stmt.query(params![session_id])?;
    if let Some(row) = rows.next()? {
        let sources_json: String = row.get(0)?;
        let agents = serde_json::from_str(&sources_json)
            .map_err(|e| crate::core::error::HarperError::Database(e.to_string()))?;
        Ok(Some(agents))
    } else {
        Ok(None)
    }
}

/// List all session IDs in the database
///
/// Retrieves all session IDs from the sessions table.
///
/// # Arguments
/// * `conn` - SQLite database connection
///
/// # Returns
/// A vector of session IDs as strings
///
/// # Errors
/// Returns `HarperError::Database` if the query fails
#[allow(dead_code)]
pub fn list_sessions(conn: &Connection) -> HarperResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM sessions ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], |row| row.get(0))?;

    let mut sessions = Vec::new();
    for session in rows {
        sessions.push(session?);
    }
    Ok(sessions)
}

/// Delete all messages for a specific session
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `session_id` - ID of the session to delete messages for
///
/// # Errors
/// Returns `HarperError::Database` if the delete operation fails
#[allow(dead_code)]
pub fn delete_messages(conn: &Connection, session_id: &str) -> HarperResult<()> {
    conn.execute("DELETE FROM messages WHERE session_id = ?", [session_id])?;
    Ok(())
}

#[derive(Debug)]
pub struct PendingToolRecord {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub args: String,
}

pub fn insert_pending_tool(
    conn: &Connection,
    id: &str,
    session_id: &str,
    tool: &str,
    args: &str,
) -> HarperResult<()> {
    conn.execute(
        "INSERT INTO pending_tools (id, session_id, tool, args) VALUES (?1, ?2, ?3, ?4)",
        params![id, session_id, tool, args],
    )?;
    Ok(())
}

pub fn get_pending_tools(
    conn: &Connection,
    session_id: &str,
) -> HarperResult<Vec<PendingToolRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, tool, args FROM pending_tools WHERE session_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok(PendingToolRecord {
            id: row.get(0)?,
            session_id: row.get(1)?,
            tool: row.get(2)?,
            args: row.get(3)?,
        })
    })?;

    let mut tools = Vec::new();
    for row in rows {
        tools.push(row?);
    }
    Ok(tools)
}

pub fn get_pending_tool(conn: &Connection, id: &str) -> HarperResult<Option<PendingToolRecord>> {
    let mut stmt =
        conn.prepare("SELECT id, session_id, tool, args FROM pending_tools WHERE id = ?1")?;
    let mut rows = stmt.query(params![id])?;

    if let Some(row) = rows.next()? {
        Ok(Some(PendingToolRecord {
            id: row.get(0)?,
            session_id: row.get(1)?,
            tool: row.get(2)?,
            args: row.get(3)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn delete_pending_tool(conn: &Connection, id: &str) -> HarperResult<()> {
    conn.execute("DELETE FROM pending_tools WHERE id = ?1", params![id])?;
    Ok(())
}

/// Delete a session and all its messages
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `session_id` - ID of the session to delete
///
/// # Errors
/// Returns `HarperError::Database` if the delete operation fails
#[allow(dead_code)]
pub fn delete_session(conn: &Connection, session_id: &str) -> HarperResult<()> {
    // First delete all messages for this session
    delete_messages(conn, session_id)?;
    conn.execute(
        "DELETE FROM session_tool_dedup WHERE session_id = ?",
        [session_id],
    )?;

    // Then delete the session itself
    conn.execute("DELETE FROM sessions WHERE id = ?", [session_id])?;

    Ok(())
}

/// Save a todo to the database
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `description` - The todo description
///
/// # Errors
/// Returns `HarperError::Database` if the insert operation fails
pub fn save_todo(conn: &Connection, description: &str) -> HarperResult<()> {
    conn.execute(
        "INSERT INTO todos (description) VALUES (?1)",
        params![description],
    )?;
    Ok(())
}

/// Load all todos from the database
///
/// # Arguments
/// * `conn` - SQLite database connection
///
/// # Returns
/// A vector of (id, description) tuples
///
/// # Errors
/// Returns `HarperError::Database` if the query fails
pub fn load_todos(conn: &Connection) -> HarperResult<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT id, description FROM todos ORDER BY id ASC")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;

    let todos = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(todos)
}

/// Delete a todo by ID
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `id` - The todo ID to delete
///
/// # Errors
/// Returns `HarperError::Database` if the delete operation fails
pub fn delete_todo(conn: &Connection, id: i64) -> HarperResult<()> {
    conn.execute("DELETE FROM todos WHERE id = ?", [id])?;
    Ok(())
}

/// Clear all todos
///
/// # Arguments
/// * `conn` - SQLite database connection
///
/// # Returns
/// Returns the number of todos that were cleared
///
/// # Errors
/// Returns `HarperError::Database` if the delete operation fails
pub fn clear_todos(conn: &Connection) -> HarperResult<usize> {
    let count = conn.execute("DELETE FROM todos", [])?;
    Ok(count)
}

/// Persisted record of a command execution attempt
#[derive(Debug, Clone)]
pub struct CommandLogRecord {
    pub session_id: Option<String>,
    pub command: String,
    pub source: String,
    pub requires_approval: bool,
    pub approved: bool,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub stdout_preview: Option<String>,
    pub stderr_preview: Option<String>,
    pub error_message: Option<String>,
}

impl CommandLogRecord {
    #[allow(clippy::too_many_arguments)]
    /// Helper constructor for building log entries from borrowed data
    pub fn new(
        session_id: Option<&str>,
        command: &str,
        source: &str,
        requires_approval: bool,
        approved: bool,
        status: &str,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
        stdout_preview: Option<String>,
        stderr_preview: Option<String>,
        error_message: Option<String>,
    ) -> Self {
        Self {
            session_id: session_id.map(|s| s.to_string()),
            command: command.to_string(),
            source: source.to_string(),
            requires_approval,
            approved,
            status: status.to_string(),
            exit_code,
            duration_ms,
            stdout_preview,
            stderr_preview,
            error_message,
        }
    }
}

/// Insert a command log entry into the audit table
/// Load the persisted tool-call dedupe keys for a session.
pub fn load_session_tool_dedup_keys(
    conn: &Connection,
    session_id: &str,
) -> HarperResult<HashSet<String>> {
    let mut stmt =
        conn.prepare("SELECT dedupe_key FROM session_tool_dedup WHERE session_id = ?1")?;
    let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row?);
    }
    Ok(keys)
}

/// Persist a tool-call dedupe key for a session (idempotent).
pub fn insert_session_tool_dedup_key(
    conn: &Connection,
    session_id: &str,
    key: &str,
) -> HarperResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO session_tool_dedup (session_id, dedupe_key) VALUES (?1, ?2)",
        params![session_id, key],
    )?;
    Ok(())
}

pub fn insert_command_log(conn: &Connection, record: &CommandLogRecord) -> HarperResult<()> {
    conn.execute(
        "INSERT INTO command_logs (
             session_id,
             command,
             source,
             requires_approval,
             approved,
             status,
             exit_code,
             duration_ms,
             stdout_preview,
             stderr_preview,
             error_message
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            record.session_id,
            record.command,
            record.source,
            record.requires_approval as i32,
            record.approved as i32,
            record.status,
            record.exit_code,
            record.duration_ms,
            record.stdout_preview,
            record.stderr_preview,
            record.error_message,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::plan::{PlanItem, PlanState, PlanStepStatus};

    #[test]
    fn init_db_adds_session_user_id_column() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        init_db(&conn).expect("db init");

        assert!(column_exists(&conn, "sessions", "user_id").expect("column lookup"));
    }

    #[test]
    fn save_session_for_user_claims_once_and_blocks_reassignment() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        init_db(&conn).expect("db init");

        assert!(save_session_for_user(&conn, "session-1", "user-a").expect("first claim"));
        assert!(session_belongs_to_user(&conn, "session-1", "user-a").expect("ownership lookup"));
        assert!(!save_session_for_user(&conn, "session-1", "user-b").expect("second claim denied"));
        assert!(
            !session_belongs_to_user(&conn, "session-1", "user-b").expect("other owner lookup")
        );
    }

    #[test]
    fn save_plan_state_records_plan_event_id() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        init_db(&conn).expect("db init");

        save_plan_state(
            &conn,
            "session-a",
            &PlanState {
                explanation: Some("Track work".to_string()),
                items: vec![PlanItem {
                    step: "Inspect".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        assert_eq!(
            load_latest_plan_event_id(&conn, "session-a").expect("latest event id"),
            Some(1)
        );
    }

    #[test]
    fn session_tool_dedup_roundtrip_is_scoped_and_idempotent() {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        init_db(&conn).expect("db init");

        insert_session_tool_dedup_key(&conn, "session-a", "run_command:cargo test")
            .expect("first insert");
        insert_session_tool_dedup_key(&conn, "session-a", "run_command:cargo test")
            .expect("second insert");
        insert_session_tool_dedup_key(&conn, "session-a", "read_file:src/lib.rs")
            .expect("third insert");
        insert_session_tool_dedup_key(&conn, "session-b", "run_command:cargo test")
            .expect("other session insert");

        let keys_a = load_session_tool_dedup_keys(&conn, "session-a").expect("load session-a");
        assert_eq!(keys_a.len(), 2);
        assert!(keys_a.contains("run_command:cargo test"));
        assert!(keys_a.contains("read_file:src/lib.rs"));

        let keys_b = load_session_tool_dedup_keys(&conn, "session-b").expect("load session-b");
        assert_eq!(keys_b.len(), 1);
        assert!(keys_b.contains("run_command:cargo test"));

        delete_session(&conn, "session-a").expect("delete session-a");
        assert!(
            load_session_tool_dedup_keys(&conn, "session-a")
                .expect("load after delete")
                .is_empty()
        );
        assert_eq!(
            load_session_tool_dedup_keys(&conn, "session-b")
                .expect("load session-b after delete")
                .len(),
            1
        );
    }
}

/// Simplified view of an audit record for presentation
#[derive(Debug, Clone)]
pub struct CommandLogEntry {
    pub command: String,
    pub source: String,
    pub requires_approval: bool,
    pub approved: bool,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub created_at: String,
}

/// Pending approval entry for the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingCommandLogEntry {
    pub id: i64,
    pub command: String,
    pub source: String,
    pub status: String,
    pub created_at: String,
}

/// Load recent command logs for a given session
pub fn load_command_logs_for_session(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> HarperResult<Vec<CommandLogEntry>> {
    let mut stmt = conn.prepare(
        "SELECT command,
                source,
                requires_approval,
                approved,
                status,
                exit_code,
                duration_ms,
                created_at
         FROM command_logs
         WHERE session_id = ?1
         ORDER BY id DESC
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![session_id, limit as i64], |row| {
        Ok(CommandLogEntry {
            command: row.get(0)?,
            source: row.get(1)?,
            requires_approval: row.get::<_, i32>(2)? != 0,
            approved: row.get::<_, i32>(3)? != 0,
            status: row.get(4)?,
            exit_code: row.get(5)?,
            duration_ms: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

/// Update approval status for a command log entry
pub fn update_command_log_approval(
    conn: &Connection,
    session_id: &str,
    approved: bool,
) -> HarperResult<()> {
    conn.execute(
        "UPDATE command_logs SET approved = ?1, status = ?2 WHERE session_id = ?3 AND status = 'pending' AND requires_approval = 1 ORDER BY id DESC LIMIT 1",
        params![approved as i32, if approved { "completed" } else { "rejected" }, session_id],
    )?;
    Ok(())
}

/// Update approval status by ID
pub fn update_command_log_approval_by_id(
    conn: &Connection,
    id: i64,
    approved: bool,
) -> HarperResult<()> {
    conn.execute(
        "UPDATE command_logs SET approved = ?1, status = ?2 WHERE id = ?3 AND status = 'pending' AND requires_approval = 1",
        params![approved as i32, if approved { "completed" } else { "rejected" }, id],
    )?;
    Ok(())
}

/// Load pending command logs requiring approval for a session
pub fn load_pending_command_logs(
    conn: &Connection,
    session_id: &str,
) -> HarperResult<Vec<PendingCommandLogEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, command, source, status, created_at
         FROM command_logs
         WHERE session_id = ?1 AND status = 'pending' AND requires_approval = 1
         ORDER BY id DESC",
    )?;

    let rows = stmt.query_map(params![session_id], |row| {
        Ok(PendingCommandLogEntry {
            id: row.get(0)?,
            command: row.get(1)?,
            source: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }
    Ok(entries)
}

/// Fetch the most recent command log entry for a session
pub fn load_latest_command_log(
    conn: &Connection,
    session_id: &str,
) -> HarperResult<Option<CommandLogEntry>> {
    let mut stmt = conn.prepare(
        "SELECT command,
                source,
                requires_approval,
                approved,
                status,
                exit_code,
                duration_ms,
                created_at
         FROM command_logs
         WHERE session_id = ?1
         ORDER BY id DESC
         LIMIT 1",
    )?;

    let result = stmt.query_row(params![session_id], |row| {
        Ok(CommandLogEntry {
            command: row.get(0)?,
            source: row.get(1)?,
            requires_approval: row.get::<_, i32>(2)? != 0,
            approved: row.get::<_, i32>(3)? != 0,
            status: row.get(4)?,
            exit_code: row.get(5)?,
            duration_ms: row.get(6)?,
            created_at: row.get(7)?,
        })
    });

    match result {
        Ok(entry) => Ok(Some(entry)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
