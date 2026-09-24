// Copyright 2026 coccinella-labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Git operations tool
//!
//! This module provides functionality for git operations
//! with safety checks and user approval.

use crate::core::ApiConfig;
use crate::core::error::HarperError;
use crate::runtime::config::ExecPolicyConfig;
use crate::tools::parsing;
use crate::tools::shell::{self, CommandAuditContext};
use colored::*;
use std::collections::HashSet;
use std::io;
use std::path::Path;

use crate::core::io_traits::UserApproval;
use std::sync::Arc;

/// Process git status output and format it for display
fn process_git_status_output(output: std::process::Output) -> String {
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            "Git working directory is clean".to_string()
        } else {
            format!(
                "Git status:
{}",
                stdout
            )
        }
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    }
}

/// Get the current git repository status
///
/// Returns the output of `git status --porcelain` which shows
/// staged, unstaged, and untracked files in a machine-readable format.
///
/// # Returns
/// A string containing the git status output
///
/// # Errors
/// Returns `HarperError::Command` if the git command fails
///
/// # Example
/// ```text
/// match harper_core::tools::git::git_status() {
///     Ok(status) => println!("Git status: {}", status),
///     Err(e) => println!("Error: {}", e),
/// }
/// ```
pub fn git_status() -> crate::core::error::HarperResult<String> {
    let output = std::process::Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run git status: {}", e)))?;

    Ok(process_git_status_output(output))
}

pub fn repo_identity() -> crate::core::error::HarperResult<String> {
    let top_level = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to determine git root: {}", e)))?;
    if !top_level.status.success() {
        return Err(HarperError::Command(
            String::from_utf8_lossy(&top_level.stderr)
                .trim()
                .to_string(),
        ));
    }

    let root = String::from_utf8_lossy(&top_level.stdout)
        .trim()
        .to_string();
    let repo_name = Path::new(&root)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");

    let branch_output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to determine branch: {}", e)))?;
    let branch = if branch_output.status.success() {
        String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_string()
    } else {
        "unknown".to_string()
    };

    Ok(format!(
        "Current repo: {} (root: {}, branch: {})",
        repo_name, root, branch
    ))
}

