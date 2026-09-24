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

//! Shell command execution tool
//!
//! This module provides functionality for executing shell commands
//! with safety checks and user approval.

use crate::core::plan::PlanJobStatus;
use crate::core::{error::HarperError, ApiConfig};
use crate::memory::storage::{self, CommandLogRecord};
use crate::runtime::config::{ApprovalProfile, ExecPolicyConfig, SandboxProfile};
use crate::tools::parsing;
use colored::*;
use harper_sandbox::{Sandbox, SandboxRequest};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

const OUTPUT_PREVIEW_LIMIT: usize = 512;

/// Context for persisting command audit logs
pub struct CommandAuditContext<'a> {
    pub conn: &'a Connection,
    pub session_id: Option<&'a str>,
    pub source: &'a str,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandSandboxIntent {
    pub declared_read_paths: Vec<PathBuf>,
    pub declared_write_paths: Vec<PathBuf>,
    pub requires_network: bool,
    pub retry_policy: Option<CommandRetryPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandRetryPolicy {
    Never,
    Safe,
}

struct CommandAttemptResult {
    output_text: String,
    success: bool,
    exit_code: Option<i32>,
    duration_ms: i64,
    stdout_preview: Option<String>,
    stderr_preview: Option<String>,
    output_preview: Option<String>,
    has_error_output: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct RunCommandPayload {
    command: String,
    #[serde(default)]
    declared_read_paths: Vec<PathBuf>,
    #[serde(default)]
    declared_write_paths: Vec<PathBuf>,
    #[serde(default)]
    requires_network: bool,
    #[serde(default)]
    retry_policy: Option<CommandRetryPolicy>,
}

fn parse_run_command_response(
    response: &str,
    fallback_intent: Option<&CommandSandboxIntent>,
) -> crate::core::error::HarperResult<(String, Option<CommandSandboxIntent>)> {
    let payload = if let Some(pos) = response.find(' ') {
        response[pos..].trim_start().trim_end_matches(']')
    } else {
        ""
    };

    if payload.is_empty() {
        return Ok((String::new(), fallback_intent.cloned()));
    }

    if payload.trim_start().starts_with('{') {
        let parsed: RunCommandPayload = serde_json::from_str(payload)
            .map_err(|e| HarperError::Command(format!("Invalid run_command payload: {}", e)))?;
        return Ok((
            parsed.command,
            Some(CommandSandboxIntent {
                declared_read_paths: parsed.declared_read_paths,
                declared_write_paths: parsed.declared_write_paths,
                requires_network: parsed.requires_network,
                retry_policy: parsed.retry_policy,
            }),
        ));
    }

    Ok((payload.to_string(), fallback_intent.cloned()))
}

use crate::core::io_traits::{RuntimeEventSink, UserApproval};
use std::sync::Arc;

pub(crate) async fn emit_activity_update(
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    session_id: Option<&str>,
    status: Option<String>,
) {
    if let (Some(sink), Some(session_id)) = (runtime_events, session_id) {
        let _ = sink.activity_updated(session_id, status).await;
    }
}

async fn emit_command_output(
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    session_id: Option<&str>,
    command: &str,
    chunk: String,
    is_error: bool,
    done: bool,
) {
    if let (Some(sink), Some(session_id)) = (runtime_events, session_id) {
        let _ = sink
            .command_output_updated(session_id, command.to_string(), chunk, is_error, done)
            .await;
    }
}

fn database_path(conn: &Connection) -> Option<String> {
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

fn build_sandbox_request(
    command_str: &str,
    intent: Option<&CommandSandboxIntent>,
) -> crate::core::error::HarperResult<SandboxRequest> {
    let args = parsing::parse_quoted_args(command_str)?;
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| HarperError::Command("No command provided".to_string()))?;

    let working_dir = std::env::current_dir().map_err(|e| HarperError::Io(e.to_string()))?;
    let rest_args: Vec<&str> = rest.iter().map(String::as_str).collect();
    let (declared_read_paths, declared_write_paths, requires_network, _retry_policy) =
        if let Some(intent) = intent {
            (
                intent.declared_read_paths.clone(),
                intent.declared_write_paths.clone(),
                intent.requires_network,
                intent.retry_policy.clone(),
            )
        } else {
            let (reads, writes) = infer_path_intent(command, &rest_args);
            (reads, writes, looks_like_network_command(command), None)
        };

    Ok(SandboxRequest {
        command: command.clone(),
        args: rest.to_vec(),
        working_dir,
        env: vec![],
        declared_read_paths,
        declared_write_paths,
        requires_network,
    })
}

fn looks_like_path(arg: &str) -> bool {
    arg.starts_with('/')
        || arg.starts_with("./")
        || arg.starts_with("../")
        || arg.contains(std::path::MAIN_SEPARATOR)
}

fn command_basename(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
}

fn infer_path_intent(command: &str, args: &[&str]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let path_args: Vec<&str> = args
        .iter()
        .copied()
        .filter(|arg| looks_like_path(arg))
        .collect();
    if path_args.is_empty() {
        return (vec![], vec![]);
    }

    match command_basename(command) {
        "rm" | "touch" | "mkdir" | "rmdir" => {
            (vec![], path_args.into_iter().map(PathBuf::from).collect())
        }
        "sed"
            if args
                .iter()
                .any(|arg| *arg == "-i" || arg.strip_prefix("-i").is_some()) =>
        {
            let mut seen_script = false;
            let writes = args
                .iter()
                .copied()
                .filter(|arg| {
                    if arg.starts_with('-') && !seen_script {
                        return false;
                    }
                    if !seen_script {
                        seen_script = true;
                        return false;
                    }
                    looks_like_path(arg)
                })
                .map(PathBuf::from)
                .collect();
            (vec![], writes)
        }
        "tee" => (
            vec![],
            args.iter()
                .copied()
                .filter(|arg| !arg.starts_with('-') && looks_like_path(arg))
                .map(PathBuf::from)
                .collect(),
        ),
        "cp" | "mv" => {
            if path_args.len() == 1 {
                (vec![], vec![PathBuf::from(path_args[0])])
            } else {
                let (sources, destination) = path_args.split_at(path_args.len() - 1);
                (
                    sources.iter().map(PathBuf::from).collect(),
                    destination.iter().map(PathBuf::from).collect(),
                )
            }
        }
        "find" if args.contains(&"-delete") => {
            let roots: Vec<PathBuf> = args
                .iter()
                .copied()
                .take_while(|arg| !arg.starts_with('-'))
                .filter(|arg| looks_like_path(arg))
                .map(PathBuf::from)
                .collect();
            if roots.is_empty() {
                (vec![], path_args.into_iter().map(PathBuf::from).collect())
            } else {
                (vec![], roots)
            }
        }
        "tar" => infer_tar_path_intent(args),
        _ => (path_args.into_iter().map(PathBuf::from).collect(), vec![]),
    }
}

fn infer_tar_path_intent(args: &[&str]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut extract_mode = false;
    let mut create_mode = false;
    let mut archive_path: Option<PathBuf> = None;
    let mut working_target: Option<PathBuf> = None;
    let mut input_paths = Vec::new();
    let mut expect_file = false;
    let mut expect_directory = false;

    for arg in args {
        if expect_file {
            if looks_like_path(arg) {
                archive_path = Some(PathBuf::from(arg));
            }
            expect_file = false;
            continue;
        }
        if expect_directory {
            if looks_like_path(arg) {
                working_target = Some(PathBuf::from(arg));
            }
            expect_directory = false;
            continue;
        }

        if *arg == "-f" {
            expect_file = true;
            continue;
        }
        if *arg == "-C" {
            expect_directory = true;
            continue;
        }
        if arg.starts_with('-') {
            extract_mode |= arg.contains('x') || *arg == "--extract";
            create_mode |= arg.contains('c') || *arg == "--create";
            if let Some(value) = arg.strip_prefix("--file=") {
                if looks_like_path(value) {
                    archive_path = Some(PathBuf::from(value));
                }
            }
            if let Some(value) = arg.strip_prefix("--directory=") {
                if looks_like_path(value) {
                    working_target = Some(PathBuf::from(value));
                }
            }
            continue;
        }
        if looks_like_path(arg) {
            if archive_path.is_none() {
                archive_path = Some(PathBuf::from(arg));
            } else {
                input_paths.push(PathBuf::from(arg));
            }
        }
    }

    if extract_mode {
        let mut reads = Vec::new();
        if let Some(archive) = archive_path {
            reads.push(archive);
        }
        let mut writes = Vec::new();
        if let Some(target) = working_target {
            writes.push(target);
        }
        (reads, writes)
    } else if create_mode {
        let mut reads = input_paths;
        let mut writes = Vec::new();
        if let Some(archive) = archive_path {
            writes.push(archive);
        } else {
            reads.extend(
                args.iter()
                    .copied()
                    .filter(|arg| looks_like_path(arg))
                    .map(PathBuf::from),
            );
        }
        (reads, writes)
    } else {
        (
            args.iter()
                .copied()
                .filter(|arg| looks_like_path(arg))
                .map(PathBuf::from)
                .collect(),
            vec![],
        )
    }
}

fn looks_like_network_command(command: &str) -> bool {
    matches!(
        command_basename(command),
        "curl" | "wget" | "gh" | "http" | "https" | "scp" | "ssh" | "ping" | "nc"
    )
}

fn configured_sandbox(exec_policy: &ExecPolicyConfig) -> Option<harper_sandbox::SandboxConfig> {
    let sandbox = exec_policy.effective_sandbox_config();
    Some(harper_sandbox::SandboxConfig {
        enabled: sandbox.enabled.unwrap_or(false),
        allowed_dirs: sandbox
            .allowed_dirs
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        writable_dirs: sandbox
            .writable_dirs
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        allowed_commands: exec_policy.allowed_commands.clone(),
        blocked_commands: exec_policy.blocked_commands.clone(),
        readonly_home: sandbox.readonly_home.unwrap_or(false),
        network_access: sandbox.network_access.unwrap_or(true),
        max_execution_time_secs: sandbox.max_execution_time_secs,
    })
}

fn sandbox_profile_name(profile: SandboxProfile) -> &'static str {
    match profile {
        SandboxProfile::Disabled => "disabled",
        SandboxProfile::Workspace => "workspace",
        SandboxProfile::NetworkedWorkspace => "networked_workspace",
    }
}

