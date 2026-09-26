// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::errors::{Result, SandboxError};
use crate::policy::SandboxConfig;
use crate::request::SandboxRequest;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    Bubblewrap,
    SandboxExec,
    None,
}

pub fn detect_backend() -> SandboxBackend {
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new("/usr/bin/bwrap").exists()
            || std::path::Path::new("/bin/bwrap").exists()
        {
            return SandboxBackend::Bubblewrap;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if std::path::Path::new("/usr/bin/sandbox-exec").exists() {
            return SandboxBackend::SandboxExec;
        }
    }
    #[allow(unreachable_code)]
    SandboxBackend::None
}

pub fn backend_name(backend: SandboxBackend) -> &'static str {
    match backend {
        SandboxBackend::Bubblewrap => "bubblewrap (bwrap)",
        SandboxBackend::SandboxExec => "sandbox-exec (macOS)",
        SandboxBackend::None => "none",
    }
}

pub async fn execute_with_backend_streaming<C>(
    backend: SandboxBackend,
    config: &SandboxConfig,
    request: &SandboxRequest,
    on_output: C,
) -> Result<std::process::Output>
where
    C: FnMut(String, bool) + Send + 'static,
{
    match backend {
        SandboxBackend::None => execute_direct(config, request, on_output).await,
        SandboxBackend::Bubblewrap => execute_bwrap(config, request, on_output).await,
        SandboxBackend::SandboxExec => execute_sandbox_exec(config, request, on_output).await,
    }
}

async fn execute_direct(
    config: &SandboxConfig,
    request: &SandboxRequest,
    on_output: impl FnMut(String, bool) + Send + 'static,
) -> Result<std::process::Output> {
    let mut cmd = Command::new(&request.command);
    cmd.args(&request.args)
        .current_dir(&request.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in &request.env {
        cmd.env(key, value);
    }

    run_command_with_timeout_streaming(config, cmd, on_output).await
}

#[cfg(target_os = "linux")]
async fn execute_bwrap(
    config: &SandboxConfig,
    request: &SandboxRequest,
    on_output: impl FnMut(String, bool) + Send + 'static,
) -> Result<std::process::Output> {
    let mut bwrap_args = vec!["--unshare-pid".to_string()];

    if !config.network_access {
        bwrap_args.push("--unshare-net".to_string());
    }

    bpush(&mut bwrap_args, "--ro-bind", "/usr".to_string());
    bpush(&mut bwrap_args, "--ro-bind", "/bin".to_string());
    bpush(&mut bwrap_args, "--ro-bind", "/lib".to_string());
    bpush(&mut bwrap_args, "--ro-bind", "/lib64".to_string());

    if config.readonly_home {
        if let Ok(home) = std::env::var("HOME") {
            bpush(&mut bwrap_args, "--ro-bind", home);
        }
    } else if let Ok(home) = std::env::var("HOME") {
        bpush(&mut bwrap_args, "--ro-bind", home);
    }

    for dir in &config.allowed_dirs {
        if let Some(dir) = existing_mount_path(dir, &request.working_dir) {
            bpush(&mut bwrap_args, "--ro-bind", dir.display().to_string());
        }
    }
    for dir in &config.writable_dirs {
        if let Some(dir) = existing_mount_path(dir, &request.working_dir) {
            bpush(&mut bwrap_args, "--bind", dir.display().to_string());
        }
    }

    bpush(
        &mut bwrap_args,
        "--chdir",
        request.working_dir.display().to_string(),
    );

    bwrap_args.push("--".to_string());
    bwrap_args.push(request.command.clone());
    bwrap_args.extend(request.args.iter().cloned());

    let mut cmd = Command::new("bwrap");
    cmd.args(&bwrap_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &request.env {
        cmd.env(key, value);
    }
    run_command_with_timeout_streaming(config, cmd, on_output).await
}

#[cfg(not(target_os = "linux"))]
async fn execute_bwrap(
    _config: &SandboxConfig,
    _request: &SandboxRequest,
    _on_output: impl FnMut(String, bool) + Send + 'static,
) -> Result<std::process::Output> {
    Err(SandboxError::BackendUnavailable(
        "Bubblewrap only available on Linux".to_string(),
    ))
}

#[cfg(target_os = "macos")]
async fn execute_sandbox_exec(
    config: &SandboxConfig,
    request: &SandboxRequest,
    on_output: impl FnMut(String, bool) + Send + 'static,
) -> Result<std::process::Output> {
    let sandbox_profile = build_sandbox_exec_profile(config, &request.working_dir);

    let mut cmd = Command::new("sandbox-exec");
    cmd.arg("-p")
        .arg(&sandbox_profile)
        .arg(&request.command)
        .args(&request.args)
        .current_dir(&request.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &request.env {
        cmd.env(key, value);
    }
    run_command_with_timeout_streaming(config, cmd, on_output).await
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn existing_mount_path(path: &Path, working_dir: &Path) -> Option<PathBuf> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    };
    if candidate.exists() {
        Some(candidate.canonicalize().unwrap_or(candidate))
    } else {
        None
    }
}

#[cfg(any(target_os = "macos", test))]
fn escape_sandbox_string(value: &Path) -> String {
    value
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(any(target_os = "macos", test))]
fn build_sandbox_exec_profile(config: &SandboxConfig, working_dir: &Path) -> String {
    let mut sandbox_rules = vec![
        "(version 1)".to_string(),
        "(deny default)".to_string(),
        "(allow process*)".to_string(),
        "(allow sysctl-read)".to_string(),
        "(allow file-read-metadata)".to_string(),
    ];

    for system_path in ["/bin", "/usr", "/System", "/Library", "/dev/null"] {
        sandbox_rules.push(format!("(allow file-read* (subpath \"{}\"))", system_path));
    }

    for dir in &config.allowed_dirs {
        if let Some(dir) = existing_mount_path(dir, working_dir) {
            sandbox_rules.push(format!(
                "(allow file-read* (subpath \"{}\"))",
                escape_sandbox_string(&dir)
            ));
        }
    }
    for dir in &config.writable_dirs {
        if let Some(dir) = existing_mount_path(dir, working_dir) {
            let escaped = escape_sandbox_string(&dir);
            sandbox_rules.push(format!("(allow file-read* (subpath \"{}\"))", escaped));
            sandbox_rules.push(format!("(allow file-write* (subpath \"{}\"))", escaped));
        }
    }

    if config.network_access {
        sandbox_rules.push("(allow network*)".to_string());
    }

    sandbox_rules.join("\n")
}

#[cfg(not(target_os = "macos"))]
async fn execute_sandbox_exec(
    _config: &SandboxConfig,
    _request: &SandboxRequest,
    _on_output: impl FnMut(String, bool) + Send + 'static,
) -> Result<std::process::Output> {
    Err(SandboxError::BackendUnavailable(
        "sandbox-exec only available on macOS".to_string(),
    ))
}

async fn run_command_with_timeout_streaming(
    config: &SandboxConfig,
    mut cmd: Command,
    on_output: impl FnMut(String, bool) + Send + 'static,
) -> Result<std::process::Output> {
    let operation = async move {
        let mut child = cmd
            .spawn()
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, bool)>();

        let stdout_task = tokio::spawn(read_stream(stdout, false, tx.clone()));
        let stderr_task = tokio::spawn(read_stream(stderr, true, tx));
        let consumer_task = tokio::spawn(async move {
            let mut on_output = on_output;
            while let Some((chunk, is_error)) = rx.recv().await {
                on_output(chunk, is_error);
            }
        });

        let status = child
            .wait()
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;
        let stdout = stdout_task
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))??;
        let stderr = stderr_task
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))??;
        consumer_task
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;

        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    };

    if let Some(timeout) = config.max_execution_time_secs {
        match tokio::time::timeout(std::time::Duration::from_secs(timeout), operation).await {
            Ok(result) => result,
            Err(_) => Err(SandboxError::Timeout {
                timeout_secs: timeout,
            }),
        }
    } else {
        operation.await
    }
}

