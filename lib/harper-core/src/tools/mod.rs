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

//! Tool execution module
//!
//! This module provides a unified interface to various tools
//! for file operations, shell commands, web search, and todos.

pub mod adx;
pub mod api;
pub mod code_analysis;
pub mod codebase_investigator;
pub mod db;
pub mod filesystem;
pub mod firmware;
pub mod git;
pub mod github;
pub mod image;
pub mod plan;
pub mod screenpipe;
pub mod shell;
pub mod todo;
pub mod web;

/// Common parsing utilities for tool arguments
pub mod parsing;

use crate::core::constants::tools;
use crate::core::error::{HarperError, HarperResult};
use crate::core::tool_call::{ToolCall, ToolCallSource};
use crate::core::{ApiConfig, Message};
use crate::runtime::config::ExecPolicyConfig;
use crate::tools::shell::CommandAuditContext;
use reqwest::Client;
use rusqlite::Connection;
use std::path::PathBuf;
use turul_mcp_client::{ContentBlock, McpClient};

// Git command constants
mod git_tools {
    pub const GIT_STATUS: &str = "[GIT_STATUS]";
    pub const GIT_DIFF: &str = "[GIT_DIFF]";
    pub const GIT_COMMIT: &str = "[GIT_COMMIT";
    pub const GIT_ADD: &str = "[GIT_ADD";
}

use crate::core::io_traits::{RuntimeEventSink, UserApproval};
use std::sync::Arc;

/// Tool execution service
pub struct ToolService<'a> {
    conn: &'a Connection,
    config: &'a ApiConfig,
    exec_policy: &'a ExecPolicyConfig,
    mcp_client: Option<&'a turul_mcp_client::McpClient>,
    session_id: Option<&'a str>,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<Arc<dyn RuntimeEventSink>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PlanSyncOutcome {
    completed_step: Option<String>,
    next_step: Option<String>,
}

impl<'a> ToolService<'a> {
    fn parse_run_command_sandbox_intent(args: &serde_json::Value) -> shell::CommandSandboxIntent {
        shell::CommandSandboxIntent {
            declared_read_paths: args
                .get("declared_read_paths")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .collect(),
            declared_write_paths: args
                .get("declared_write_paths")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .collect(),
            requires_network: args
                .get("requires_network")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            retry_policy: args
                .get("retry_policy")
                .and_then(|v| v.as_str())
                .and_then(|value| {
                    serde_json::from_value::<shell::CommandRetryPolicy>(serde_json::Value::String(
                        value.to_string(),
                    ))
                    .ok()
                }),
        }
    }

    /// Create a new tool service
    pub fn new(
        conn: &'a Connection,
        config: &'a ApiConfig,
        exec_policy: &'a ExecPolicyConfig,
        mcp_client: Option<&'a McpClient>,
        session_id: Option<&'a str>,
    ) -> Self {
        Self {
            conn,
            config,
            exec_policy,
            mcp_client,
            session_id,
            approver: None,
            runtime_events: None,
        }
    }

    /// Set a custom user approval provider
    pub fn with_approver(mut self, approver: Arc<dyn UserApproval>) -> Self {
        self.approver = Some(approver);
        self
    }

    pub fn with_runtime_events(mut self, runtime_events: Arc<dyn RuntimeEventSink>) -> Self {
        self.runtime_events = Some(runtime_events);
        self
    }

