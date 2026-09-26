// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("Sandbox backend not available: {0}")]
    BackendUnavailable(String),
    #[error("Command execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Command blocked by sandbox policy: {command}")]
    CommandBlocked { command: String },
    #[error("Path blocked by sandbox policy: {path}")]
    PathBlocked { path: PathBuf },
    #[error("Network blocked by sandbox policy")]
    NetworkBlocked,
    #[error("Command timed out after {timeout_secs} seconds")]
    Timeout { timeout_secs: u64 },
    #[error("Configuration error: {0}")]
    ConfigError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SandboxError>;