fn sandbox_status_line(exec_policy: &ExecPolicyConfig, sandbox: Option<&Sandbox>) -> String {
    let config = exec_policy.effective_sandbox_config();
    if !config.enabled.unwrap_or(false) {
        return "sandbox: disabled".to_string();
    }

    let profile = exec_policy.effective_sandbox_profile();
    let mode = match profile {
        SandboxProfile::Disabled => "custom",
        _ => sandbox_profile_name(profile),
    };
    let backend = sandbox.map(Sandbox::backend_name).unwrap_or("unavailable");
    let network = if config.network_access.unwrap_or(true) {
        "on"
    } else {
        "off"
    };

    format!("sandbox: {mode} ({backend}, network: {network})")
}

fn path_within_root(base_dir: &Path, path: &Path, root: &Path) -> bool {
    let normalized_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    let normalized_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        base_dir.join(root)
    };
    normalized_path.starts_with(normalized_root)
}

fn writes_within_configured_writable_dirs(
    exec_policy: &ExecPolicyConfig,
    intent: &CommandSandboxIntent,
) -> bool {
    if intent.declared_write_paths.is_empty() {
        return true;
    }

    let sandbox = exec_policy.effective_sandbox_config();
    let writable_dirs = sandbox.writable_dirs.unwrap_or_default();
    if writable_dirs.is_empty() {
        return false;
    }

    let Ok(base_dir) = std::env::current_dir() else {
        return false;
    };
    let writable_roots: Vec<PathBuf> = writable_dirs.into_iter().map(PathBuf::from).collect();

    intent.declared_write_paths.iter().all(|path| {
        writable_roots
            .iter()
            .any(|root| path_within_root(&base_dir, path, root))
    })
}

fn approval_required_for_command(
    exec_policy: &ExecPolicyConfig,
    command_str: &str,
    intent: Option<&CommandSandboxIntent>,
) -> bool {
    match exec_policy.effective_approval_profile() {
        ApprovalProfile::Strict => true,
        ApprovalProfile::AllowListed => {
            let allowlisted = exec_policy
                .allowed_commands
                .as_ref()
                .map(|allowed| allowed.iter().any(|cmd| command_str.starts_with(cmd)))
                .unwrap_or(false);
            let intent_requires_approval = intent.is_some_and(|intent| {
                intent.requires_network
                    || (!intent.declared_write_paths.is_empty()
                        && !writes_within_configured_writable_dirs(exec_policy, intent))
            });
            !allowlisted || intent_requires_approval
        }
        ApprovalProfile::AllowAll => false,
    }
}

