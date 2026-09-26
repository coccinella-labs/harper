// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::errors::{Result, SandboxError};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub allowed_dirs: Vec<PathBuf>,
    pub writable_dirs: Vec<PathBuf>,
    pub allowed_commands: Option<Vec<String>>,
    pub blocked_commands: Option<Vec<String>>,
    pub readonly_home: bool,
    pub network_access: bool,
    pub max_execution_time_secs: Option<u64>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_dirs: vec![],
            writable_dirs: vec![],
            allowed_commands: None,
            blocked_commands: None,
            readonly_home: true,
            network_access: false,
            max_execution_time_secs: Some(30),
        }
    }
}

pub fn from_env() -> SandboxConfig {
    let enabled = std::env::var("HARPER_SANDBOX_ENABLED")
        .map(|v| v == "true")
        .unwrap_or(false);

    let allowed_dirs: Vec<PathBuf> = std::env::var("HARPER_SANDBOX_ALLOWED_DIRS")
        .map(|v| v.split(':').map(PathBuf::from).collect())
        .unwrap_or_default();
    let writable_dirs: Vec<PathBuf> = std::env::var("HARPER_SANDBOX_WRITABLE_DIRS")
        .map(|v| v.split(':').map(PathBuf::from).collect())
        .unwrap_or_default();

    let allowed_commands: Option<Vec<String>> = std::env::var("HARPER_SANDBOX_ALLOWED_COMMANDS")
        .map(|v| v.split(':').map(String::from).collect())
        .ok();

    let blocked_commands: Option<Vec<String>> = std::env::var("HARPER_SANDBOX_BLOCKED_COMMANDS")
        .map(|v| v.split(':').map(String::from).collect())
        .ok();

    let network_access = std::env::var("HARPER_SANDBOX_NETWORK")
        .map(|v| v == "true")
        .unwrap_or(false);

    let readonly_home = std::env::var("HARPER_SANDBOX_READONLY_HOME")
        .map(|v| v != "false")
        .unwrap_or(true);

    SandboxConfig {
        enabled,
        allowed_dirs,
        writable_dirs,
        allowed_commands,
        blocked_commands,
        network_access,
        readonly_home,
        max_execution_time_secs: Some(30),
    }
}

pub fn is_command_allowed(config: &SandboxConfig, command: &str) -> bool {
    if let Some(blocked) = &config.blocked_commands {
        if blocked
            .iter()
            .any(|blocked| blocked == command_name(command))
        {
            return false;
        }
    }

    match &config.allowed_commands {
        Some(allowed) => allowed
            .iter()
            .any(|allowed| allowed == command_name(command)),
        None => true,
    }
}

pub fn validate_command(config: &SandboxConfig, command: &str) -> Result<()> {
    if is_command_allowed(config, command) {
        Ok(())
    } else {
        Err(SandboxError::CommandBlocked {
            command: command_name(command).to_string(),
        })
    }
}

pub fn validate_working_dir(config: &SandboxConfig, path: &Path) -> Result<()> {
    validate_read_path_access(config, path, path.parent().unwrap_or(Path::new("/")))
}

pub fn validate_requested_paths(
    config: &SandboxConfig,
    args: &[&str],
    base_dir: &Path,
) -> Result<()> {
    for arg in args {
        if !looks_like_path(arg) {
            continue;
        }
        validate_read_path_access(config, Path::new(arg), base_dir)?;
    }
    Ok(())
}

pub fn validate_read_path_access(
    config: &SandboxConfig,
    path: &Path,
    base_dir: &Path,
) -> Result<()> {
    if config.allowed_dirs.is_empty() && config.writable_dirs.is_empty() {
        return Ok(());
    }

    let normalized = normalize_path(path, base_dir)?;
    let allowed = config
        .allowed_dirs
        .iter()
        .chain(config.writable_dirs.iter())
        .filter_map(|dir| normalize_path(dir, base_dir).ok())
        .any(|allowed_dir| normalized.starts_with(&allowed_dir));

    if allowed {
        Ok(())
    } else {
        Err(SandboxError::PathBlocked { path: normalized })
    }
}

pub fn validate_write_path_access(
    config: &SandboxConfig,
    path: &Path,
    base_dir: &Path,
) -> Result<()> {
    let normalized = normalize_path(path, base_dir)?;
    validate_read_path_access(config, &normalized, base_dir)?;

    if !config.writable_dirs.is_empty() {
        let writable = config
            .writable_dirs
            .iter()
            .filter_map(|dir| normalize_path(dir, base_dir).ok())
            .any(|writable_dir| normalized.starts_with(&writable_dir));
        if !writable {
            return Err(SandboxError::PathBlocked { path: normalized });
        }
    }

    if config.readonly_home
        && std::env::var("HOME")
            .ok()
            .map(PathBuf::from)
            .is_some_and(|home| normalized.starts_with(home))
    {
        return Err(SandboxError::PathBlocked { path: normalized });
    }

    Ok(())
}

pub fn validate_network_request(config: &SandboxConfig, requires_network: bool) -> Result<()> {
    if requires_network && !config.network_access {
        return Err(SandboxError::NetworkBlocked);
    }
    Ok(())
}

fn command_name(command: &str) -> &str {
    let executable = command.split_whitespace().next().unwrap_or(command);
    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(executable)
}

fn looks_like_path(arg: &str) -> bool {
    arg.starts_with('/')
        || arg.starts_with("./")
        || arg.starts_with("../")
        || arg.contains(std::path::MAIN_SEPARATOR)
}

fn normalize_path(path: &Path, base_dir: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };

    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    Ok(normalized)
}
