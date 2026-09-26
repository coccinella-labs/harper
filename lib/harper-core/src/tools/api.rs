// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! API testing tool
//!
//! This module provides functionality for testing APIs with HTTP requests.

use crate::core::error::HarperError;
use crate::tools::parsing;
use colored::*;
use reqwest::Client;

/// Test an API endpoint
pub async fn test_api(response: &str) -> crate::core::error::HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[API_TEST", 4)?;
    let method = &args[0];
    let url = &args[1];
    let headers = &args[2]; // JSON string like {"Content-Type": "application/json"}
    let body = &args[3]; // Optional body

    println!(
        "{} Test API {} {} with headers '{}' and body '{}' ? (y/n): ",
        "System:".bold().magenta(),
        method.magenta(),
        url.magenta(),
        headers.magenta(),
        body.magenta()
    );
    let mut approval = String::new();
    std::io::stdin().read_line(&mut approval)?;
    if !approval.trim().eq_ignore_ascii_case("y") {
        return Ok("API test cancelled by user".to_string());
    }

    println!(
        "{} Testing API: {} {}",
        "System:".bold().magenta(),
        method.magenta(),
        url.magenta()
    );

    let client = Client::new();
    let mut request = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        _ => {
            return Err(HarperError::Command(format!(
                "Unsupported method: {}",
                method
            )));
        }
    };

    // Add headers if provided
    if !headers.is_empty() {
        let header_map: std::collections::HashMap<String, String> =
            serde_json::from_str(headers)
                .map_err(|e| HarperError::Command(format!("Invalid headers JSON: {}", e)))?;
        for (key, value) in header_map {
            request = request.header(&key, &value);
        }
    }

    // Add body if provided
    if !body.is_empty() {
        request = request.body(body.to_string());
    }

    let response = request
        .send()
        .await
        .map_err(|e| HarperError::Command(format!("Request failed: {}", e)))?;

    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .text()
        .await
        .map_err(|e| HarperError::Command(format!("Failed to read response: {}", e)))?;

    let result = format!("Status: {}\nHeaders: {:?}\nBody: {}", status, headers, body);

    Ok(result)
}