fn autonomous_retry_safe(
    exec_policy: &ExecPolicyConfig,
    command_str: &str,
    requires_approval: bool,
    intent: Option<&CommandSandboxIntent>,
) -> bool {
    if requires_approval {
        return false;
    }
    let Some(intent) = intent else {
        return true;
    };
    if let Some(policy) = intent.retry_policy.as_ref() {
        return matches!(policy, CommandRetryPolicy::Safe);
    }
    let command = parsing::parse_quoted_args(command_str)
        .ok()
        .and_then(|args| args.first().cloned())
        .map(|command| command_basename(&command).to_string())
        .unwrap_or_else(|| command_basename(command_str).to_string());
    let has_writes = !intent.declared_write_paths.is_empty();

    if intent.requires_network {
        return !has_writes && exec_policy.retries_network_command(&command);
    }
    if has_writes {
        return writes_within_configured_writable_dirs(exec_policy, intent)
            && exec_policy.retries_write_command(&command);
    }
    true
}

async fn execute_sandboxed_once(
    sandbox: &Sandbox,
    request: SandboxRequest,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    audit_ctx: Option<&CommandAuditContext<'_>>,
    command_str: &str,
) -> crate::core::error::HarperResult<CommandAttemptResult> {
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<(String, bool)>();
    let runtime_events_clone = runtime_events.cloned();
    let session_id_owned = audit_ctx
        .and_then(|ctx| ctx.session_id)
        .map(ToString::to_string);
    let command_owned = command_str.to_string();
    let stream_forwarder = tokio::spawn(async move {
        while let Some((chunk, is_error)) = stream_rx.recv().await {
            emit_command_output(
                runtime_events_clone.as_ref(),
                session_id_owned.as_deref(),
                &command_owned,
                chunk,
                is_error,
                false,
            )
            .await;
        }
    });
    let start = Instant::now();
    let result = sandbox
        .execute_request_streaming(request, move |chunk, is_error| {
            let _ = stream_tx.send((chunk, is_error));
        })
        .await?;
    stream_forwarder
        .await
        .map_err(|e| HarperError::Command(format!("Sandbox output forwarding failed: {}", e)))?;
    let duration_ms = start.elapsed().as_millis() as i64;
    let exit_code = result.output.status.code();
    let stdout_preview = bytes_to_preview(&result.output.stdout);
    let stderr_preview = bytes_to_preview(&result.output.stderr);
    let stdout_text = String::from_utf8_lossy(&result.output.stdout).into_owned();
    let stderr_text = String::from_utf8_lossy(&result.output.stderr).into_owned();
    let output_preview = if result.output.status.success() {
        preview_text(&stdout_text)
    } else if !stderr_text.trim().is_empty() {
        preview_text(&stderr_text)
    } else {
        preview_text(&stdout_text)
    };
    let has_error_output = !result.output.status.success() || !stderr_text.trim().is_empty();

    emit_command_output(
        runtime_events,
        audit_ctx.and_then(|ctx| ctx.session_id),
        command_str,
        String::new(),
        has_error_output,
        true,
    )
    .await;

    Ok(CommandAttemptResult {
        output_text: if result.output.status.success() {
            stdout_text
        } else {
            stderr_text
        },
        success: result.output.status.success(),
        exit_code,
        duration_ms,
        stdout_preview,
        stderr_preview,
        output_preview,
        has_error_output,
    })
}

async fn execute_direct_once(
    command_str: &str,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    audit_ctx: Option<&CommandAuditContext<'_>>,
) -> crate::core::error::HarperResult<CommandAttemptResult> {
    let start = Instant::now();
    let mut child = TokioCommand::new("sh")
        .arg("-c")
        .arg(command_str)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| HarperError::Command(format!("Failed to execute command: {}", e)))?;

    let mut stdout_task = None;
    if let Some(stdout) = child.stdout.take() {
        let runtime_events = runtime_events.cloned();
        let session_id = audit_ctx.and_then(|ctx| ctx.session_id).map(str::to_string);
        let db_path = audit_ctx.and_then(|ctx| database_path(ctx.conn));
        let command = command_str.to_string();
        stdout_task = Some(tokio::spawn(async move {
            let live_conn = db_path
                .as_deref()
                .and_then(|path| crate::memory::storage::create_connection(path).ok());
            let mut lines = BufReader::new(stdout).lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&line);
                collected.push('\n');
                if let (Some(conn), Some(session_id)) = (&live_conn, session_id.as_deref()) {
                    let _ = crate::tools::plan::append_active_plan_job_output(
                        conn,
                        session_id,
                        &format!("{}\n", line),
                        false,
                    );
                    if let Some(sink) = runtime_events.as_ref() {
                        let plan = crate::memory::storage::load_plan_state(conn, session_id)
                            .ok()
                            .flatten();
                        let _ = sink.plan_updated(session_id, plan).await;
                    }
                }
                emit_command_output(
                    runtime_events.as_ref(),
                    session_id.as_deref(),
                    &command,
                    format!("{}\n", line),
                    false,
                    false,
                )
                .await;
            }
            collected
        }));
    }

    let mut stderr_task = None;
    if let Some(stderr) = child.stderr.take() {
        let runtime_events = runtime_events.cloned();
        let session_id = audit_ctx.and_then(|ctx| ctx.session_id).map(str::to_string);
        let db_path = audit_ctx.and_then(|ctx| database_path(ctx.conn));
        let command = command_str.to_string();
        stderr_task = Some(tokio::spawn(async move {
            let live_conn = db_path
                .as_deref()
                .and_then(|path| crate::memory::storage::create_connection(path).ok());
            let mut lines = BufReader::new(stderr).lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&line);
                collected.push('\n');
                if let (Some(conn), Some(session_id)) = (&live_conn, session_id.as_deref()) {
                    let _ = crate::tools::plan::append_active_plan_job_output(
                        conn,
                        session_id,
                        &format!("{}\n", line),
                        true,
                    );
                    if let Some(sink) = runtime_events.as_ref() {
                        let plan = crate::memory::storage::load_plan_state(conn, session_id)
                            .ok()
                            .flatten();
                        let _ = sink.plan_updated(session_id, plan).await;
                    }
                }
                emit_command_output(
                    runtime_events.as_ref(),
                    session_id.as_deref(),
                    &command,
                    format!("{}\n", line),
                    true,
                    false,
                )
                .await;
            }
            collected
        }));
    }

    let status = child
        .wait()
        .await
        .map_err(|e| HarperError::Command(format!("Failed to wait for command: {}", e)))?;
    let duration_ms = start.elapsed().as_millis() as i64;
    let stdout_text = match stdout_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    let stderr_text = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    emit_command_output(
        runtime_events,
        audit_ctx.and_then(|ctx| ctx.session_id),
        command_str,
        String::new(),
        !status.success() || !stderr_text.trim().is_empty(),
        true,
    )
    .await;
    let exit_code = status.code();
    let stdout_preview = bytes_to_preview(stdout_text.as_bytes());
    let stderr_preview = bytes_to_preview(stderr_text.as_bytes());
    let output_preview = if status.success() {
        preview_text(&stdout_text)
    } else if !stderr_text.trim().is_empty() {
        preview_text(&stderr_text)
    } else {
        preview_text(&stdout_text)
    };
    let has_error_output = !status.success() || !stderr_text.trim().is_empty();

    Ok(CommandAttemptResult {
        output_text: if status.success() {
            stdout_text
        } else {
            stderr_text
        },
        success: status.success(),
        exit_code,
        duration_ms,
        stdout_preview,
        stderr_preview,
        output_preview,
        has_error_output,
    })
}