    fn emit_activity_update(&self, status: Option<String>) {
        let (Some(runtime_events), Some(session_id)) = (&self.runtime_events, self.session_id)
        else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let runtime_events = runtime_events.clone();
            let session_id = session_id.to_string();
            handle.spawn(async move {
                let _ = runtime_events.activity_updated(&session_id, status).await;
            });
        }
    }

    /// Handle tool usage (commands, web search, file operations)
    pub async fn handle_tool_use(
        &mut self,
        client: &Client,
        history: &[Message],
        tool_call: &ToolCall,
        web_search_enabled: bool,
    ) -> Result<Option<(String, String)>, HarperError> {
        let raw_response = tool_call.to_raw_string();

        // MCP tool call
        if tool_call.name.starts_with("mcp__") {
            return self
                .handle_mcp_tool_call(
                    client,
                    history,
                    &tool_call.name,
                    &tool_call.arguments,
                    &raw_response,
                )
                .await;
        }

        // Legacy bracket format preserves the raw bracket string downstream
        if matches!(tool_call.source, ToolCallSource::BracketLegacy) {
            return self
                .handle_bracket_tool_use(client, history, &raw_response, web_search_enabled)
                .await;
        }

        // Structured JSON tool call: dispatch directly on name and args
        self.handle_regular_json_tool(
            client,
            history,
            &tool_call.name,
            &tool_call.arguments,
            &raw_response,
            web_search_enabled,
        )
        .await
    }

    /// Handle legacy bracket-format tool calls.
    async fn handle_bracket_tool_use(
        &mut self,
        client: &Client,
        history: &[Message],
        response: &str,
        web_search_enabled: bool,
    ) -> Result<Option<(String, String)>, HarperError> {
        // Fallback to old bracket format
        if response.to_uppercase().starts_with(tools::RUN_COMMAND) {
            self.sync_plan_before_tool("run_command")?;
            let audit_ctx = CommandAuditContext {
                conn: self.conn,
                session_id: self.session_id,
                source: "tool_run_command",
            };
            let command_result = shell::execute_command(
                response,
                self.config,
                self.exec_policy,
                None,
                Some(&audit_ctx),
                self.approver.clone(),
                self.runtime_events.clone(),
            )
            .await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &command_result)
                .await?;
            Ok(Some((final_response, command_result)))
        } else if response.to_uppercase().starts_with(tools::READ_FILE) {
            self.sync_plan_before_tool("read_file")?;
            let tool_result = filesystem::read_file(response, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &tool_result)
                .await?;
            Ok(Some((final_response, tool_result)))
        } else if response.to_uppercase().starts_with(tools::WRITE_FILE) {
            self.sync_plan_before_tool("write_file")?;
            let tool_result = filesystem::write_file(response, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &tool_result)
                .await?;
            Ok(Some((final_response, tool_result)))
        } else if response.to_uppercase().starts_with(tools::SEARCH_REPLACE) {
            self.sync_plan_before_tool("search_replace")?;
            let tool_result = filesystem::search_replace(response, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &tool_result)
                .await?;
            Ok(Some((final_response, tool_result)))
        } else if response.to_uppercase().starts_with(tools::TODO) {
            self.sync_plan_before_tool("todo")?;
            self.execute_sync_tool(client, history, response, |conn, response| {
                let conn = conn.ok_or_else(|| {
                    HarperError::Database("Connection required for todo management".to_string())
                })?;
                todo::manage_todo(conn, response)
            })
            .await
        } else if web_search_enabled && response.to_uppercase().starts_with(tools::SEARCH) {
            self.sync_plan_before_tool("search")?;
            let search_result = web::perform_web_search(response).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &search_result)
                .await?;
            Ok(Some((final_response, search_result)))
        } else if response.to_uppercase().starts_with(git_tools::GIT_STATUS) {
            self.sync_plan_before_tool("git_status")?;
            self.execute_sync_tool(client, history, response, |_, _| git::git_status())
                .await
        } else if response.to_uppercase().starts_with(git_tools::GIT_DIFF) {
            self.sync_plan_before_tool("git_diff")?;
            self.execute_sync_tool(client, history, response, |_, _| git::git_diff())
                .await
        } else if response.to_uppercase().starts_with(git_tools::GIT_COMMIT) {
            self.sync_plan_before_tool("git_commit")?;
            let commit_result = git::git_commit(response, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &commit_result)
                .await?;
            Ok(Some((final_response, commit_result)))
        } else if response.to_uppercase().starts_with(git_tools::GIT_ADD) {
            self.sync_plan_before_tool("git_add")?;
            let add_result = git::git_add(response, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &add_result)
                .await?;
            Ok(Some((final_response, add_result)))
        } else if response.to_uppercase().starts_with(tools::GITHUB_ISSUE) {
            self.sync_plan_before_tool("github_issue")?;
            self.execute_sync_tool(client, history, response, |_, response| {
                github::create_issue(response)
            })
            .await
        } else if response.to_uppercase().starts_with(tools::GITHUB_PR) {
            self.sync_plan_before_tool("github_pr")?;
            self.execute_sync_tool(client, history, response, |_, response| {
                github::create_pr(response)
            })
            .await
        } else if response.to_uppercase().starts_with(tools::API_TEST) {
            self.sync_plan_before_tool("api_test")?;
            let api_result = api::test_api(response).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &api_result)
                .await?;
            Ok(Some((final_response, api_result)))
        } else if response.to_uppercase().starts_with(tools::CODE_ANALYZE) {
            self.sync_plan_before_tool("code_analyze")?;
            self.execute_sync_tool(client, history, response, |_, response| {
                code_analysis::analyze_code(response)
            })
            .await
        } else if response
            .to_uppercase()
            .starts_with(tools::CODEBASE_INVESTIGATE)
        {
            self.sync_plan_before_tool("codebase_investigator")?;
            let tool_result =
                codebase_investigator::investigate_codebase(response, self.approver.clone())
                    .await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &tool_result)
                .await?;
            Ok(Some((final_response, tool_result)))
        } else if response.to_uppercase().starts_with(tools::DB_QUERY) {
            self.sync_plan_before_tool("db_query")?;
            let tool_result = db::run_query(response, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &tool_result)
                .await?;
            Ok(Some((final_response, tool_result)))
        } else if response.to_uppercase().starts_with(tools::ADX_QUERY) {
            self.sync_plan_before_tool("adx_query")?;
            let args = adx::args_from_bracket_call(response)?;
            let tool_result = adx::query_from_json(client, &args, self.approver.clone()).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &tool_result)
                .await?;
            Ok(Some((final_response, tool_result)))
        } else if response.to_uppercase().starts_with(tools::IMAGE_INFO) {
            self.sync_plan_before_tool("image_info")?;
            self.execute_sync_tool(client, history, response, |_, response| {
                image::get_image_info(response)
            })
            .await
        } else if response.to_uppercase().starts_with(tools::IMAGE_RESIZE) {
            self.sync_plan_before_tool("image_resize")?;
            self.execute_sync_tool(client, history, response, |_, response| {
                image::resize_image(response)
            })
            .await
        } else if response.to_uppercase().starts_with(tools::SCREENPIPE) {
            self.sync_plan_before_tool("screenpipe")?;
            let search_result = screenpipe::search_screenpipe(response).await?;
            let final_response = self
                .call_llm_after_tool(client, history, response, &search_result)
                .await?;
            Ok(Some((final_response, search_result)))
        } else if response.to_uppercase().starts_with(tools::FIRMWARE) {
            self.sync_plan_before_tool("firmware")?;
            self.execute_sync_tool(client, history, response, |_, response| {
                firmware::handle_firmware_command(response)
            })
            .await
        } else {
            Ok(None)
        }
    }

    #[allow(dead_code)]
    async fn handle_regular_json_tool(
        &mut self,
        client: &Client,
        history: &[Message],
        tool_name: &str,
        args: &serde_json::Value,
        raw_response: &str,
        web_search_enabled: bool,
    ) -> Result<Option<(String, String)>, HarperError> {
        self.sync_plan_before_tool(tool_name)?;
        match tool_name {
            "run_command" => {
                if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
                    let bracket_command = format!("[RUN_COMMAND {}]", command);
                    let sandbox_intent = Self::parse_run_command_sandbox_intent(args);
                    let audit_ctx = CommandAuditContext {
                        conn: self.conn,
                        session_id: self.session_id,
                        source: "tool_run_command",
                    };
                    let command_result = shell::execute_command(
                        &bracket_command,
                        self.config,
                        self.exec_policy,
                        Some(&sandbox_intent),
                        Some(&audit_ctx),
                        self.approver.clone(),
                        self.runtime_events.clone(),
                    )
                    .await?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &command_result)
                        .await?;
                    Ok(Some((final_response, command_result)))
                } else {
                    Ok(None)
                }
            }

            "search" => {
                if !web_search_enabled {
                    return Ok(Some((
                        "Web search is off. Enable web mode and try again.".to_string(),
                        "Web search is disabled for this session.".to_string(),
                    )));
                }
                if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
                    let bracket_command = format!("[SEARCH: {}]", query);
                    let search_result = web::perform_web_search(&bracket_command).await?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &search_result)
                        .await?;
                    Ok(Some((final_response, search_result)))
                } else {
                    Ok(None)
                }
            }

            "read_file" => {
                if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                    let bracket_command = format!("[READ_FILE {}]", path);
                    let read_result =
                        filesystem::read_file(&bracket_command, self.approver.clone()).await?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &read_result)
                        .await?;
                    Ok(Some((final_response, read_result)))
                } else {
                    Ok(None)
                }
            }

            "write_file" => {
                let path = args.get("path").and_then(|v| v.as_str());
                let content = args.get("content").and_then(|v| v.as_str());
                if let (Some(path), Some(content)) = (path, content) {
                    let write_result =
                        filesystem::write_file_direct(path, content, self.approver.clone()).await?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &write_result)
                        .await?;
                    Ok(Some((final_response, write_result)))
                } else {
                    Ok(None)
                }
            }
            "search_replace" => {
                let path = args.get("path").and_then(|v| v.as_str());
                let old_string = args.get("old_string").and_then(|v| v.as_str());
                let new_string = args.get("new_string").and_then(|v| v.as_str());
                if let (Some(path), Some(old_string), Some(new_string)) =
                    (path, old_string, new_string)
                {
                    let bracket_command =
                        format!("[SEARCH_REPLACE {} {} {}]", path, old_string, new_string);
                    let replace_result =
                        filesystem::search_replace(&bracket_command, self.approver.clone()).await?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &replace_result)
                        .await?;
                    Ok(Some((final_response, replace_result)))
                } else {
                    Ok(None)
                }
            }
            "todo" => {
                if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
                    let bracket_command = match action {
                        "add" => {
                            if let Some(description) =
                                args.get("description").and_then(|v| v.as_str())
                            {
                                format!("[TODO add {}]", description)
                            } else {
                                "".to_string()
                            }
                        }
                        "list" => "[TODO list]".to_string(),
                        "remove" => {
                            if let Some(index) = args.get("index").and_then(|v| v.as_i64()) {
                                format!("[TODO remove {}]", index)
                            } else {
                                "".to_string()
                            }
                        }
                        "clear" => "[TODO clear]".to_string(),
                        _ => "".to_string(),
                    };
                    if bracket_command.is_empty() {
                        return Ok(None);
                    }
                    let todo_result = todo::manage_todo(self.conn, &bracket_command)?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &todo_result)
                        .await?;
                    Ok(Some((final_response, todo_result)))
                } else {
                    Ok(None)
                }
            }
            "adx_query" | "azure_data_explorer" => {
                let query_result =
                    adx::query_from_json(client, args, self.approver.clone()).await?;
                Ok(Some((query_result.clone(), query_result)))
            }
            "update_plan" => {
                let Some(session_id) = self.session_id else {
                    return Err(HarperError::Validation(
                        "update_plan requires an active session".to_string(),
                    ));
                };
                let plan_result = plan::update_plan(self.conn, session_id, args)?;
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &plan_result)
                    .await?;
                Ok(Some((final_response, plan_result)))
            }
            "codebase_investigator" => {
                let action = args.get("action").and_then(|v| v.as_str());
                if let Some(act) = action {
                    let mut bracket_command = format!("[CODEBASE_INVESTIGATE {}", act);
                    match act {
                        "find_calls" => {
                            if let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) {
                                bracket_command.push_str(&format!(" {}]", symbol));
                            }
                        }
                        "trace_relationship" => {
                            let x = args.get("x").and_then(|v| v.as_str());
                            let y = args.get("y").and_then(|v| v.as_str());
                            if let (Some(x), Some(y)) = (x, y) {
                                bracket_command.push_str(&format!(" {} {}]", x, y));
                            }
                        }
                        "clone_context" => {
                            if let Some(repo_url) = args.get("repo_url").and_then(|v| v.as_str()) {
                                bracket_command.push_str(&format!(" {}]", repo_url));
                            }
                        }
                        _ => {}
                    }
                    if bracket_command.ends_with(']') {
                        let tool_result = codebase_investigator::investigate_codebase(
                            &bracket_command,
                            self.approver.clone(),
                        )
                        .await?;
                        let final_response = self
                            .call_llm_after_tool(client, history, raw_response, &tool_result)
                            .await?;
                        return Ok(Some((final_response, tool_result)));
                    }
                }
                Ok(None)
            }
            "git_status" => {
                let status_result = git::git_status()?;
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &status_result)
                    .await?;
                Ok(Some((final_response, status_result)))
            }
            "git_diff" => {
                let diff_result = git::git_diff()?;
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &diff_result)
                    .await?;
                Ok(Some((final_response, diff_result)))
            }
            "git_add" => {
                let files = args.get("files").and_then(|v| v.as_str());
                let bracket_command = format!("[GIT_ADD {}]", files.unwrap_or("."));
                let add_result = git::git_add(&bracket_command, self.approver.clone()).await?;
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &add_result)
                    .await?;
                Ok(Some((final_response, add_result)))
            }
            "git_commit" => {
                if let Some(message) = args.get("message").and_then(|v| v.as_str()) {
                    let bracket_command = format!("[GIT_COMMIT {}]", message);
                    let commit_result =
                        git::git_commit(&bracket_command, self.approver.clone()).await?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &commit_result)
                        .await?;
                    Ok(Some((final_response, commit_result)))
                } else {
                    Ok(None)
                }
            }
            "list_changed_files" => {
                let ext = args.get("ext").and_then(|v| v.as_str());
                let tracked_only = args
                    .get("tracked_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let since = args.get("since").and_then(|v| v.as_str());

                let audit_ctx = CommandAuditContext {
                    conn: self.conn,
                    session_id: self.session_id,
                    source: "tool_list_changed_files",
                };
                let files_result = git::list_changed_files_with_policy(
                    self.config,
                    self.exec_policy,
                    Some(&audit_ctx),
                    self.approver.clone(),
                    ext,
                    tracked_only,
                    since,
                )
                .await?;
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &files_result)
                    .await?;
                Ok(Some((final_response, files_result)))
            }
            "firmware_list" => {
                let result = firmware::handle_firmware_command("[FIRMWARE list]")?;
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &result)
                    .await?;
                Ok(Some((final_response, result)))
            }
            "firmware_info" => {
                if let Some(device) = args.get("device").and_then(|v| v.as_str()) {
                    let command = format!("[FIRMWARE info {}]", device);
                    let result = firmware::handle_firmware_command(&command)?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &result)
                        .await?;
                    Ok(Some((final_response, result)))
                } else {
                    Ok(None)
                }
            }
            "firmware_gpio" => {
                if let Some(pin) = args.get("pin").and_then(|v| v.as_i64()) {
                    let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("high");
                    let command = format!("[FIRMWARE gpio {} {}]", pin, state);
                    let result = firmware::handle_firmware_command(&command)?;
                    let final_response = self
                        .call_llm_after_tool(client, history, raw_response, &result)
                        .await?;
                    Ok(Some((final_response, result)))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Handle MCP tool calls
    async fn handle_mcp_tool_call(
        &mut self,
        client: &Client,
        history: &[Message],
        tool_name: &str,
        args: &serde_json::Value,
        raw_response: &str,
    ) -> Result<Option<(String, String)>, HarperError> {
        let Some(mcp_client) = self.mcp_client else {
            let error_msg = "Error: MCP client not configured".to_string();
            let final_response = self
                .call_llm_after_tool(client, history, raw_response, &error_msg)
                .await?;
            return Ok(Some((final_response, error_msg)));
        };

        match mcp_client.call_tool(tool_name, args.clone()).await {
            Ok(result) => {
                // Format the response for the LLM
                let mut result_parts = Vec::new();

                for content in result.content {
                    match content {
                        ContentBlock::Text { text, .. } => {
                            result_parts.push(text.clone());
                        }
                        ContentBlock::Image {
                            data, mime_type, ..
                        } => {
                            result_parts.push(format!(
                                "[Image: {} bytes, type: {}]",
                                data.len(),
                                mime_type
                            ));
                        }
                        ContentBlock::Audio { .. } => {
                            result_parts.push("[Audio content]".to_string());
                        }
                        ContentBlock::ResourceLink { .. } => {
                            result_parts.push("[Resource link]".to_string());
                        }
                        ContentBlock::Resource { .. } => {
                            result_parts.push("[Resource content]".to_string());
                        }
                        ContentBlock::ToolUse { .. } => {
                            result_parts.push("[Tool use]".to_string());
                        }
                        ContentBlock::ToolResult { .. } => {
                            result_parts.push("[Tool result]".to_string());
                        }
                    }
                }

                let tool_result = if result_parts.is_empty() {
                    "Tool executed successfully (no output)".to_string()
                } else {
                    result_parts.join("\n")
                };

                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &tool_result)
                    .await?;
                Ok(Some((final_response, tool_result)))
            }
            Err(e) => {
                let error_msg = format!("MCP tool call failed: {}", e);
                let final_response = self
                    .call_llm_after_tool(client, history, raw_response, &error_msg)
                    .await?;
                Ok::<Option<(String, String)>, HarperError>(Some((final_response, error_msg)))
            }
        }
    }

    /// Execute a synchronous tool
    async fn execute_sync_tool<F>(
        &mut self,
        client: &Client,
        history: &[Message],
        response: &str,
        tool_fn: F,
    ) -> Result<Option<(String, String)>, HarperError>
    where
        F: FnOnce(Option<&Connection>, &str) -> HarperResult<String>,
    {
        let tool_result = tool_fn(Some(self.conn), response)?;
        let final_response = self
            .call_llm_after_tool(client, history, response, &tool_result)
            .await?;
        Ok(Some((final_response, tool_result)))
    }

    /// Call LLM after tool usage
    async fn call_llm_after_tool(
        &self,
        client: &Client,
        history: &[Message],
        tool_call_json: &str,
        tool_output: &str,
    ) -> Result<String, HarperError> {
        self.emit_activity_update(Some("thinking".to_string()));
        let completed_tool_name = Self::tool_name_from_call(tool_call_json);
        let plan_sync_outcome = self.sync_plan_after_tool(tool_call_json)?;
        // Create a new history vector by cloning the existing one
        let mut new_history = history.to_vec();

        // Add the assistant's tool call message
        // This ensures the model knows it just asked for this tool
        new_history.push(Message {
            role: "assistant".to_string(),
            content: tool_call_json.to_string(),
        });

        // Add a new system message containing the tool's output and an instruction
        let mut system_message = format!(
            "Tool execution result:
{}

---
SYSTEM INSTRUCTION: The tool has completed successfully. The output above is the result.
1. DO NOT output the tool call JSON again.
2. DO NOT repeat the raw output.
3. Analyze the result and provide a human-readable answer to the user's request.
4. If the user asked to see a file, summarize it or show a relevant snippet, but do not dump the entire content if it is large.",
            tool_output
        );
        if !Self::is_update_plan_call(tool_call_json) {
            if let Some(plan_instruction) = self.plan_followup_instruction(&plan_sync_outcome)? {
                system_message.push_str("\n5. ");
                system_message.push_str(&plan_instruction);
            }
        }
        new_history.push(Message {
            role: "system".to_string(),
            content: system_message,
        });

        match crate::core::llm_client::call_llm(client, self.config, &new_history).await {
            Ok(response)
                if completed_tool_name.as_deref() == Some("read_file")
                    && Self::response_looks_like_file_tool_call(&response) =>
            {
                new_history.push(Message {
                    role: "system".to_string(),
                    content: "You already have the completed file contents. Do not call read_file, write_file, or search_replace again. Answer the user now in plain language only from the file result you already have.".to_string(),
                });
                match crate::core::llm_client::call_llm(client, self.config, &new_history).await {
                    Ok(retry_response) => Ok(Self::finalize_read_file_followup_response(
                        &retry_response,
                        tool_output,
                    )),
                    Err(_) => Ok(format!("Tool result:\n{}", tool_output)),
                }
            }
            Ok(response) if Self::response_looks_like_tool_call(&response) => {
                new_history.push(Message {
                    role: "system".to_string(),
                    content: "You already have the completed tool result. Do not call any tool again. Respond now in plain language only.".to_string(),
                });
                match crate::core::llm_client::call_llm(client, self.config, &new_history).await {
                    Ok(retry_response) => Ok(Self::finalize_tool_followup_response(
                        completed_tool_name.as_deref(),
                        &retry_response,
                        tool_output,
                    )),
                    Err(_) => Ok(format!("Tool result:\n{}", tool_output)),
                }
            }
            Ok(response) => Ok(Self::finalize_tool_followup_response(
                completed_tool_name.as_deref(),
                &response,
                tool_output,
            )),
            Err(_) => Ok(format!("Tool result:\n{}", tool_output)),
        }
    }

    fn response_looks_like_file_tool_call(response: &str) -> bool {
        matches!(
            Self::tool_name_from_call(response).as_deref(),
            Some("read_file" | "write_file" | "search_replace")
        )
    }

    fn response_looks_like_tool_call(response: &str) -> bool {
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(response.trim()) {
            if let Some(tool_calls) = json_value.as_array() {
                if let Some(first_call) = tool_calls.first() {
                    if first_call.get("function").is_some() {
                        return true;
                    }
                }
            }

            if json_value
                .get("tool")
                .or_else(|| json_value.get("name"))
                .or_else(|| json_value.get("mcp_tool"))
                .is_some()
            {
                return true;
            }
        }

        let upper = response.trim().to_uppercase();
        upper.starts_with(tools::RUN_COMMAND)
            || upper.starts_with(tools::READ_FILE)
            || upper.starts_with(tools::WRITE_FILE)
            || upper.starts_with(tools::SEARCH_REPLACE)
            || upper.starts_with(tools::TODO)
            || upper.starts_with(tools::SEARCH)
            || upper.starts_with(git_tools::GIT_STATUS)
            || upper.starts_with(git_tools::GIT_DIFF)
            || upper.starts_with(git_tools::GIT_COMMIT)
            || upper.starts_with(git_tools::GIT_ADD)
            || upper.starts_with(tools::GITHUB_ISSUE)
            || upper.starts_with(tools::GITHUB_PR)
            || upper.starts_with(tools::API_TEST)
            || upper.starts_with(tools::CODE_ANALYZE)
            || upper.starts_with(tools::CODEBASE_INVESTIGATE)
            || upper.starts_with(tools::DB_QUERY)
            || upper.starts_with(tools::ADX_QUERY)
            || upper.starts_with(tools::IMAGE_INFO)
            || upper.starts_with(tools::IMAGE_RESIZE)
            || upper.starts_with(tools::SCREENPIPE)
            || upper.starts_with(tools::FIRMWARE)
    }

    fn finalize_tool_followup_response(
        tool_name: Option<&str>,
        response: &str,
        tool_output: &str,
    ) -> String {
        if response.trim().is_empty()
            || Self::response_looks_like_tool_call(response)
            || Self::response_is_low_value_tool_echo(response)
        {
            Self::format_tool_followup_fallback(tool_name, tool_output)
        } else {
            response.to_string()
        }
    }

    fn finalize_read_file_followup_response(response: &str, tool_output: &str) -> String {
        if response.trim().is_empty()
            || Self::response_looks_like_file_tool_call(response)
            || Self::response_is_low_value_tool_echo(response)
        {
            Self::format_tool_followup_fallback(Some("read_file"), tool_output)
        } else {
            response.to_string()
        }
    }

    fn response_is_low_value_tool_echo(response: &str) -> bool {
        let trimmed = response.trim();
        trimmed.starts_with("Tool result:")
            || trimmed.starts_with("Tool Result:")
            || trimmed.starts_with("tool result:")
    }

    fn format_tool_followup_fallback(tool_name: Option<&str>, tool_output: &str) -> String {
        match tool_name {
            Some("git_status") => Self::format_git_status(tool_output),
            Some("git_diff") => Self::format_git_diff(tool_output),
            Some("write_file") => Self::format_write_file(tool_output),
            Some("run_command") => Self::format_run_command(tool_output),
            _ => format!("Tool result:\n{}", tool_output),
        }
    }

    fn format_git_status(tool_content: &str) -> String {
        let body = tool_content
            .strip_prefix("Git status:\n")
            .unwrap_or(tool_content)
            .trim();
        if body.is_empty() || body == "clean" {
            return "Git working directory is clean.".to_string();
        }

        let mut modified = 0usize;
        let mut added = 0usize;
        let mut deleted = 0usize;
        let mut renamed = 0usize;
        let mut untracked = 0usize;
        let mut notable = Vec::new();

        for line in body.lines() {
            let line = line.trim_end();
            if line.len() < 3 {
                continue;
            }
            let status = &line[..2];
            let path = line[2..].trim();
            if notable.len() < 6 && !path.is_empty() {
                notable.push(path.to_string());
            }
            match status {
                "??" => untracked += 1,
                s if s.contains('M') => modified += 1,
                s if s.contains('A') => added += 1,
                s if s.contains('D') => deleted += 1,
                s if s.contains('R') => renamed += 1,
                _ => modified += 1,
            }
        }

        let mut parts = Vec::new();
        if modified > 0 {
            parts.push(format!("{} modified", modified));
        }
        if added > 0 {
            parts.push(format!("{} added", added));
        }
        if deleted > 0 {
            parts.push(format!("{} deleted", deleted));
        }
        if renamed > 0 {
            parts.push(format!("{} renamed", renamed));
        }
        if untracked > 0 {
            parts.push(format!("{} untracked", untracked));
        }

        let summary = if parts.is_empty() {
            "Git working directory has changes.".to_string()
        } else {
            format!("Git working directory has {}.", parts.join(", "))
        };

        if notable.is_empty() {
            summary
        } else {
            format!("{} Notable files: {}.", summary, notable.join(", "))
        }
    }

    fn format_git_diff(tool_content: &str) -> String {
        format!("Git diff:\n```diff\n{}\n```", tool_content.trim_end())
    }

    fn format_write_file(tool_content: &str) -> String {
        let mut path = None;
        let mut content = None;
        for line in tool_content.lines() {
            if let Some(value) = line.strip_prefix("Wrote file: ") {
                path = Some(value.trim().to_string());
            }
            if let Some(value) = tool_content.strip_prefix("Wrote file: ") {
                let rest = value.lines().collect::<Vec<_>>();
                if let Some(idx) = rest.iter().position(|line| line.starts_with("CONTENT:")) {
                    let joined = rest[idx..].join("\n");
                    content = joined
                        .strip_prefix("CONTENT:")
                        .map(|v| v.trim_start().to_string());
                }
                break;
            }
        }

        match (path, content) {
            (Some(path), Some(content)) => {
                let ext = std::path::Path::new(&path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("txt");
                format!(
                    "Created `{}`:\n```{}\n{}\n```",
                    path,
                    ext,
                    content.trim_end()
                )
            }
            _ => format!("Tool result:\n{}", tool_content),
        }
    }

    fn format_run_command(tool_content: &str) -> String {
        let command = tool_content
            .lines()
            .find_map(|line| line.strip_prefix("COMMAND: "))
            .unwrap_or("command");
        let output = tool_content
            .split_once("OUTPUT:\n")
            .map(|(_, output)| output.trim_end())
            .unwrap_or("");

        if output.is_empty() {
            format!("Ran `{}` successfully.", command)
        } else {
            format!("Ran `{}` successfully.\n\n{}", command, output)
        }
    }

    fn sync_plan_before_tool(&self, tool_name: &str) -> HarperResult<()> {
        if tool_name == "update_plan" {
            return Ok(());
        }

        self.emit_activity_update(Some(format!("running {}", tool_name)));

        let Some(session_id) = self.session_id else {
            return Ok(());
        };
        let Some(mut plan) = crate::memory::storage::load_plan_state(self.conn, session_id)? else {
            return Ok(());
        };
        if plan.items.is_empty() {
            return Ok(());
        }
        let mut runtime = plan.runtime.unwrap_or_default();

        if plan
            .items
            .iter()
            .any(|item| matches!(item.status, crate::core::plan::PlanStepStatus::InProgress))
        {
            runtime.set_active_tool_state(tool_name.to_string(), None, "running".to_string());
            plan.runtime = Some(runtime);
            crate::memory::storage::save_plan_state(self.conn, session_id, &plan)?;
            return Ok(());
        }

        if let Some(first_pending) = plan
            .items
            .iter_mut()
            .find(|item| matches!(item.status, crate::core::plan::PlanStepStatus::Pending))
        {
            first_pending.status = crate::core::plan::PlanStepStatus::InProgress;
        }
        runtime.set_active_tool_state(tool_name.to_string(), None, "running".to_string());
        plan.runtime = Some(runtime);
        crate::memory::storage::save_plan_state(self.conn, session_id, &plan)?;

        Ok(())
    }

    fn sync_plan_after_tool(&self, tool_call_json: &str) -> HarperResult<PlanSyncOutcome> {
        let Some(session_id) = self.session_id else {
            return Ok(PlanSyncOutcome::default());
        };
        let Some(mut plan) = crate::memory::storage::load_plan_state(self.conn, session_id)? else {
            return Ok(PlanSyncOutcome::default());
        };
        let Some(current_index) = plan
            .items
            .iter()
            .position(|item| matches!(item.status, crate::core::plan::PlanStepStatus::InProgress))
        else {
            return Ok(PlanSyncOutcome::default());
        };

        let tool_name = Self::tool_name_from_call(tool_call_json);
        let Some(tool_name) = tool_name else {
            return Ok(PlanSyncOutcome::default());
        };
        if !Self::is_safe_auto_advance_tool(&tool_name) {
            let mut runtime = plan.runtime.take().unwrap_or_default();
            runtime.clear_active_state();
            let current_step = plan.items[current_index].step.clone();
            runtime.set_checkpoint_followup(current_step, None);
            plan.runtime = (!runtime.is_empty()).then_some(runtime);
            crate::memory::storage::save_plan_state(self.conn, session_id, &plan)?;
            return Ok(PlanSyncOutcome::default());
        }

        let step_text = plan.items[current_index].step.to_ascii_lowercase();
        if !Self::step_matches_safe_tool(&step_text, &tool_name) {
            let mut runtime = plan.runtime.take().unwrap_or_default();
            runtime.clear_active_state();
            let current_step = plan.items[current_index].step.clone();
            runtime.set_checkpoint_followup(current_step, None);
            plan.runtime = (!runtime.is_empty()).then_some(runtime);
            crate::memory::storage::save_plan_state(self.conn, session_id, &plan)?;
            return Ok(PlanSyncOutcome::default());
        }

        let completed_step = plan.items[current_index].step.clone();
        plan.items[current_index].status = crate::core::plan::PlanStepStatus::Completed;
        let next_step = if let Some(next_pending) = plan
            .items
            .iter_mut()
            .find(|item| matches!(item.status, crate::core::plan::PlanStepStatus::Pending))
        {
            let next_step = next_pending.step.clone();
            next_pending.status = crate::core::plan::PlanStepStatus::InProgress;
            Some(next_step)
        } else {
            None
        };
        let mut runtime = plan.runtime.take().unwrap_or_default();
        runtime.clear_active_state();
        runtime.set_checkpoint_followup(completed_step.clone(), next_step.clone());
        plan.runtime = (!runtime.is_empty()).then_some(runtime);
        crate::memory::storage::save_plan_state(self.conn, session_id, &plan)?;
        Ok(PlanSyncOutcome {
            completed_step: Some(completed_step),
            next_step,
        })
    }

    fn plan_followup_instruction(
        &self,
        sync_outcome: &PlanSyncOutcome,
    ) -> HarperResult<Option<String>> {
        let Some(session_id) = self.session_id else {
            return Ok(None);
        };
        let Some(plan) = crate::memory::storage::load_plan_state(self.conn, session_id)? else {
            return Ok(None);
        };
        if plan.items.is_empty() {
            return Ok(None);
        }
        let has_remaining = plan
            .items
            .iter()
            .any(|item| !matches!(item.status, crate::core::plan::PlanStepStatus::Completed));
        if !has_remaining {
            return Ok(None);
        }

        if let Some(followup) = plan
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.followup.as_ref())
        {
            match followup {
                crate::core::plan::PlanFollowup::RetryOrReplan {
                    step,
                    command,
                    retry_count,
                } => {
                    let command = command.as_deref().unwrap_or(step.as_str());
                    if *retry_count <= 1 {
                        return Ok(Some(format!(
                            "The current plan step '{}' just failed while running '{}'. Retry once if the issue looks transient; otherwise call update_plan to revise the remaining work.",
                            step, command
                        )));
                    }
                    return Ok(Some(format!(
                        "The current plan step '{}' has already failed {} times while running '{}'. Do not keep retrying blindly; call update_plan to revise or de-scope the remaining work.",
                        step, retry_count, command
                    )));
                }
                crate::core::plan::PlanFollowup::Checkpoint { step, next_step } => {
                    let next_clause = next_step
                        .as_deref()
                        .map(|next_step| {
                            format!(
                                " Continue with '{}' only after summarizing what changed.",
                                next_step
                            )
                        })
                        .unwrap_or_else(|| {
                            " If that closed the task, confirm completion instead of repeating the plan."
                                .to_string()
                        });
                    return Ok(Some(format!(
                        "Plan checkpoint: '{}' advanced. Briefly summarize what changed before continuing.{}",
                        step, next_clause
                    )));
                }
            }
        }

        if let Some(blocked_step) = plan
            .items
            .iter()
            .find(|item| matches!(item.status, crate::core::plan::PlanStepStatus::Blocked))
        {
            return Ok(Some(format!(
                "A session plan is blocked at step '{}'. Before more tool work, call update_plan to explain the blocker, retry, or revise the remaining steps.",
                blocked_step.step
            )));
        }

        if let Some(completed_step) = sync_outcome.completed_step.as_deref() {
            let next_clause = sync_outcome
                .next_step
                .as_deref()
                .map(|step| format!(" Continue with '{}' once the checkpoint is clear.", step))
                .unwrap_or_else(|| {
                    " If that closed the task, confirm completion instead of repeating the plan."
                        .to_string()
                });
            return Ok(Some(format!(
                "Plan checkpoint: '{}' just completed. Briefly summarize what changed before continuing.{}",
                completed_step, next_clause
            )));
        }

        Ok(Some(
            "A session plan is active. If this tool meaningfully advanced the work, call update_plan to mark the current step completed or move the next step to in_progress.".to_string(),
        ))
    }

    fn is_update_plan_call(tool_call_json: &str) -> bool {
        tool_call_json.contains("\"update_plan\"") || tool_call_json.contains("'update_plan'")
    }

    pub fn target_paths_for_tool_call(tool_call: &str) -> Vec<PathBuf> {
        let trimmed = tool_call.trim();
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let tool_name = json_value
                .get("tool")
                .and_then(|v| v.as_str())
                .or_else(|| json_value.get("name").and_then(|v| v.as_str()));
            let args = json_value
                .get("args")
                .or_else(|| json_value.get("arguments"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            return extract_target_paths_from_json(tool_name, &args);
        }

        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("[READ_FILE") {
            return parsing::extract_tool_arg(trimmed, "[READ_FILE")
                .map(|path| vec![PathBuf::from(path)])
                .unwrap_or_default();
        }
        if upper.starts_with("[WRITE_FILE") {
            return parsing::extract_tool_args(trimmed, "[WRITE_FILE", 2)
                .map(|args| vec![PathBuf::from(args[0].clone())])
                .unwrap_or_default();
        }
        if upper.starts_with("[SEARCH_REPLACE") {
            return parsing::extract_tool_args(trimmed, "[SEARCH_REPLACE", 3)
                .map(|args| vec![PathBuf::from(args[0].clone())])
                .unwrap_or_default();
        }

        Vec::new()
    }

    fn tool_name_from_call(tool_call_json: &str) -> Option<String> {
        let trimmed = tool_call_json.trim();
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(name) = json_value
                .get("tool")
                .and_then(|v| v.as_str())
                .or_else(|| json_value.get("name").and_then(|v| v.as_str()))
            {
                return Some(name.to_string());
            }
            if let Some(first) = json_value.as_array().and_then(|arr| arr.first()) {
                if let Some(name) = first
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                {
                    return Some(name.to_string());
                }
            }
        }

        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("[READ_FILE") {
            Some("read_file".to_string())
        } else if upper.starts_with("[RUN_COMMAND") {
            Some("run_command".to_string())
        } else if upper.starts_with("[GIT_STATUS") {
            Some("git_status".to_string())
        } else if upper.starts_with("[GIT_DIFF") {
            Some("git_diff".to_string())
        } else if upper.starts_with("[SEARCH_REPLACE") {
            Some("search_replace".to_string())
        } else if upper.starts_with("[WRITE_FILE") {
            Some("write_file".to_string())
        } else {
            None
        }
    }

    fn is_safe_auto_advance_tool(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "read_file"
                | "git_status"
                | "git_diff"
                | "list_changed_files"
                | "grep"
                | "codebase_investigator"
        )
    }

    fn step_matches_safe_tool(step_text: &str, tool_name: &str) -> bool {
        let inspect_words = [
            "inspect",
            "check",
            "review",
            "read",
            "look",
            "trace",
            "understand",
            "investigate",
            "explore",
            "find",
            "compare",
            "diff",
        ];
        if !inspect_words.iter().any(|word| step_text.contains(word)) {
            return false;
        }

        match tool_name {
            "read_file" => {
                step_text.contains("file")
                    || step_text.contains("read")
                    || step_text.contains("inspect")
            }
            "git_status" => step_text.contains("status") || step_text.contains("repo"),
            "git_diff" | "list_changed_files" => {
                step_text.contains("diff")
                    || step_text.contains("changed")
                    || step_text.contains("compare")
            }
            "grep" => {
                step_text.contains("find")
                    || step_text.contains("search")
                    || step_text.contains("grep")
            }
            "codebase_investigator" => {
                step_text.contains("trace")
                    || step_text.contains("investigate")
                    || step_text.contains("understand")
                    || step_text.contains("explore")
            }
            _ => false,
        }
    }
}

