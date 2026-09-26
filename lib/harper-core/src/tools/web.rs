// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Web search tool
//!
//! This module provides functionality for performing web searches.

use crate::runtime::utils::web_search;
use colored::*;

/// Perform web search
pub async fn perform_web_search(response: &str) -> crate::core::error::HarperResult<String> {
    let query_part = response
        .split_once(':')
        .map(|x| x.1)
        .unwrap_or("")
        .trim_end_matches(']')
        .trim();

    println!(
        "{} Searching the web for: {}",
        "System:".bold().magenta(),
        query_part.magenta()
    );

    web_search(query_part).await
}