/// Execute a shell command with safety checks
pub async fn execute_command(
    response: &str,
    _config: &ApiConfig,
    exec_policy: &ExecPolicyConfig,
    sandbox_intent: Option<&CommandSandboxIntent>,
    audit_ctx: Option<&CommandAuditContext<'_>>,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<Arc<dyn RuntimeEventSink>>,
) -> crate::core::error::HarperResult<String> {
    let (command_string, resolved_intent) = parse_run_command_response(response, sandbox_intent)?;
    let command_str = command_string.as_str();

    if command_str.is_empty() {
        maybe_log_command(
            audit_ctx,
            runtime_events.as_ref(),
            command_str,
            "invalid",
            true,
            false,
            None,
            None,
            None,
            None,
            Some("No command provided".to_string()),
        )
        .await;
        return Err(HarperError::Command("No command provided".to_string()));
    }

    // Security check to prevent shell injection and dangerous commands
    // Note: This is a defense-in-depth measure. The primary security comes from user approval.
    // We allow wildcards (*, ?) and git revision syntax (~, ^) but block chaining and subshells.
    let dangerous_chars = [';', '|', '&', '`', '$', '(', ')', '<', '>', '\n', '\r'];
    if command_str.chars().any(|c| dangerous_chars.contains(&c)) {
        let message = "Command contains potentially dangerous shell metacharacters (like ;, |, &) or newlines. \
             Command chaining and redirection are not allowed for security.";
        maybe_log_command(
            audit_ctx,
            runtime_events.as_ref(),
            command_str,
            "blocked",
            true,
            false,
            None,
            None,
            None,
            None,
            Some(message.to_string()),
        )
        .await;
        return Err(HarperError::Command(message.to_string()));
    }

    // Additional check for common dangerous patterns
    let dangerous_patterns = [
        "rm -rf",
        "rmdir",
        "del ",
        "fdisk",
        "mkfs",
        "dd if=",
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
        "sudo",
        "su ",
        "chmod 777",
        "chown root",
        "passwd",
        "/etc/",
        "/bin/",
        "/sbin/",
        "/usr/bin/",
        "/usr/sbin/",
    ];

    for pattern in &dangerous_patterns {
        if command_str.contains(pattern) {
            let err = format!(
                "Command contains potentially dangerous pattern: '{}'. \
                        This command is not allowed for security reasons.",
                pattern
            );
            maybe_log_command(
                audit_ctx,
                runtime_events.as_ref(),
                command_str,
                "blocked",
                true,
                false,
                None,
                None,
                None,
                None,
                Some(err.clone()),
            )
            .await;
            return Err(HarperError::Command(err));
        }
    }

    // Check exec policy
    let mut requires_approval = true;

    if let Some(blocked) = &exec_policy.blocked_commands {
        if blocked.iter().any(|cmd| command_str.starts_with(cmd)) {
            let err = format!("Command '{}' is blocked by exec policy.", command_str);
            maybe_log_command(
                audit_ctx,
                runtime_events.as_ref(),
                command_str,
                "blocked",
                requires_approval,
                false,
                None,
                None,
                None,
                None,
                Some(err.clone()),
            )
            .await;
            return Err(HarperError::Command(err));
        }
    }

    requires_approval =
        approval_required_for_command(exec_policy, command_str, resolved_intent.as_ref());
    let mut approved = !requires_approval;

    // Ask for approval if required
    if requires_approval {
        emit_activity_update(
            runtime_events.as_ref(),
            audit_ctx.and_then(|ctx| ctx.session_id),
            Some(format!("waiting approval: {}", command_str)),
        )
        .await;
        if let Some(ctx) =
            audit_ctx.and_then(|ctx| ctx.session_id.map(|session_id| (ctx.conn, session_id)))
        {
            let _ = crate::tools::plan::start_plan_job(
                ctx.0,
                ctx.1,
                "run_command",
                Some(command_str.to_string()),
                PlanJobStatus::WaitingApproval,
            );
            emit_plan_update(runtime_events.as_ref(), ctx.0, ctx.1).await;
        }
        let is_approved = if let Some(appr) = approver {
            appr.approve("Execute command?", command_str).await?
        } else {
            // Fallback to spawn_blocking for stdin if no approver provided (legacy support)
            let prompt = "Execute command?".to_string();
            let cmd = command_str.to_string();
            let show_prompt = runtime_events.is_none();
            tokio::task::spawn_blocking(move || {
                if show_prompt {
                    println!(
                        "{} {} {} (y/n): ",
                        "System:".bold().magenta(),
                        prompt.bold().magenta(),
                        cmd.magenta()
                    );
                    io::stdout()
                        .flush()
                        .map_err(|e| HarperError::Io(e.to_string()))?;
                }

                let mut approval = String::new();
                io::stdin()
                    .read_line(&mut approval)
                    .map_err(|e| HarperError::Io(e.to_string()))?;
                Ok::<bool, HarperError>(approval.trim().eq_ignore_ascii_case("y"))
            })
            .await
            .map_err(|e| HarperError::Command(format!("Task execution failed: {}", e)))??
        };

        if !is_approved {
            emit_activity_update(
                runtime_events.as_ref(),
                audit_ctx.and_then(|ctx| ctx.session_id),
                Some("approval rejected".to_string()),
            )
            .await;
            if let Some(ctx) =
                audit_ctx.and_then(|ctx| ctx.session_id.map(|session_id| (ctx.conn, session_id)))
            {
                let _ = crate::tools::plan::finish_active_plan_job(
                    ctx.0,
                    ctx.1,
                    PlanJobStatus::Blocked,
                );
                emit_plan_update(runtime_events.as_ref(), ctx.0, ctx.1).await;
            }
            maybe_log_command(
                audit_ctx,
                runtime_events.as_ref(),
                command_str,
                "cancelled",
                requires_approval,
                false,
                None,
                None,
                None,
                None,
                Some("User rejected command".to_string()),
            )
            .await;
            return Ok("Command execution cancelled by user".to_string());
        }
        approved = true;
    }

    let sandbox = configured_sandbox(exec_policy)
        .filter(|config| config.enabled)
        .map(Sandbox::new);
    let sandbox_status = sandbox_status_line(exec_policy, sandbox.as_ref());

    if let Some(ctx) =
        audit_ctx.and_then(|ctx| ctx.session_id.map(|session_id| (ctx.conn, session_id)))
    {
        emit_activity_update(
            runtime_events.as_ref(),
            Some(ctx.1),
            Some(format!(
                "running command: {}; {}",
                command_str, sandbox_status
            )),
        )
        .await;
        let has_active_job = crate::memory::storage::load_plan_state(ctx.0, ctx.1)
            .ok()
            .flatten()
            .and_then(|plan| plan.runtime)
            .and_then(|runtime| runtime.active_job_id)
            .is_some();
        let _ = if has_active_job {
            crate::tools::plan::update_active_plan_job(ctx.0, ctx.1, PlanJobStatus::Running)
        } else {
            crate::tools::plan::start_plan_job(
                ctx.0,
                ctx.1,
                "run_command",
                Some(command_str.to_string()),
                PlanJobStatus::Running,
            )
        };
        emit_plan_update(runtime_events.as_ref(), ctx.0, ctx.1).await;
    }

    let retry_safe = autonomous_retry_safe(
        exec_policy,
        command_str,
        requires_approval,
        resolved_intent.as_ref(),
    );
    emit_command_output(
        runtime_events.as_ref(),
        audit_ctx.and_then(|ctx| ctx.session_id),
        command_str,
        format!("{}\n", sandbox_status),
        false,
        false,
    )
    .await;
    let mut attempt = 0;
    loop {
        let attempt_result = if let Some(sandbox) = sandbox.as_ref() {
            let request = build_sandbox_request(command_str, resolved_intent.as_ref())?;
            execute_sandboxed_once(
                sandbox,
                request,
                runtime_events.as_ref(),
                audit_ctx,
                command_str,
            )
            .await?
        } else {
            execute_direct_once(command_str, runtime_events.as_ref(), audit_ctx).await?
        };

        if attempt_result.success
            || !retry_safe
            || attempt >= exec_policy.effective_retry_max_attempts()
        {
            maybe_log_command(
                audit_ctx,
                runtime_events.as_ref(),
                command_str,
                if attempt_result.success {
                    "succeeded"
                } else {
                    "failed"
                },
                requires_approval,
                approved,
                attempt_result.exit_code,
                Some(attempt_result.duration_ms),
                attempt_result.stdout_preview,
                attempt_result.stderr_preview,
                (!attempt_result.success)
                    .then_some("Command exited with non-zero status".to_string()),
            )
            .await;

            if let Some(ctx) =
                audit_ctx.and_then(|ctx| ctx.session_id.map(|session_id| (ctx.conn, session_id)))
            {
                let _ = crate::tools::plan::finish_active_plan_job_with_output(
                    ctx.0,
                    ctx.1,
                    if attempt_result.success {
                        PlanJobStatus::Succeeded
                    } else {
                        PlanJobStatus::Failed
                    },
                    attempt_result.output_preview,
                    attempt_result.has_error_output,
                );
                emit_plan_update(runtime_events.as_ref(), ctx.0, ctx.1).await;
            }

            return Ok(attempt_result.output_text);
        }

        maybe_log_command(
            audit_ctx,
            runtime_events.as_ref(),
            command_str,
            "retrying",
            requires_approval,
            approved,
            attempt_result.exit_code,
            Some(attempt_result.duration_ms),
            attempt_result.stdout_preview.clone(),
            attempt_result.stderr_preview.clone(),
            Some("Autonomous retry scheduled after first failure".to_string()),
        )
        .await;
        if let Some(ctx) =
            audit_ctx.and_then(|ctx| ctx.session_id.map(|session_id| (ctx.conn, session_id)))
        {
            let _ = crate::tools::plan::record_active_plan_retry_followup(
                ctx.0,
                ctx.1,
                Some(command_str.to_string()),
            );
            let _ =
                crate::tools::plan::update_active_plan_job(ctx.0, ctx.1, PlanJobStatus::Running);
            emit_plan_update(runtime_events.as_ref(), ctx.0, ctx.1).await;
        }
        emit_activity_update(
            runtime_events.as_ref(),
            audit_ctx.and_then(|ctx| ctx.session_id),
            Some(format!("retrying command: {}", command_str)),
        )
        .await;
        attempt += 1;
    }
}

