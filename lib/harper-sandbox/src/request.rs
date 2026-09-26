// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::backend::SandboxBackend;
use crate::errors::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequest {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub env: Vec<(String, String)>,
    pub declared_read_paths: Vec<PathBuf>,
    pub declared_write_paths: Vec<PathBuf>,
    pub requires_network: bool,
}

impl SandboxRequest {
    pub fn new(command: impl Into<String>, args: &[&str]) -> Result<Self> {
        Ok(Self {
            command: command.into(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            working_dir: std::env::current_dir()?,
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: false,
        })
    }
}

#[derive(Debug)]
pub struct SandboxExecutionResult {
    pub request: SandboxRequest,
    pub backend: SandboxBackend,
    pub output: std::process::Output,
}
