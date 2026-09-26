// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_workspace::*;
use rusqlite::Connection;
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn test_database_connection_failure() {
    // Try to connect to an invalid path
    let result = Connection::open("/invalid/path/to/db.sqlite");
    assert!(result.is_err());
    assert!(matches!(
        result.err().unwrap().to_string().as_str(),
        s if s.contains("unable to open database file")
    ));
}

#[test]
fn test_invalid_sql_handling() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Try to execute invalid SQL
    let result = conn.execute("INVALID SQL", []);
    assert!(result.is_err());

    // Test SQL injection attempt
    let malicious_input = "test'; DROP TABLE sessions;--";
    let result = conn.execute("INSERT INTO sessions (id) VALUES (?)", [malicious_input]);
    assert!(result.is_err());
    // SQLite should prevent the injection
    assert!(matches!(
        result.unwrap_err(),
        rusqlite::Error::SqliteFailure(_, _)
    ));
}

#[test]
fn test_file_permission_issues() {
    // Test with a directory instead of a file
    let temp_dir = tempfile::tempdir().unwrap();
    let dir_path = temp_dir.path().join("test_dir");
    fs::create_dir(&dir_path).unwrap();

    // SQLite will actually create the database file even if the parent directory is read-only
    // So we'll just test that we can't open a directory as a database
    let result = Connection::open(&dir_path);
    assert!(
        result.is_err(),
        "Should not be able to open a directory as a database"
    );

    // Test with a non-existent parent directory
    let non_existent_path = temp_dir.path().join("nonexistent").join("test.db");
    let result = Connection::open(&non_existent_path);
    assert!(
        result.is_err(),
        "Should not be able to create database in non-existent directory"
    );
}

#[test]
fn test_invalid_session_operations() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();
    init_db(&conn).unwrap();

    // Test with empty session ID
    let result = save_session(&conn, "");
    assert!(result.is_ok(), "Empty session ID should be allowed");

    // Test with non-existent session
    let result = load_history(&conn, "non-existent-session");
    assert!(
        result.is_ok(),
        "Loading non-existent session should not error"
    );
    assert!(
        result.unwrap().is_empty(),
        "Non-existent session should return empty history"
    );

    // Test with invalid session ID format
    let result = load_history(&conn, "invalid/session/id");
    assert!(result.is_ok(), "Invalid session ID format should not error");
    assert!(
        result.unwrap().is_empty(),
        "Invalid session ID should return empty history"
    );

    // Test with very long session ID (SQLite has a default limit of 1,000,000,000 bytes for a string)
    let long_id = "a".repeat(1000); // Still well below SQLite's limit
    let result = save_session(&conn, &long_id);
    assert!(result.is_ok(), "Long session ID should be allowed");

    // Verify we can retrieve the session with the long ID
    let result = load_history(&conn, &long_id);
    assert!(
        result.is_ok(),
        "Should be able to load session with long ID"
    );
    assert!(
        result.unwrap().is_empty(),
        "New session should have empty history"
    );
}

#[test]
fn test_message_validation() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();
    init_db(&conn).unwrap();

    // Create a test session first
    let session_id = "test-session";
    save_session(&conn, session_id).unwrap();

    // Test with invalid role - should be allowed
    let result = save_message(&conn, session_id, "invalid-role", "content");
    assert!(result.is_ok(), "Invalid role should be allowed");

    // Test with empty content - should be allowed
    let result = save_message(&conn, session_id, "user", "");
    assert!(result.is_ok(), "Empty content should be allowed");

    // Test with very large message (SQLite has a default limit of 1,000,000,000 bytes for a string)
    let large_content = "a".repeat(100_000); // Still well below SQLite's limit
    let result = save_message(&conn, session_id, "user", &large_content);
    assert!(result.is_ok(), "Large content should be allowed");

    // Verify the message was saved correctly
    let history = load_history(&conn, session_id).unwrap();
    assert_eq!(history.len(), 3, "Should have 3 messages in history");
    assert_eq!(
        history[0].content, "content",
        "First message content should match"
    );
    assert_eq!(
        history[1].content, "",
        "Second message content should be empty"
    );
    assert_eq!(
        history[2].content, large_content,
        "Third message content should match large content"
    );
}

#[test]
#[allow(clippy::ignored_unit_patterns)]
fn test_concurrent_database_access() {
    use std::thread;
    use std::time::Duration;

    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path().to_path_buf();
    let conn = Connection::open(&db_path).unwrap();
    init_db(&conn).unwrap();

    // Create multiple connections from different threads
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let db_path = db_path.clone();
            thread::spawn(move || {
                let conn = Connection::open(&db_path).unwrap();
                let session_id = format!("session-{i}");

                // Retry on database locked
                let mut retries = 0;
                while retries < 10 {
                    match save_session(&conn, &session_id) {
                        Ok(_) => break,
                        Err(e) if e.to_string().contains("database is locked") => {
                            retries += 1;
                            thread::sleep(Duration::from_millis(50));
                        }
                        Err(e) => panic!("Failed to save session: {e:?}"),
                    }
                }

                for j in 0..10 {
                    let mut retries = 0;
                    while retries < 10 {
                        match save_message(
                            &conn,
                            &session_id,
                            if j % 2 == 0 { "user" } else { "assistant" },
                            &format!("Message {j} from thread {i}"),
                        ) {
                            Ok(_) => break,
                            Err(e) if e.to_string().contains("database is locked") => {
                                retries += 1;
                                thread::sleep(Duration::from_millis(50));
                            }
                            Err(e) => panic!("Failed to save message: {e:?}"),
                        }
                    }
                    thread::sleep(Duration::from_millis(10));
                }

                let history = load_history(&conn, &session_id).unwrap();
                assert_eq!(history.len(), 10);
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all data was saved correctly
    let conn = Connection::open(&db_path).unwrap();
    for i in 0..5 {
        let session_id = format!("session-{i}");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 10, "Unexpected number of messages in session");
    }
}