async fn emit_plan_update(
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    conn: &Connection,
    session_id: &str,
) {
    if let Some(sink) = runtime_events {
        let plan = crate::memory::storage::load_plan_state(conn, session_id)
            .ok()
            .flatten();
        let _ = sink.plan_updated(session_id, plan).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn maybe_log_command(
    audit_ctx: Option<&CommandAuditContext<'_>>,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    command: &str,
    status: &str,
    requires_approval: bool,
    approved: bool,
    exit_code: Option<i32>,
    duration_ms: Option<i64>,
    stdout_preview: Option<String>,
    stderr_preview: Option<String>,
    error_message: Option<String>,
) {
    if let Some(ctx) = audit_ctx {
        let record = CommandLogRecord::new(
            ctx.session_id,
            command,
            ctx.source,
            requires_approval,
            approved,
            status,
            exit_code,
            duration_ms,
            stdout_preview,
            stderr_preview,
            error_message,
        );
        if let Err(err) = storage::insert_command_log(ctx.conn, &record) {
            if runtime_events.is_none() {
                eprintln!("Warning: failed to persist command log: {}", err);
            } else {
                emit_activity_update(
                    runtime_events,
                    ctx.session_id,
                    Some(format!("Warning: failed to persist command log: {}", err)),
                )
                .await;
            }
        }
    }
}

fn preview_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let preview: String = trimmed.chars().take(OUTPUT_PREVIEW_LIMIT).collect();
    if trimmed.chars().count() > OUTPUT_PREVIEW_LIMIT {
        Some(format!("{}…", preview))
    } else {
        Some(preview)
    }
}

fn bytes_to_preview(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(bytes);
    let mut preview = text.chars().take(OUTPUT_PREVIEW_LIMIT).collect::<String>();
    if text.chars().count() > OUTPUT_PREVIEW_LIMIT {
        preview.push('…');
    }
    Some(preview)
}

#[cfg(test)]
mod tests {
    use super::{
        approval_required_for_command, autonomous_retry_safe, build_sandbox_request,
        configured_sandbox, execute_command, infer_path_intent, looks_like_network_command,
        maybe_log_command, parse_run_command_response, sandbox_status_line, CommandAuditContext,
        CommandRetryPolicy, CommandSandboxIntent,
    };
    use crate::core::error::HarperResult;
    use crate::core::io_traits::RuntimeEventSink;
    use crate::core::plan::{PlanFollowup, PlanItem, PlanState, PlanStepStatus};
    use crate::core::{ApiConfig, ApiProvider};
    use crate::runtime::config::{
        ApprovalProfile, ExecPolicyConfig, SandboxConfig as RuntimeSandboxConfig, SandboxProfile,
    };
    use rusqlite::Connection;

    #[derive(Default)]
    struct CapturingSink {
        activities: std::sync::Mutex<Vec<String>>,
    }

    impl CapturingSink {
        fn activities(&self) -> Vec<String> {
            self.activities.lock().expect("activities lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeEventSink for CapturingSink {
        async fn plan_updated(
            &self,
            _session_id: &str,
            _plan: Option<PlanState>,
        ) -> HarperResult<()> {
            Ok(())
        }

        async fn agents_updated(
            &self,
            _session_id: &str,
            _agents: Option<crate::core::agents::ResolvedAgents>,
        ) -> HarperResult<()> {
            Ok(())
        }

        async fn activity_updated(
            &self,
            _session_id: &str,
            status: Option<String>,
        ) -> HarperResult<()> {
            self.activities
                .lock()
                .expect("activities lock")
                .push(status.unwrap_or_default());
            Ok(())
        }

        async fn command_output_updated(
            &self,
            _session_id: &str,
            _command: String,
            _chunk: String,
            _is_error: bool,
            _done: bool,
        ) -> HarperResult<()> {
            Ok(())
        }
    }

    fn test_config() -> ApiConfig {
        ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        }
    }

    #[test]
    fn build_sandbox_request_extracts_command_args_and_declared_paths() {
        let request = build_sandbox_request(r#"cat "./notes file.txt" ../README.md"#, None)
            .expect("sandbox request");

        assert_eq!(request.command, "cat");
        assert_eq!(
            request.args,
            vec!["./notes file.txt".to_string(), "../README.md".to_string()]
        );
        assert_eq!(
            request.declared_read_paths,
            vec![
                std::path::PathBuf::from("./notes file.txt"),
                std::path::PathBuf::from("../README.md")
            ]
        );
        assert!(request.declared_write_paths.is_empty());
        assert!(!request.requires_network);
    }

    #[test]
    fn build_sandbox_request_marks_network_tools() {
        let request =
            build_sandbox_request("curl https://example.com", None).expect("sandbox request");
        assert_eq!(request.command, "curl");
        assert!(request.requires_network);
        assert!(looks_like_network_command("gh"));
        assert!(!looks_like_network_command("cat"));
    }

    #[test]
    fn build_sandbox_request_prefers_explicit_intent_over_heuristics() {
        let intent = CommandSandboxIntent {
            declared_read_paths: vec![std::path::PathBuf::from("./input.txt")],
            declared_write_paths: vec![std::path::PathBuf::from("./out.txt")],
            requires_network: true,
            retry_policy: Some(CommandRetryPolicy::Safe),
        };
        let request =
            build_sandbox_request("cat ./ignored.txt", Some(&intent)).expect("sandbox request");

        assert_eq!(request.declared_read_paths, intent.declared_read_paths);
        assert_eq!(request.declared_write_paths, intent.declared_write_paths);
        assert!(request.requires_network);
    }

    #[test]
    fn parse_run_command_response_reads_enhanced_bracket_payload() {
        let (command, intent) = parse_run_command_response(
            r#"[RUN_COMMAND {"command":"cp ./src.txt ./out.txt","declared_read_paths":["./src.txt"],"declared_write_paths":["./out.txt"],"requires_network":true,"retry_policy":"safe"}]"#,
            None,
        )
        .expect("parse run command");

        assert_eq!(command, "cp ./src.txt ./out.txt");
        let intent = intent.expect("sandbox intent");
        assert_eq!(
            intent.declared_read_paths,
            vec![std::path::PathBuf::from("./src.txt")]
        );
        assert_eq!(
            intent.declared_write_paths,
            vec![std::path::PathBuf::from("./out.txt")]
        );
        assert!(intent.requires_network);
        assert_eq!(intent.retry_policy, Some(CommandRetryPolicy::Safe));
    }

    #[test]
    fn parse_run_command_response_keeps_plain_bracket_command() {
        let fallback = CommandSandboxIntent {
            declared_read_paths: vec![std::path::PathBuf::from("./input.txt")],
            declared_write_paths: vec![],
            requires_network: false,
            retry_policy: Some(CommandRetryPolicy::Never),
        };
        let (command, intent) =
            parse_run_command_response("[RUN_COMMAND cat ./input.txt]", Some(&fallback))
                .expect("parse run command");

        assert_eq!(command, "cat ./input.txt");
        assert_eq!(intent, Some(fallback));
    }

    #[test]
    fn infer_path_intent_marks_rm_targets_as_writes() {
        let (reads, writes) = infer_path_intent("rm", &["./tmp/file.txt"]);
        assert!(reads.is_empty());
        assert_eq!(writes, vec![std::path::PathBuf::from("./tmp/file.txt")]);
    }

    #[test]
    fn infer_path_intent_marks_cp_sources_and_destination() {
        let (reads, writes) = infer_path_intent("cp", &["./src.txt", "./dest.txt"]);
        assert_eq!(reads, vec![std::path::PathBuf::from("./src.txt")]);
        assert_eq!(writes, vec![std::path::PathBuf::from("./dest.txt")]);
    }

    #[test]
    fn infer_path_intent_marks_mv_sources_and_destination() {
        let (reads, writes) = infer_path_intent("mv", &["./a.txt", "./b.txt", "./target/"]);
        assert_eq!(
            reads,
            vec![
                std::path::PathBuf::from("./a.txt"),
                std::path::PathBuf::from("./b.txt")
            ]
        );
        assert_eq!(writes, vec![std::path::PathBuf::from("./target/")]);
    }

    #[test]
    fn infer_path_intent_marks_sed_in_place_targets_as_writes() {
        let (reads, writes) = infer_path_intent("sed", &["-i", "s/a/b/", "./file.txt"]);
        assert!(reads.is_empty());
        assert_eq!(writes, vec![std::path::PathBuf::from("./file.txt")]);
    }

    #[test]
    fn infer_path_intent_marks_tee_targets_as_writes() {
        let (reads, writes) = infer_path_intent("tee", &["./out.txt", "./audit.log"]);
        assert!(reads.is_empty());
        assert_eq!(
            writes,
            vec![
                std::path::PathBuf::from("./out.txt"),
                std::path::PathBuf::from("./audit.log")
            ]
        );
    }

    #[test]
    fn infer_path_intent_marks_tar_extract_archive_read_and_target_write() {
        let (reads, writes) = infer_path_intent("tar", &["-xf", "./archive.tar", "-C", "./out"]);
        assert_eq!(reads, vec![std::path::PathBuf::from("./archive.tar")]);
        assert_eq!(writes, vec![std::path::PathBuf::from("./out")]);
    }

    #[test]
    fn infer_path_intent_marks_tar_create_inputs_read_and_archive_write() {
        let (reads, writes) =
            infer_path_intent("tar", &["-cf", "./archive.tar", "./src", "./README.md"]);
        assert_eq!(
            reads,
            vec![
                std::path::PathBuf::from("./src"),
                std::path::PathBuf::from("./README.md")
            ]
        );
        assert_eq!(writes, vec![std::path::PathBuf::from("./archive.tar")]);
    }

    #[test]
    fn infer_path_intent_marks_find_delete_roots_as_writes() {
        let (reads, writes) = infer_path_intent("find", &["./tmp", "-name", "*.log", "-delete"]);
        assert!(reads.is_empty());
        assert_eq!(writes, vec![std::path::PathBuf::from("./tmp")]);
    }

    #[test]
    fn configured_sandbox_maps_runtime_policy() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowListed),
            execution_strategy: None,
            allowed_commands: Some(vec!["git".to_string()]),
            blocked_commands: Some(vec!["rm".to_string()]),
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: Some(RuntimeSandboxConfig {
                enabled: Some(true),
                allowed_dirs: Some(vec!["/tmp".to_string()]),
                writable_dirs: Some(vec!["/tmp/work".to_string()]),
                network_access: Some(false),
                readonly_home: Some(true),
                max_execution_time_secs: Some(15),
            }),
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };

        let sandbox = configured_sandbox(&exec_policy).expect("sandbox config");
        assert!(sandbox.enabled);
        assert_eq!(sandbox.allowed_dirs, vec![std::path::PathBuf::from("/tmp")]);
        assert_eq!(
            sandbox.writable_dirs,
            vec![std::path::PathBuf::from("/tmp/work")]
        );
        assert_eq!(sandbox.allowed_commands, exec_policy.allowed_commands);
        assert_eq!(sandbox.blocked_commands, exec_policy.blocked_commands);
        assert!(!sandbox.network_access);
        assert!(sandbox.readonly_home);
        assert_eq!(sandbox.max_execution_time_secs, Some(15));
    }

    #[test]
    fn sandbox_status_line_reports_disabled_mode() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: None,
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };

        assert_eq!(sandbox_status_line(&exec_policy, None), "sandbox: disabled");
    }

    #[test]
    fn sandbox_status_line_reports_enabled_mode() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: None,
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Workspace),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        let sandbox = configured_sandbox(&exec_policy).map(harper_sandbox::Sandbox::new);

        let status = sandbox_status_line(&exec_policy, sandbox.as_ref());
        assert!(status.starts_with("sandbox: workspace ("));
        assert!(status.ends_with(", network: off)"));
    }

    #[test]
    fn approval_profile_allow_all_skips_approval() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowAll),
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        assert!(!approval_required_for_command(
            &exec_policy,
            "git status",
            None
        ));
    }

    #[test]
    fn approval_profile_strict_requires_approval_even_for_allowlisted_commands() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::Strict),
            execution_strategy: None,
            allowed_commands: Some(vec!["git".to_string()]),
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        assert!(approval_required_for_command(
            &exec_policy,
            "git status",
            None
        ));
    }

    #[test]
    fn approval_profile_allow_listed_uses_allowlist() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowListed),
            execution_strategy: None,
            allowed_commands: Some(vec!["git".to_string()]),
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        assert!(!approval_required_for_command(
            &exec_policy,
            "git status",
            None
        ));
        assert!(approval_required_for_command(&exec_policy, "ls -la", None));
    }

    #[test]
    fn approval_profile_allow_listed_requires_approval_for_declared_writes() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowListed),
            execution_strategy: None,
            allowed_commands: Some(vec!["cp".to_string()]),
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Workspace),
            sandbox: Some(crate::runtime::config::SandboxConfig {
                enabled: Some(true),
                allowed_dirs: Some(vec![".".to_string()]),
                writable_dirs: Some(vec!["./safe".to_string()]),
                network_access: Some(false),
                readonly_home: Some(true),
                max_execution_time_secs: Some(30),
            }),
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        let intent = CommandSandboxIntent {
            declared_read_paths: vec![std::path::PathBuf::from("./src.txt")],
            declared_write_paths: vec![std::path::PathBuf::from("./out.txt")],
            requires_network: false,
            retry_policy: None,
        };
        assert!(approval_required_for_command(
            &exec_policy,
            "cp ./src.txt ./out.txt",
            Some(&intent)
        ));
    }

    #[test]
    fn approval_profile_allow_listed_skips_approval_for_writes_inside_writable_roots() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowListed),
            execution_strategy: None,
            allowed_commands: Some(vec!["cp".to_string()]),
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Workspace),
            sandbox: Some(crate::runtime::config::SandboxConfig {
                enabled: Some(true),
                allowed_dirs: Some(vec![".".to_string()]),
                writable_dirs: Some(vec!["./safe".to_string()]),
                network_access: Some(false),
                readonly_home: Some(true),
                max_execution_time_secs: Some(30),
            }),
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        let intent = CommandSandboxIntent {
            declared_read_paths: vec![std::path::PathBuf::from("./src.txt")],
            declared_write_paths: vec![std::path::PathBuf::from("./safe/out.txt")],
            requires_network: false,
            retry_policy: None,
        };
        assert!(!approval_required_for_command(
            &exec_policy,
            "cp ./src.txt ./safe/out.txt",
            Some(&intent)
        ));
    }

    #[test]
    fn approval_profile_allow_listed_requires_approval_for_network_intent() {
        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowListed),
            execution_strategy: None,
            allowed_commands: Some(vec!["curl".to_string()]),
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        let intent = CommandSandboxIntent {
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: true,
            retry_policy: None,
        };
        assert!(approval_required_for_command(
            &exec_policy,
            "curl https://example.com",
            Some(&intent)
        ));
    }

    #[test]
    fn autonomous_retry_safe_allows_non_network_readonly_commands() {
        let exec_policy = ExecPolicyConfig::default();
        let intent = CommandSandboxIntent {
            declared_read_paths: vec![std::path::PathBuf::from("./input.txt")],
            declared_write_paths: vec![],
            requires_network: false,
            retry_policy: None,
        };
        assert!(autonomous_retry_safe(
            &exec_policy,
            "cat ./input.txt",
            false,
            Some(&intent)
        ));
        assert!(!autonomous_retry_safe(
            &exec_policy,
            "cp ./input.txt ./out.txt",
            false,
            Some(&CommandSandboxIntent {
                declared_write_paths: vec![std::path::PathBuf::from("./out.txt")],
                ..intent.clone()
            })
        ));
    }

    #[test]
    fn autonomous_retry_safe_allows_known_network_and_write_commands() {
        let exec_policy = ExecPolicyConfig {
            sandbox: Some(crate::runtime::config::SandboxConfig {
                enabled: None,
                allowed_dirs: None,
                writable_dirs: Some(vec![".".to_string()]),
                network_access: None,
                readonly_home: None,
                max_execution_time_secs: None,
            }),
            ..Default::default()
        };

        assert!(autonomous_retry_safe(
            &exec_policy,
            "curl https://example.com",
            false,
            Some(&CommandSandboxIntent {
                requires_network: true,
                ..Default::default()
            })
        ));
        assert!(autonomous_retry_safe(
            &exec_policy,
            "mkdir ./build",
            false,
            Some(&CommandSandboxIntent {
                declared_write_paths: vec![std::path::PathBuf::from("./build")],
                ..Default::default()
            })
        ));
        assert!(!autonomous_retry_safe(
            &exec_policy,
            "tar -xf archive.tar ./dst",
            false,
            Some(&CommandSandboxIntent {
                declared_write_paths: vec![std::path::PathBuf::from("./dst")],
                ..Default::default()
            })
        ));
    }

    #[test]
    fn autonomous_retry_safe_honors_explicit_retry_policy() {
        let exec_policy = ExecPolicyConfig::default();
        assert!(autonomous_retry_safe(
            &exec_policy,
            "custom-tool ./input",
            false,
            Some(&CommandSandboxIntent {
                declared_read_paths: vec![],
                declared_write_paths: vec![std::path::PathBuf::from("./out.txt")],
                requires_network: true,
                retry_policy: Some(CommandRetryPolicy::Safe),
            })
        ));
        assert!(!autonomous_retry_safe(
            &exec_policy,
            "cat ./input",
            false,
            Some(&CommandSandboxIntent {
                declared_read_paths: vec![std::path::PathBuf::from("./input.txt")],
                declared_write_paths: vec![],
                requires_network: false,
                retry_policy: Some(CommandRetryPolicy::Never),
            })
        ));
    }

    #[tokio::test]
    async fn execute_command_retries_once_for_retry_safe_failure() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "retry-safe-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Check failing command".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let exec_policy = ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowAll),
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };
        let audit_ctx = CommandAuditContext {
            conn: &conn,
            session_id: Some("retry-safe-session"),
            source: "test",
        };

        let _ = execute_command(
            "[RUN_COMMAND false]",
            &test_config(),
            &exec_policy,
            None,
            Some(&audit_ctx),
            None,
            None,
        )
        .await
        .expect("command execution should return stderr text");

        let plan = crate::memory::storage::load_plan_state(&conn, "retry-safe-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items[0].status, PlanStepStatus::Blocked);
        assert!(matches!(
            plan.runtime
                .as_ref()
                .and_then(|runtime| runtime.followup.as_ref()),
            Some(PlanFollowup::RetryOrReplan { retry_count: 2, .. })
        ));
    }

    #[tokio::test]
    async fn maybe_log_command_routes_persist_failure_through_sink() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        let audit_ctx = CommandAuditContext {
            conn: &conn,
            session_id: Some("session-warn"),
            source: "test",
        };
        let sink = std::sync::Arc::new(CapturingSink::default());
        let trait_sink: std::sync::Arc<dyn RuntimeEventSink> = sink.clone();

        maybe_log_command(
            Some(&audit_ctx),
            Some(&trait_sink),
            "echo hi",
            "succeeded",
            true,
            true,
            Some(0),
            Some(1),
            None,
            None,
            None,
        )
        .await;

        let activities = sink.activities();
        assert_eq!(activities.len(), 1);
        assert!(
            activities[0].starts_with("Warning: failed to persist command log:"),
            "unexpected activity: {}",
            activities[0]
        );

        maybe_log_command(
            Some(&audit_ctx),
            None,
            "echo hi",
            "succeeded",
            true,
            true,
            Some(0),
            Some(1),
            None,
            None,
            None,
        )
        .await;
        assert_eq!(sink.activities().len(), 1);
    }
}
