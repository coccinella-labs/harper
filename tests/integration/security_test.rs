// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_workspace::*;
use rusqlite::Connection;
use tempfile::NamedTempFile;

#[test]
fn test_sql_injection_protection() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();
    init_db(&conn).unwrap();

    // Test various SQL injection patterns
    let test_cases = [
        "test'; DROP TABLE sessions;--",
        "' OR '1'='1",
        "' UNION SELECT * FROM users;--",
        "; SHUTDOWN;--",
        "1; SELECT pg_sleep(10);--",
    ];

    for (i, &malicious_input) in test_cases.iter().enumerate() {
        // Test session ID injection
        save_session(&conn, malicious_input).unwrap();

        // Verify the session was created with the exact ID
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?",
                [malicious_input],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(count, 1, "Failed test case {i}: {malicious_input}");

        // Test message content injection
        let result = save_message(
            &conn,
            malicious_input,
            "user",
            &format!("Test message with SQL: {malicious_input}"),
        );
        assert!(
            result.is_ok(),
            "Failed to save message with SQL injection attempt: {malicious_input}"
        );
    }
}

#[test]
fn test_path_traversal_protection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_path = temp_dir.path().to_path_buf();

    // Create a test database in the temp directory
    let db_path = base_path.join("test.db");
    let conn = Connection::open(&db_path).unwrap();
    init_db(&conn).unwrap();

    // Test various path traversal attempts in session IDs and message content
    // These should be treated as literal strings, not as paths
    let test_cases = [
        "harper_attempt_1_etc_passwd",
        "harper_abs_path_etc_passwd",
        "harper_windows_path_hosts",
        "harper_win_cmd_escape",
        "harper_url_encoded_etc_passwd",
        "harper_double_url_encoded_path",
    ];

    for (i, path) in test_cases.iter().enumerate() {
        // Test with path in session ID
        let session_id = format!("session-{i}-{path}");
        let save_result = save_session(&conn, &session_id);
        assert!(
            save_result.is_ok(),
            "Should be able to save session with path-like ID: {path}"
        );

        // Verify the session was created with the exact ID
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Session with path-like ID not found: {path}");

        // Test with path in message content
        let message_result = save_message(
            &conn,
            &session_id,
            "user",
            &format!("Test message with path: {path}"),
        );
        assert!(
            message_result.is_ok(),
            "Should be able to save message with path-like content: {path}"
        );

        // Clean up
        delete_session(&conn, &session_id).unwrap();
    }

    // Test that we can't open files outside the temp directory
    let outside_paths = [
        "harper_outside_path_1",
        "harper_abs_path_check",
        "harper_windows_path_check",
    ];

    for path in &outside_paths {
        let result = std::fs::File::open(path);
        if result.is_ok() {
            // If we can open the file directly, make sure it's not in our temp dir
            let canonical_path =
                std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
            assert!(
                !canonical_path.starts_with(&base_path),
                "Path traversal vulnerability detected: {path} is accessible and inside temp dir"
            );
        }
    }
}

#[test]
fn test_xss_protection() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();
    init_db(&conn).unwrap();

    // Test various XSS payloads
    let xss_test_cases = [
        ("<script>alert('xss')</script>", "user"),
        ("<img src='x' onerror='alert(1)'>", "assistant"),
        ("<a href=\"javascript:alert('xss')\">Click me</a>", "user"),
        ("<svg/onload=alert('xss')>", "system"),
        ("'><script>alert(1)</script>", "user"),
        ("\"'><img src='x' onerror='alert(1)'>", "assistant"),
        ("javascript:alert('xss')", "user"),
        (
            "data:text/html;base64,PHNjcmlwdD5hbGVydCgneHNzJyk8L3NjcmlwdD4=",
            "system",
        ),
    ];

    let session_id = "xss-test-session";
    save_session(&conn, session_id).unwrap();

    for (i, (payload, role)) in xss_test_cases.iter().enumerate() {
        // Save message with potential XSS
        save_message(&conn, session_id, role, payload).unwrap();

        // Load the message back
        let history = load_history(&conn, session_id).unwrap();
        let last_message = history.last().unwrap();

        // The content should be stored exactly as provided
        // Note: XSS protection should be handled at the application level when displaying content
        assert_eq!(
            last_message.content, *payload,
            "Message content was modified in test case {i}: {payload}"
        );

        assert!(
            ["user", "assistant", "system"].contains(&last_message.role.as_str()),
            "Invalid role in test case {i}: {}",
            last_message.role
        );

        assert!(
            history.iter().any(|m| m.content == *payload),
            "Message with payload not found in history: {payload}"
        );
    }
}
