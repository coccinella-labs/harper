// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! I/O traits for abstracting user interaction

use crate::core::agents::ResolvedAgents;
use crate::core::error::HarperResult;
use crate::core::plan::PlanState;
use async_trait::async_trait;

/// Trait for obtaining user approval for sensitive operations
#[async_trait]
pub trait UserApproval: Send + Sync {
    /// Request approval from the user
    ///
    /// # Arguments
    /// * `prompt` - The message to show the user
    /// * `command` - The command or operation being approved
    ///
    /// # Returns
    /// `true` if approved, `false` otherwise
    async fn approve(&self, prompt: &str, command: &str) -> HarperResult<bool>;
}

#[async_trait]
pub trait RuntimeEventSink: Send + Sync {
    async fn plan_updated(&self, session_id: &str, plan: Option<PlanState>) -> HarperResult<()>;
    async fn agents_updated(
        &self,
        session_id: &str,
        agents: Option<ResolvedAgents>,
    ) -> HarperResult<()>;
    async fn activity_updated(&self, session_id: &str, status: Option<String>) -> HarperResult<()>;
    async fn command_output_updated(
        &self,
        session_id: &str,
        command: String,
        chunk: String,
        is_error: bool,
        done: bool,
    ) -> HarperResult<()>;
}

pub struct NoopRuntimeEventSink;

#[async_trait]
impl RuntimeEventSink for NoopRuntimeEventSink {
    async fn plan_updated(&self, _session_id: &str, _plan: Option<PlanState>) -> HarperResult<()> {
        Ok(())
    }

    async fn agents_updated(
        &self,
        _session_id: &str,
        _agents: Option<ResolvedAgents>,
    ) -> HarperResult<()> {
        Ok(())
    }

    async fn activity_updated(
        &self,
        _session_id: &str,
        _status: Option<String>,
    ) -> HarperResult<()> {
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

/// A default implementation that uses standard I/O (blocking)
pub struct StdinApproval;

#[async_trait]
impl UserApproval for StdinApproval {
    async fn approve(&self, prompt: &str, command: &str) -> HarperResult<bool> {
        use colored::*;
        use std::io::{self, Write};

        let prompt = prompt.to_string();
        let command = command.to_string();

        tokio::task::spawn_blocking(move || {
            println!(
                "{} {} {}",
                prompt.bold().magenta(),
                command.magenta(),
                "(y/n): ".bold().magenta()
            );
            io::stdout()
                .flush()
                .map_err(|e| crate::core::error::HarperError::Io(e.to_string()))?;

            let mut approval = String::new();
            io::stdin()
                .read_line(&mut approval)
                .map_err(|e| crate::core::error::HarperError::Io(e.to_string()))?;
            Ok::<bool, crate::core::error::HarperError>(approval.trim().eq_ignore_ascii_case("y"))
        })
        .await
        .map_err(|e| crate::core::error::HarperError::Command(format!("Task failed: {}", e)))?
    }
}

/// A non-interactive implementation that always denies approval
pub struct DenyApproval;

#[async_trait]
impl UserApproval for DenyApproval {
    async fn approve(&self, _prompt: &str, _command: &str) -> HarperResult<bool> {
        Ok(false)
    }
}

/// Trait for reading input
pub trait Input: Send + Sync {
    /// Read a line of input
    fn read_line(&self) -> HarperResult<String>;
}

/// Trait for writing output
pub trait Output: Send + Sync {
    /// Print text without a newline
    fn print(&self, text: &str) -> HarperResult<()>;
    /// Print text with a newline
    fn println(&self, text: &str) -> HarperResult<()>;
    /// Flush the output buffer
    fn flush(&self) -> HarperResult<()>;
}

/// A default implementation of Input using standard input
pub struct StdInput;
impl Input for StdInput {
    fn read_line(&self) -> HarperResult<String> {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        Ok(input)
    }
}

/// A default implementation of Output using standard output
pub struct StdOutput;
impl Output for StdOutput {
    fn print(&self, text: &str) -> HarperResult<()> {
        print!("{}", text);
        Ok(())
    }
    fn println(&self, text: &str) -> HarperResult<()> {
        println!("{}", text);
        Ok(())
    }
    fn flush(&self) -> HarperResult<()> {
        use std::io::Write;
        // Correct flush implementation
        let mut stdout = std::io::stdout();
        stdout.flush()?;
        Ok(())
    }
}
