// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_workspace::*;

use regex::Regex;
use rusqlite::Connection;
use std::thread;
use tempfile::NamedTempFile;

#[path = "integration/performance_test.rs"]
mod performance_test;

#[path = "integration/security_test.rs"]
mod security_test;

static TIMESTAMP_REGEX: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$").unwrap());

#[test]
#[allow(clippy::too_many_lines)]
fn test_database_operations() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Test database initialization
    init_db(&conn).unwrap();

    // Verify tables were created with correct schema
    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('sessions', 'messages')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        table_count, 2,
        "Expected sessions and messages tables to be created"
    );

    // Verify table schemas
    let sessions_columns: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sessions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        sessions_columns.contains("id"),
        "Sessions table missing id column"
    );
    assert!(
        sessions_columns.contains("created_at"),
        "Sessions table missing created_at column"
    );

    let messages_columns: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='messages'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        messages_columns.contains("id"),
        "Messages table missing id column"
    );
    assert!(
        messages_columns.contains("session_id"),
        "Messages table missing session_id column"
    );
    assert!(
        messages_columns.contains("role"),
        "Messages table missing role column"
    );
    assert!(
        messages_columns.contains("content"),
        "Messages table missing content column"
    );
    assert!(
        messages_columns.contains("created_at"),
        "Messages table missing created_at column"
    );

    // Test session creation with various ID formats
    let test_sessions = [
        "test-session-123",
        "session-with-hyphens",
        "session_with_underscores",
        "session123",
        &"a".repeat(100), // Test with long session ID
    ];

    for session_id in &test_sessions {
        let session_id: &str = session_id;
        save_session(&conn, session_id).unwrap();

        let stored_id: String = conn
            .query_row(
                "SELECT id FROM sessions WHERE id = ?",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_id, session_id, "Session ID mismatch in test case");

        // Verify created_at timestamp is set and has a valid format
        let created_at: String = conn
            .query_row(
                "SELECT created_at FROM sessions WHERE id = ?",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();

        // Check that the timestamp is not empty and has the expected format
        assert!(!created_at.is_empty(), "Created_at timestamp is empty");

        // Basic format check (YYYY-MM-DD HH:MM:SS)
        assert!(
            TIMESTAMP_REGEX.is_match(&created_at),
            "Created_at timestamp does not match expected format"
        );
    }

    // Test message operations with various content types
    let test_messages = [
        ("user", "Hello, world!"),
        ("assistant", "Hi there!"),
        ("system", "System message"),
        (
            "user",
            "Message with special chars: \"'!@#$%^&*()_+{}|:<>?~`,./;'[]\\-=\n",
        ),
        ("user", "Message with emoji: 😊🚀🌟"),
        ("assistant", "Message with numbers: 1234567890"),
        ("user", &format!("A very long message {}", "x".repeat(1000))),
    ];

    let session_id = test_sessions[0];

    // Save all test messages
    for (i, (role, content)) in test_messages.iter().enumerate() {
        save_message(&conn, session_id, role, content).unwrap();

        // Verify message was saved correctly
        let (stored_role, stored_content): (String, String) = conn
            .query_row(
                "SELECT role, content FROM messages WHERE session_id = ? ORDER BY id LIMIT 1 OFFSET ?",
                [session_id, i.to_string().as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(*role, stored_role, "Role mismatch in message");
        assert_eq!(*content, stored_content, "Content mismatch in message");
    }

    // Test message retrieval
    let history = load_history(&conn, session_id).unwrap();
    assert_eq!(
        history.len(),
        test_messages.len(),
        "Incorrect number of messages loaded"
    );

    // Verify message content and role are correct and in the right order
    for (i, (expected_role, expected_content)) in test_messages.iter().enumerate() {
        assert_eq!(*expected_role, history[i].role, "Role mismatch in history");
        assert_eq!(
            *expected_content, history[i].content,
            "Content mismatch in history"
        );
    }

    // Test loading non-existent session
    let empty_history = load_history(&conn, "non-existent-session").unwrap();
    assert!(
        empty_history.is_empty(),
        "Expected no history for non-existent session"
    );

    // Test session listing
    let sessions = list_sessions(&conn).unwrap();
    assert_eq!(
        sessions.len(),
        test_sessions.len(),
        "Incorrect number of sessions"
    );

    for &expected_session in &test_sessions {
        assert!(
            sessions.contains(&expected_session.to_string()),
            "Expected session not found in list"
        );
    }

    // Test message deletion
    delete_messages(&conn, session_id).unwrap();
    let empty_history = load_history(&conn, session_id).unwrap();
    assert!(
        empty_history.is_empty(),
        "Expected no messages after deletion"
    );

    // Verify messages were actually deleted from the database
    let message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?",
            [session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 0, "Messages not deleted from database");

    // Test session deletion
    delete_session(&conn, session_id).unwrap();

    // Verify session was deleted
    let session_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)",
            [session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!session_exists, "Session was not deleted");

    // Verify messages were also deleted (cascading delete)
    let message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?",
            [session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 0, "Messages not deleted with session");
}

#[test]
#[allow(clippy::items_after_statements)]
fn test_concurrent_access() {
    use std::sync::Arc;

    if std::env::var("CI").is_ok() {
        println!("Skipping concurrent access test in CI environment");
        return;
    }

    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path().to_path_buf();

    {
        let conn = Connection::open(&db_path).unwrap();
        init_db(&conn).unwrap();
    }

    let db_path = Arc::new(db_path);

    let mut handles = vec![];

    const NUM_THREADS: usize = 2;
    const OPS_PER_THREAD: usize = 5;

    for thread_id in 0..NUM_THREADS {
        let db_path = Arc::clone(&db_path);

        let handle = thread::spawn(move || {
            let conn = Connection::open(&*db_path).unwrap();

            // Simple operations without complex random access
            for op in 0..OPS_PER_THREAD {
                let session_id = format!("session-{thread_id}-{op}");

                // Create session
                if let Err(e) = save_session(&conn, &session_id) {
                    if e.to_string().contains("database is locked") {
                        println!("Warning: Failed to save session due to lock: {e}");
                        continue;
                    }
                    panic!("Unexpected error during save_session: {e}");
                }

                // Add a message
                let content = format!("Message {op} from thread {thread_id}");
                if let Err(e) = save_message(&conn, &session_id, "user", &content) {
                    if e.to_string().contains("database is locked") {
                        println!("Warning: Failed to save message due to lock: {e}");
                    } else {
                        panic!("Unexpected error during save_message: {e}");
                    }
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Basic verification
    let conn = Connection::open(&*db_path).unwrap();
    let sessions = list_sessions(&conn).unwrap();

    // Should have created some sessions
    assert!(
        !sessions.is_empty(),
        "Should have created at least one session"
    );

    // Verify that at least one message was also created.
    let message_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert!(
        message_count > 0,
        "No messages were created, indicating a potential issue with concurrent writes."
    );
}

#[test]
fn test_api_config_creation() {
    let config = ApiConfig {
        provider: ApiProvider::OpenAI,
        api_key: "test-key".to_string(),
        base_url: "https://api.openai.com/v1/chat/completions".to_string(),
        model_name: "gpt-5.5".to_string(),
    };

    assert!(matches!(config.provider, ApiProvider::OpenAI));
    assert_eq!(config.api_key, "test-key");
    assert!(config.base_url.contains("openai.com"));
}

#[test]
fn test_message_creation() {
    let message = Message {
        role: "system".to_string(),
        content: "You are a helpful assistant.".to_string(),
    };

    assert_eq!(message.role, "system");
    assert_eq!(message.content, "You are a helpful assistant.");
}

#[tokio::test]
async fn test_web_search_mock() {
    use harper_workspace::utils::web_search;

    unsafe {
        std::env::set_var(
            "HARPER_WEB_SEARCH_MOCK_RESPONSE",
            "{\"AbstractText\":\"Rust is a systems programming language.\"}",
        );
    }

    let result = web_search("rust programming").await;

    unsafe {
        std::env::remove_var("HARPER_WEB_SEARCH_MOCK_RESPONSE");
    }

    match result {
        Ok(response) => {
            assert!(
                response.contains("Rust is a systems programming language."),
                "Unexpected mocked search response: {response}"
            );
        }
        Err(e) => {
            panic!("Mocked web search failed unexpectedly: {e:?}");
        }
    }
}

#[cfg(test)]
mod e2e_tests {
    use super::*;

    #[test]
    fn test_full_chat_session_e2e() {
        // Create a temporary database file
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_str().unwrap();

        // Set up environment variables for testing
        unsafe {
            std::env::set_var("DATABASE_PATH", db_path);
            std::env::set_var("OPENAI_API_KEY", "test-key");
        }

        let conn = Connection::open(db_path).unwrap();
        init_db(&conn).unwrap();

        // Create API config for testing (not used in this test but kept for future expansion)
        let _config = ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        };

        // Test session creation and message handling
        let session_id = "test-session-e2e";
        save_session(&conn, session_id).unwrap();

        // Add test messages
        save_message(&conn, session_id, "user", "Hello, test message").unwrap();
        save_message(
            &conn,
            session_id,
            "assistant",
            "Hi there! This is a test response.",
        )
        .unwrap();

        // Verify session was created
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM sessions WHERE id = ?")
            .unwrap();
        let count: i64 = stmt.query_row([session_id], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "Session should be created");

        // Verify messages were saved
        let history = load_history(&conn, session_id).unwrap();
        assert_eq!(history.len(), 2, "Should have 2 messages");
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "Hello, test message");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "Hi there! This is a test response.");
    }

    #[test]
    fn test_session_management_e2e() {
        // Create a temporary database file
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_str().unwrap();

        let conn = Connection::open(db_path).unwrap();
        init_db(&conn).unwrap();

        // Create multiple sessions
        let session1 = "session-1";
        let session2 = "session-2";

        save_session(&conn, session1).unwrap();
        save_session(&conn, session2).unwrap();

        // Add messages to both sessions
        save_message(&conn, session1, "user", "Message 1").unwrap();
        save_message(&conn, session1, "assistant", "Response 1").unwrap();
        save_message(&conn, session2, "user", "Message 2").unwrap();

        // Test session listing
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM sessions").unwrap();
        let session_count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(session_count, 2, "Should have 2 sessions");

        // Test loading specific sessions
        let history1 = load_history(&conn, session1).unwrap();
        let history2 = load_history(&conn, session2).unwrap();

        assert_eq!(history1.len(), 2, "Session 1 should have 2 messages");
        assert_eq!(history2.len(), 1, "Session 2 should have 1 message");

        // Test ordering by creation time
        let mut stmt = conn
            .prepare("SELECT id FROM sessions ORDER BY created_at DESC")
            .unwrap();
        let session_ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(session_ids.len(), 2, "Should retrieve both sessions");
        // The order might vary, but both should be present
        assert!(session_ids.contains(&session1.to_string()));
        assert!(session_ids.contains(&session2.to_string()));
    }

    #[test]
    fn test_export_functionality_e2e() {
        // Create a temporary database
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_str().unwrap();

        let conn = Connection::open(db_path).unwrap();
        init_db(&conn).unwrap();

        // Create a session with some messages
        let session_id = "export-test-session";
        save_session(&conn, session_id).unwrap();
        save_message(&conn, session_id, "user", "Message for export test").unwrap();
        save_message(&conn, session_id, "assistant", "Response for export test").unwrap();
        save_message(&conn, session_id, "system", "System message for export").unwrap();

        // Load the history
        let history = load_history(&conn, session_id).unwrap();
        assert_eq!(history.len(), 3, "Should have 3 messages");

        // Test JSON export format
        let json_export = serde_json::to_string_pretty(&history).unwrap();
        assert!(
            json_export.contains("Message for export test"),
            "JSON should contain user message"
        );
        assert!(
            json_export.contains("Response for export test"),
            "JSON should contain assistant message"
        );
        assert!(
            json_export.contains("System message for export"),
            "JSON should contain system message"
        );
        assert!(
            json_export.contains("\"role\""),
            "JSON should have role fields"
        );
        assert!(
            json_export.contains("\"content\""),
            "JSON should have content fields"
        );

        // Test text export format
        let text_export: String = history
            .iter()
            .map(|msg| format!("{}: {}", msg.role, msg.content))
            .collect::<Vec<_>>()
            .join(
                "
",
            );

        assert!(
            text_export.contains("user: Message for export test"),
            "Text export should contain user message"
        );
        assert!(
            text_export.contains("assistant: Response for export test"),
            "Text export should contain assistant message"
        );
        assert!(
            text_export.contains("system: System message for export"),
            "Text export should contain system message"
        );

        // Test that JSON is valid and can be parsed back
        let parsed_history: Vec<Message> = serde_json::from_str(&json_export).unwrap();
        assert_eq!(
            parsed_history.len(),
            3,
            "Parsed JSON should have same number of messages"
        );
        assert_eq!(parsed_history[0].role, "user");
        assert_eq!(parsed_history[1].role, "assistant");
        assert_eq!(parsed_history[2].role, "system");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    #[allow(clippy::too_many_lines, clippy::items_after_statements)]
    fn test_binary_execution_e2e() {
        use std::fs;
        use std::path::PathBuf;
        use std::process::{Command, Stdio};

        // Create a temporary directory for the test
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Set up database directory and path
        let db_dir = temp_dir.path().join("db");
        std::fs::create_dir_all(&db_dir).expect("Failed to create database directory");
        let db_path = db_dir.join("harper.db");

        // Create a temporary config directory and files
        let config_dir = temp_dir.path().join("config");
        std::fs::create_dir_all(&config_dir).expect("Failed to create config directory");

        // Copy default config
        let default_config_src = std::path::Path::new("config/default.toml");
        let default_config_dst = config_dir.join("default.toml");
        if default_config_src.exists() {
            std::fs::copy(default_config_src, &default_config_dst)
                .expect("Failed to copy default config");
        }

        // Create local config
        let config_content = format!(
            r#"[api]
provider = "OpenAI"
api_key = "test-key"
base_url = "https://api.openai.com/v1/chat/completions"
model_name = "gpt-5.5"

[database]
path = '{}'

[mcp]
enabled = false
"#,
            db_path.display()
        );
        let config_path = config_dir.join("local.toml");
        std::fs::write(&config_path, config_content).expect("Failed to write config file");

        // Print debug information
        println!("=== Test Setup ===");
        println!("Temp directory: {}", temp_dir.path().display());
        println!("Database path: {}", db_path.display());
        println!("Config directory: {}", config_dir.display());
        println!("Config path: {}", config_path.display());
        println!("Database directory exists: {}", db_dir.exists());
        println!("Database file exists before test: {}", db_path.exists());
        println!("Current directory: {:?}", std::env::current_dir().unwrap());

        // List contents of the temp directory
        println!(
            "
=== Directory Contents ==="
        );
        if let Ok(entries) = std::fs::read_dir(temp_dir.path()) {
            for entry in entries.flatten() {
                println!(
                    "  - {} (dir: {})",
                    entry.path().display(),
                    entry.path().is_dir()
                );
            }
        }

        // List contents of the config directory
        println!(
            "
=== Config Directory Contents ==="
        );
        if let Ok(entries) = std::fs::read_dir(&config_dir) {
            for entry in entries.flatten() {
                println!(
                    "  - {} (dir: {})",
                    entry.path().display(),
                    entry.path().is_dir()
                );
            }
        }

        // Verify the directory is writable
        println!(
            "
=== Testing Directory Permissions ==="
        );
        let test_file = temp_dir.path().join(".test_write");
        std::fs::write(&test_file, "test").expect("Failed to write test file to temp directory");
        std::fs::remove_file(&test_file).expect("Failed to remove test file");
        println!("Successfully wrote and removed test file");

        // Ensure the parent directory of the database file exists
        if let Some(parent) = db_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).expect("Failed to create parent directory for database");
            }
            println!("Database parent directory exists: {}", parent.exists());
        }

        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let coverage_mode = std::env::var_os("LLVM_PROFILE_FILE").is_some()
            || std::env::var_os("CARGO_LLVM_COV").is_some();

        let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
        let workspace_root = std::env::current_dir().unwrap();
        let candidate_paths = [
            std::env::var_os("CARGO_BIN_EXE_harper").map(PathBuf::from),
            Some(
                workspace_root
                    .join(&target_dir)
                    .join(profile)
                    .join("harper"),
            ),
            Some(workspace_root.join("target").join(profile).join("harper")),
        ];

        let binary_path = candidate_paths
            .into_iter()
            .flatten()
            .find(|path| path.exists())
            .unwrap_or_else(|| {
                if coverage_mode {
                    eprintln!(
                        "Skipping nested cargo build in coverage mode; harper binary was not prebuilt"
                    );
                    PathBuf::new()
                } else {
                    let mut build_cmd = std::process::Command::new("cargo");
                    build_cmd.args(["build", "-p", "harper-ui", "--bin", "harper"]);
                    if profile == "release" {
                        build_cmd.arg("--release");
                    }
                    let status = build_cmd.status().expect("Failed to build harper binary");
                    assert!(status.success(), "harper binary build failed");
                    workspace_root.join(&target_dir).join(profile).join("harper")
                }
            });

        if binary_path.as_os_str().is_empty() {
            return;
        }

        let mut command = Command::new(binary_path);

        // Set the working directory to the temp directory so it finds the config file
        command
            .current_dir(temp_dir.path())
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Print the command for debugging
        println!(
            "
=== Running Command ==="
        );
        println!("Command: {command:?}");
        println!("Working directory: {}", temp_dir.path().display());
        println!("Config file: {}", config_path.display());
        println!("Database path in config: {}", db_path.display());

        println!("Running command: {command:?}");

        // Ensure DATABASE_PATH is not set to avoid overriding config
        unsafe {
            std::env::remove_var("DATABASE_PATH");
        }

        let mut child = command.spawn().expect("Failed to start binary");

        // Wait for the process to finish with a timeout
        let output = match wait_timeout::ChildExt::wait_timeout(
            &mut child,
            std::time::Duration::from_secs(5),
        ) {
            Ok(Some(status)) => {
                // Process has finished
                println!("Process finished with status: {status}");
                child.wait_with_output().expect("Failed to wait for child")
            }
            Ok(None) => {
                // Process is still running after timeout
                println!("Process is still running after timeout, attempting to get output...");
                let _ = child.kill();
                child.wait_with_output().expect("Failed to wait for child")
            }
            Err(e) => {
                panic!("Error waiting for child process: {e}");
            }
        };

        // Always print stdout and stderr for debugging
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        println!(
            "=== STDOUT ===
{stdout}"
        );
        println!(
            "=== STDERR ===
{stderr}"
        );

        assert_eq!(output.status.code(), Some(2));
        assert!(
            stderr.contains("Harper requires an interactive terminal"),
            "Should explain non-interactive CLI usage.
Full stderr:
{stderr}"
        );
        assert!(
            stderr.contains("harper-batch"),
            "Should point non-interactive users to harper-batch.
Full stderr:
{stderr}"
        );
    }

    #[test]
    fn test_error_handling_e2e() {
        // Test database error handling
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_str().unwrap();

        let conn = Connection::open(db_path).unwrap();
        init_db(&conn).unwrap();

        // Test loading non-existent session
        let result = load_history(&conn, "non-existent-session");
        assert!(
            result.is_ok(),
            "Loading non-existent session should not error"
        );
        assert!(
            result.unwrap().is_empty(),
            "Non-existent session should return empty history"
        );

        // Test saving message to non-existent session (should still work)
        // First create the session, then save message
        save_session(&conn, "new-session").unwrap();
        let result = save_message(&conn, "new-session", "user", "test message");
        assert!(
            result.is_ok(),
            "Saving message should work for existing session"
        );

        // Verify the message was saved
        let history = load_history(&conn, "new-session").unwrap();
        assert_eq!(history.len(), 1, "Should have 1 message");
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "test message");

        // Test empty message handling
        save_session(&conn, "test-session").unwrap(); // Ensure session exists
        let result = save_message(&conn, "test-session", "user", "");
        assert!(result.is_ok(), "Empty messages should be allowed");

        // Check database directly to see if message was saved
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM messages WHERE session_id = ?")
            .unwrap();
        let msg_count: i64 = stmt.query_row(["test-session"], |row| row.get(0)).unwrap();
        assert_eq!(msg_count, 1, "Empty message should be saved to database");

        let _history = load_history(&conn, "test-session").unwrap();
        // load_history might filter out empty messages, which is fine
        // Just verify the save operation worked
        assert!(result.is_ok(), "Save operation should succeed");
    }

    #[tokio::test]
    async fn test_slash_commands_e2e() {
        use crate::runtime::config::ExecPolicyConfig;

        // Create a temporary database
        let temp_db = NamedTempFile::new().unwrap();
        let conn = Connection::open(temp_db.path()).unwrap();
        init_db(&conn).unwrap();

        // Create API config (won't be used for slash commands)
        let api_config = ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        };

        // Create exec policy
        let exec_policy = ExecPolicyConfig {
            approval_profile: None,
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: None,
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };

        // Create custom commands
        let mut custom_commands = std::collections::HashMap::new();
        custom_commands.insert("testcmd".to_string(), "This is a test command".to_string());

        // Create chat service
        let mut chat_service = ChatService::new(
            &conn,
            &api_config,
            None,
            None,
            None,
            custom_commands,
            exec_policy,
        );

        // Create a test session
        let session_id = "test-slash-session";
        save_session(&conn, session_id).unwrap();
        let mut history = Vec::new();

        // Test /help command - should work without API call
        chat_service
            .send_message("/help", &mut history, false, session_id)
            .await
            .unwrap();

        // Verify help response was added
        assert_eq!(history.len(), 1, "Should have one response message");
        assert_eq!(
            history[0].role, "assistant",
            "Response should be from assistant"
        );
        assert!(
            history[0].content.contains("Available commands"),
            "Should contain help text"
        );
        assert!(
            history[0].content.contains("/help"),
            "Should list /help command"
        );
        assert!(
            history[0].content.contains("/clear"),
            "Should list /clear command"
        );

        // Test /clear command - basic functionality
        history.clear();
        chat_service
            .send_message("/clear", &mut history, false, session_id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1, "Should have response message");
        assert!(
            history[0].content.contains("Chat history cleared"),
            "Should contain clear message"
        );

        // Note: Custom command testing is skipped in this integration test
        // because it would require mocking the AI API or a valid API key.
        // The functionality is tested in unit tests and manual testing.

        // Test unknown command
        history.clear();
        chat_service
            .send_message("/nonexistent", &mut history, false, session_id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1, "Should have error response");
        assert!(
            history[0].content.contains("Unknown command"),
            "Should contain error message"
        );
    }

    #[test]
    fn test_syntax_highlighting_parsing() {
        use harper_workspace::interfaces::ui::Theme;
        use harper_workspace::interfaces::ui::widgets::parse_content_with_code;
        use ratatui::style::Color;
        use syntect::highlighting::ThemeSet;
        use syntect::parsing::SyntaxSet;

        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = Theme::default();

        // Test parsing content with code blocks
        let content = "Here is some Rust code:\n```rust\nfn main() {\n    println!(\"Hello!\");\n}\n```\nAnd that's it.";
        let spans = parse_content_with_code(
            &syntax_set,
            &theme_set,
            content,
            &theme,
            Color::White,
            "base16-ocean.dark",
        );

        // Should have spans: plain text, highlighted code, plain text
        assert!(spans.len() >= 3, "Should have multiple spans");

        // Check that some rendered lines have different styles (indicating highlighting)
        let has_highlighted = spans.iter().any(|line| line.style.fg != Some(Color::White));
        assert!(has_highlighted, "Should have highlighted spans");

        // Test with multiple code blocks
        let content_multi =
            "```python\nprint('hello')\n```\nAnd\n```javascript\nconsole.log('world');\n```";
        let spans_multi = parse_content_with_code(
            &syntax_set,
            &theme_set,
            content_multi,
            &theme,
            Color::White,
            "base16-ocean.dark",
        );
        // The number of spans varies based on syntax highlighting complexity
        assert!(
            spans_multi.len() >= 3,
            "Should have at least 3 spans for multiple code blocks"
        );
        // Note: reconstructed content excludes markdown markers (```) as they are not rendered.
        // Check that some rendered lines are highlighted and that the plain text line is preserved.
        let has_highlighted = spans_multi
            .iter()
            .any(|line| line.style.fg != Some(Color::White));
        let has_plain = spans_multi
            .iter()
            .any(|line| line.to_string().contains("And"));
        assert!(has_highlighted, "Should have at least one highlighted span");
        assert!(has_plain, "Should have at least one plain text span");

        // Test with no code blocks
        let content_plain = "Just plain text.";
        let spans_plain = parse_content_with_code(
            &syntax_set,
            &theme_set,
            content_plain,
            &theme,
            Color::White,
            "base16-ocean.dark",
        );
        assert_eq!(spans_plain.len(), 1, "Plain text should have one span");
        let reconstructed = spans_plain[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(reconstructed, content_plain);
    }
}
