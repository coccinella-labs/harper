// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Database operations tool
//!
//! This module provides functionality for running read-only SQL queries on SQLite databases.

use crate::core::error::{HarperError, HarperResult};
use crate::tools::parsing;
use colored::*;
use rusqlite::{Connection, Result as SqlResult};
use std::io::{self, Write};

use crate::core::io_traits::UserApproval;
use std::sync::Arc;

/// Run a database query
pub async fn run_query(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
) -> HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[DB_QUERY", 2)?;
    let db_path = &args[0];
    let query = &args[1];

    let is_approved = if let Some(appr) = approver {
        appr.approve(&format!("Run query on DB {}?", db_path), query)
            .await?
    } else {
        let db = db_path.clone();
        let q = query.clone();
        tokio::task::spawn_blocking(move || {
            println!(
                "{} Run query on DB {} ? (y/n): {}",
                "System:".bold().magenta(),
                db.magenta(),
                q.magenta()
            );
            io::stdout()
                .flush()
                .map_err(|e| HarperError::Io(e.to_string()))?;
            let mut approval = String::new();
            io::stdin()
                .read_line(&mut approval)
                .map_err(|e| HarperError::Io(e.to_string()))?;
            Ok::<bool, HarperError>(approval.trim().eq_ignore_ascii_case("y"))
        })
        .await
        .map_err(|e| HarperError::Command(format!("Task failed: {}", e)))??
    };

    if !is_approved {
        return Ok("Query cancelled by user".to_string());
    }

    // Safety: only allow SELECT queries
    if !query.trim().to_uppercase().starts_with("SELECT") {
        return Err(HarperError::Command(
            "Only SELECT queries are allowed".to_string(),
        ));
    }

    println!(
        "{} Running query on: {}",
        "System:".bold().magenta(),
        db_path.magenta()
    );

    let conn = Connection::open(db_path)
        .map_err(|e| HarperError::Command(format!("Failed to open DB {}: {}", db_path, e)))?;

    let mut stmt = conn
        .prepare(query)
        .map_err(|e| HarperError::Command(format!("Failed to prepare query: {}", e)))?;

    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt
        .query([])
        .map_err(|e| HarperError::Command(format!("Failed to execute query: {}", e)))?;

    let mut result = format!("Columns: {:?}\n", column_names);
    let mut count = 0;
    while let Some(row) = rows
        .next()
        .map_err(|e| HarperError::Command(format!("Failed to read row: {}", e)))?
    {
        let row_str = column_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let value: SqlResult<String> = row.get(i);
                format!(
                    "'{:?}': '{:?}'",
                    name,
                    value.unwrap_or_else(|_| "NULL".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        result.push_str(&format!("Row {}: {{{}}}\n", count, row_str));
        count += 1;
        if count > 100 {
            // limit rows
            result.push_str("... (truncated)\n");
            break;
        }
    }

    Ok(format!(
        "Query executed, {} rows returned.\n{}",
        count, result
    ))
}
