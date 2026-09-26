// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! GitHub operations tool
//!
//! This module provides functionality for GitHub operations like creating issues and PRs.

use crate::core::error::HarperError;
use crate::tools::parsing;
use colored::*;
use std::process::Command;

/// Create a GitHub issue
pub fn create_issue(response: &str) -> crate::core::error::HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[GITHUB_ISSUE", 2)?;
    let title = &args[0];
    let body = &args[1];

    println!(
        "{} Create GitHub issue '{}' with body '{}' ? (y/n): ",
        "System:".bold().magenta(),
        title.magenta(),
        body.magenta()
    );
    let mut approval = String::new();
    std::io::stdin().read_line(&mut approval)?;
    if !approval.trim().eq_ignore_ascii_case("y") {
        return Ok("Issue creation cancelled by user".to_string());
    }

    println!(
        "{} Creating GitHub issue: {}",
        "System:".bold().magenta(),
        title.magenta()
    );

    let output = Command::new("gh")
        .args(["issue", "create", "--title", title, "--body", body])
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run gh command: {}", e)))?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout);
        Ok(format!("Issue created: {}", result.trim()))
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(HarperError::Command(format!(
            "gh command failed: {}",
            error
        )))
    }
}

/// Create a GitHub pull request
pub fn create_pr(response: &str) -> crate::core::error::HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[GITHUB_PR", 3)?;
    let title = &args[0];
    let body = &args[1];
    let branch = &args[2];

    println!(
        "{} Create PR '{}' from branch '{}' ? (y/n): ",
        "System:".bold().magenta(),
        title.magenta(),
        branch.magenta()
    );
    let mut approval = String::new();
    std::io::stdin().read_line(&mut approval)?;
    if !approval.trim().eq_ignore_ascii_case("y") {
        return Ok("PR creation cancelled by user".to_string());
    }

    println!(
        "{} Creating GitHub PR: {}",
        "System:".bold().magenta(),
        title.magenta()
    );

    let output = Command::new("gh")
        .args([
            "pr", "create", "--title", title, "--body", body, "--head", branch,
        ])
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run gh command: {}", e)))?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout);
        Ok(format!("PR created: {}", result.trim()))
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(HarperError::Command(format!(
            "gh command failed: {}",
            error
        )))
    }
}
