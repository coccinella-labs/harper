// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Todo management tool
//!
//! This module provides functionality for managing todo lists.
//! Uses persistent SQLite storage.

use crate::core::error::HarperError;
use crate::memory::storage;
use crate::tools::parsing;

/// Manage todo list operations for task tracking
///
/// Supports the following commands:
/// - `[TODO add "task description"]` - Add a new todo item
/// - `[TODO list]` - List all todo items
/// - `[TODO remove N]` - Remove todo item by index (1-based)
///
/// # Arguments
/// * `conn` - SQLite database connection
/// * `response` - Input string containing the todo command
///
/// # Returns
/// A success message or the list of todos
///
/// # Errors
/// Returns `HarperError::Command` for invalid commands or database errors
///
/// # Example
/// ```text
/// // This function requires a database connection
/// // Use it in your application with a proper rusqlite::Connection
/// // let result = manage_todo(&connection, "[TODO add \"fix bug\"]")?;
/// // println!("Result: {}", result);
/// ```
pub fn manage_todo(
    conn: &rusqlite::Connection,
    response: &str,
) -> crate::core::error::HarperResult<String> {
    let arg_str = parsing::extract_tool_arg(response, "[TODO")?;
    let args = parsing::parse_quoted_args(&arg_str)?;

    if args.is_empty() {
        return Err(HarperError::Command("No todo command provided".to_string()));
    }

    let command = &args[0];

    match command.as_str() {
        "add" => {
            if args.len() < 2 {
                return Err(HarperError::Command(
                    "Todo add requires a description".to_string(),
                ));
            }
            let description = args[1..].join(" ");
            storage::save_todo(conn, &description)?;
            Ok(format!("Added todo: {}", description))
        }
        "list" => {
            let todos = storage::load_todos(conn)?;
            if todos.is_empty() {
                Ok("No todos found".to_string())
            } else {
                let list = todos
                    .iter()
                    .enumerate()
                    .map(|(i, (_id, desc))| format!("{}. {}", i + 1, desc))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(format!("Current todos:\n{}", list))
            }
        }
        "remove" => {
            if args.len() < 2 {
                return Err(HarperError::Command(
                    "Todo remove requires an index".to_string(),
                ));
            }
            let index: usize = args[1]
                .parse()
                .map_err(|_| HarperError::Command("Invalid todo index".to_string()))?;
            if index == 0 {
                return Err(HarperError::Command(
                    "Invalid todo index: 1-based index expected".to_string(),
                ));
            }
            let (id, desc): (i64, String) = match conn.query_row(
                "SELECT id, description FROM todos ORDER BY id ASC LIMIT 1 OFFSET ?1",
                [(index - 1) as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ) {
                Ok(todo) => todo,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Err(HarperError::Command(format!(
                        "Invalid todo index: {}",
                        index
                    )));
                }
                Err(e) => return Err(HarperError::Database(e.to_string())),
            };

            storage::delete_todo(conn, id)?;
            Ok(format!("Removed todo: {}", desc))
        }
        "clear" => {
            let count = storage::clear_todos(conn)?;
            Ok(format!("Cleared {} todos", count))
        }
        _ => Err(HarperError::Command(format!(
            "Unknown todo command: {}. Supported: add, list, remove, clear",
            command
        ))),
    }
}
