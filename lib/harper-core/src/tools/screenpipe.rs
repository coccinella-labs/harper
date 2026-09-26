// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Screenpipe tool
//!
//! This module provides functionality for searching screenpipe's screen/audio history.

use crate::core::error::HarperError;
use crate::tools::parsing;
use colored::*;
use reqwest::Client;
use serde::Deserialize;

const DEFAULT_SCREENPIPE_URL: &str = "http://localhost:3030";
const DEFAULT_LIMIT: usize = 10;

#[derive(Debug, Deserialize)]
struct ScreenpipeSearchResponse {
    data: Vec<ScreenpipeItem>,
    #[serde(default)]
    pagination: Option<ScreenpipePagination>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ScreenpipeItem {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    window_title: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ScreenpipePagination {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    total: Option<usize>,
}

pub async fn search_screenpipe(response: &str) -> crate::core::error::HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[SCREENPIPE", 3)?;

    let query = &args[0];
    let content_type = if args.len() > 1 && !args[1].is_empty() {
        &args[1]
    } else {
        "ocr"
    };
    let limit = if args.len() > 2 && !args[2].is_empty() {
        args[2].parse().unwrap_or(DEFAULT_LIMIT)
    } else {
        DEFAULT_LIMIT
    };

    let screenpipe_url =
        std::env::var("SCREENPIPE_URL").unwrap_or_else(|_| DEFAULT_SCREENPIPE_URL.to_string());

    let url = format!(
        "{}/search?q={}&content_type={}&limit={}",
        screenpipe_url,
        urlencoding::encode(query),
        content_type,
        limit
    );

    println!(
        "{} Searching screenpipe: query='{}', type='{}', limit={}",
        "System:".bold().magenta(),
        query.magenta(),
        content_type.magenta(),
        limit
    );

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| HarperError::Command(format!("Failed to create HTTP client: {}", e)))?;

    let response = client.get(&url).send().await.map_err(|e| {
        if e.is_connect() {
            HarperError::Command(format!(
                "Could not connect to screenpipe at {}. Is screenpipe running?",
                screenpipe_url
            ))
        } else {
            HarperError::Command(format!("Screenpipe request failed: {}", e))
        }
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(HarperError::Command(format!(
            "Screenpipe API error ({}): {}",
            status, body
        )));
    }

    let search_result: ScreenpipeSearchResponse = response
        .json()
        .await
        .map_err(|e| HarperError::Command(format!("Failed to parse screenpipe response: {}", e)))?;

    if search_result.data.is_empty() {
        return Ok(format!("No results found for query: '{}'", query));
    }

    let mut output = format!(
        "Found {} result(s) for '{}':\n\n",
        search_result.data.len(),
        query
    );

    for (i, item) in search_result.data.iter().enumerate() {
        output.push_str(&format!("--- Result {} ---\n", i + 1));

        if let Some(ref timestamp) = item.timestamp {
            output.push_str(&format!("Time: {}\n", timestamp));
        }
        if let Some(ref app) = item.app_name {
            output.push_str(&format!("App: {}\n", app));
        }
        if let Some(ref window) = item.window_title {
            output.push_str(&format!("Window: {}\n", window));
        }

        let empty_string = String::new();
        let text = item
            .text
            .as_ref()
            .or(item.content.as_ref())
            .unwrap_or(&empty_string);
        if !text.is_empty() {
            let truncated = if text.len() > 500 {
                format!("{}...", &text[..500])
            } else {
                text.clone()
            };
            output.push_str(&format!("Content: {}\n", truncated));
        }

        output.push('\n');
    }

    if let Some(ref pagination) = search_result.pagination {
        if let Some(total) = pagination.total {
            output.push_str(&format!("Total matches: {}\n", total));
        }
    }

    Ok(output)
}