pub fn current_branch() -> crate::core::error::HarperResult<String> {
    let branch_output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to determine branch: {}", e)))?;
    if !branch_output.status.success() {
        return Err(HarperError::Command(
            String::from_utf8_lossy(&branch_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let branch = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();
    Ok(format!("Current branch: {}", branch))
}

/// Get git status asynchronously
pub async fn git_status_async() -> crate::core::error::HarperResult<String> {
    let output = tokio::process::Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .output()
        .await
        .map_err(|e| HarperError::Command(format!("Failed to run git status: {}", e)))?;

    Ok(process_git_status_output(output))
}

/// Get the current git diff
///
/// Returns the output of `git diff` showing unstaged changes
/// between the working directory and the index.
///
/// # Returns
/// A string containing the git diff output
///
/// # Errors
/// Returns `HarperError::Command` if the git command fails
///
/// # Example
/// ```text
/// match harper_core::tools::git::git_diff() {
///     Ok(diff) if diff.is_empty() => println!("No unstaged changes"),
///     Ok(diff) => println!("Unstaged changes:\n{}", diff),
///     Err(e) => println!("Error: {}", e),
/// }
/// ```
pub fn git_diff() -> crate::core::error::HarperResult<String> {
    let output = std::process::Command::new("git")
        .arg("diff")
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run git diff: {}", e)))?;

    let result = if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            "No changes to show".to_string()
        } else {
            format!(
                "Git diff:
{}",
                stdout
            )
        }
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    };

    Ok(result)
}

/// List changed files with optional filters.
///
/// - `ext`: file extension filter (for example `rs` or `.rs`)
/// - `tracked_only`: when true, excludes untracked files from working tree changes
/// - `since`: git date expression for commit history filtering (for example `today`, `2 days ago`)
pub fn list_changed_files(
    ext: Option<&str>,
    tracked_only: bool,
    since: Option<&str>,
) -> crate::core::error::HarperResult<String> {
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run git status: {}", e)))?;
    if !status_output.status.success() {
        return Err(HarperError::Command(
            String::from_utf8_lossy(&status_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let status_stdout = String::from_utf8_lossy(&status_output.stdout).to_string();
    let log_stdout = if let Some(since_expr) = since.map(str::trim).filter(|s| !s.is_empty()) {
        let log_output = std::process::Command::new("git")
            .args([
                "log",
                "--name-only",
                "--pretty=format:",
                "--since",
                since_expr,
            ])
            .output()
            .map_err(|e| HarperError::Command(format!("Failed to run git log: {}", e)))?;
        if !log_output.status.success() {
            return Err(HarperError::Command(
                String::from_utf8_lossy(&log_output.stderr)
                    .trim()
                    .to_string(),
            ));
        }
        Some(String::from_utf8_lossy(&log_output.stdout).to_string())
    } else {
        None
    };

    Ok(format_changed_files_response(
        &status_stdout,
        log_stdout.as_deref(),
        ext,
        tracked_only,
    ))
}

/// Same behavior as `list_changed_files`, but executed through the shell tool path
/// so command policy, approval, and audit logging are consistently enforced.
pub async fn list_changed_files_with_policy(
    config: &ApiConfig,
    exec_policy: &ExecPolicyConfig,
    audit_ctx: Option<&CommandAuditContext<'_>>,
    approver: Option<Arc<dyn UserApproval>>,
    ext: Option<&str>,
    tracked_only: bool,
    since: Option<&str>,
) -> crate::core::error::HarperResult<String> {
    let status_stdout = shell::execute_command(
        "[RUN_COMMAND git status --porcelain]",
        config,
        exec_policy,
        None,
        audit_ctx,
        approver.clone(),
        None,
    )
    .await?;

    let log_stdout = if let Some(since_expr) = since.map(str::trim).filter(|s| !s.is_empty()) {
        let escaped = since_expr.replace('"', "\\\"");
        let command = format!(
            "[RUN_COMMAND git log --name-only --pretty=format: --since \"{}\"]",
            escaped
        );
        Some(
            shell::execute_command(
                &command,
                config,
                exec_policy,
                None,
                audit_ctx,
                approver.clone(),
                None,
            )
            .await?,
        )
    } else {
        None
    };

    Ok(format_changed_files_response(
        &status_stdout,
        log_stdout.as_deref(),
        ext,
        tracked_only,
    ))
}

/// Commit changes
pub async fn git_commit(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
) -> crate::core::error::HarperResult<String> {
    let message = extract_commit_message(response)?;

    let is_approved = if let Some(appr) = approver {
        appr.approve("Commit with message:", &message).await?
    } else {
        println!(
            "{} Commit with message: '{}' ? (y/n): ",
            "System:".bold().magenta(),
            message.magenta()
        );
        let mut approval = String::new();
        io::stdin().read_line(&mut approval)?;
        approval.trim().eq_ignore_ascii_case("y")
    };

    if !is_approved {
        return Ok("Git commit cancelled by user".to_string());
    }

    let output = std::process::Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg(&message)
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run git commit: {}", e)))?;

    let result = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    };

    Ok(result)
}

/// Add files to git
pub async fn git_add(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
) -> crate::core::error::HarperResult<String> {
    let files = extract_files(response)?;

    let files_list = files.join(", ");
    let is_approved = if let Some(appr) = approver {
        appr.approve("Add files:", &files_list).await?
    } else {
        println!(
            "{} Add files: {} ? (y/n): ",
            "System:".bold().magenta(),
            files_list.magenta()
        );
        let mut approval = String::new();
        io::stdin().read_line(&mut approval)?;
        approval.trim().eq_ignore_ascii_case("y")
    };

    if !is_approved {
        return Ok("Git add cancelled by user".to_string());
    }

    let mut command = std::process::Command::new("git");
    command.arg("add");
    for file in &files {
        command.arg(file);
    }

    let output = command
        .output()
        .map_err(|e| HarperError::Command(format!("Failed to run git add: {}", e)))?;

    let result = if output.status.success() {
        format!("Added {} files to git", files.len())
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    };

    Ok(result)
}

/// Extract commit message from response
fn extract_commit_message(response: &str) -> crate::core::error::HarperResult<String> {
    let message = response
        .strip_prefix("[GIT_COMMIT")
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| HarperError::Command("Invalid git commit format".to_string()))?
        .trim();

    if message.is_empty() {
        return Err(HarperError::Command(
            "No commit message provided".to_string(),
        ));
    }

    Ok(message.to_string())
}

/// Extract files from response
fn extract_files(response: &str) -> crate::core::error::HarperResult<Vec<String>> {
    let files_str = response
        .strip_prefix("[GIT_ADD")
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| HarperError::Command("Invalid git add format".to_string()))?
        .trim();

    if files_str.is_empty() {
        return Ok(vec![".".to_string()]); // Add all if no files specified
    }

    parsing::parse_quoted_args(files_str)
}

fn maybe_add_file(
    path: &str,
    ext_filter: Option<&str>,
    files: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let normalized = path.trim();
    if normalized.is_empty() {
        return;
    }
    if let Some(ext) = ext_filter {
        let path_ext = std::path::Path::new(normalized)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if path_ext.as_deref() != Some(ext) {
            return;
        }
    }
    let key = normalized.to_string();
    if seen.insert(key.clone()) {
        files.push(key);
    }
}

fn format_changed_files_response(
    status_stdout: &str,
    log_stdout: Option<&str>,
    ext: Option<&str>,
    tracked_only: bool,
) -> String {
    let normalized_ext = ext
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_start_matches('.').to_ascii_lowercase());

    let mut files: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    for line in status_stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let is_untracked = line.starts_with("??");
        if tracked_only && is_untracked {
            continue;
        }
        let path_part = &line[3..];
        let path = path_part
            .split(" -> ")
            .last()
            .map(str::trim)
            .unwrap_or(path_part);
        maybe_add_file(path, normalized_ext.as_deref(), &mut files, &mut seen);
    }

    if let Some(log_output) = log_stdout {
        for line in log_output.lines() {
            let path = line.trim();
            if path.is_empty() {
                continue;
            }
            maybe_add_file(path, normalized_ext.as_deref(), &mut files, &mut seen);
        }
    }

    if files.is_empty() {
        return "No changed files found.".to_string();
    }

    let mut response = String::from("Changed files:\n");
    for file in files {
        response.push_str("- ");
        response.push_str(&file);
        response.push('\n');
    }
    response.trim_end().to_string()
}
