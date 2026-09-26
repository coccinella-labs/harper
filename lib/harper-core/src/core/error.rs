// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use colored::Colorize;

/// Custom error type for Harper application
#[derive(Debug, thiserror::Error)]
pub enum HarperError {
    /// Validation related errors
    #[error("Validation error: {0}")]
    Validation(String),
    /// Configuration related errors
    #[error("Configuration error: {0}")]
    Config(String),
    /// Database related errors
    #[error("Database error: {0}")]
    Database(String),
    /// API related errors
    #[error("API error: {0}")]
    Api(String),
    /// MCP related errors
    #[allow(dead_code)]
    #[error("MCP error: {0}")]
    Mcp(String),
    /// Cryptography related errors
    #[error("Cryptography error: {0}")]
    Crypto(String),
    /// I/O related errors
    #[error("I/O error: {0}")]
    Io(String),
    /// File operation errors
    #[error("File operation error: {0}")]
    File(String),
    /// Command execution errors
    #[error("Command execution error: {0}")]
    Command(String),
    /// Web search errors
    #[allow(dead_code)]
    #[error("Web search error: {0}")]
    WebSearch(String),
    /// Firmware errors
    #[error("Firmware error: {0}")]
    Firmware(#[from] harper_firmware::FirmwareError),
    /// Sandbox errors
    #[error("Sandbox error: {0}")]
    Sandbox(#[from] harper_sandbox::SandboxError),
}

impl HarperError {
    /// Get colored error message for CLI
    pub fn cli_message(&self) -> colored::ColoredString {
        self.to_string().red()
    }
}

impl From<rusqlite::Error> for HarperError {
    fn from(err: rusqlite::Error) -> Self {
        HarperError::Database(err.to_string())
    }
}

impl From<reqwest::Error> for HarperError {
    fn from(err: reqwest::Error) -> Self {
        HarperError::Api(err.to_string())
    }
}

impl From<config::ConfigError> for HarperError {
    fn from(err: config::ConfigError) -> Self {
        HarperError::Config(err.to_string())
    }
}

impl From<std::io::Error> for HarperError {
    fn from(err: std::io::Error) -> Self {
        HarperError::Io(err.to_string())
    }
}

impl From<uuid::Error> for HarperError {
    fn from(err: uuid::Error) -> Self {
        HarperError::Io(format!("UUID generation error: {}", err))
    }
}

impl From<serde_json::Error> for HarperError {
    fn from(err: serde_json::Error) -> Self {
        HarperError::Api(format!("JSON serialization error: {}", err))
    }
}

impl From<rustyline::error::ReadlineError> for HarperError {
    fn from(err: rustyline::error::ReadlineError) -> Self {
        HarperError::Io(err.to_string())
    }
}

/// Result type alias for Harper operations
pub type HarperResult<T> = Result<T, HarperError>;