fn extract_target_paths_from_json(
    tool_name: Option<&str>,
    args: &serde_json::Value,
) -> Vec<PathBuf> {
    match tool_name {
        Some("read_file") | Some("write_file") | Some("search_replace") => args
            .get("path")
            .or_else(|| args.get("filePath"))
            .and_then(|v| v.as_str())
            .map(|path| vec![PathBuf::from(path)])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanSyncOutcome, ToolService};
    use crate::core::plan::{PlanItem, PlanState, PlanStepStatus};
    use crate::core::{ApiConfig, ApiProvider};
    use crate::runtime::config::ExecPolicyConfig;
    use reqwest::Client;
    use rusqlite::Connection;
    use serde_json::json;
    use std::path::PathBuf;

    fn test_config() -> ApiConfig {
        ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        }
    }

    #[test]
    fn finalize_tool_followup_response_falls_back_when_blank() {
        let response = ToolService::finalize_tool_followup_response(None, "   \n", "Plan updated.");
        assert_eq!(response, "Tool result:\nPlan updated.");
    }

    #[test]
    fn finalize_tool_followup_response_keeps_non_empty_response() {
        let response = ToolService::finalize_tool_followup_response(
            None,
            "I updated the plan and will inspect the codebase next.",
            "Plan updated.",
        );
        assert_eq!(
            response,
            "I updated the plan and will inspect the codebase next."
        );
    }

    #[test]
    fn response_looks_like_tool_call_detects_json_tool_calls() {
        assert!(ToolService::response_looks_like_tool_call(
            r#"[{"function":{"name":"read_file","arguments":{"path":"Cargo.toml"}}}]"#
        ));
        assert!(ToolService::response_looks_like_tool_call(
            r#"{"tool":"read_file","args":{"path":"Cargo.toml"}}"#
        ));
    }

    #[tokio::test]
    async fn disabled_web_search_json_tool_returns_user_message() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let client = Client::new();
        let mut service = ToolService::new(&conn, &config, &exec_policy, None, None);
        let result = service
            .handle_regular_json_tool(
                &client,
                &[],
                "search",
                &json!({"query": "current rust version"}),
                r#"{"tool":"search","args":{"query":"current rust version"}}"#,
                false,
            )
            .await
            .expect("disabled search handling")
            .expect("user-facing response");

        assert_eq!(
            result.0,
            "Web search is off. Enable web mode and try again."
        );
        assert_eq!(result.1, "Web search is disabled for this session.");
    }

    #[tokio::test]
    async fn adx_json_tool_missing_query_returns_terminal_guidance() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let client = Client::new();
        let mut service = ToolService::new(&conn, &config, &exec_policy, None, None);
        let result = service
            .handle_regular_json_tool(
                &client,
                &[],
                "adx_query",
                &json!({}),
                r#"{"tool":"adx_query","args":{}}"#,
                false,
            )
            .await
            .expect("adx guidance")
            .expect("terminal response");

        assert_eq!(result.0, result.1);
        assert!(result.0.contains("needs a `query` field"));
    }

    #[test]
    fn finalize_tool_followup_response_falls_back_when_tool_call_repeats() {
        let response = ToolService::finalize_tool_followup_response(
            Some("read_file"),
            r#"{"tool":"read_file","args":{"path":"Cargo.toml"}}"#,
            "package = \"harper-workspace\"",
        );
        assert_eq!(response, "Tool result:\npackage = \"harper-workspace\"");
    }

    #[test]
    fn finalize_tool_followup_response_formats_git_status_fallback() {
        let response = ToolService::finalize_tool_followup_response(
            Some("git_status"),
            "   \n",
            "Git status:\nM src/main.rs\n?? docs/testing/",
        );
        assert_eq!(
            response,
            "Git working directory has 1 modified, 1 untracked. Notable files: src/main.rs, docs/testing/."
        );
    }

    #[test]
    fn finalize_tool_followup_response_formats_run_command_fallback() {
        let response = ToolService::finalize_tool_followup_response(
            Some("run_command"),
            "   \n",
            "COMMAND: cargo fmt --all\nOUTPUT:\n",
        );
        assert_eq!(response, "Ran `cargo fmt --all` successfully.");
    }

    #[test]
    fn response_looks_like_file_tool_call_detects_file_tools() {
        assert!(ToolService::response_looks_like_file_tool_call(
            r#"{"tool":"read_file","args":{"path":"Cargo.toml"}}"#
        ));
        assert!(ToolService::response_looks_like_file_tool_call(
            r#"[SEARCH_REPLACE src/main.rs old new]"#
        ));
        assert!(!ToolService::response_looks_like_file_tool_call(
            r#"{"tool":"run_command","args":{"command":"git status"}}"#
        ));
    }

    #[test]
    fn finalize_read_file_followup_response_falls_back_when_file_tool_repeats() {
        let response = ToolService::finalize_read_file_followup_response(
            r#"{"tool":"read_file","args":{"path":"package.json"}}"#,
            "[package]\nname = \"harper-workspace\"\n",
        );
        assert_eq!(
            response,
            "Tool result:\n[package]\nname = \"harper-workspace\"\n"
        );
    }

    #[test]
    fn sync_plan_before_tool_promotes_first_pending_step() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "sync-plan-session",
            &PlanState {
                explanation: Some("Testing sync".to_string()),
                items: vec![
                    PlanItem {
                        step: "Inspect".to_string(),
                        status: PlanStepStatus::Pending,
                        job_id: None,
                    },
                    PlanItem {
                        step: "Patch".to_string(),
                        status: PlanStepStatus::Pending,
                        job_id: None,
                    },
                ],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("sync-plan-session"),
        );

        service
            .sync_plan_before_tool("read_file")
            .expect("sync plan before tool");

        let plan = crate::memory::storage::load_plan_state(&conn, "sync-plan-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items[0].status, PlanStepStatus::InProgress);
        assert_eq!(plan.items[1].status, PlanStepStatus::Pending);
    }

    #[test]
    fn sync_plan_after_tool_completes_matching_inspection_step() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "after-tool-session",
            &PlanState {
                explanation: Some("Testing auto advance".to_string()),
                items: vec![
                    PlanItem {
                        step: "Inspect server file".to_string(),
                        status: PlanStepStatus::InProgress,
                        job_id: None,
                    },
                    PlanItem {
                        step: "Patch handler".to_string(),
                        status: PlanStepStatus::Pending,
                        job_id: None,
                    },
                ],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("after-tool-session"),
        );

        service
            .sync_plan_after_tool(r#"{"tool":"read_file","args":{"path":"src/server.rs"}}"#)
            .expect("sync plan after tool");

        let plan = crate::memory::storage::load_plan_state(&conn, "after-tool-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items[0].status, PlanStepStatus::Completed);
        assert_eq!(plan.items[1].status, PlanStepStatus::InProgress);
    }

    #[test]
    fn sync_plan_after_tool_returns_checkpoint_summary_state() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "checkpoint-session",
            &PlanState {
                explanation: Some("Testing checkpoint".to_string()),
                items: vec![
                    PlanItem {
                        step: "Inspect server file".to_string(),
                        status: PlanStepStatus::InProgress,
                        job_id: None,
                    },
                    PlanItem {
                        step: "Patch handler".to_string(),
                        status: PlanStepStatus::Pending,
                        job_id: None,
                    },
                ],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("checkpoint-session"),
        );

        let outcome = service
            .sync_plan_after_tool(r#"{"tool":"read_file","args":{"path":"src/server.rs"}}"#)
            .expect("sync plan after tool");

        assert_eq!(
            outcome.completed_step.as_deref(),
            Some("Inspect server file")
        );
        assert_eq!(outcome.next_step.as_deref(), Some("Patch handler"));
    }

    #[test]
    fn sync_plan_after_tool_does_not_complete_write_step() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "no-auto-complete-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Patch handler".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("no-auto-complete-session"),
        );

        service
            .sync_plan_after_tool(r#"{"tool":"read_file","args":{"path":"src/server.rs"}}"#)
            .expect("sync plan after tool");

        let plan = crate::memory::storage::load_plan_state(&conn, "no-auto-complete-session")
            .expect("load plan")
            .expect("plan present");
        assert_eq!(plan.items[0].status, PlanStepStatus::InProgress);
        assert!(matches!(
            plan.runtime.as_ref().and_then(|runtime| runtime.followup.as_ref()),
            Some(crate::core::plan::PlanFollowup::Checkpoint {
                step,
                next_step: None
            }) if step == "Patch handler"
        ));
    }

    #[test]
    fn plan_followup_instruction_prioritizes_blocked_steps() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "blocked-followup-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Retry the failing command".to_string(),
                    status: PlanStepStatus::Blocked,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("blocked-followup-session"),
        );

        let instruction = service
            .plan_followup_instruction(&PlanSyncOutcome::default())
            .expect("followup")
            .expect("instruction present");

        assert!(instruction.contains("blocked at step 'Retry the failing command'"));
        assert!(instruction.contains("call update_plan"));
    }

    #[test]
    fn plan_followup_instruction_uses_checkpoint_summary_for_completed_step() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "checkpoint-followup-session",
            &PlanState {
                explanation: None,
                items: vec![
                    PlanItem {
                        step: "Inspect server file".to_string(),
                        status: PlanStepStatus::Completed,
                        job_id: None,
                    },
                    PlanItem {
                        step: "Patch handler".to_string(),
                        status: PlanStepStatus::InProgress,
                        job_id: None,
                    },
                ],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("checkpoint-followup-session"),
        );

        let instruction = service
            .plan_followup_instruction(&PlanSyncOutcome {
                completed_step: Some("Inspect server file".to_string()),
                next_step: Some("Patch handler".to_string()),
            })
            .expect("followup")
            .expect("instruction present");

        assert!(instruction.contains("Plan checkpoint: 'Inspect server file' just completed."));
        assert!(instruction.contains("Continue with 'Patch handler'"));
    }

    #[test]
    fn plan_followup_instruction_requests_retry_once_for_first_failure() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "retry-followup-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Run migration".to_string(),
                    status: PlanStepStatus::Blocked,
                    job_id: None,
                }],
                runtime: Some(crate::core::plan::PlanRuntime {
                    followup: Some(crate::core::plan::PlanFollowup::RetryOrReplan {
                        step: "Run migration".to_string(),
                        command: Some("cargo test".to_string()),
                        retry_count: 1,
                    }),
                    ..Default::default()
                }),
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("retry-followup-session"),
        );

        let instruction = service
            .plan_followup_instruction(&PlanSyncOutcome::default())
            .expect("followup")
            .expect("instruction present");

        assert!(instruction.contains("Retry once if the issue looks transient"));
        assert!(instruction.contains("Run migration"));
    }

    #[test]
    fn plan_followup_instruction_stops_repeated_retries() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "retry-limit-session",
            &PlanState {
                explanation: None,
                items: vec![PlanItem {
                    step: "Run migration".to_string(),
                    status: PlanStepStatus::Blocked,
                    job_id: None,
                }],
                runtime: Some(crate::core::plan::PlanRuntime {
                    followup: Some(crate::core::plan::PlanFollowup::RetryOrReplan {
                        step: "Run migration".to_string(),
                        command: Some("cargo test".to_string()),
                        retry_count: 2,
                    }),
                    ..Default::default()
                }),
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let service = ToolService::new(
            &conn,
            &config,
            &exec_policy,
            None,
            Some("retry-limit-session"),
        );

        let instruction = service
            .plan_followup_instruction(&PlanSyncOutcome::default())
            .expect("followup")
            .expect("instruction present");

        assert!(instruction.contains("Do not keep retrying blindly"));
        assert!(instruction.contains("already failed 2 times"));
    }

    #[test]
    fn extracts_target_paths_from_json_tool_call() {
        let paths = ToolService::target_paths_for_tool_call(
            r#"{"tool":"write_file","args":{"path":"src/main.rs","content":"hi"}}"#,
        );
        assert_eq!(paths, vec![PathBuf::from("src/main.rs")]);
    }

    #[test]
    fn extracts_target_paths_from_bracket_tool_call() {
        let paths = ToolService::target_paths_for_tool_call(r#"[READ_FILE src/main.rs]"#);
        assert_eq!(paths, vec![PathBuf::from("src/main.rs")]);
    }

    #[test]
    fn parse_run_command_sandbox_intent_reads_explicit_fields() {
        let intent = ToolService::parse_run_command_sandbox_intent(&json!({
            "command": "cp ./src.txt ./build/out.txt",
            "declared_read_paths": ["./src.txt"],
            "declared_write_paths": ["./build/out.txt"],
            "requires_network": true,
            "retry_policy": "safe"
        }));

        assert_eq!(intent.declared_read_paths, vec![PathBuf::from("./src.txt")]);
        assert_eq!(
            intent.declared_write_paths,
            vec![PathBuf::from("./build/out.txt")]
        );
        assert!(intent.requires_network);
        assert_eq!(
            intent.retry_policy,
            Some(crate::tools::shell::CommandRetryPolicy::Safe)
        );
    }
}
