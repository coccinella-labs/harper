// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Application constants
//!
//! This module defines named constants to replace magic numbers throughout the codebase.

use std::time::Duration;

/// Application version
pub const VERSION: &str = match option_env!("HARPER_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// Timeout durations
pub mod timeouts {
    use super::Duration;

    /// Default API request timeout (90 seconds)
    #[allow(dead_code)]
    pub const API_REQUEST: Duration = Duration::from_secs(90);

    /// Web search request timeout (15 seconds)
    pub const WEB_SEARCH: Duration = Duration::from_secs(15);

    // MCP service timeout (30 seconds) - temporarily disabled
    // pub const MCP_SERVICE: Duration = Duration::from_secs(30);
}

/// Cache configuration
pub mod cache {
    use super::Duration;

    /// Default API response cache TTL (5 minutes)
    pub const API_RESPONSE_TTL: Duration = Duration::from_secs(300);
}

/// Cryptographic constants
pub mod crypto {
    /// AES-256 key length in bytes (256 bits)
    pub const AES_256_KEY_LEN: usize = 32;

    /// AES-GCM authentication tag length in bytes
    pub const AES_GCM_TAG_LEN: usize = 16;

    /// AES-GCM nonce length in bytes (96 bits)
    pub const AES_GCM_NONCE_LEN: usize = 12;

    /// Minimum encrypted data length (nonce + tag)
    pub const MIN_ENCRYPTED_LEN: usize = AES_GCM_NONCE_LEN + AES_GCM_TAG_LEN;

    /// SHA-256 hash length in bytes
    #[allow(dead_code)]
    pub const SHA256_LEN: usize = 32;
}

/// Test data sizes
pub mod test_data {
    /// Large test message size (1MB)
    #[allow(dead_code)]
    pub const LARGE_MESSAGE_SIZE: usize = 1024 * 1024;

    /// Invalid key length for testing (128 bits = 16 bytes)
    #[allow(dead_code)]
    pub const INVALID_KEY_LEN: usize = 16;

    /// Short data length for testing (less than minimum encrypted length)
    #[allow(dead_code)]
    pub const SHORT_DATA_LEN: usize = 20;
}

/// Tool commands
pub mod tools {
    /// Command execution prefix
    #[allow(dead_code)]
    pub const RUN_COMMAND: &str = "[RUN_COMMAND";

    /// Web search prefix
    #[allow(dead_code)]
    pub const SEARCH: &str = "[SEARCH:";

    /// Read file prefix
    #[allow(dead_code)]
    pub const READ_FILE: &str = "[READ_FILE";

    /// Write file prefix
    #[allow(dead_code)]
    pub const WRITE_FILE: &str = "[WRITE_FILE";

    /// Search and replace prefix
    #[allow(dead_code)]
    pub const SEARCH_REPLACE: &str = "[SEARCH_REPLACE";

    /// Todo management prefix
    #[allow(dead_code)]
    pub const TODO: &str = "[TODO";

    /// GitHub issue creation prefix
    #[allow(dead_code)]
    pub const GITHUB_ISSUE: &str = "[GITHUB_ISSUE";

    /// GitHub PR creation prefix
    #[allow(dead_code)]
    pub const GITHUB_PR: &str = "[GITHUB_PR";

    /// API testing prefix
    #[allow(dead_code)]
    pub const API_TEST: &str = "[API_TEST";

    /// Code analysis prefix
    #[allow(dead_code)]
    pub const CODE_ANALYZE: &str = "[CODE_ANALYZE";

    /// Codebase investigation prefix
    #[allow(dead_code)]
    pub const CODEBASE_INVESTIGATE: &str = "[CODEBASE_INVESTIGATE";

    /// Database query prefix
    #[allow(dead_code)]
    pub const DB_QUERY: &str = "[DB_QUERY";

    /// Azure Data Explorer query prefix
    #[allow(dead_code)]
    pub const ADX_QUERY: &str = "[ADX_QUERY";

    /// Image info prefix
    #[allow(dead_code)]
    pub const IMAGE_INFO: &str = "[IMAGE_INFO";

    /// Image resize prefix
    #[allow(dead_code)]
    pub const IMAGE_RESIZE: &str = "[IMAGE_RESIZE";

    /// Screenpipe search prefix
    #[allow(dead_code)]
    pub const SCREENPIPE: &str = "[SCREENPIPE";

    /// Firmware operations prefix
    #[allow(dead_code)]
    pub const FIRMWARE: &str = "[FIRMWARE";

    /// Command suffix
    #[allow(dead_code)]
    pub const COMMAND_SUFFIX: &str = "]";
}
