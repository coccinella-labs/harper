// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Code analysis tool
//!
//! This module provides functionality for analyzing code metrics like complexity.

use crate::core::error::HarperError;
use crate::tools::parsing;
use colored::*;
// No regex needed

/// Analyze code metrics
pub fn analyze_code(response: &str) -> crate::core::error::HarperResult<String> {
    let path = parsing::extract_tool_arg(response, "[CODE_ANALYZE")?;

    println!(
        "{} Analyzing code in file: {}",
        "System:".bold().magenta(),
        path.magenta()
    );

    let content = std::fs::read_to_string(&path)
        .map_err(|e| HarperError::Command(format!("Failed to read file {}: {}", path, e)))?;

    // Simple metrics
    let lines = content.lines().count();
    let non_empty_lines = content.lines().filter(|l| !l.trim().is_empty()).count();

    // For Rust code, count some elements (simple)
    let fn_count = content.matches("fn ").count();
    let struct_count = content.matches("struct ").count();
    let enum_count = content.matches("enum ").count();
    let impl_count = content.matches("impl ").count();

    // Simple cyclomatic complexity estimate (rough)
    let complexity = fn_count * 2 + struct_count + enum_count; // placeholder

    let result = format!(
        "File: {}\n\
         Total lines: {}\n\
         Non-empty lines: {}\n\
         Functions: {}\n\
         Structs: {}\n\
         Enums: {}\n\
         Impls: {}\n\
         Estimated complexity: {}",
        path, lines, non_empty_lines, fn_count, struct_count, enum_count, impl_count, complexity
    );

    Ok(result)
}
