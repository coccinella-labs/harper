// Copyright 2026 coccinella-labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Common parsing utilities for tool arguments
//!
//! This module provides functions for parsing quoted arguments and tool commands.

use crate::core::error::HarperError;

/// Parse space-separated arguments with quote support
pub fn parse_quoted_args(input: &str) -> Result<Vec<String>, HarperError> {
    let mut args = Vec::new();
    let mut current_arg = String::new();
    let mut in_quotes = false;

    for ch in input.chars() {
        match ch {
            '"' => {
                if in_quotes {
                    // End of quoted string
                    in_quotes = false;
                    if current_arg.is_empty() {
                        args.push("".to_string());
                    }
                } else {
                    // Start of quoted string
                    in_quotes = true;
                }
            }
            ' ' => {
                if in_quotes {
                    // Space inside quotes is part of the argument
                    current_arg.push(ch);
                } else if !current_arg.is_empty() {
                    // End of unquoted argument
                    args.push(current_arg);
                    current_arg = String::new();
                }
                // Skip multiple spaces
            }
            _ => {
                current_arg.push(ch);
            }
        }
    }

    // Add the last argument if any
    if !current_arg.is_empty() {
        args.push(current_arg);
    }

    // Check for unclosed quotes
    if in_quotes {
        return Err(HarperError::Command(
            "Unclosed quote in arguments".to_string(),
        ));
    }

    Ok(args)
}

/// Extract multiple arguments from tool command with proper quote handling
pub fn extract_tool_args(
    response: &str,
    prefix: &str,
    num_args: usize,
) -> Result<Vec<String>, HarperError> {
    let arg_str = extract_tool_arg(response, prefix)?;
    let args = parse_quoted_args(&arg_str)?;

    if args.len() != num_args {
        return Err(HarperError::Command(format!(
            "Expected {} arguments, got {}",
            num_args,
            args.len()
        )));
    }

    Ok(args)
}

/// Extract a single tool argument from response
pub fn extract_tool_arg(response: &str, prefix: &str) -> Result<String, HarperError> {
    let arg_str = response
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| HarperError::Command(format!("Invalid {} format", prefix)))?
        .trim();

    if arg_str.is_empty() {
        return Err(HarperError::Command("No argument provided".to_string()));
    }

    Ok(arg_str.to_string())
}
