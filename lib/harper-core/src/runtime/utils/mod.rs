// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Utility functions for web search and cryptography
//!
//! This module provides helper functions for web searching and cryptographic operations.

use crate::core::constants::timeouts;
use crate::core::error::HarperResult;

pub mod crypto;

/// Perform a web search using DuckDuckGo API
///
/// Searches the web for the given query and returns the results.
/// This is used by the AI assistant to gather information when needed.
///
/// # Arguments
/// * `query` - The search query string
///
/// # Returns
/// Search results as a string, or an error message if the search fails
///
/// # Errors
/// Returns `HarperError::WebSearch` if the API request fails
pub async fn web_search(query: &str) -> HarperResult<String> {
    if let Ok(mock_response) = std::env::var("HARPER_WEB_SEARCH_MOCK_RESPONSE") {
        return Ok(mock_response);
    }

    let client = reqwest::Client::builder()
        .timeout(timeouts::WEB_SEARCH)
        .build()?;
    let search_url = std::env::var("HARPER_WEB_SEARCH_URL")
        .unwrap_or_else(|_| "https://api.duckduckgo.com/".to_string());
    let response = client
        .get(search_url)
        .query(&[("q", query), ("format", "json")])
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = format!(
            "Search API returned a non-success status: {}. Body: {}",
            response.status(),
            response
                .text()
                .await
                .unwrap_or_else(|_| "Could not read body".to_string())
        );
        return Ok(error_text);
    }

    Ok(response.text().await?)
}