async fn read_stream(
    stream: Option<impl tokio::io::AsyncRead + Unpin>,
    is_error: bool,
    tx: mpsc::UnboundedSender<(String, bool)>,
) -> Result<Vec<u8>> {
    let Some(stream) = stream else {
        return Ok(Vec::new());
    };

    let mut reader = BufReader::new(stream);
    let mut all = Vec::new();
    loop {
        let mut buf = Vec::new();
        let read = reader
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|e| SandboxError::ExecutionFailed(e.to_string()))?;
        if read == 0 {
            break;
        }
        all.extend_from_slice(&buf);
        let chunk = String::from_utf8_lossy(&buf).into_owned();
        let _ = tx.send((chunk, is_error));
    }
    Ok(all)
}

#[cfg(target_os = "linux")]
fn bpush(args: &mut Vec<String>, flag: &str, val: impl Into<String>) {
    args.push(flag.to_string());
    args.push(val.into());
}

#[cfg(test)]
mod tests {
    use super::{build_sandbox_exec_profile, escape_sandbox_string, existing_mount_path};
    use crate::policy::SandboxConfig;
    use std::path::Path;

    #[test]
    fn relative_mounts_are_resolved_against_working_dir() {
        let working_dir = std::env::current_dir().expect("current dir");
        let mounted = existing_mount_path(Path::new("."), &working_dir).expect("mount path");

        assert!(mounted.is_absolute());
        assert_eq!(mounted, working_dir.canonicalize().expect("canonical cwd"));
    }

    #[test]
    fn sandbox_exec_profile_denies_default_and_scopes_paths() {
        let working_dir = std::env::current_dir().expect("current dir");
        let config = SandboxConfig {
            enabled: true,
            allowed_dirs: vec![Path::new(".").to_path_buf()],
            writable_dirs: vec![Path::new(".").to_path_buf()],
            network_access: false,
            ..SandboxConfig::default()
        };

        let profile = build_sandbox_exec_profile(&config, &working_dir);

        assert!(profile.contains("(deny default)"));
        assert!(!profile.contains("(allow default)"));
        assert!(!profile.contains("(allow network*)"));
        let escaped_working_dir =
            escape_sandbox_string(&working_dir.canonicalize().expect("canonical cwd"));
        assert!(profile.contains(&format!(
            "(allow file-read* (subpath \"{}\"))",
            escaped_working_dir
        )));
        assert!(profile.contains(&format!(
            "(allow file-write* (subpath \"{}\"))",
            escaped_working_dir
        )));
    }

    #[test]
    fn sandbox_exec_profile_allows_network_when_configured() {
        let config = SandboxConfig {
            enabled: true,
            network_access: true,
            ..SandboxConfig::default()
        };

        let profile =
            build_sandbox_exec_profile(&config, &std::env::current_dir().expect("current dir"));

        assert!(profile.contains("(allow network*)"));
    }
}
