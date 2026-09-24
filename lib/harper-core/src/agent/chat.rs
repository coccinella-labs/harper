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

//! Chat interaction module
//!
//! This module handles user input, chat loops, and message processing.

use crate::agent::intent::{DeterministicIntent, route_intent};
use crate::agent::offline_shell::plan_offline_shell_commands;
use crate::agent::prompt::PromptBuilder;
use crate::core::cache::{ApiCacheKey, ApiResponseCache};
use crate::core::error::{HarperError, HarperResult};
use crate::core::plan::AuthoringPhase;
use crate::core::tool_call::{ToolCall, ToolCallSource, parse_tool_calls};
use crate::core::{ApiConfig, Message};
use crate::harness::{
    LlmCompleter, RealLlmCompleter, RealToolDispatcher, ToolDispatchContext, ToolDispatcher,
};
use crate::memory::storage::CommandLogEntry;
use crate::parsing;
use crate::runtime::config::{ExecPolicyConfig, ExecutionStrategy};
use crate::runtime::scheduler::{TaskPriority, TaskScheduler};
use crate::tools::ToolExecOutcome;
use crate::tools::ToolService;
use crate::tools::shell::CommandAuditContext;

use colored::Colorize;
use reqwest::Client;
use rusqlite::Connection;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use serde::Serialize;

use rustyline::Editor;
use rustyline::Helper;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use turul_mcp_client::{McpClient, ResourceContent};

#[derive(Debug)]
enum CommandAction {
    Clear,
    Exit,
    Audit(AuditParams),
    Strategy(Option<ExecutionStrategy>),
    Custom(String),
    Unknown(String),
}

#[derive(Debug, Clone)]
enum ChatBackgroundTask {
    TodoReminder,
    AuditRefresh { session_id: String },
}

#[derive(Debug, Clone, Default)]
struct AuditParams {
    limit: Option<usize>,
    status: Option<String>,
    approval: Option<AuditApprovalFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuditApprovalFilter {
    Approved,
    Rejected,
    Auto,
}

struct FileCompleter;

#[derive(Debug, Clone)]
struct AuthoringRequestContext {
    prompt: String,
    candidate_paths: HashSet<PathBuf>,
}

impl Helper for FileCompleter {}

impl Hinter for FileCompleter {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for FileCompleter {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        std::borrow::Cow::Borrowed(line)
    }
}

impl Validator for FileCompleter {
    fn validate(
        &self,
        _ctx: &mut rustyline::validate::ValidationContext,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        Ok(rustyline::validate::ValidationResult::Valid(None))
    }
}

impl Completer for FileCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        // Find the last @ before the cursor position
        let at_pos = match line[..pos].rfind('@') {
            Some(pos) => pos,
            None => return Ok((pos, Vec::new())), // No @ found, no completion
        };

        let prefix = &line[at_pos + 1..pos];
        let path = Path::new(prefix);

        // Determine directory to read and file prefix to match
        let (dir_to_read, file_prefix) = if prefix.is_empty() {
            (Path::new("."), "")
        } else if prefix.ends_with('/') {
            (path, "")
        } else if prefix == "." {
            (Path::new("."), ".")
        } else if prefix.starts_with('.') && !prefix.contains('/') {
            (Path::new("."), prefix)
        } else {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let file_part = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            (parent, file_part)
        };

        let mut candidates = Vec::new();

        if let Ok(entries) = fs::read_dir(dir_to_read) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with(file_prefix) {
                        let mut completion = prefix.to_string();
                        if prefix.is_empty() || prefix.ends_with('/') {
                            completion.push_str(name);
                        } else {
                            // Replace the file part
                            if let Some(parent) = path.parent() {
                                let parent_str = parent.to_string_lossy();
                                if parent_str.is_empty() {
                                    completion = name.to_string();
                                } else {
                                    completion = format!("{}/{}", parent_str, name);
                                }
                            } else {
                                completion = name.to_string();
                            }
                        }
                        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                            completion.push('/');
                        }
                        candidates.push(Pair {
                            display: completion.clone(),
                            replacement: completion,
                        });
                    }
                }
            }
        }

        let start = at_pos + 1;
        Ok((start, candidates))
    }
}

use crate::core::io_traits::{RuntimeEventSink, UserApproval};
use std::sync::Arc;

/// Chat service for handling conversations
pub struct ChatService<'a> {
    conn: &'a Connection,
    config: &'a ApiConfig,
    api_cache: Option<&'a mut ApiResponseCache>,
    #[allow(dead_code)]
    mcp_client: Option<&'a McpClient>,
    #[allow(dead_code)]
    todos: Vec<String>,
    prompt_id: Option<String>,
    custom_commands: HashMap<String, String>,
    exec_policy: ExecPolicyConfig,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<Arc<dyn RuntimeEventSink>>,
    completer: Arc<dyn LlmCompleter>,
    dispatcher: Arc<dyn ToolDispatcher>,
    tool_call_source: Option<ToolCallSource>,
    background_tasks: TaskScheduler<ChatBackgroundTask>,
    todo_reminder_armed: bool,
    last_audit_refresh: Option<Instant>,
    execution_strategy: ExecutionStrategy,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatTurnDebugSummary {
    pub strategy: String,
    pub task_mode: String,
    pub deterministic_intent: Option<String>,
    pub normalized_command: Option<String>,
    pub clarification: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskMode {
    DirectDeterministic,
    PlanThenAct,
    InspectThenExecute,
    ToolAssisted,
    RespondOnly,
}

impl TaskMode {
    fn initial_activity_label(self) -> &'static str {
        match self {
            Self::DirectDeterministic => "executing deterministic action",
            Self::PlanThenAct => "planning task",
            Self::InspectThenExecute => "inspecting repo context",
            Self::ToolAssisted => "selecting tool",
            Self::RespondOnly => "responding",
        }
    }

    fn model_activity_label(self) -> &'static str {
        match self {
            Self::DirectDeterministic => "summarizing result",
            Self::PlanThenAct => "reasoning over plan",
            Self::InspectThenExecute => "reasoning over grounded context",
            Self::ToolAssisted => "thinking",
            Self::RespondOnly => "responding",
        }
    }
}

#[allow(dead_code)]
impl<'a> ChatService<'a> {
    const AUDIT_LOG_LIMIT: usize = 10;
    const AUDIT_REFRESH_DEBOUNCE: Duration = Duration::from_secs(3);

    /// Create a new chat service
    pub fn new(
        conn: &'a Connection,
        config: &'a ApiConfig,
        mcp_client: Option<&'a McpClient>,
        api_cache: Option<&'a mut ApiResponseCache>,
        prompt_id: Option<String>,
        custom_commands: HashMap<String, String>,
        exec_policy: ExecPolicyConfig,
    ) -> Self {
        let execution_strategy = exec_policy.effective_execution_strategy();
        Self {
            conn,
            config,
            mcp_client,
            api_cache,
            todos: Vec::new(),
            prompt_id,
            custom_commands,
            exec_policy,
            approver: None,
            runtime_events: None,
            completer: Arc::new(RealLlmCompleter),
            dispatcher: Arc::new(RealToolDispatcher),
            tool_call_source: None,
            background_tasks: TaskScheduler::new(),
            todo_reminder_armed: false,
            last_audit_refresh: None,
            execution_strategy,
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

    /// Set the model-completion seam (defaults to the real LLM client).
    pub fn with_completer(mut self, completer: Arc<dyn LlmCompleter>) -> Self {
        self.completer = completer;
        self
    }

    /// Set the tool-execution seam (defaults to the real tool service).
    pub fn with_dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.dispatcher = dispatcher;
        self
    }

    /// Override tool-call parsing source for deterministic replay tests.
    pub fn with_tool_call_source(mut self, tool_call_source: ToolCallSource) -> Self {
        self.tool_call_source = Some(tool_call_source);
        self
    }

    pub fn debug_turn_summary(&self, history: &[Message], user_msg: &str) -> ChatTurnDebugSummary {
        let deterministic_intent = route_intent(user_msg).or_else(|| {
            Self::infer_followup_write_file_intent(history, user_msg)
                .map(DeterministicIntent::WriteFile)
                .or_else(|| {
                    Self::infer_followup_run_command_intent(history, user_msg)
                        .map(DeterministicIntent::RunCommand)
                })
        });
        let task_mode = if route_intent(user_msg).is_none() && deterministic_intent.is_some() {
            TaskMode::DirectDeterministic
        } else {
            Self::decide_task_mode(self.execution_strategy, user_msg)
        };

        let normalized_command = deterministic_intent
            .as_ref()
            .and_then(|intent| match intent {
                DeterministicIntent::RunCommand(intent) => {
                    Some(Self::normalize_run_command_candidate(&intent.command))
                }
                _ => None,
            });

        let clarification = normalized_command.as_ref().and_then(|command| {
            let tool_call = format!("[RUN_COMMAND {}]", command);
            Self::clarification_for_underspecified_tool_call(user_msg, &tool_call)
        });

        ChatTurnDebugSummary {
            strategy: Self::execution_strategy_name(self.execution_strategy).to_string(),
            task_mode: Self::format_task_mode(task_mode).to_string(),
            deterministic_intent: deterministic_intent
                .as_ref()
                .map(Self::format_deterministic_intent),
            normalized_command,
            clarification,
        }
    }

    /// Create a new chat service for testing
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new_test(conn: &'a Connection, config: &'a ApiConfig) -> Self {
        Self {
            conn,
            config,
            mcp_client: None,
            api_cache: None,
            todos: Vec::new(),
            prompt_id: None,
            custom_commands: HashMap::new(),
            exec_policy: ExecPolicyConfig {
                approval_profile: None,
                allowed_commands: None,
                blocked_commands: None,
                sandbox_profile: None,
                sandbox: None,
                retry_max_attempts: None,
                retry_network_commands: None,
                retry_write_commands: None,
                execution_strategy: None,
            },
            approver: None,
            runtime_events: None,
            completer: Arc::new(RealLlmCompleter),
            dispatcher: Arc::new(RealToolDispatcher),
            tool_call_source: None,
            background_tasks: TaskScheduler::new(),
            todo_reminder_armed: false,
            last_audit_refresh: None,
            execution_strategy: ExecutionStrategy::Auto,
        }
    }

    /// Start a new interactive chat session
    pub async fn create_session(
        &self,
        web_search_enabled: bool,
    ) -> Result<(Vec<Message>, String), HarperError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        self.save_session(&session_id)?;

        let system_prompt = self.build_system_prompt(web_search_enabled).await;

        let history = vec![Message {
            role: "system".to_string(),
            content: system_prompt,
        }];

        Ok((history, session_id))
    }

    /// Send a message and get response
    pub async fn send_message(
        &mut self,
        user_msg: &str,
        history: &mut Vec<Message>,
        web_search_enabled: bool,
        session_id: &str,
    ) -> Result<(), HarperError> {
        self.schedule_todo_reminder();
        self.poll_background_tasks(session_id);
        // Handle slash commands locally
        if let Some(command) = user_msg.strip_prefix('/') {
            let response = match command {
                "help" => self.generate_help_text(),
                _ => match self.handle_command(command) {
                    CommandAction::Exit => "Session ended. Type /help for commands.".to_string(),
                    CommandAction::Clear => {
                        history.clear();
                        "Chat history cleared.".to_string()
                    }
                    CommandAction::Audit(params) => {
                        self.format_command_audit(session_id, &params)?
                    }
                    CommandAction::Strategy(Some(strategy)) => {
                        self.execution_strategy = strategy;
                        format!(
                            "Execution strategy set to `{}`.",
                            Self::execution_strategy_name(strategy)
                        )
                    }
                    CommandAction::Strategy(None) => {
                        format!(
                            "Current execution strategy: `{}`.",
                            Self::execution_strategy_name(self.execution_strategy)
                        )
                    }
                    CommandAction::Custom(desc) => {
                        self.add_user_message(history, session_id, &desc)?;
                        let response = self
                            .process_message(history, web_search_enabled, session_id)
                            .await?;
                        self.add_assistant_message(history, session_id, &response)?;
                        self.trim_history(history);
                        return Ok(());
                    }
                    CommandAction::Unknown(s) => s,
                },
            };
            // Add as assistant message
            self.add_assistant_message(history, session_id, &response)?;
            return Ok(());
        }

        // Preprocess @file and @mcp_resource references
        let processed_msg = self.preprocess_file_references(user_msg);
        let processed_msg = self
            .preprocess_mcp_resource_references(&processed_msg)
            .await;

        // Normal message processing
        self.add_user_message(history, session_id, &processed_msg)?;
        if let Some(local_response) = self
            .try_handle_offline_shell_proxy(&processed_msg, session_id)
            .await?
        {
            self.add_assistant_message(history, session_id, &local_response)?;
            self.trim_history(history);
            return Ok(());
        }
        let response = self
            .process_message(history, web_search_enabled, session_id)
            .await?;
        self.add_assistant_message(history, session_id, &response)?;
        self.trim_history(history);
        self.poll_background_tasks(session_id);
        Ok(())
    }

    /// Preprocess @mcp_resource references into content
    pub(crate) async fn preprocess_mcp_resource_references(&self, user_msg: &str) -> String {
        if self.mcp_client.is_none() {
            return user_msg.to_string();
        }

        let mut processed = user_msg.to_string();
        let mut search_start = 0;

        // Process all @mcp_resource references in the message
        while let Some(at_pos) = processed[search_start..].find('@') {
            let absolute_at_pos = search_start + at_pos;

            // Find the end of the resource URI (next space or end of string)
            let after_at = &processed[absolute_at_pos + 1..];
            let resource_end = after_at.find(' ').unwrap_or(after_at.len());
            let resource_part = &after_at[..resource_end];

            // Check if it starts with "mcp:" to identify MCP resources
            if let Some(resource_uri) = resource_part.strip_prefix("mcp:") {
                // Try to read the MCP resource
                match self.read_mcp_resource(resource_uri).await {
                    Ok(content) => {
                        // Replace @mcp:uri with the resource content
                        let replacement =
                            format!("\n[READ MCP RESOURCE: {}]\n{}\n", resource_uri, content);
                        let end_of_pattern = absolute_at_pos + 1 + resource_part.len();
                        processed.replace_range(absolute_at_pos..end_of_pattern, &replacement);

                        // Update search position to continue after this replacement
                        search_start = absolute_at_pos + replacement.len();
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to read MCP resource {}: {}",
                            resource_uri, e
                        );
                        search_start = absolute_at_pos + 1;
                    }
                }
            } else {
                // Not an MCP resource, skip this @
                search_start = absolute_at_pos + 1;
            }
        }

        processed
    }

    /// Read an MCP resource by URI
    async fn read_mcp_resource(&self, uri: &str) -> Result<String, HarperError> {
        let Some(client) = self.mcp_client else {
            return Err(HarperError::Config("MCP client not available".to_string()));
        };

        match client.read_resource(uri).await {
            Ok(contents) => {
                let mut content_parts = Vec::new();
                for item in &contents {
                    match item {
                        ResourceContent::Text(text_content) => {
                            content_parts.push(text_content.text.clone());
                        }
                        ResourceContent::Blob(blob_content) => {
                            content_parts.push(format!(
                                "[Binary content: {} bytes, type: {}]",
                                blob_content.blob.len(),
                                blob_content
                                    .mime_type
                                    .clone()
                                    .unwrap_or("unknown".to_string())
                            ));
                        }
                    }
                }
                Ok(content_parts.join("\n"))
            }
            Err(e) => Err(HarperError::Mcp(format!("MCP resource read failed: {}", e))),
        }
    }

    /// Preprocess @file references into file or image tool commands.
    pub(crate) fn preprocess_file_references(&self, user_msg: &str) -> String {
        if let Some(image_path) = parse_standalone_image_reference(user_msg) {
            return format!("[IMAGE_INFO {}]", image_path);
        }

        let mut processed = user_msg.to_string();
        let mut search_start = 0;

        // Process all @file references in the message
        while let Some(at_pos) = processed[search_start..].find('@') {
            let absolute_at_pos = search_start + at_pos;

            let after_at = &processed[absolute_at_pos + 1..];
            let Some(reference) = parse_at_file_reference(after_at) else {
                search_start = absolute_at_pos + 1;
                continue;
            };
            let file_path = reference.path.as_str();

            let is_image_reference = is_supported_image_reference(file_path);
            if !file_path.is_empty()
                && !file_path.starts_with('!')
                && (!file_path.starts_with('/') || is_image_reference)
            {
                let replacement = if is_image_reference {
                    format!("[IMAGE_INFO {}]", file_path)
                } else {
                    format!("[READ_FILE {}]", file_path)
                };
                let replace_end = absolute_at_pos + 1 + reference.consumed;
                processed.replace_range(absolute_at_pos..replace_end, &replacement);
                search_start = absolute_at_pos + replacement.len();
            } else {
                // Not a valid file reference, skip this @ and continue
                search_start = absolute_at_pos + 1;
            }
        }

        processed
    }

    /// Generate help text for commands
    fn generate_help_text(&self) -> String {
        let mut help_text = "Available commands:\n\n".to_string();
        help_text.push_str("/help\n");
        help_text.push_str("  Show this help.\n\n");
        help_text.push_str("/exit\n");
        help_text.push_str("  Exit the session.\n\n");
        help_text.push_str("/clear\n");
        help_text.push_str("  Clear chat history.\n\n");
        help_text.push_str("/audit [limit]\n");
        help_text.push_str("  Show recent command executions.\n\n");
        help_text.push_str("/strategy [auto|grounded|deterministic|model]\n");
        help_text.push_str("  Set or show execution strategy.\n\n");
        help_text.push_str("!command\n");
        help_text.push_str("  Execute shell command directly.\n\n");
        help_text.push_str("@file\n");
        help_text.push_str("  Reference and read files with Tab completion.\n");
        for (cmd, desc) in &self.custom_commands {
            help_text.push_str(&format!("\n/{}\n  {}\n", cmd, desc));
        }
        help_text
    }

    fn handle_command(&self, command: &str) -> CommandAction {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return CommandAction::Unknown("No command provided.".to_string());
        }

        let mut split_idx = None;
        for (idx, ch) in trimmed.char_indices() {
            if ch.is_whitespace() {
                split_idx = Some(idx);
                break;
            }
        }

        let (name, args) = if let Some(idx) = split_idx {
            let (left, right) = trimmed.split_at(idx);
            (left, Some(right.trim()))
        } else {
            (trimmed, None)
        };

        match name {
            "exit" => CommandAction::Exit,
            "clear" => CommandAction::Clear,
            "strategy" => match args {
                Some(value) => match Self::parse_execution_strategy(value) {
                    Some(strategy) => CommandAction::Strategy(Some(strategy)),
                    None => CommandAction::Unknown(format!(
                        "Unknown strategy '{}'. Use: auto, grounded, deterministic, model.",
                        value
                    )),
                },
                None => CommandAction::Strategy(None),
            },
            "audit" => match parse_audit_params(args) {
                Ok(params) => CommandAction::Audit(params),
                Err(err) => CommandAction::Unknown(err.to_string()),
            },
            cmd => {
                if let Some(desc) = self.custom_commands.get(cmd) {
                    CommandAction::Custom(desc.clone())
                } else {
                    CommandAction::Unknown(format!(
                        "Unknown command '{}'. Type /help for available commands.",
                        cmd
                    ))
                }
            }
        }
    }

    /// Build system prompt
    pub async fn build_system_prompt(&self, web_search_enabled: bool) -> String {
        let prompt_builder =
            PromptBuilder::new(self.config, self.prompt_id.clone(), self.mcp_client);
        prompt_builder.build_system_prompt(web_search_enabled).await
    }

    fn schedule_todo_reminder(&mut self) {
        if self.todo_reminder_armed {
            return;
        }
        self.todo_reminder_armed = true;
        self.background_tasks.schedule_in(
            ChatBackgroundTask::TodoReminder,
            Duration::from_secs(45),
            TaskPriority::LOW,
        );
    }

    fn notify_command_activity(&mut self, session_id: &str) {
        let now = Instant::now();
        if let Some(last) = self.last_audit_refresh {
            if now.duration_since(last) < Self::AUDIT_REFRESH_DEBOUNCE {
                return;
            }
        }
        self.last_audit_refresh = Some(now);
        self.background_tasks.schedule_in(
            ChatBackgroundTask::AuditRefresh {
                session_id: session_id.to_string(),
            },
            Duration::from_millis(1200),
            TaskPriority::HIGH,
        );
    }

    fn poll_background_tasks(&mut self, default_session_id: &str) {
        let ready = self.background_tasks.drain_ready(Instant::now());
        for item in ready {
            match item.payload {
                ChatBackgroundTask::TodoReminder => {
                    self.todo_reminder_armed = false;
                    if let Err(err) = self.emit_todo_reminder() {
                        eprintln!("Warning: failed to load todos for reminder: {}", err);
                    }
                    self.schedule_todo_reminder();
                }
                ChatBackgroundTask::AuditRefresh { session_id } => {
                    let target = if session_id.is_empty() {
                        default_session_id.to_string()
                    } else {
                        session_id
                    };
                    self.emit_quick_audit_refresh(&target);
                }
            }
        }
    }

    fn emit_todo_reminder(&self) -> HarperResult<()> {
        if self.runtime_events.is_some() {
            return Ok(());
        }

        let todos = crate::memory::storage::load_todos(self.conn)?;
        if todos.is_empty() {
            return Ok(());
        }

        println!(
            "
{} Pending todos ({} total):",
            "Reminder:".bold().cyan(),
            todos.len()
        );
        for (_, desc) in todos.iter().take(3) {
            println!("  - {}", desc);
        }
        if todos.len() > 3 {
            println!("  …and {} more", todos.len() - 3);
        }
        Ok(())
    }

    fn emit_quick_audit_refresh(&self, session_id: &str) {
        if self.runtime_events.is_some() {
            return;
        }

        match self.fetch_command_logs(session_id, 3) {
            Ok(entries) if !entries.is_empty() => {
                println!(
                    "
{} Latest commands:",
                    "Audit:".bold().cyan()
                );
                for entry in entries {
                    let approval_state = if entry.requires_approval {
                        if entry.approved {
                            "approved"
                        } else {
                            "rejected"
                        }
                    } else {
                        "auto"
                    };
                    println!(
                        "  [{} | {} | {:?}] {}",
                        entry.status, approval_state, entry.exit_code, entry.command
                    );
                }
            }
            Ok(_) => {}
            Err(err) => eprintln!("Warning: failed to refresh audit summary: {}", err),
        }
    }

    /// Run the chat loop
    async fn run_chat_loop(
        &mut self,
        session_id: &str,
        history: &mut Vec<Message>,
        web_search_enabled: bool,
    ) -> Result<(), HarperError> {
        loop {
            self.poll_background_tasks(session_id);
            let user_input = self.get_user_input()?;
            if self.should_exit(&user_input) {
                self.display_session_end();
                break;
            }

            // Handle direct shell commands
            if let Some(command) = user_input.strip_prefix('!') {
                let audit_ctx = CommandAuditContext {
                    conn: self.conn,
                    session_id: Some(session_id),
                    source: "user_shell",
                };
                match crate::tools::shell::execute_command(
                    &format!("[RUN_COMMAND {}]", command),
                    self.config,
                    &self.exec_policy,
                    None,
                    Some(&audit_ctx),
                    self.approver.clone(),
                    self.runtime_events.clone(),
                )
                .await
                {
                    Ok(result) => println!("{} {}", "Shell:".bold().cyan(), result.cyan()),
                    Err(e) => println!("{} {}", "Error:".bold().red(), e),
                }
                self.notify_command_activity(session_id);
                continue;
            }

            // Handle slash commands
            if let Some(command) = user_input.strip_prefix('/') {
                match command {
                    "help" => {
                        let help_text = self.generate_help_text();
                        let mut lines = help_text.lines();
                        if let Some(first_line) = lines.next() {
                            println!("{}", first_line.bold().yellow());
                            lines.for_each(|line| println!("{}", line));
                        }
                        continue;
                    }
                    _ => match self.handle_command(command) {
                        CommandAction::Exit => {
                            self.display_session_end();
                            break;
                        }
                        CommandAction::Clear => {
                            history.clear();
                            println!("{}", "Chat history cleared.".bold().green());
                            continue;
                        }
                        CommandAction::Audit(params) => {
                            let report = self.format_command_audit(session_id, &params)?;
                            for line in report.lines() {
                                println!("{}", line);
                            }
                            continue;
                        }
                        CommandAction::Strategy(Some(strategy)) => {
                            self.execution_strategy = strategy;
                            println!(
                                "Execution strategy set to `{}`.",
                                Self::execution_strategy_name(strategy)
                            );
                            continue;
                        }
                        CommandAction::Strategy(None) => {
                            println!(
                                "Current execution strategy: `{}`.",
                                Self::execution_strategy_name(self.execution_strategy)
                            );
                            continue;
                        }
                        CommandAction::Custom(desc) => {
                            println!("Custom command: {}", desc);
                            self.add_user_message(history, session_id, &desc)?;
                            let response = self
                                .process_message(history, web_search_enabled, session_id)
                                .await
                                .map_err(|e| {
                                    HarperError::Api(format!(
                                        "Failed to process message in session {}: {}",
                                        session_id, e
                                    ))
                                })?;
                            self.display_response(&response);
                            self.add_assistant_message(history, session_id, &response)?;
                            self.trim_history(history);
                            continue;
                        }
                        CommandAction::Unknown(s) => {
                            println!("{} {}", "Error:".bold().red(), s);
                            continue;
                        }
                    },
                }
            }

            self.add_user_message(history, session_id, &user_input)?;
            let response = self
                .process_message(history, web_search_enabled, session_id)
                .await
                .map_err(|e| {
                    HarperError::Api(format!(
                        "Failed to process message in session {}: {}",
                        session_id, e
                    ))
                })?;
            self.display_response(&response);
            self.add_assistant_message(history, session_id, &response)?;
            self.trim_history(history);
        }
        Ok(())
    }

    /// Get user input with completion
    fn get_user_input(&self) -> Result<String, HarperError> {
        let config = rustyline::Config::builder().build();
        let mut rl: Editor<FileCompleter, _> = Editor::with_config(config)?;
        rl.set_helper(Some(FileCompleter));
        let prompt = format!("{} ", "You:".bold().blue());
        match rl.readline(&prompt) {
            Ok(line) => {
                let _ = rl.add_history_entry(&line);
                Ok(line.trim().to_string())
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => Ok("exit".to_string()),
            Err(e) => Err(e.into()),
        }
    }

    /// Check if should exit
    pub fn should_exit(&self, input: &str) -> bool {
        input.is_empty() || input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit")
    }

    /// Display session end
    fn display_session_end(&self) {
        println!("{}", "Session ended.".bold().yellow());
    }

    /// Process message
    pub(crate) async fn process_message(
        &mut self,
        history: &mut Vec<Message>,
        web_search_enabled: bool,
        session_id: &str,
    ) -> Result<String, HarperError> {
        const MAX_TOOL_ROUNDS: usize = 4;
        const MAX_GUIDANCE_TURNS: usize = 4;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()?;
        let mut history_for_llm = history.clone();
        if let Some(plan_prompt) = self.plan_prompt_for_request(history, session_id)? {
            history_for_llm.push(Message {
                role: "system".to_string(),
                content: plan_prompt,
            });
        }
        let last_user_msg = history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.clone())
            .unwrap_or_default();
        let task_mode =
            if self.should_force_followup_deterministic(history, &last_user_msg, session_id) {
                TaskMode::DirectDeterministic
            } else {
                Self::decide_task_mode(self.execution_strategy, &last_user_msg)
            };
        self.emit_activity_update(
            session_id,
            Some(task_mode.initial_activity_label().to_string()),
        );
        self.persist_loop_stage(
            session_id,
            match task_mode {
                TaskMode::DirectDeterministic | TaskMode::ToolAssisted => {
                    crate::core::plan::PlanLoopStage::Executing
                }
                TaskMode::PlanThenAct => crate::core::plan::PlanLoopStage::Planning,
                TaskMode::InspectThenExecute => crate::core::plan::PlanLoopStage::Inspecting,
                TaskMode::RespondOnly => crate::core::plan::PlanLoopStage::Responding,
            },
            Some(task_mode.initial_activity_label().to_string()),
        );
        let mut authoring_context = None;
        if matches!(task_mode, TaskMode::PlanThenAct) {
            self.emit_activity_update(session_id, Some("planning task".to_string()));
            self.persist_loop_stage(
                session_id,
                crate::core::plan::PlanLoopStage::Planning,
                Some("planning task".to_string()),
            );
        }
        if matches!(
            task_mode,
            TaskMode::PlanThenAct | TaskMode::InspectThenExecute
        ) && Self::request_needs_authoring_flow(&last_user_msg)
        {
            self.emit_activity_update(session_id, Some("inspecting repo context".to_string()));
            self.persist_loop_stage(
                session_id,
                crate::core::plan::PlanLoopStage::Inspecting,
                Some("inspecting repo context".to_string()),
            );
        }
        if let Some(authoring_request_context) =
            self.authoring_prompt_for_request(&last_user_msg).await?
        {
            history_for_llm.push(Message {
                role: "system".to_string(),
                content: authoring_request_context.prompt.clone(),
            });
            authoring_context = Some(authoring_request_context);
        }

        let should_try_deterministic_now =
            self.should_force_followup_deterministic(history, &last_user_msg, session_id)
                || (Self::should_prefer_deterministic_routing(
                    self.execution_strategy,
                    &last_user_msg,
                ) && route_intent(&last_user_msg).is_some());
        if !matches!(task_mode, TaskMode::RespondOnly) && should_try_deterministic_now {
            self.emit_activity_update(
                session_id,
                Some("executing deterministic action".to_string()),
            );
            self.persist_loop_stage(
                session_id,
                crate::core::plan::PlanLoopStage::Executing,
                Some("executing deterministic action".to_string()),
            );
            if let Some(response) = self
                .complete_via_deterministic_intent(
                    &client,
                    &mut history_for_llm,
                    history,
                    session_id,
                    &last_user_msg,
                    true,
                )
                .await?
            {
                return Ok(response);
            }
        }

        self.emit_activity_update(
            session_id,
            Some(task_mode.model_activity_label().to_string()),
        );
        self.persist_loop_stage(
            session_id,
            match task_mode {
                TaskMode::PlanThenAct => crate::core::plan::PlanLoopStage::Planning,
                TaskMode::InspectThenExecute => crate::core::plan::PlanLoopStage::Inspecting,
                TaskMode::DirectDeterministic | TaskMode::ToolAssisted => {
                    crate::core::plan::PlanLoopStage::Executing
                }
                TaskMode::RespondOnly => crate::core::plan::PlanLoopStage::Responding,
            },
            Some(task_mode.model_activity_label().to_string()),
        );
        let mut response = match self.call_llm(&client, &history_for_llm).await {
            Ok(response) => response,
            Err(HarperError::Api(_)) | Err(HarperError::Command(_)) => {
                if matches!(task_mode, TaskMode::RespondOnly) {
                    return Ok(self.persist_backend_unavailable(session_id));
                }
                if let Some(response) = self
                    .complete_via_deterministic_intent(
                        &client,
                        &mut history_for_llm,
                        history,
                        session_id,
                        &last_user_msg,
                        false,
                    )
                    .await?
                {
                    return Ok(response);
                }
                return Ok(self.persist_backend_unavailable(session_id));
            }
            Err(err) => return Err(err),
        };
        let mut executed_tool_calls: HashSet<String> =
            crate::memory::storage::load_session_tool_dedup_keys(self.conn, session_id)?;
        let mut injected_agents_guidance: HashSet<String> = HashSet::new();
        let mut last_tool_content: Option<String> = None;
        // Forced retry fires at most once per message by design. After a
        // tool executes, dispatch produces a plain-language followup that
        // must be accepted as the final answer; resetting this latch would
        // force past every followup and exhaust MAX_GUIDANCE_TURNS on each
        // tool-requiring request. Batched tool calls in one response are
        // unaffected by this budget.
        let mut forced_tool_retry = false;
        let mut saw_retry_guidance = false;
        let mut tool_rounds = 0usize;
        let mut guidance_turns = 0usize;
        let mut loop_outcome = crate::core::plan::PlanLoopOutcome::Responded;
        let mut loop_feedback = Some("response delivered".to_string());
        let mut broke_out_of_rounds = false;
        let persisted_authoring = self
            .prompt_id
            .as_deref()
            .and_then(|session_id| {
                crate::memory::storage::load_plan_state(self.conn, session_id).ok()
            })
            .flatten()
            .and_then(|plan| plan.runtime)
            .and_then(|runtime| runtime.authoring);
        let persisted_authoring_scope: HashSet<PathBuf> = persisted_authoring
            .as_ref()
            .map(|authoring| {
                authoring
                    .edit_scope
                    .iter()
                    .map(|path| Self::normalize_authoring_path(PathBuf::from(path)))
                    .collect()
            })
            .unwrap_or_default();
        let mut saw_authoring_inspection = persisted_authoring
            .as_ref()
            .and_then(|authoring| authoring.phase.as_ref())
            .is_some_and(|phase| {
                matches!(
                    phase,
                    AuthoringPhase::FilesInspected
                        | AuthoringPhase::EditsApplied
                        | AuthoringPhase::Validated
                )
            });
        let mut saw_plan_update = persisted_authoring
            .as_ref()
            .and_then(|authoring| authoring.phase.as_ref())
            .is_some_and(|phase| {
                matches!(
                    phase,
                    AuthoringPhase::PlanCreated
                        | AuthoringPhase::FilesInspected
                        | AuthoringPhase::EditsApplied
                        | AuthoringPhase::Validated
                )
            });
        let mut inspected_paths: HashSet<PathBuf> = persisted_authoring
            .as_ref()
            .map(|authoring| {
                authoring
                    .inspected_files
                    .iter()
                    .map(|path| Self::normalize_authoring_path(PathBuf::from(path)))
                    .collect()
            })
            .unwrap_or_default();
        let has_structured_authoring_plan = persisted_authoring
            .as_ref()
            .and_then(|authoring| authoring.structured_plan.as_ref())
            .is_some();

        let tool_call_source = self
            .tool_call_source
            .unwrap_or_else(|| ToolCallSource::from(&self.config.provider));
        'round: loop {
            let clean_response = Self::sanitize_model_response(&response);
            let mut tool_calls = parse_tool_calls(&clean_response, tool_call_source);
            for tool_call in &mut tool_calls {
                tool_call.normalize_run_command();
            }

            // Plain text response without any tool call
            if tool_calls.is_empty() {
                if self.execution_strategy != ExecutionStrategy::Deterministic
                    && matches!(
                        self.execution_strategy,
                        ExecutionStrategy::Auto | ExecutionStrategy::Grounded
                    )
                    && Self::response_looks_like_generic_capability_refusal(&clean_response)
                {
                    if let Some(response) = self
                        .complete_via_deterministic_intent(
                            &client,
                            &mut history_for_llm,
                            history,
                            session_id,
                            &last_user_msg,
                            false,
                        )
                        .await?
                    {
                        return Ok(response);
                    }
                }
                if let Some(required_tool) = Self::forced_tool_retry_target(
                    &last_user_msg,
                    &clean_response,
                    forced_tool_retry,
                ) {
                    if guidance_turns >= MAX_GUIDANCE_TURNS {
                        broke_out_of_rounds = true;
                        break;
                    }
                    guidance_turns += 1;
                    forced_tool_retry = true;
                    saw_retry_guidance = true;
                    history_for_llm.push(Message {
                        role: "system".to_string(),
                        content: format!(
                            "The user request requires an actual tool call. Do not answer with prose. Respond now with exactly one JSON tool call using `{}`.",
                            required_tool
                        ),
                    });
                    self.emit_activity_update(
                        session_id,
                        Some(task_mode.model_activity_label().to_string()),
                    );
                    response = match self.call_llm(&client, &history_for_llm).await {
                        Ok(response) => response,
                        Err(HarperError::Api(_)) | Err(HarperError::Command(_)) => {
                            if matches!(task_mode, TaskMode::RespondOnly) {
                                return Ok(self.persist_backend_unavailable(session_id));
                            }
                            if let Some(response) = self
                                .complete_via_deterministic_intent(
                                    &client,
                                    &mut history_for_llm,
                                    history,
                                    session_id,
                                    &last_user_msg,
                                    false,
                                )
                                .await?
                            {
                                return Ok(response);
                            }
                            return Ok(self.persist_backend_unavailable(session_id));
                        }
                        Err(err) => return Err(err),
                    };
                    continue;
                }
                broke_out_of_rounds = true;
                break;
            }

            let merged_candidate_paths = authoring_context
                .as_ref()
                .map(|ctx| {
                    let mut merged = ctx.candidate_paths.clone();
                    merged.extend(persisted_authoring_scope.iter().cloned());
                    merged
                })
                .or_else(|| {
                    (!persisted_authoring_scope.is_empty())
                        .then_some(persisted_authoring_scope.clone())
                });

            let mut charged_tool_round = false;
            for tool_call in &tool_calls {
                let dedupe_key = tool_call.dedupe_key();

                if let Some(authoring_retry_prompt) = Self::authoring_tool_retry_prompt(
                    &last_user_msg,
                    &tool_call.to_raw_string(),
                    merged_candidate_paths.as_ref(),
                    saw_authoring_inspection,
                    saw_plan_update,
                    has_structured_authoring_plan,
                    &inspected_paths,
                ) {
                    if guidance_turns >= MAX_GUIDANCE_TURNS {
                        loop_outcome = crate::core::plan::PlanLoopOutcome::Failed;
                        loop_feedback =
                            Some("guidance budget exhausted before safe authoring".to_string());
                        broke_out_of_rounds = true;
                        break 'round;
                    }
                    guidance_turns += 1;
                    saw_retry_guidance = true;
                    history_for_llm.push(Message {
                        role: "system".to_string(),
                        content: authoring_retry_prompt,
                    });
                    self.emit_activity_update(
                        session_id,
                        Some(task_mode.model_activity_label().to_string()),
                    );
                    response = self.call_llm(&client, &history_for_llm).await?;
                    continue 'round;
                }
                if let Some(agents_prompt) = self.agents_guidance_for_tool_call(
                    tool_call,
                    session_id,
                    &injected_agents_guidance,
                )? {
                    injected_agents_guidance.insert(dedupe_key.clone());
                    if guidance_turns < MAX_GUIDANCE_TURNS {
                        guidance_turns += 1;
                        history_for_llm.push(Message {
                            role: "system".to_string(),
                            content: agents_prompt,
                        });
                        self.emit_activity_update(
                            session_id,
                            Some(task_mode.model_activity_label().to_string()),
                        );
                        response = self.call_llm(&client, &history_for_llm).await?;
                        continue 'round;
                    }
                }
                if !charged_tool_round {
                    if tool_rounds >= MAX_TOOL_ROUNDS {
                        break 'round;
                    }
                    tool_rounds += 1;
                    charged_tool_round = true;
                }
                if executed_tool_calls.contains(&dedupe_key) {
                    if saw_retry_guidance {
                        self.persist_loop_stage(
                            session_id,
                            crate::core::plan::PlanLoopStage::Feedback,
                            Some("duplicate tool call".to_string()),
                        );
                        self.persist_loop_outcome(
                            session_id,
                            crate::core::plan::PlanLoopOutcome::Duplicate,
                            Some("duplicate tool call".to_string()),
                        );
                        return Ok(
                            "This tool was already executed this round, consider a different tool."
                                .to_string(),
                        );
                    }
                    if let Some(content) = last_tool_content {
                        self.persist_loop_stage(
                            session_id,
                            crate::core::plan::PlanLoopStage::Feedback,
                            Some("duplicate tool call".to_string()),
                        );
                        self.persist_loop_outcome(
                            session_id,
                            crate::core::plan::PlanLoopOutcome::Duplicate,
                            Some("duplicate tool call".to_string()),
                        );
                        if matches!(tool_call.name.as_str(), "adx_query" | "azure_data_explorer") {
                            return Ok(content);
                        }
                        return Ok(format!("Tool result:\n{}", content));
                    }
                    loop_outcome = crate::core::plan::PlanLoopOutcome::Duplicate;
                    loop_feedback = Some("duplicate tool call".to_string());
                    broke_out_of_rounds = true;
                    break 'round;
                }
                if let Some(clarification) = Self::clarification_for_underspecified_tool_call(
                    &last_user_msg,
                    &tool_call.to_raw_string(),
                ) {
                    self.persist_loop_stage(
                        session_id,
                        crate::core::plan::PlanLoopStage::Feedback,
                        Some("clarification required".to_string()),
                    );
                    self.persist_loop_outcome(
                        session_id,
                        crate::core::plan::PlanLoopOutcome::Failed,
                        Some(clarification.clone()),
                    );
                    return Ok(clarification);
                }
                let tool_outcome = self
                    .dispatcher
                    .dispatch(
                        ToolDispatchContext {
                            conn: self.conn,
                            config: self.config,
                            exec_policy: &self.exec_policy,
                            session_id: Some(session_id),
                            mcp_client: self.mcp_client,
                            approver: self.approver.clone(),
                            runtime_events: self.runtime_events.clone(),
                        },
                        &client,
                        &history_for_llm,
                        tool_call,
                        web_search_enabled,
                    )
                    .await?;

                match tool_outcome {
                    ToolExecOutcome::NoTool => {
                        broke_out_of_rounds = true;
                        break 'round;
                    }
                    ToolExecOutcome::Recoverable {
                        tool_result,
                        tool_content,
                    }
                    | ToolExecOutcome::Fatal {
                        tool_result,
                        tool_content,
                    } => {
                        self.notify_command_activity(session_id);
                        let mut terminal_tool_result = false;
                        if matches!(tool_call.name.as_str(), "adx_query" | "azure_data_explorer") {
                            terminal_tool_result = true;
                        }
                        if tool_call.name == "update_plan" {
                            saw_plan_update = true;
                            if let Some(authoring_request_context) = authoring_context.as_ref() {
                                let _ = crate::tools::plan::seed_plan_authoring_context(
                                    self.conn,
                                    session_id,
                                    &last_user_msg,
                                    authoring_request_context
                                        .candidate_paths
                                        .iter()
                                        .map(|path| path.display().to_string())
                                        .collect(),
                                );
                            }
                            let _ = crate::tools::plan::mark_plan_authoring_plan_created(
                                self.conn, session_id,
                            );
                        }
                        if matches!(
                            tool_call.name.as_str(),
                            "read_file"
                                | "codebase_investigator"
                                | "git_diff"
                                | "git_status"
                                | "list_changed_files"
                                | "grep"
                        ) {
                            saw_authoring_inspection = true;
                            let inspected = tool_call
                                .target_paths()
                                .into_iter()
                                .map(Self::normalize_authoring_path)
                                .collect::<Vec<_>>();
                            inspected_paths.extend(inspected.iter().cloned());
                            let _ = crate::tools::plan::mark_plan_authoring_inspection(
                                self.conn,
                                session_id,
                                inspected
                                    .into_iter()
                                    .map(|path| path.display().to_string())
                                    .collect(),
                            );
                        }
                        if matches!(tool_call.name.as_str(), "search_replace" | "write_file") {
                            let edited = tool_call
                                .target_paths()
                                .into_iter()
                                .map(Self::normalize_authoring_path)
                                .collect::<Vec<_>>();
                            let _ = crate::tools::plan::mark_plan_authoring_edit_applied(
                                self.conn,
                                session_id,
                                edited
                                    .into_iter()
                                    .map(|path| path.display().to_string())
                                    .collect(),
                            );
                        }
                        if tool_call.name == "run_command"
                            && Self::is_authoring_validation_command(&tool_call.to_raw_string())
                        {
                            let _ = crate::tools::plan::mark_plan_authoring_validated(
                                self.conn, session_id,
                            );
                        }
                        let rejected_by_user = tool_content.ends_with("cancelled by user");
                        if rejected_by_user {
                            loop_outcome = crate::core::plan::PlanLoopOutcome::Rejected;
                            loop_feedback = Some("approval rejected".to_string());
                            broke_out_of_rounds = true;
                        } else {
                            let _ = crate::memory::storage::insert_session_tool_dedup_key(
                                self.conn,
                                session_id,
                                &dedupe_key,
                            );
                            executed_tool_calls.insert(dedupe_key);
                        }
                        last_tool_content = Some(tool_content.clone());
                        let tool_message = Message {
                            role: "system".to_string(),
                            content: tool_content,
                        };
                        history.push(tool_message.clone());
                        history_for_llm.push(tool_message);
                        response = tool_result;
                        if terminal_tool_result || rejected_by_user {
                            if terminal_tool_result {
                                broke_out_of_rounds = true;
                            }
                            break 'round;
                        }
                    }
                }
            }
        }
        if !broke_out_of_rounds && loop_outcome == crate::core::plan::PlanLoopOutcome::Responded {
            loop_outcome = crate::core::plan::PlanLoopOutcome::MaxToolRounds;
            loop_feedback = Some("max tool rounds exhausted".to_string());
        }

        self.persist_loop_stage(
            session_id,
            crate::core::plan::PlanLoopStage::Feedback,
            Some(
                loop_feedback
                    .clone()
                    .unwrap_or_else(|| "response ready".to_string()),
            ),
        );
        self.persist_loop_outcome(session_id, loop_outcome, loop_feedback);

        Ok(Self::finalize_assistant_response(
            &response,
            last_tool_content.as_deref(),
        ))
    }

    fn finalize_assistant_response(response: &str, last_tool_content: Option<&str>) -> String {
        if !response.trim().is_empty() {
            return response.to_string();
        }

        if let Some(tool_content) = last_tool_content.filter(|content| !content.trim().is_empty()) {
            return format!("Tool result:\n{}", tool_content);
        }

        "The model returned an empty response.".to_string()
    }

    fn agents_guidance_for_tool_call(
        &self,
        tool_call: &ToolCall,
        session_id: &str,
        injected_agents_guidance: &HashSet<String>,
    ) -> Result<Option<String>, HarperError> {
        // The membership check and the caller-side insert must use the same
        // canonical key. Checking the raw-call signature while inserting the
        // canonical dedupe key meant the guard never engaged and guidance
        // re-injected every round.
        if injected_agents_guidance.contains(&tool_call.dedupe_key()) {
            return Ok(None);
        }

        let target_paths = ToolService::target_paths_for_tool_call(&tool_call.to_raw_string());
        if target_paths.is_empty() {
            crate::memory::storage::save_active_agents(self.conn, session_id, None)?;
            self.emit_agents_update(session_id, None);
            return Ok(None);
        }

        let cwd = std::env::current_dir().map_err(|err| {
            HarperError::Io(format!(
                "Failed to resolve current working directory: {}",
                err
            ))
        })?;
        let target_refs: Vec<&Path> = target_paths.iter().map(PathBuf::as_path).collect();
        let resolved_agents = crate::core::agents::resolve_agents_for_targets(&cwd, target_refs)?;
        crate::memory::storage::save_active_agents(self.conn, session_id, Some(&resolved_agents))?;
        self.emit_agents_update(session_id, Some(resolved_agents.clone()));
        let Some(rendered) = resolved_agents.render_for_prompt() else {
            return Ok(None);
        };

        Ok(Some(format!(
            "Before executing this path-targeting tool, apply these scoped AGENTS.md instructions for the affected files:\n{}",
            rendered
        )))
    }

    fn emit_agents_update(
        &self,
        session_id: &str,
        agents: Option<crate::core::agents::ResolvedAgents>,
    ) {
        let Some(runtime_events) = &self.runtime_events else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let runtime_events = runtime_events.clone();
            let session_id = session_id.to_string();
            handle.spawn(async move {
                let _ = runtime_events.agents_updated(&session_id, agents).await;
            });
        }
    }

    fn emit_activity_update(&self, session_id: &str, status: Option<String>) {
        let Some(runtime_events) = &self.runtime_events else {
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

    fn persist_loop_stage(
        &self,
        session_id: &str,
        stage: crate::core::plan::PlanLoopStage,
        feedback: Option<String>,
    ) {
        let _ = crate::tools::plan::set_plan_loop_stage(self.conn, session_id, stage, feedback);
    }

    fn persist_loop_outcome(
        &self,
        session_id: &str,
        outcome: crate::core::plan::PlanLoopOutcome,
        feedback: Option<String>,
    ) {
        let _ =
            crate::tools::plan::record_plan_loop_outcome(self.conn, session_id, outcome, feedback);
    }

    fn persist_backend_unavailable(&self, session_id: &str) -> String {
        let reply = Self::model_backend_unavailable_reply();
        self.persist_loop_stage(
            session_id,
            crate::core::plan::PlanLoopStage::Feedback,
            Some(reply.clone()),
        );
        self.persist_loop_outcome(
            session_id,
            crate::core::plan::PlanLoopOutcome::BackendUnavailable,
            Some(reply.clone()),
        );
        reply
    }

    fn plan_prompt_for_request(
        &self,
        history: &[Message],
        session_id: &str,
    ) -> Result<Option<String>, HarperError> {
        let Some(last_user_msg) = history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.trim())
        else {
            return Ok(None);
        };

        if !Self::request_needs_plan(last_user_msg) {
            return Ok(None);
        }

        let existing_plan = crate::memory::storage::load_plan_state(self.conn, session_id)?;
        let has_active_plan = existing_plan.as_ref().is_some_and(|plan| {
            !plan.items.is_empty()
                && plan.items.iter().any(|item| {
                    !matches!(item.status, crate::core::plan::PlanStepStatus::Completed)
                })
        });

        if has_active_plan {
            if let Some(followup) = existing_plan
                .as_ref()
                .and_then(|plan| plan.runtime.as_ref())
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
                                "This multi-step task hit a failure at '{}' while running '{}'. Retry once if the issue looks transient; otherwise call update_plan to revise the remaining steps before more tool work.",
                                step, command
                            )));
                        }
                        return Ok(Some(format!(
                            "This multi-step task has already failed {} times at '{}' while running '{}'. Do not keep retrying blindly; call update_plan to revise the plan before more tool work.",
                            retry_count, step, command
                        )));
                    }
                    crate::core::plan::PlanFollowup::Checkpoint { step, next_step } => {
                        let next_clause = next_step
                            .as_deref()
                            .map(|next_step| {
                                format!(
                                    " Continue with '{}' only after a short checkpoint summary.",
                                    next_step
                                )
                            })
                            .unwrap_or_else(|| {
                                " If the task is done, confirm completion instead of repeating the plan."
                                    .to_string()
                            });
                        return Ok(Some(format!(
                            "This multi-step task has an open checkpoint on '{}'. Briefly summarize what changed before more tool work.{}",
                            step, next_clause
                        )));
                    }
                }
            }
            if let Some(blocked_step) = existing_plan.as_ref().and_then(|plan| {
                plan.items
                    .iter()
                    .find(|item| matches!(item.status, crate::core::plan::PlanStepStatus::Blocked))
            }) {
                return Ok(Some(format!(
                    "This multi-step task is currently blocked at '{}'. Before more tool work, call update_plan to explain the blocker, retry, or revise the remaining steps.",
                    blocked_step.step
                )));
            }
            return Ok(Some(
                "This is an active multi-step task. Keep the plan current with update_plan, and add a brief checkpoint summary whenever a step finishes or the next step changes."
                    .to_string(),
            ));
        }

        Ok(Some(
            "This request looks multi-step. Before doing substantial work, call update_plan with concise steps and exactly one in_progress item."
                .to_string(),
        ))
    }

    fn request_needs_plan(user_msg: &str) -> bool {
        let lower = user_msg.to_ascii_lowercase();
        let word_count = lower.split_whitespace().count();
        if word_count < 8 {
            return false;
        }

        let action_markers = [
            "fix",
            "implement",
            "update",
            "refactor",
            "debug",
            "investigate",
            "wire",
            "integrate",
            "add",
            "build",
            "make",
        ];
        let sequencing_markers = [
            " then ", " and ", " also ", " after ", " before ", " next ", " while ",
        ];

        let has_action = action_markers.iter().any(|marker| lower.contains(marker));
        let has_sequence = sequencing_markers
            .iter()
            .any(|marker| lower.contains(marker))
            || lower.contains(',')
            || lower.contains(':');

        has_action && (has_sequence || word_count >= 14)
    }

    async fn authoring_prompt_for_request(
        &self,
        user_msg: &str,
    ) -> Result<Option<AuthoringRequestContext>, HarperError> {
        if !Self::request_needs_authoring_flow(user_msg) {
            return Ok(None);
        }

        let overview = crate::tools::codebase_investigator::authoring_context(user_msg).await?;
        Ok(Some(AuthoringRequestContext {
            prompt: format!(
                "This is an open-ended code authoring request. Do not answer with prose-only guidance. Use actual Harper tools to act on the repository. First inspect the grounded repo context below. If the task is multi-step, keep the task state current with update_plan before substantial edits. When the target is ambiguous, inspect the codebase before editing. Prefer this flow: understand the repo context, inspect relevant files, update_plan if needed, then use write_file/search_replace/run_command to make the change. Use the AUTHORING_CONTEXT and CANDIDATES sections to decide which files to inspect or edit first.\n\nGrounded repo context:\n{}",
                overview
            ),
            candidate_paths: Self::extract_authoring_candidate_paths(&overview),
        }))
    }

    fn request_needs_authoring_flow(user_msg: &str) -> bool {
        let lower = user_msg.to_ascii_lowercase();
        let has_authoring_signal = [
            "create",
            "make",
            "modify",
            "update",
            "change",
            "edit",
            "refactor",
            "implement",
            "add",
            "build",
            "wire",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
        if !has_authoring_signal {
            return false;
        }

        let is_direct_deterministic_action = crate::agent::intent::route_intent(user_msg)
            .is_some_and(|intent| {
                matches!(
                    intent,
                    crate::agent::intent::DeterministicIntent::WriteFile(_)
                        | crate::agent::intent::DeterministicIntent::ReadFile(_)
                        | crate::agent::intent::DeterministicIntent::RunCommand(_)
                )
            });
        if is_direct_deterministic_action {
            return false;
        }

        lower.contains("repo")
            || lower.contains("codebase")
            || lower.contains("subsystem")
            || lower.contains("feature")
            || lower.contains("screen")
            || lower.contains("tui")
            || lower.contains("ui")
            || lower.contains("core")
            || lower.contains("planner")
            || lower.contains("flow")
            || lower.contains("behavior")
    }

    fn request_requires_tool(user_msg: &str) -> Option<&'static str> {
        let lower = user_msg.to_ascii_lowercase();
        if lower.contains("git diff")
            || lower.contains("what changed")
            || lower.contains("show diff")
        {
            return Some("git_diff");
        }
        if lower.contains("git status") || lower.contains("status of repo") {
            return Some("git_status");
        }
        if (lower.contains("repo")
            || lower.contains("repository")
            || lower.contains("codebase")
            || lower.contains("project"))
            && (lower.contains("find ")
                || lower.contains("where ")
                || lower.contains("render")
                || lower.contains("located")
                || lower.contains("lives")
                || lower.contains("defined")
                || lower.contains("used"))
        {
            return Some("codebase_investigator");
        }
        if lower.contains("where is")
            || lower.contains("where does")
            || lower.contains("find where")
            || lower.contains("find all")
            || lower.contains("who calls")
            || lower.contains("what renders")
        {
            return Some("codebase_investigator");
        }
        if (lower.contains("read ")
            || lower.contains("show ")
            || lower.contains("open ")
            || lower.contains("view ")
            || lower.contains("look at "))
            && (lower.contains(".rs")
                || lower.contains(".md")
                || lower.contains(".toml")
                || lower.contains("file"))
        {
            return Some("read_file");
        }
        if (lower.contains("fix ")
            || lower.contains("edit ")
            || lower.contains("change ")
            || lower.contains("update "))
            && (lower.contains("file") || lower.contains(".rs") || lower.contains(".md"))
        {
            return Some("search_replace");
        }
        if Self::request_needs_authoring_flow(user_msg) {
            return Some("codebase_investigator");
        }
        if lower.contains("run ")
            || lower.contains("execute ")
            || lower.contains("make ")
            || lower.contains("cargo ")
            || lower.contains("grep ")
            || lower.contains("rg ")
        {
            return Some("run_command");
        }
        None
    }

    fn forced_tool_retry_target<'b>(
        user_msg: &'b str,
        model_response: &str,
        forced_tool_retry: bool,
    ) -> Option<&'b str> {
        if forced_tool_retry || Self::tool_call_signature(model_response).is_some() {
            return None;
        }
        Self::request_requires_tool(user_msg)
    }

    fn sanitize_model_response(response: &str) -> String {
        let trimmed_response = response.trim();

        if trimmed_response.starts_with("```") {
            let lines: Vec<&str> = trimmed_response.lines().collect();
            if lines.len() >= 2 {
                lines[1..lines.len() - 1].join("\n").trim().to_string()
            } else {
                trimmed_response.trim_matches('`').trim().to_string()
            }
        } else {
            trimmed_response
                .trim_matches(|c| c == '\'' || c == '\"' || c == '`')
                .trim()
                .to_string()
        }
    }

    fn authoring_tool_retry_prompt(
        user_msg: &str,
        model_response: &str,
        candidate_paths: Option<&HashSet<PathBuf>>,
        saw_authoring_inspection: bool,
        saw_plan_update: bool,
        has_structured_authoring_plan: bool,
        inspected_paths: &HashSet<PathBuf>,
    ) -> Option<String> {
        if !Self::request_needs_authoring_flow(user_msg) {
            return None;
        }
        let tool_name = Self::tool_name_from_tool_call(model_response)?;
        if !matches!(tool_name.as_str(), "search_replace" | "write_file") {
            return None;
        }
        if Self::request_needs_plan(user_msg) && !saw_plan_update {
            return Some(
                "This is an open-ended multi-step authoring request. Before the first edit, call update_plan with concise steps and exactly one in_progress item. Do not edit any file yet.".to_string(),
            );
        }
        if !has_structured_authoring_plan && Self::request_needs_plan(user_msg) {
            return Some(
                "Before the first edit on this open-ended authoring task, call update_plan with an authoring_plan that lists primary_files, supporting_files, planned_edits, and validation_plan. Do not edit any file yet.".to_string(),
            );
        }
        if !saw_authoring_inspection {
            let candidate_hint = candidate_paths
                .map(Self::format_authoring_candidate_hint)
                .unwrap_or_else(|| "Inspect the grounded repo context first.".to_string());
            return Some(format!(
                "Do not edit yet. This open-ended authoring request requires inspection before the first write/search_replace. First call codebase_investigator or read_file on a concrete repo file, then edit. {}",
                candidate_hint
            ));
        }
        let target_paths = ToolService::target_paths_for_tool_call(model_response);
        if !target_paths.is_empty()
            && !Self::authoring_targets_allowed(&target_paths, candidate_paths, inspected_paths)
        {
            let candidate_hint = candidate_paths
                .map(Self::format_authoring_candidate_hint)
                .unwrap_or_else(|| "Edit only files you inspected in this session.".to_string());
            return Some(format!(
                "Do not edit that path yet. For this authoring request, edits must target files from the grounded candidate set or files you already inspected. {}",
                candidate_hint
            ));
        }
        None
    }

    fn tool_name_from_tool_call(tool_call_json: &str) -> Option<String> {
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
        } else if upper.starts_with("[UPDATE_PLAN") {
            Some("update_plan".to_string())
        } else if upper.starts_with("[CODEBASE_INVESTIGATOR") {
            Some("codebase_investigator".to_string())
        } else {
            None
        }
    }

    fn extract_run_command_text(tool_call_json: &str) -> Option<String> {
        let trimmed = tool_call_json.trim();
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let args = json_value
                .get("args")
                .or_else(|| json_value.get("arguments"))?;
            return args
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        parsing::extract_tool_arg(trimmed, "[RUN_COMMAND").ok()
    }

    fn normalize_tool_call_for_execution(tool_call_json: &str) -> String {
        let Some(tool_name) = Self::tool_name_from_tool_call(tool_call_json) else {
            return tool_call_json.to_string();
        };
        if tool_name != "run_command" {
            return tool_call_json.to_string();
        }
        let Some(command) = Self::extract_run_command_text(tool_call_json) else {
            return tool_call_json.to_string();
        };
        let normalized_command = Self::normalize_run_command_candidate(&command);
        if normalized_command == command {
            return tool_call_json.to_string();
        }

        let trimmed = tool_call_json.trim();
        if let Ok(mut json_value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(args) = json_value.get_mut("args") {
                if let Some(command_value) = args.get_mut("command") {
                    *command_value = serde_json::Value::String(normalized_command.clone());
                    return json_value.to_string();
                }
            }
            if let Some(args) = json_value.get_mut("arguments") {
                if let Some(command_value) = args.get_mut("command") {
                    *command_value = serde_json::Value::String(normalized_command.clone());
                    return json_value.to_string();
                }
            }
        }

        format!("[RUN_COMMAND {}]", normalized_command)
    }

    fn normalize_run_command_candidate(command: &str) -> String {
        let mut candidate = command.trim();
        if let Some(stripped) = candidate.strip_prefix("the ") {
            candidate = stripped.trim();
        }
        if let Some(stripped) = candidate.strip_suffix(" command") {
            candidate = stripped.trim();
        }
        candidate = candidate.trim_matches(|c: char| matches!(c, '"' | '\''));
        candidate = candidate.trim_end_matches(['.', ',', ';', ':']);
        let candidate = Self::trim_run_command_suffixes(candidate);
        match candidate.to_ascii_lowercase().as_str() {
            "git status" => "git status".to_string(),
            "git diff" => "git diff".to_string(),
            "clear" => "clear".to_string(),
            "pwd" => "pwd".to_string(),
            "ls" => "ls".to_string(),
            "date" => "date".to_string(),
            "whoami" => "whoami".to_string(),
            _ => candidate.to_string(),
        }
    }

    fn trim_run_command_suffixes(candidate: &str) -> &str {
        let lowered = candidate.to_ascii_lowercase();
        for suffix in [
            " and summarize it",
            " and summarize",
            " then summarize it",
            " then summarize",
            " and explain it",
            " and explain",
            " and show me",
        ] {
            if lowered.ends_with(suffix) {
                let idx = candidate.len() - suffix.len();
                return candidate[..idx].trim();
            }
        }
        candidate
    }

    fn is_authoring_validation_command(tool_call_json: &str) -> bool {
        let Some(command) = Self::extract_run_command_text(tool_call_json) else {
            return false;
        };
        let lower = command.to_ascii_lowercase();
        lower.contains("cargo check")
            || lower.contains("cargo test")
            || lower.contains("cargo fmt")
            || lower.contains("pytest")
            || lower.contains("npm test")
            || lower.contains("pnpm test")
    }

    fn extract_authoring_candidate_paths(overview: &str) -> HashSet<PathBuf> {
        overview
            .lines()
            .filter_map(|line| line.strip_prefix("FILE: "))
            .map(|path| Self::normalize_authoring_path(PathBuf::from(path.trim())))
            .collect()
    }

    fn normalize_authoring_path(path: PathBuf) -> PathBuf {
        let path_str = path.to_string_lossy();
        if let Some(stripped) = path_str.strip_prefix("./") {
            PathBuf::from(stripped)
        } else {
            path
        }
    }

    fn format_authoring_candidate_hint(candidate_paths: &HashSet<PathBuf>) -> String {
        if candidate_paths.is_empty() {
            return "Inspect a concrete repo file first.".to_string();
        }
        let mut candidates = candidate_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        format!(
            "Inspect or edit one of these candidate files first: {}.",
            candidates
                .into_iter()
                .take(4)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    fn authoring_targets_allowed(
        target_paths: &[PathBuf],
        candidate_paths: Option<&HashSet<PathBuf>>,
        inspected_paths: &HashSet<PathBuf>,
    ) -> bool {
        target_paths.iter().all(|target_path| {
            let normalized_target = Self::normalize_authoring_path(target_path.clone());
            let target_name = normalized_target.file_name().and_then(|name| name.to_str());
            inspected_paths.iter().any(|allowed| {
                let normalized_allowed = Self::normalize_authoring_path(allowed.clone());
                normalized_allowed == normalized_target
                    || normalized_allowed.ends_with(&normalized_target)
                    || normalized_target.ends_with(&normalized_allowed)
                    || target_name
                        .zip(
                            normalized_allowed
                                .file_name()
                                .and_then(|name| name.to_str()),
                        )
                        .is_some_and(|(target, allowed)| target == allowed)
            }) || candidate_paths.is_some_and(|candidates| {
                candidates.iter().any(|allowed| {
                    let normalized_allowed = Self::normalize_authoring_path(allowed.clone());
                    normalized_allowed == normalized_target
                        || normalized_allowed.ends_with(&normalized_target)
                        || normalized_target.ends_with(&normalized_allowed)
                        || target_name
                            .zip(
                                normalized_allowed
                                    .file_name()
                                    .and_then(|name| name.to_str()),
                            )
                            .is_some_and(|(target, allowed)| target == allowed)
                })
            })
        })
    }

    fn tool_call_signature(response: &str) -> Option<String> {
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(response) {
            // OpenAI tool_calls array format
            if let Some(arr) = json_value.as_array() {
                if let Some(first) = arr.first() {
                    let name = first
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())?;
                    let args = first
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    return Some(format!("json:{}:{}", name, args));
                }
            }

            // Gemini / regular JSON format
            if let Some(name) = json_value
                .get("tool")
                .and_then(|v| v.as_str())
                .or_else(|| json_value.get("name").and_then(|v| v.as_str()))
                .or_else(|| json_value.get("mcp_tool").and_then(|v| v.as_str()))
            {
                let args = json_value
                    .get("args")
                    .cloned()
                    .or_else(|| json_value.get("arguments").cloned())
                    .unwrap_or(serde_json::Value::Null);
                return Some(format!("json:{}:{}", name, args));
            }
        }

        let upper = response.to_uppercase();
        if upper.starts_with("[READ_FILE") {
            return Some(response.trim().to_string());
        }
        if upper.starts_with("[RUN_COMMAND") {
            return Some(response.trim().to_string());
        }
        if upper.starts_with("[WRITE_FILE")
            || upper.starts_with("[SEARCH_REPLACE")
            || upper.starts_with("[GIT_STATUS")
            || upper.starts_with("[GIT_DIFF")
            || upper.starts_with("[GIT_ADD")
            || upper.starts_with("[GIT_COMMIT")
        {
            return Some(response.trim().to_string());
        }
        None
    }

    fn clarification_for_underspecified_tool_call(
        user_msg: &str,
        tool_call: &str,
    ) -> Option<String> {
        let tool_name = Self::tool_name_from_tool_call(tool_call)?;
        let underspecified_prompt = Self::is_underspecified_prompt(user_msg);
        if matches!(
            tool_name.as_str(),
            "read_file" | "write_file" | "search_replace"
        ) && underspecified_prompt
        {
            let target_paths = ToolService::target_paths_for_tool_call(tool_call);
            if target_paths
                .iter()
                .any(|path| Self::looks_like_placeholder_target(path))
            {
                return Some(
                    "Your message is too ambiguous to act on safely. Name a real repository file or describe the task more specifically."
                        .to_string(),
                );
            }
        }

        if tool_name == "run_command" {
            let command = Self::extract_run_command_text(tool_call)
                .map(|value| Self::normalize_run_command_candidate(&value))
                .unwrap_or_default();
            if Self::is_followup_run_command_placeholder(&command) {
                return Some(
                    "I do not know which previous command you mean. Name the exact command you want to run."
                        .to_string(),
                );
            }
        }

        None
    }

    fn is_underspecified_prompt(user_msg: &str) -> bool {
        let trimmed = user_msg.trim();
        if trimmed.is_empty() {
            return true;
        }
        let tokens = trimmed.split_whitespace().collect::<Vec<_>>();
        tokens.len() <= 1
            && trimmed.len() <= 4
            && !trimmed.contains('/')
            && !trimmed.contains('.')
            && !trimmed.starts_with('/')
            && !trimmed.starts_with('!')
            && !trimmed.starts_with('@')
    }

    fn looks_like_placeholder_target(path: &Path) -> bool {
        let rendered = path.to_string_lossy().to_ascii_lowercase();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        rendered.contains("example.")
            || rendered.contains("path/to/")
            || rendered.contains("your_file")
            || rendered.contains("placeholder")
            || matches!(
                file_name.as_str(),
                "example.txt" | "example.rs" | "example.md" | "file.txt" | "note.txt" | "main.rs"
            )
    }

    fn is_followup_run_command_placeholder(command: &str) -> bool {
        matches!(
            command.trim().to_ascii_lowercase().as_str(),
            "that"
                | "this"
                | "it"
                | "that one"
                | "this one"
                | "that command"
                | "this command"
                | "the command"
        )
    }

    async fn complete_via_deterministic_intent(
        &mut self,
        client: &Client,
        history_for_llm: &mut Vec<Message>,
        history: &mut Vec<Message>,
        session_id: &str,
        last_user_msg: &str,
        persist_success: bool,
    ) -> Result<Option<String>, HarperError> {
        let Some((tool_name, tool_content)) = self
            .try_handle_deterministic_intent(history, last_user_msg, session_id)
            .await?
        else {
            return Ok(None);
        };
        if persist_success {
            self.emit_activity_update(session_id, Some("summarizing result".to_string()));
            self.persist_loop_stage(
                session_id,
                crate::core::plan::PlanLoopStage::Feedback,
                Some("summarizing result".to_string()),
            );
            self.persist_loop_outcome(
                session_id,
                crate::core::plan::PlanLoopOutcome::Succeeded,
                Some("deterministic action completed".to_string()),
            );
        }
        Ok(Some(
            self.summarize_deterministic_tool_result(
                client,
                history_for_llm,
                history,
                session_id,
                &tool_name,
                &tool_content,
            )
            .await?,
        ))
    }

    async fn try_handle_deterministic_intent(
        &self,
        history: &[Message],
        user_msg: &str,
        session_id: &str,
    ) -> Result<Option<(String, String)>, HarperError> {
        let routed_intent = route_intent(user_msg).or_else(|| {
            Self::infer_followup_write_file_intent(history, user_msg)
                .map(DeterministicIntent::WriteFile)
                .or_else(|| {
                    self.infer_followup_run_command_intent_for_session(
                        history, user_msg, session_id,
                    )
                    .map(DeterministicIntent::RunCommand)
                })
        });
        match routed_intent {
            Some(DeterministicIntent::ListChangedFiles(filters)) => {
                let audit_ctx = CommandAuditContext {
                    conn: self.conn,
                    session_id: Some(session_id),
                    source: "intent_list_changed_files",
                };
                let response = crate::tools::git::list_changed_files_with_policy(
                    self.config,
                    &self.exec_policy,
                    Some(&audit_ctx),
                    self.approver.clone(),
                    filters.ext.as_deref(),
                    filters.tracked_only,
                    filters.since.as_deref(),
                )
                .await?;
                Ok(Some(("list_changed_files".to_string(), response)))
            }
            Some(DeterministicIntent::GitStatus) => {
                let response = crate::tools::git::git_status()?;
                Ok(Some(("git_status".to_string(), response)))
            }
            Some(DeterministicIntent::GitDiff) => {
                let response = crate::tools::git::git_diff()?;
                Ok(Some(("git_diff".to_string(), response)))
            }
            Some(DeterministicIntent::GitBranch) => {
                let response = crate::tools::git::current_branch()?;
                Ok(Some(("git_branch".to_string(), response)))
            }
            Some(DeterministicIntent::CurrentDirectory) => {
                let cwd = std::env::current_dir().map_err(|e| {
                    HarperError::Command(format!("Failed to determine current directory: {}", e))
                })?;
                Ok(Some((
                    "current_directory".to_string(),
                    format!("Current working directory: {}", cwd.display()),
                )))
            }
            Some(DeterministicIntent::RepoIdentity) => {
                let response = crate::tools::git::repo_identity()?;
                Ok(Some(("repo_identity".to_string(), response)))
            }
            Some(DeterministicIntent::CodebaseOverview) => {
                let response = crate::tools::codebase_investigator::overview_snapshot().await?;
                Ok(Some(("codebase_overview".to_string(), response)))
            }
            Some(DeterministicIntent::ReadFile(intent)) => {
                let response = crate::tools::filesystem::read_file(
                    &format!("[READ_FILE {}]", intent.path),
                    self.approver.clone(),
                    self.runtime_events.as_ref(),
                    Some(session_id),
                )
                .await?;
                Ok(Some(("read_file".to_string(), response)))
            }
            Some(DeterministicIntent::WriteFile(intent)) => {
                let response = crate::tools::filesystem::write_file_direct(
                    &intent.path,
                    &intent.content,
                    self.approver.clone(),
                    self.runtime_events.as_ref(),
                    Some(session_id),
                )
                .await?;
                Ok(Some(("write_file".to_string(), response)))
            }
            Some(DeterministicIntent::CodebaseSearch(intent)) => {
                let search_intent =
                    Self::infer_codebase_search_intent_kind(user_msg, &intent.pattern);
                let response = crate::tools::codebase_investigator::search_text_with_intent(
                    &intent.pattern,
                    search_intent,
                )
                .await?;
                Ok(Some(("codebase_search".to_string(), response)))
            }
            Some(DeterministicIntent::RunCommand(intent)) => {
                let command = Self::normalize_run_command_candidate(&intent.command);
                let audit_ctx = CommandAuditContext {
                    conn: self.conn,
                    session_id: Some(session_id),
                    source: "intent_run_command",
                };
                let response = crate::tools::shell::execute_command(
                    &format!("[RUN_COMMAND {}]", command),
                    self.config,
                    &self.exec_policy,
                    None,
                    Some(&audit_ctx),
                    self.approver.clone(),
                    self.runtime_events.clone(),
                )
                .await?;
                let rendered = format!("COMMAND: {}\nOUTPUT:\n{}", command, response.trim_end());
                Ok(Some(("run_command".to_string(), rendered)))
            }
            None => Ok(None),
        }
    }

    fn infer_followup_write_file_intent(
        history: &[Message],
        user_msg: &str,
    ) -> Option<crate::agent::intent::WriteFileIntent> {
        let normalized = user_msg.to_ascii_lowercase();
        let wants_creation = normalized.contains("create the file")
            || normalized.contains("create this file")
            || normalized.contains("make this file")
            || normalized.contains("write this file")
            || normalized.contains("can you create this file")
            || normalized.contains("can you make this file")
            || normalized.contains("make the file")
            || normalized.contains("write the file")
            || normalized == "create it"
            || normalized == "write it"
            || normalized == "make it";
        if !wants_creation {
            return None;
        }

        let assistant_msg = history
            .iter()
            .rev()
            .find(|message| message.role == "assistant")?;
        let backtick_segments = Self::extract_backtick_segments(&assistant_msg.content);
        let path = backtick_segments
            .iter()
            .find(|segment| {
                segment.ends_with(".py")
                    || segment.ends_with(".rs")
                    || segment.ends_with(".js")
                    || segment.ends_with(".ts")
                    || segment.ends_with(".txt")
                    || segment.ends_with(".md")
                    || segment.ends_with(".json")
                    || segment.ends_with(".sh")
            })?
            .to_string();
        let path = Self::sanitize_followup_write_path(&path)?;

        let content = backtick_segments
            .iter()
            .find(|segment| {
                Self::sanitize_followup_write_path(segment)
                    .as_deref()
                    .is_none_or(|sanitized| sanitized != path)
                    && !segment.starts_with("python")
                    && !segment.starts_with("python3")
                    && !segment.starts_with("cargo ")
                    && !segment.starts_with("node ")
                    && !segment.starts_with("bash ")
            })
            .cloned()
            .or_else(|| {
                Self::infer_code_from_plain_assistant_text(&assistant_msg.content, &path)
            })?;

        Some(crate::agent::intent::WriteFileIntent { path, content })
    }

    fn infer_followup_run_command_intent(
        history: &[Message],
        user_msg: &str,
    ) -> Option<crate::agent::intent::RunCommandIntent> {
        let normalized = user_msg.to_ascii_lowercase();
        let wants_run = normalized == "run it"
            || normalized == "run that"
            || normalized == "run this"
            || normalized == "run that command"
            || normalized == "run this command"
            || normalized == "execute it"
            || normalized == "execute that"
            || normalized == "execute this"
            || normalized == "execute that command"
            || normalized == "execute this command"
            || normalized == "can you run that"
            || normalized == "can you run this"
            || normalized == "can you run that command"
            || normalized == "can you execute that";
        if !wants_run {
            return None;
        }

        if let Some(previous_user_msg) = Self::previous_user_message(history, user_msg) {
            if let Some(command) = route_intent(previous_user_msg)
                .and_then(Self::canonical_command_for_deterministic_intent)
            {
                return Some(crate::agent::intent::RunCommandIntent { command });
            }
        }

        let assistant_msg = history
            .iter()
            .rev()
            .find(|message| message.role == "assistant")?;
        let command = Self::extract_followup_run_command(&assistant_msg.content)?;
        Some(crate::agent::intent::RunCommandIntent { command })
    }

    fn infer_followup_run_command_intent_for_session(
        &self,
        history: &[Message],
        user_msg: &str,
        session_id: &str,
    ) -> Option<crate::agent::intent::RunCommandIntent> {
        let normalized = user_msg.to_ascii_lowercase();
        let wants_run = normalized == "run it"
            || normalized == "run that"
            || normalized == "run this"
            || normalized == "run that command"
            || normalized == "run this command"
            || normalized == "execute it"
            || normalized == "execute that"
            || normalized == "execute this"
            || normalized == "execute that command"
            || normalized == "execute this command"
            || normalized == "can you run that"
            || normalized == "can you run this"
            || normalized == "can you run that command"
            || normalized == "can you execute that";
        if !wants_run {
            return None;
        }

        if let Ok(entries) = self.fetch_command_logs(session_id, 12) {
            if let Some(command) = entries
                .into_iter()
                .map(|entry| Self::normalize_run_command_candidate(&entry.command))
                .find(|command| {
                    Self::looks_like_runnable_command(command)
                        && !Self::is_followup_run_command_placeholder(command)
                })
            {
                return Some(crate::agent::intent::RunCommandIntent { command });
            }
        }

        Self::infer_followup_run_command_intent(history, user_msg)
    }

    fn should_force_followup_deterministic(
        &self,
        history: &[Message],
        user_msg: &str,
        session_id: &str,
    ) -> bool {
        route_intent(user_msg).is_none()
            && (Self::infer_followup_write_file_intent(history, user_msg).is_some()
                || self
                    .infer_followup_run_command_intent_for_session(history, user_msg, session_id)
                    .is_some())
    }

    fn canonical_command_for_deterministic_intent(intent: DeterministicIntent) -> Option<String> {
        match intent {
            DeterministicIntent::GitStatus => Some("git status".to_string()),
            DeterministicIntent::GitDiff => Some("git diff".to_string()),
            DeterministicIntent::GitBranch => Some("git branch --show-current".to_string()),
            DeterministicIntent::RunCommand(intent) => {
                Some(Self::normalize_run_command_candidate(&intent.command))
            }
            _ => None,
        }
    }

    fn previous_user_message<'h>(
        history: &'h [Message],
        current_user_msg: &str,
    ) -> Option<&'h str> {
        let mut skipped_current = false;
        for message in history
            .iter()
            .rev()
            .filter(|message| message.role == "user")
        {
            if !skipped_current && message.content.trim() == current_user_msg.trim() {
                skipped_current = true;
                continue;
            }
            return Some(message.content.as_str());
        }
        None
    }

    fn extract_backtick_segments(content: &str) -> Vec<String> {
        let mut segments = Vec::new();
        let mut remaining = content;
        while let Some(start) = remaining.find('`') {
            let after_start = &remaining[start + 1..];
            let Some(end) = after_start.find('`') else {
                break;
            };
            let segment = after_start[..end].trim();
            if !segment.is_empty() {
                segments.push(segment.to_string());
            }
            remaining = &after_start[end + 1..];
        }
        segments
    }

    fn extract_followup_run_command(content: &str) -> Option<String> {
        let mut candidates = Self::extract_backtick_segments(content)
            .into_iter()
            .map(|segment| Self::normalize_run_command_candidate(&segment))
            .filter(|segment| Self::looks_like_runnable_command(segment))
            .filter(|segment| !Self::is_followup_run_command_placeholder(segment))
            .collect::<Vec<_>>();

        candidates.extend(content.lines().filter_map(|line| {
            let candidate = Self::extract_command_from_followup_line(line)?;
            let candidate = Self::normalize_run_command_candidate(&candidate);
            if Self::looks_like_runnable_command(&candidate) {
                Some(candidate)
            } else {
                None
            }
        }));

        candidates
            .into_iter()
            .find(|candidate| Self::looks_like_execution_command(candidate))
            .or_else(|| {
                Self::extract_backtick_segments(content)
                    .into_iter()
                    .map(|segment| Self::normalize_run_command_candidate(&segment))
                    .find(|segment| {
                        Self::looks_like_runnable_command(segment)
                            && !Self::is_followup_run_command_placeholder(segment)
                    })
            })
    }

    fn extract_command_from_followup_line(line: &str) -> Option<String> {
        let trimmed = line.trim().trim_matches('`');
        if let Some(value) = trimmed.strip_prefix("COMMAND: ") {
            return Some(value.trim().to_string());
        }
        if let Some(value) = trimmed.strip_prefix("$ ") {
            let value = value
                .split(" sh:")
                .next()
                .unwrap_or(value)
                .split(" bash:")
                .next()
                .unwrap_or(value)
                .trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if let Some(value) = trimmed.strip_prefix("Ran `") {
            let value = value.split('`').next().unwrap_or(value).trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if Self::looks_like_runnable_command(trimmed) {
            return Some(trimmed.to_string());
        }
        None
    }

    fn looks_like_runnable_command(candidate: &str) -> bool {
        let trimmed = candidate.trim();
        let lowered = trimmed.to_ascii_lowercase();
        trimmed.starts_with("./")
            || trimmed.starts_with('/')
            || lowered.starts_with("cargo ")
            || lowered.starts_with("python ")
            || lowered.starts_with("python3 ")
            || lowered.starts_with("bash ")
            || lowered.starts_with("sh ")
            || lowered.starts_with("chmod ")
            || lowered.starts_with("npm ")
            || lowered.starts_with("pnpm ")
            || lowered.starts_with("git ")
    }

    fn looks_like_execution_command(candidate: &str) -> bool {
        let trimmed = candidate.trim();
        let lowered = trimmed.to_ascii_lowercase();
        trimmed.starts_with("./")
            || trimmed.starts_with('/')
            || lowered.starts_with("cargo ")
            || lowered.starts_with("python ")
            || lowered.starts_with("python3 ")
            || lowered.starts_with("bash ")
            || lowered.starts_with("sh ")
            || lowered.starts_with("npm ")
            || lowered.starts_with("pnpm ")
            || lowered.starts_with("git ")
    }

    fn looks_like_absolute_followup_path(path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        normalized.starts_with('/')
            || normalized.starts_with("~/")
            || normalized.starts_with("//")
            || normalized
                .as_bytes()
                .get(1)
                .is_some_and(|byte| *byte == b':')
    }

    fn sanitize_followup_write_path(path: &str) -> Option<String> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return None;
        }
        let candidate = std::path::Path::new(trimmed);
        let sanitized = if candidate.is_absolute()
            || Self::looks_like_absolute_followup_path(trimmed)
            || trimmed.contains("..")
        {
            candidate.file_name()?.to_str()?.to_string()
        } else {
            trimmed.to_string()
        };
        if sanitized.is_empty() || sanitized.contains('/') || sanitized.contains('\\') {
            return None;
        }
        Some(sanitized)
    }

    fn infer_code_from_plain_assistant_text(content: &str, path: &str) -> Option<String> {
        if path.ends_with(".py") {
            return content
                .lines()
                .find(|line| line.contains("print("))
                .map(|line| line.trim().to_string());
        }
        if path.ends_with(".rs") {
            return content
                .lines()
                .find(|line| line.contains("fn main"))
                .map(|line| line.trim().to_string());
        }
        if path.ends_with(".md") {
            let markdown_lines = content
                .lines()
                .skip_while(|line| {
                    let trimmed = line.trim();
                    !trimmed.starts_with('#')
                        && !trimmed.starts_with("- ")
                        && !trimmed.starts_with("* ")
                        && !trimmed.chars().next().is_some_and(|c| c.is_ascii_digit())
                })
                .map(str::trim_end)
                .collect::<Vec<_>>();
            let markdown = markdown_lines.join("\n").trim().to_string();
            if !markdown.is_empty() {
                return Some(markdown);
            }
        }
        if path.ends_with(".txt") {
            let text = content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                return Some(text);
            }
        }
        None
    }

    async fn summarize_deterministic_tool_result(
        &mut self,
        client: &Client,
        history_for_llm: &mut Vec<Message>,
        history: &mut Vec<Message>,
        session_id: &str,
        tool_name: &str,
        tool_content: &str,
    ) -> Result<String, HarperError> {
        self.notify_command_activity(session_id);
        if !Self::should_model_summarize_deterministic_result(self.execution_strategy, tool_name) {
            return Ok(Self::compact_deterministic_fallback(
                tool_name,
                tool_content,
            ));
        }

        let tool_message = Message {
            role: "system".to_string(),
            content: tool_content.to_string(),
        };
        history.push(tool_message.clone());
        history_for_llm.push(tool_message);
        history_for_llm.push(Message {
            role: "system".to_string(),
            content: Self::deterministic_summary_instruction(tool_name).to_string(),
        });
        self.emit_activity_update(session_id, Some("summarizing result".to_string()));
        let response = match self.call_llm(client, history_for_llm).await {
            Ok(response) => response,
            Err(HarperError::Api(_)) | Err(HarperError::Command(_)) => {
                return Ok(Self::compact_deterministic_fallback(
                    tool_name,
                    tool_content,
                ));
            }
            Err(err) => return Err(err),
        };
        if response.trim().is_empty()
            || Self::tool_call_signature(&response).is_some()
            || Self::deterministic_summary_is_low_value(tool_name, &response, tool_content)
        {
            return Ok(Self::compact_deterministic_fallback(
                tool_name,
                tool_content,
            ));
        }
        Ok(response)
    }

    fn should_model_summarize_deterministic_result(
        strategy: ExecutionStrategy,
        tool_name: &str,
    ) -> bool {
        if matches!(strategy, ExecutionStrategy::Deterministic) {
            return false;
        }

        !matches!(
            tool_name,
            "repo_identity"
                | "git_branch"
                | "git_status"
                | "git_diff"
                | "write_file"
                | "run_command"
        )
    }

    fn deterministic_summary_instruction(tool_name: &str) -> &'static str {
        match tool_name {
            "codebase_search" => {
                "A deterministic `codebase_search` tool was already selected and executed. Do not call any tool. Use only this structured search context. Prioritize files with the strongest ROLE and REASONS for the query focus, explain which file is most relevant first, and avoid repeating raw match dumps."
            }
            "codebase_overview" => {
                "A deterministic `codebase_overview` tool was already selected and executed. Do not call any tool. Use only this workspace snapshot. Answer briefly with a grounded codebase overview: summarize the repo structure, major crates, and likely hotspots without repeating the raw snapshot verbatim."
            }
            _ => {
                "A deterministic tool was already selected and executed. Do not call any tool. Answer in plain language using only this tool result. Keep the answer short and summarize the most relevant result instead of repeating raw output."
            }
        }
    }

    fn deterministic_summary_is_low_value(
        tool_name: &str,
        response: &str,
        tool_content: &str,
    ) -> bool {
        if tool_name != "codebase_search" && tool_name != "codebase_overview" {
            return false;
        }

        let trimmed = response.trim();
        if trimmed.starts_with("Top matches for ")
            || trimmed.starts_with("Top source matches:")
            || trimmed.starts_with("Tool result:")
            || trimmed.contains("route_intent(")
        {
            return true;
        }

        let repeated_lines = tool_content
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .take(3)
            .filter(|line| trimmed.contains(line.trim()))
            .count();

        let repetition_threshold = if tool_name == "codebase_overview" {
            1
        } else {
            2
        };

        repeated_lines >= repetition_threshold
    }

    fn compact_deterministic_fallback(tool_name: &str, tool_content: &str) -> String {
        if tool_name == "repo_identity"
            || tool_name == "git_branch"
            || tool_name == "current_directory"
        {
            return tool_content.to_string();
        }
        if tool_name == "git_status" {
            return Self::format_git_status(tool_content);
        }
        if tool_name == "git_diff" {
            return Self::format_git_diff(tool_content);
        }
        if tool_name == "write_file" {
            return Self::format_write_file(tool_content);
        }
        if tool_name == "run_command" {
            return Self::format_run_command(tool_content);
        }
        if tool_name == "codebase_overview" {
            return Self::summarize_codebase_overview(tool_content);
        }
        if tool_name == "codebase_search" {
            return Self::summarize_codebase_search_result(tool_content);
        }

        format!("Tool result:\n{}", tool_content)
    }

    fn format_git_status(tool_content: &str) -> String {
        let body = tool_content
            .strip_prefix(
                "Git status:
",
            )
            .unwrap_or(tool_content)
            .trim();
        if body.is_empty() {
            return "Git working directory is clean".to_string();
        }

        let mut modified = Vec::new();
        let mut added = Vec::new();
        let mut deleted = Vec::new();
        let mut untracked = Vec::new();
        let mut renamed = Vec::new();
        let mut other = Vec::new();

        for line in body.lines() {
            let line = line.trim_end();
            if line.len() < 3 {
                continue;
            }
            let status = &line[..2];
            let path = line[3..].trim();
            match status {
                "??" => untracked.push(path.to_string()),
                "A " | "AM" | " M" if status.starts_with('A') => added.push(path.to_string()),
                "D " | " D" | "MD" | "AD" => deleted.push(path.to_string()),
                "R " | "R?" | "RM" | "RR" => renamed.push(path.to_string()),
                _ if status.contains('M') => modified.push(path.to_string()),
                _ => other.push(format!("{} {}", status, path)),
            }
        }

        let mut parts = Vec::new();
        if !modified.is_empty() {
            parts.push(format!("{} modified", modified.len()));
        }
        if !added.is_empty() {
            parts.push(format!("{} added", added.len()));
        }
        if !deleted.is_empty() {
            parts.push(format!("{} deleted", deleted.len()));
        }
        if !renamed.is_empty() {
            parts.push(format!("{} renamed", renamed.len()));
        }
        if !untracked.is_empty() {
            parts.push(format!("{} untracked", untracked.len()));
        }
        if !other.is_empty() {
            parts.push(format!("{} other", other.len()));
        }

        let summary = if parts.is_empty() {
            "Git working directory has changes.".to_string()
        } else {
            format!("Git working directory has {}.", parts.join(", "))
        };

        let mut notable = Vec::new();
        notable.extend(modified.iter().take(3).cloned());
        notable.extend(added.iter().take(3 - notable.len()).cloned());
        if notable.len() < 3 {
            notable.extend(untracked.iter().take(3 - notable.len()).cloned());
        }

        if notable.is_empty() {
            summary
        } else {
            format!("{} Notable files: {}.", summary, notable.join(", "))
        }
    }

    fn format_git_diff(tool_content: &str) -> String {
        let trimmed = tool_content.trim();
        if trimmed == "No changes to show" {
            return trimmed.to_string();
        }
        let diff_body = tool_content
            .strip_prefix("Git diff:\n")
            .unwrap_or(tool_content)
            .trim_end();
        format!("Git diff:\n```diff\n{}\n```", diff_body)
    }

    fn format_write_file(tool_content: &str) -> String {
        let mut path = None;
        let mut content = None;
        for line in tool_content.lines() {
            if let Some(value) = line.strip_prefix("Wrote file: ") {
                path = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("CONTENT: ") {
                content = Some(value.to_string());
            }
        }

        match (path, content) {
            (Some(path), Some(content)) => {
                let language = std::path::Path::new(&path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("txt");
                format!("Created `{}`:\n```{}\n{}\n```", path, language, content)
            }
            _ => format!("Tool result:\n{}", tool_content),
        }
    }

    fn format_run_command(tool_content: &str) -> String {
        let mut command = None;
        let mut in_output = false;
        let mut output_lines = Vec::new();

        for line in tool_content.lines() {
            if let Some(value) = line.strip_prefix("COMMAND: ") {
                command = Some(value.trim().to_string());
                continue;
            }
            if line.trim() == "OUTPUT:" {
                in_output = true;
                continue;
            }
            if in_output {
                output_lines.push(line);
            }
        }

        let command = command.unwrap_or_else(|| "command".to_string());
        let output = output_lines.join("\n").trim().to_string();
        if output == "Command execution cancelled by user" {
            return format!("Cancelled `{}`.", command);
        }
        if output.is_empty() {
            return format!("Ran `{}` successfully.", command);
        }

        format!("Ran `{}`.\n```\n{}\n```", command, output)
    }

    fn summarize_codebase_search_result(tool_content: &str) -> String {
        let mut bullets = Vec::new();
        let mut current_file = None;
        let mut current_snippet = None;
        let mut current_reasons: Option<String> = None;
        let mut intent = "general";

        for line in tool_content.lines().skip(1) {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("INTENT: ") {
                intent = value;
            } else if let Some(value) = trimmed.strip_prefix("FILE: ") {
                if let (Some(file), Some(snippet)) = (current_file.take(), current_snippet.take()) {
                    let reasons = current_reasons.take().unwrap_or_default();
                    let reason_suffix = if reasons.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", reasons)
                    };
                    bullets.push(format!("- `{}` → {}{}", file, snippet, reason_suffix));
                }
                current_file = Some(value.to_string());
                current_reasons = None;
                current_snippet = None;
            } else if let Some(value) = trimmed.strip_prefix("REASONS: ") {
                current_reasons = Some(value.to_string());
            } else if let Some(value) = trimmed.strip_prefix("- ") {
                if current_snippet.is_none() {
                    current_snippet = Some(value.to_string());
                }
            }
            if bullets.len() >= 4 {
                break;
            }
        }

        if bullets.len() < 4 {
            if let (Some(file), Some(snippet)) = (current_file.take(), current_snippet.take()) {
                let reasons = current_reasons.take().unwrap_or_default();
                let reason_suffix = if reasons.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", reasons)
                };
                bullets.push(format!("- `{}` → {}{}", file, snippet, reason_suffix));
            }
        }

        if bullets.is_empty() {
            return format!("Tool result:\n{}", tool_content);
        }

        if intent == "defined" {
            bullets.retain(|bullet| {
                let lower = bullet.to_ascii_lowercase();
                lower.contains(" enum ")
                    || lower.contains(" struct ")
                    || lower.contains(" trait ")
                    || lower.contains(" type ")
                    || lower.contains(" fn ")
            });
            if bullets.is_empty() {
                return format!("Tool result:\n{}", tool_content);
            }
            bullets.truncate(2);
        } else if intent == "calls" {
            bullets.retain(|bullet| !bullet.to_ascii_lowercase().contains(" fn "));
            if bullets.is_empty() {
                return format!("Tool result:\n{}", tool_content);
            }
            bullets.truncate(4);
        }

        let heading = match intent {
            "used" => "Top usage sites:",
            "calls" => "Top call sites:",
            "defined" => "Top definitions:",
            _ => "Top source matches:",
        };

        format!("{}\n{}", heading, bullets.join("\n"))
    }

    fn infer_codebase_search_intent_kind(
        user_msg: &str,
        pattern: &str,
    ) -> crate::tools::codebase_investigator::SearchIntentKind {
        let lower = user_msg.to_ascii_lowercase();
        if lower.contains("what calls")
            || lower.contains("who calls")
            || lower.contains(&format!("calls {}", pattern.to_ascii_lowercase()))
        {
            return crate::tools::codebase_investigator::SearchIntentKind::Calls;
        }
        if lower.contains("where is defined")
            || lower.contains("what file defines")
            || lower.contains("where is") && lower.contains(" defined ")
        {
            return crate::tools::codebase_investigator::SearchIntentKind::Defined;
        }
        if lower.contains("where is used")
            || lower.contains("used in")
            || lower.contains("where is") && lower.contains(" used ")
        {
            return crate::tools::codebase_investigator::SearchIntentKind::Used;
        }

        crate::tools::codebase_investigator::SearchIntentKind::General
    }

    fn summarize_codebase_overview(tool_content: &str) -> String {
        let mut lines = Vec::new();

        for line in tool_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("Workspace root: ") {
                lines.push(format!("Workspace root: `{}`", value));
            } else if let Some(value) = trimmed.strip_prefix("Workspace package: ") {
                lines.push(format!("Workspace package: `{}`", value));
            } else if let Some(value) = trimmed.strip_prefix("Workspace members: ") {
                lines.push(format!("Workspace members: {}", value));
            } else if let Some(value) = trimmed.strip_prefix("Crate roles: ") {
                lines.push(format!("Crate roles: {}", value));
            } else if let Some(value) = trimmed.strip_prefix("Top-level entries: ") {
                lines.push(format!("Top-level entries: {}", value));
            } else if let Some(value) = trimmed.strip_prefix("Likely hotspots: ") {
                lines.push(format!("Likely hotspots: {}", value));
            }
        }

        if lines.is_empty() {
            return format!("Tool result:\n{}", tool_content);
        }

        format!("Codebase overview:\n- {}", lines.join("\n- "))
    }

    fn model_backend_unavailable_reply() -> String {
        "Model backend unavailable. Start it with `ollama serve`, or ask a file/command-specific question.".to_string()
    }

    async fn try_handle_offline_shell_proxy(
        &mut self,
        user_msg: &str,
        session_id: &str,
    ) -> Result<Option<String>, HarperError> {
        if self.execution_strategy != ExecutionStrategy::Deterministic {
            return Ok(None);
        }
        let commands = plan_offline_shell_commands(user_msg)
            .into_iter()
            .map(|command| Self::normalize_run_command_candidate(&command))
            .collect::<Vec<_>>();
        if commands.is_empty() {
            return Ok(None);
        }
        if commands
            .iter()
            .any(|command| Self::is_followup_run_command_placeholder(command))
        {
            return Ok(None);
        }

        let audit_ctx = CommandAuditContext {
            conn: self.conn,
            session_id: Some(session_id),
            source: "offline_shell_nlu",
        };
        let mut sections = Vec::new();
        for command in commands {
            let bracket = format!("[RUN_COMMAND {}]", command);
            let output = crate::tools::shell::execute_command(
                &bracket,
                self.config,
                &self.exec_policy,
                None,
                Some(&audit_ctx),
                self.approver.clone(),
                self.runtime_events.clone(),
            )
            .await?;
            if output.trim().is_empty() {
                sections.push(format!("$ {}\n[no output]", command));
            } else {
                sections.push(format!("$ {}\n{}", command, output.trim_end()));
            }
        }
        self.notify_command_activity(session_id);
        Ok(Some(sections.join("\n\n")))
    }

    fn format_command_audit(
        &self,
        session_id: &str,
        params: &AuditParams,
    ) -> Result<String, HarperError> {
        let requested_limit = params.limit.unwrap_or(Self::AUDIT_LOG_LIMIT);
        let limit = requested_limit.clamp(1, 100);
        let fetch_limit = std::cmp::max(limit * 3, limit).min(300);

        let entries = self.fetch_command_logs(session_id, fetch_limit)?;

        let filtered: Vec<_> = entries
            .into_iter()
            .filter(|entry| params.matches(entry))
            .take(limit)
            .collect();

        let mut lines = Vec::new();
        let mut filters = Vec::new();
        if let Some(status) = &params.status {
            filters.push(format!("status={}", status));
        }
        if let Some(appr) = params.approval {
            filters.push(format!(
                "approval={}",
                match appr {
                    AuditApprovalFilter::Approved => "approved",
                    AuditApprovalFilter::Rejected => "rejected",
                    AuditApprovalFilter::Auto => "auto",
                }
            ));
        }
        if !filters.is_empty() {
            lines.push(format!(
                "Recent command executions (showing up to {}, filters: {})",
                limit,
                filters.join(", ")
            ));
        } else {
            lines.push(format!(
                "Recent command executions (showing up to {}):",
                limit
            ));
        }

        if filtered.is_empty() {
            lines.push("No commands have been executed yet.".to_string());
            return Ok(lines.join("\n"));
        }

        for (idx, entry) in filtered.iter().enumerate() {
            let approval_state = if entry.requires_approval {
                if entry.approved {
                    "approved"
                } else {
                    "rejected"
                }
            } else {
                "auto"
            };
            let exit = entry
                .exit_code
                .map(|code| format!("exit {}", code))
                .unwrap_or_else(|| "no exit code".to_string());
            let duration = entry
                .duration_ms
                .map(|ms| format!("{} ms", ms))
                .unwrap_or_else(|| "-".to_string());
            lines.push(format!(
                "{:>2}. {} [{} | {} | {}]\n    {}",
                idx + 1,
                entry.status,
                approval_state,
                exit,
                duration,
                entry.command
            ));
        }

        Ok(lines.join("\n"))
    }

    fn parse_execution_strategy(value: &str) -> Option<ExecutionStrategy> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ExecutionStrategy::Auto),
            "grounded" => Some(ExecutionStrategy::Grounded),
            "deterministic" => Some(ExecutionStrategy::Deterministic),
            "model" | "model-only" | "model_only" => Some(ExecutionStrategy::ModelOnly),
            _ => None,
        }
    }

    fn execution_strategy_name(strategy: ExecutionStrategy) -> &'static str {
        match strategy {
            ExecutionStrategy::Auto => "auto",
            ExecutionStrategy::Grounded => "grounded",
            ExecutionStrategy::Deterministic => "deterministic",
            ExecutionStrategy::ModelOnly => "model",
        }
    }

    fn format_task_mode(task_mode: TaskMode) -> &'static str {
        match task_mode {
            TaskMode::DirectDeterministic => "direct_deterministic",
            TaskMode::PlanThenAct => "plan_then_act",
            TaskMode::InspectThenExecute => "inspect_then_execute",
            TaskMode::ToolAssisted => "tool_assisted",
            TaskMode::RespondOnly => "respond_only",
        }
    }

    fn format_deterministic_intent(intent: &DeterministicIntent) -> String {
        match intent {
            DeterministicIntent::ListChangedFiles(filters) => format!(
                "list_changed_files(ext={:?},tracked_only={},since={:?})",
                filters.ext, filters.tracked_only, filters.since
            ),
            DeterministicIntent::GitStatus => "git_status".to_string(),
            DeterministicIntent::GitDiff => "git_diff".to_string(),
            DeterministicIntent::GitBranch => "git_branch".to_string(),
            DeterministicIntent::CurrentDirectory => "current_directory".to_string(),
            DeterministicIntent::RepoIdentity => "repo_identity".to_string(),
            DeterministicIntent::ReadFile(intent) => format!("read_file({})", intent.path),
            DeterministicIntent::WriteFile(intent) => format!("write_file({})", intent.path),
            DeterministicIntent::CodebaseOverview => "codebase_overview".to_string(),
            DeterministicIntent::CodebaseSearch(intent) => {
                format!("codebase_search({})", intent.pattern)
            }
            DeterministicIntent::RunCommand(intent) => format!("run_command({})", intent.command),
        }
    }

    fn should_prefer_deterministic_routing(strategy: ExecutionStrategy, user_msg: &str) -> bool {
        match strategy {
            ExecutionStrategy::Deterministic => true,
            ExecutionStrategy::Grounded => crate::agent::intent::route_intent(user_msg).is_some(),
            ExecutionStrategy::Auto | ExecutionStrategy::ModelOnly => false,
        }
    }

    fn decide_task_mode(strategy: ExecutionStrategy, user_msg: &str) -> TaskMode {
        let needs_plan = Self::request_needs_plan(user_msg);
        let needs_authoring = Self::request_needs_authoring_flow(user_msg);
        if needs_authoring && needs_plan {
            return TaskMode::PlanThenAct;
        }
        if needs_authoring {
            return TaskMode::InspectThenExecute;
        }
        if needs_plan {
            return TaskMode::PlanThenAct;
        }
        let is_routable = crate::agent::intent::route_intent(user_msg).is_some();
        if Self::should_prefer_deterministic_routing(strategy, user_msg) && is_routable {
            return TaskMode::DirectDeterministic;
        }
        if Self::request_requires_tool(user_msg).is_some() {
            return TaskMode::ToolAssisted;
        }
        TaskMode::RespondOnly
    }

    fn response_looks_like_generic_capability_refusal(response: &str) -> bool {
        let lower = response.to_ascii_lowercase();
        lower.contains("none of the provided tools")
            || lower.contains("i can't assist with that request")
            || lower.contains("there is no way to perform")
            || lower.contains("would be necessary to use a tool designed")
            || lower.contains("the provided tools")
    }

    fn fetch_command_logs(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<CommandLogEntry>, HarperError> {
        crate::memory::storage::load_command_logs_for_session(self.conn, session_id, limit)
    }

    /// Call LLM
    async fn call_llm(
        &mut self,
        client: &Client,
        history: &[Message],
    ) -> Result<String, HarperError> {
        // Check cache
        if let Some(cache) = &self.api_cache {
            let cache_key = ApiCacheKey::new(
                &format!("{}", self.config.provider),
                &self.config.model_name,
                history,
            );

            if let Some(cached_response) = cache.get(&cache_key) {
                return Ok(cached_response.clone());
            }
        }

        // Make API call
        let response = self
            .completer
            .complete(client, self.config, history)
            .await?;

        // Cache response
        if let Some(cache) = &mut self.api_cache {
            let cache_key = ApiCacheKey::new(
                &format!("{}", self.config.provider),
                &self.config.model_name,
                history,
            );
            cache.insert(cache_key, response.clone());
        }

        Ok(response)
    }

    /// Display response
    fn display_response(&self, response: &str) {
        println!(
            "{} {}
",
            "Assistant:".bold().green(),
            response.green()
        );
    }

    /// Add user message
    fn add_user_message(
        &self,
        history: &mut Vec<Message>,
        session_id: &str,
        content: &str,
    ) -> Result<(), HarperError> {
        // Ensure session exists
        self.save_session(session_id)?;
        history.push(Message {
            role: "user".to_string(),
            content: content.to_string(),
        });
        let user_message_count = history
            .iter()
            .filter(|message| message.role == "user")
            .count();
        if user_message_count == 1 {
            let title = Self::derive_session_title(content);
            let _ = crate::memory::storage::update_session_title(self.conn, session_id, &title);
        }
        crate::memory::storage::save_message(self.conn, session_id, "user", content)
    }

    /// Add assistant message
    fn add_assistant_message(
        &self,
        history: &mut Vec<Message>,
        session_id: &str,
        content: &str,
    ) -> Result<(), HarperError> {
        history.push(Message {
            role: "assistant".to_string(),
            content: content.to_string(),
        });
        crate::memory::storage::save_message(self.conn, session_id, "assistant", content)
    }

    /// Trim history
    fn trim_history(&self, history: &mut Vec<Message>) {
        const MAX_HISTORY: usize = 50;
        if history.len() > MAX_HISTORY {
            let to_remove_count = history.len() - MAX_HISTORY;
            history.drain(1..1 + to_remove_count);
        }
    }

    /// Save session
    fn save_session(&self, session_id: &str) -> Result<(), HarperError> {
        crate::memory::storage::save_session(self.conn, session_id)
    }

    fn derive_session_title(content: &str) -> String {
        let single_line = content
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(content)
            .trim();
        let normalized = single_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            return "New session".to_string();
        }
        const MAX_LEN: usize = 48;
        if normalized.chars().count() <= MAX_LEN {
            normalized
        } else {
            let truncated = normalized.chars().take(MAX_LEN).collect::<String>();
            format!("{}...", truncated.trim_end())
        }
    }

    /// Load custom prompt
    fn load_custom_prompt(&self, prompt_id: &str) -> Result<String, HarperError> {
        let home = dirs::home_dir()
            .ok_or_else(|| HarperError::Config("Home directory not found".to_string()))?;
        let prompt_path = home
            .join(".harper")
            .join("prompts")
            .join(format!("{}.md", prompt_id));
        std::fs::read_to_string(&prompt_path).map_err(|e| {
            HarperError::Config(format!(
                "Failed to load custom prompt {}: {}",
                prompt_path.display(),
                e
            ))
        })
    }

    /// Get project context
    fn get_project_context(&self) -> Result<String, HarperError> {
        let mut context = String::new();

        let current_dir = std::env::current_dir()
            .map_err(|e| HarperError::Command(format!("Failed to get current dir: {}", e)))?;

        let entries = std::fs::read_dir(&current_dir)
            .map_err(|e| HarperError::Command(format!("Failed to read dir: {}", e)))?;

        let mut files = Vec::new();
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                if !file_name.starts_with('.')
                    && file_name != "target"
                    && file_name != "node_modules"
                {
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_dir() {
                            files.push(format!("{}/", file_name));
                        } else {
                            files.push(file_name);
                        }
                    }
                }
            }
        }
        files.sort();

        context.push_str(&format!(
            "Current directory: {}
",
            current_dir.display()
        ));
        context.push_str(&format!(
            "Files in project root: {}
",
            files.join(", ")
        ));

        // Git status
        if let Ok(git_status) = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .output()
        {
            if git_status.status.success() {
                let status = String::from_utf8_lossy(&git_status.stdout);
                if !status.trim().is_empty() {
                    context.push_str(&format!(
                        "Git status:
{}",
                        status
                    ));
                } else {
                    context.push_str(
                        "Git status: clean
",
                    );
                }
            }
        }

        Ok(context)
    }
}

struct AtFileReference {
    path: String,
    consumed: usize,
}

fn parse_at_file_reference(input: &str) -> Option<AtFileReference> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;

    if first == '"' || first == '\'' {
        let mut path = String::new();
        let mut escaped = false;
        for (index, ch) in chars {
            if escaped {
                path.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == first {
                return Some(AtFileReference {
                    path,
                    consumed: index + ch.len_utf8(),
                });
            }
            path.push(ch);
        }
        return None;
    }

    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    if end == 0 {
        return None;
    }

    Some(AtFileReference {
        path: input[..end].to_string(),
        consumed: end,
    })
}

fn is_supported_image_reference(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

fn parse_standalone_image_reference(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let unquoted = if let Some(value) = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        value
    } else if let Some(value) = trimmed
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        value
    } else {
        trimmed
    };

    if !is_supported_image_reference(unquoted) {
        return None;
    }

    let path = std::path::Path::new(unquoted);
    path.is_file().then(|| unquoted.to_string())
}

fn parse_audit_params(args: Option<&str>) -> HarperResult<AuditParams> {
    let mut params = AuditParams::default();

    let Some(arg_text) = args.filter(|a| !a.is_empty()) else {
        return Ok(params);
    };

    for token in arg_text.split_whitespace() {
        if token.chars().all(|c| c.is_ascii_digit()) {
            if params.limit.is_some() {
                return Err(HarperError::Api(
                    "Audit limit specified multiple times.".to_string(),
                ));
            }
            let parsed = token.parse::<usize>().map_err(|_| {
                HarperError::Api(format!(
                    "Invalid limit '{}'. Use a positive integer.",
                    token
                ))
            })?;
            if parsed == 0 {
                return Err(HarperError::Api(
                    "Audit limit must be greater than zero.".to_string(),
                ));
            }
            params.limit = Some(parsed);
            continue;
        }

        if let Some(value) = token.strip_prefix("status=") {
            if params.status.is_some() {
                return Err(HarperError::Api(
                    "Status filter specified multiple times.".to_string(),
                ));
            }
            if value.is_empty() {
                return Err(HarperError::Api(
                    "Status filter cannot be empty.".to_string(),
                ));
            }
            params.status = Some(value.to_lowercase());
            continue;
        }

        if let Some(value) = token.strip_prefix("approval=") {
            if params.approval.is_some() {
                return Err(HarperError::Api(
                    "Approval filter specified multiple times.".to_string(),
                ));
            }
            params.approval = Some(match value.to_lowercase().as_str() {
                "approved" => AuditApprovalFilter::Approved,
                "rejected" => AuditApprovalFilter::Rejected,
                "auto" => AuditApprovalFilter::Auto,
                _ => {
                    return Err(HarperError::Api(
                        "Approval filter must be one of: approved, rejected, auto.".to_string(),
                    ));
                }
            });
            continue;
        }

        let lower = token.to_lowercase();
        if matches!(
            lower.as_str(),
            "failed" | "succeeded" | "error" | "cancelled" | "blocked"
        ) {
            if params.status.is_some() {
                return Err(HarperError::Api(
                    "Status filter specified multiple times.".to_string(),
                ));
            }
            params.status = Some(lower);
            continue;
        }

        if matches!(lower.as_str(), "approved" | "rejected" | "auto") {
            if params.approval.is_some() {
                return Err(HarperError::Api(
                    "Approval filter specified multiple times.".to_string(),
                ));
            }
            params.approval = Some(match lower.as_str() {
                "approved" => AuditApprovalFilter::Approved,
                "rejected" => AuditApprovalFilter::Rejected,
                "auto" => AuditApprovalFilter::Auto,
                // Defensive: the outer matches! narrows to the three values
                // above, so this arm is unreachable through parse_audit_params
                // today. It exists so a future filter-list edit degrades to
                // an error instead of a panic.
                other => {
                    return Err(HarperError::Api(format!(
                        "Unknown approval filter '{}'. Use approved, rejected, or auto.",
                        other
                    )));
                }
            });
            continue;
        }

        return Err(HarperError::Api(format!(
            "Unknown audit argument '{}'. Use /audit [limit] [status=...] [approval=approved|rejected|auto].",
            token
        )));
    }

    Ok(params)
}

impl AuditParams {
    fn matches(&self, entry: &CommandLogEntry) -> bool {
        let status_match = self
            .status
            .as_ref()
            .map(|s| entry.status.eq_ignore_ascii_case(s))
            .unwrap_or(true);

        let approval_match = match self.approval {
            Some(AuditApprovalFilter::Approved) => entry.requires_approval && entry.approved,
            Some(AuditApprovalFilter::Rejected) => entry.requires_approval && !entry.approved,
            Some(AuditApprovalFilter::Auto) => !entry.requires_approval,
            None => true,
        };

        status_match && approval_match
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent::{DeterministicIntent, route_intent};
    use crate::core::{ApiConfig, ApiProvider};
    use crate::memory::storage::CommandLogRecord;

    #[derive(Debug)]
    struct TurnDebug {
        task_mode: TaskMode,
        deterministic_intent: Option<DeterministicIntent>,
        normalized_command: Option<String>,
        clarification: Option<String>,
    }

    fn debug_turn(strategy: ExecutionStrategy, history: &[Message], user_msg: &str) -> TurnDebug {
        let deterministic_intent = route_intent(user_msg).or_else(|| {
            ChatService::infer_followup_write_file_intent(history, user_msg)
                .map(DeterministicIntent::WriteFile)
                .or_else(|| {
                    ChatService::infer_followup_run_command_intent(history, user_msg)
                        .map(DeterministicIntent::RunCommand)
                })
        });
        let task_mode = if route_intent(user_msg).is_none() && deterministic_intent.is_some() {
            TaskMode::DirectDeterministic
        } else {
            ChatService::decide_task_mode(strategy, user_msg)
        };

        let normalized_command = deterministic_intent
            .as_ref()
            .and_then(|intent| match intent {
                DeterministicIntent::RunCommand(intent) => Some(
                    ChatService::normalize_run_command_candidate(&intent.command),
                ),
                _ => None,
            });

        let clarification = normalized_command.as_ref().and_then(|command| {
            let tool_call = format!("[RUN_COMMAND {}]", command);
            ChatService::clarification_for_underspecified_tool_call(user_msg, &tool_call)
        });

        TurnDebug {
            task_mode,
            deterministic_intent,
            normalized_command,
            clarification,
        }
    }

    fn test_config() -> ApiConfig {
        ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        }
    }

    #[test]
    fn parse_audit_no_args() {
        let params = parse_audit_params(None).expect("parse");
        assert!(params.limit.is_none());
        assert!(params.status.is_none());
    }

    #[test]
    fn parse_audit_with_limit_and_filters() {
        let params = parse_audit_params(Some("25 status=failed approval=rejected")).expect("parse");
        assert_eq!(params.limit, Some(25));
        assert_eq!(params.status.as_deref(), Some("failed"));
        assert_eq!(params.approval, Some(AuditApprovalFilter::Rejected));
    }

    #[test]
    fn parse_audit_errors_on_unknown_token() {
        let err = parse_audit_params(Some("foo=bar")).expect_err("should fail");
        assert!(err.to_string().contains("Unknown audit argument"));
    }

    #[test]
    fn audit_params_matches_filters() {
        let mut entry = CommandLogEntry {
            command: "echo hi".to_string(),
            source: "test".to_string(),
            requires_approval: true,
            approved: true,
            status: "succeeded".to_string(),
            exit_code: Some(0),
            duration_ms: Some(10),
            created_at: "2026-02-12".to_string(),
        };

        let params = AuditParams {
            limit: Some(5),
            status: Some("succeeded".to_string()),
            approval: Some(AuditApprovalFilter::Approved),
        };
        assert!(params.matches(&entry));

        entry.status = "failed".to_string();
        assert!(!params.matches(&entry));
    }

    #[test]
    fn parse_audit_shorthand_tokens() {
        let params = parse_audit_params(Some("failed approved")).expect("parse");
        assert_eq!(params.status.as_deref(), Some("failed"));
        assert_eq!(params.approval, Some(AuditApprovalFilter::Approved));
    }

    #[test]
    fn parse_audit_duplicate_status_errors() {
        let err = parse_audit_params(Some("status=failed succeeded")).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("Status filter specified multiple times")
        );
    }

    #[test]
    fn request_needs_plan_for_multi_step_work() {
        assert!(ChatService::request_needs_plan(
            "fix the HTTP tool path and then wire the UI refresh too"
        ));
        assert!(ChatService::request_needs_plan(
            "implement storage, update the prompt, and add tests"
        ));
        assert!(!ChatService::request_needs_plan("run git status"));
        assert!(!ChatService::request_needs_plan("fix typo"));
    }

    #[test]
    fn plan_prompt_mentions_blocked_step_when_plan_is_blocked() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "blocked-plan-session",
            &crate::core::plan::PlanState {
                explanation: Some("blocked".to_string()),
                items: vec![crate::core::plan::PlanItem {
                    step: "Fix failing migration".to_string(),
                    status: crate::core::plan::PlanStepStatus::Blocked,
                    job_id: None,
                }],
                runtime: None,
                updated_at: None,
            },
        )
        .expect("save plan");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let chat = ChatService::new(
            &conn,
            &config,
            None,
            None,
            Some("blocked-plan-session".to_string()),
            HashMap::new(),
            exec_policy,
        );
        let history = vec![Message {
            role: "user".to_string(),
            content: "fix the migration and then update the docs".to_string(),
        }];

        let prompt = chat
            .plan_prompt_for_request(&history, "blocked-plan-session")
            .expect("plan prompt")
            .expect("prompt present");

        assert!(prompt.contains("blocked at 'Fix failing migration'"));
        assert!(prompt.contains("call update_plan"));
    }

    #[test]
    fn plan_prompt_mentions_retry_limit_when_followup_requires_replan() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::save_plan_state(
            &conn,
            "retry-plan-session",
            &crate::core::plan::PlanState {
                explanation: Some("retry".to_string()),
                items: vec![crate::core::plan::PlanItem {
                    step: "Patch handler".to_string(),
                    status: crate::core::plan::PlanStepStatus::Blocked,
                    job_id: None,
                }],
                runtime: Some(crate::core::plan::PlanRuntime {
                    followup: Some(crate::core::plan::PlanFollowup::RetryOrReplan {
                        step: "Patch handler".to_string(),
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
        let chat = ChatService::new(
            &conn,
            &config,
            None,
            None,
            Some("retry-plan-session".to_string()),
            HashMap::new(),
            exec_policy,
        );
        let history = vec![Message {
            role: "user".to_string(),
            content: "fix the handler and then rerun the tests".to_string(),
        }];

        let prompt = chat
            .plan_prompt_for_request(&history, "retry-plan-session")
            .expect("plan prompt")
            .expect("prompt present");

        assert!(prompt.contains("already failed 2 times"));
        assert!(prompt.contains("Do not keep retrying blindly"));
    }

    #[test]
    fn forced_tool_retry_targets_prose_when_tool_is_required() {
        let user_msg = "read lib/harper-core/src/agent/chat.rs";
        let prose = "Sorry, I cannot inspect files directly from here.";
        assert_eq!(
            ChatService::forced_tool_retry_target(user_msg, prose, false),
            Some("read_file")
        );
    }

    #[test]
    fn forced_tool_retry_skips_real_tool_calls() {
        let user_msg = "read lib/harper-core/src/agent/chat.rs";
        let tool_call =
            r#"{"tool":"read_file","args":{"path":"lib/harper-core/src/agent/chat.rs"}}"#;
        assert_eq!(
            ChatService::forced_tool_retry_target(user_msg, tool_call, false),
            None
        );
    }

    #[test]
    fn forced_tool_retry_targets_codebase_queries() {
        let user_msg = "Find where retry metadata is rendered in this repo.";
        let prose = "You can grep for retry and metadata in the repository.";
        assert_eq!(
            ChatService::forced_tool_retry_target(user_msg, prose, false),
            Some("codebase_investigator")
        );
    }

    #[test]
    fn forced_tool_retry_runs_only_once() {
        let user_msg = "run cargo test";
        let prose = "I would run tests to verify the change.";
        assert_eq!(
            ChatService::forced_tool_retry_target(user_msg, prose, true),
            None
        );
    }

    #[test]
    fn compact_deterministic_fallback_summarizes_codebase_search() {
        let tool_content = "Top matches for 'retry metadata':\nFILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1036\nROLE: ui_widget_rendering\nREASONS: role=ui_widget_rendering, contains_all_terms, widget_render_path\nSNIPPETS:\n- 1036: \" Plan (Ctrl+S • C complete • I in-progress • B blocked • R retry • U replan • K ack • X clear) \"\nFILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1229\nROLE: ui_widget_rendering\nREASONS: role=ui_widget_rendering, contains_all_terms, widget_render_path\nSNIPPETS:\n- 1229: PlanFollowup::RetryOrReplan {\nFILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1232\nROLE: ui_widget_rendering\nREASONS: role=ui_widget_rendering, contains_all_terms, widget_render_path\nSNIPPETS:\n- 1232: retry_count,";
        let summary = ChatService::compact_deterministic_fallback("codebase_search", tool_content);

        assert!(summary.starts_with("Top source matches:\n"));
        assert!(summary.contains("`./lib/harper-ui/src/interfaces/ui/widgets.rs:1036`"));
        assert!(summary.contains("PlanFollowup::RetryOrReplan"));
    }

    #[test]
    fn compact_deterministic_fallback_uses_intent_specific_codebase_heading() {
        let tool_content = "CODEBASE_SEARCH\nQUERY: update_plan\nFOCUS: general\nINTENT: calls\nTOP_MATCHES:\nFILE: ./lib/harper-core/src/tools/mod.rs\nROLE: tooling\nREASONS: contains_all_terms\nSNIPPETS:\n- 513: let plan_result = plan::update_plan(self.conn, session_id, args)?;";
        let summary = ChatService::compact_deterministic_fallback("codebase_search", tool_content);

        assert!(summary.starts_with("Top call sites:\n"));
        assert!(summary.contains("plan::update_plan"));
    }

    #[test]
    fn compact_deterministic_fallback_summarizes_codebase_overview() {
        let tool_content = "Workspace root: /Users/niladri/harper\nWorkspace package: harper-workspace\nWorkspace members: lib/harper-core, lib/harper-ui\nCrate roles: harper-core: runtime, tools, storage, agent logic; harper-ui: TUI state, events, widgets\nTop-level entries: AGENTS.md, Cargo.toml, lib\nLikely hotspots: lib/harper-core/src/agent/chat.rs, lib/harper-ui/src/interfaces/ui/widgets.rs";
        let summary =
            ChatService::compact_deterministic_fallback("codebase_overview", tool_content);

        assert!(summary.starts_with("Codebase overview:\n- "));
        assert!(summary.contains("Workspace package: `harper-workspace`"));
        assert!(summary.contains("Likely hotspots:"));
    }

    #[test]
    fn compact_deterministic_fallback_formats_git_diff_as_fenced_block() {
        let tool_content = "Git diff:\ndiff --git a/file.rs b/file.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let summary = ChatService::compact_deterministic_fallback("git_diff", tool_content);

        assert!(summary.starts_with("Git diff:\n```diff\n"));
        assert!(summary.contains("+new"));
        assert!(summary.ends_with("\n```"));
    }

    #[test]
    fn compact_deterministic_fallback_formats_write_file_as_fenced_block() {
        let tool_content = "Wrote file: hello-world.txt\nCONTENT: Hello World";
        let summary = ChatService::compact_deterministic_fallback("write_file", tool_content);

        assert!(summary.starts_with("Created `hello-world.txt`:\n```txt\n"));
        assert!(summary.contains("Hello World"));
        assert!(summary.ends_with("\n```"));
    }

    #[test]
    fn deterministic_summary_is_low_value_for_raw_codebase_echo() {
        let tool_content = "Top matches for 'retry metadata':\nFILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1036\nROLE: ui_widget_rendering\nREASONS: role=ui_widget_rendering, contains_all_terms\nSNIPPETS:\n- 1036: one\nFILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1229\nROLE: ui_widget_rendering\nREASONS: role=ui_widget_rendering, contains_all_terms\nSNIPPETS:\n- 1229: two\nFILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1232\nROLE: ui_widget_rendering\nREASONS: role=ui_widget_rendering, contains_all_terms\nSNIPPETS:\n- 1232: three";
        let response = "Top matches for 'retry metadata': FILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1036 ROLE: ui_widget_rendering SNIPPETS: 1036: one FILE: ./lib/harper-ui/src/interfaces/ui/widgets.rs:1229";
        assert!(ChatService::deterministic_summary_is_low_value(
            "codebase_search",
            response,
            tool_content
        ));
    }

    #[test]
    fn deterministic_strategy_does_not_model_summarize_codebase_search() {
        assert!(!ChatService::should_model_summarize_deterministic_result(
            ExecutionStrategy::Deterministic,
            "codebase_search"
        ));
        assert!(!ChatService::should_model_summarize_deterministic_result(
            ExecutionStrategy::Deterministic,
            "codebase_overview"
        ));
        assert!(!ChatService::should_model_summarize_deterministic_result(
            ExecutionStrategy::Deterministic,
            "read_file"
        ));
    }

    #[test]
    fn grounded_strategy_can_still_model_summarize_codebase_search() {
        assert!(ChatService::should_model_summarize_deterministic_result(
            ExecutionStrategy::Grounded,
            "codebase_search"
        ));
    }

    #[test]
    fn finalize_assistant_response_keeps_non_empty_text() {
        let result = ChatService::finalize_assistant_response("Plan updated.", None);
        assert_eq!(result, "Plan updated.");
    }

    #[test]
    fn finalize_assistant_response_falls_back_to_tool_output() {
        let result =
            ChatService::finalize_assistant_response("   \n", Some("Plan updated: 3 steps."));
        assert_eq!(result, "Tool result:\nPlan updated: 3 steps.");
    }

    #[test]
    fn finalize_assistant_response_uses_explicit_empty_message_when_no_tool_output() {
        let result = ChatService::finalize_assistant_response("   \n", None);
        assert_eq!(result, "The model returned an empty response.");
    }

    #[test]
    fn derive_session_title_uses_first_meaningful_line() {
        let title = ChatService::derive_session_title(
            "\n\nfix the tui spinner visibility\nand session names",
        );
        assert_eq!(title, "fix the tui spinner visibility");
    }

    #[test]
    fn derive_session_title_truncates_long_input() {
        let title = ChatService::derive_session_title(
            "this is a very long request title that should be shortened for the session list display",
        );
        assert!(title.ends_with("..."));
        assert!(title.len() <= 51);
    }

    #[test]
    fn infer_followup_write_file_intent_uses_previous_assistant_filename_and_code() {
        let history = vec![
            Message {
                role: "user".to_string(),
                content: "hey can you create a python hello joy file".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "Sure. Save this content into a file named `hello_joy.py`: `print(\"Hello Joy!\")` and run it with `python3 hello_joy.py`.".to_string(),
            },
        ];

        let intent =
            ChatService::infer_followup_write_file_intent(&history, "please create the file")
                .expect("follow-up write intent");

        assert_eq!(intent.path, "hello_joy.py");
        assert_eq!(intent.content, "print(\"Hello Joy!\")");
    }

    #[test]
    fn infer_followup_write_file_intent_sanitizes_absolute_filename() {
        let history = vec![
            Message {
                role: "user".to_string(),
                content: "create a new file explaining ai in markdown".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "Save this as `/home/user/ai.md`\n\n# Introduction to Artificial Intelligence\n\nAI is a field of computer science.".to_string(),
            },
        ];

        let intent =
            ChatService::infer_followup_write_file_intent(&history, "can you create this file")
                .expect("follow-up write intent");

        assert_eq!(intent.path, "ai.md");
        assert!(
            intent
                .content
                .contains("# Introduction to Artificial Intelligence")
        );
    }

    #[test]
    fn infer_followup_write_file_intent_extracts_markdown_body() {
        let history = vec![
            Message {
                role: "user".to_string(),
                content: "create a new file explaining ai in markdown".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "Sure, use `ai.md`\n\n# Introduction to Artificial Intelligence\n\n## What Is AI?\n\nArtificial intelligence is ...".to_string(),
            },
        ];

        let intent =
            ChatService::infer_followup_write_file_intent(&history, "please create the file")
                .expect("follow-up write intent");

        assert_eq!(intent.path, "ai.md");
        assert!(
            intent
                .content
                .starts_with("# Introduction to Artificial Intelligence")
        );
        assert!(intent.content.contains("## What Is AI?"));
    }

    #[test]
    fn infer_followup_run_command_intent_uses_previous_assistant_command() {
        let history = vec![
            Message {
                role: "user".to_string(),
                content: "can you create a sample file name hello and put 1 to 10 in numbers"
                    .to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "Save this script as `create_hello.sh`, make it executable with `chmod +x create_hello.sh`, and run it with `./create_hello.sh`.".to_string(),
            },
        ];

        let intent = ChatService::infer_followup_run_command_intent(&history, "run that command")
            .expect("follow-up run command intent");

        assert_eq!(intent.command, "./create_hello.sh");
    }

    #[test]
    fn infer_followup_run_command_intent_prefers_previous_user_git_intent() {
        let history = vec![
            Message {
                role: "user".to_string(),
                content: "run the git status".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content:
                    "Git working directory has 11 modified, 1 untracked. Notable files: app.rs."
                        .to_string(),
            },
        ];

        let intent = ChatService::infer_followup_run_command_intent(&history, "run that command")
            .expect("follow-up run command intent");

        assert_eq!(intent.command, "git status");
    }

    #[test]
    fn infer_followup_run_command_intent_returns_none_without_previous_command() {
        let history = vec![Message {
            role: "assistant".to_string(),
            content: "I created the file directly.".to_string(),
        }];

        let intent = ChatService::infer_followup_run_command_intent(&history, "run that command");
        assert!(intent.is_none());
    }

    #[test]
    fn infer_followup_run_command_prefers_session_command_log_over_assistant_summary() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");
        crate::memory::storage::insert_command_log(
            &conn,
            &CommandLogRecord::new(
                Some("followup-session"),
                "git status",
                "test",
                true,
                false,
                "rejected",
                None,
                None,
                None,
                None,
                Some("Command execution cancelled by user".to_string()),
            ),
        )
        .expect("insert command log");

        let config = test_config();
        let exec_policy = ExecPolicyConfig::default();
        let chat = ChatService::new(
            &conn,
            &config,
            None,
            None,
            Some("followup-session".to_string()),
            HashMap::new(),
            exec_policy,
        );
        let history = vec![Message {
            role: "assistant".to_string(),
            content: "Git working directory has 11 modified, 1 untracked. Notable files: app.rs."
                .to_string(),
        }];

        let intent = chat
            .infer_followup_run_command_intent_for_session(&history, "run that", "followup-session")
            .expect("follow-up run command intent");

        assert_eq!(intent.command, "git status");
    }

    #[test]
    fn ambiguous_short_prompt_blocks_placeholder_read_file_tool_call() {
        let clarification = ChatService::clarification_for_underspecified_tool_call(
            "as",
            r#"[READ_FILE example.txt]"#,
        );
        assert!(clarification.is_some());
        assert!(
            clarification
                .unwrap()
                .contains("too ambiguous to act on safely")
        );
    }

    #[test]
    fn non_placeholder_read_file_is_not_blocked_for_specific_prompt() {
        let clarification = ChatService::clarification_for_underspecified_tool_call(
            "read src/main.rs",
            r#"[READ_FILE src/main.rs]"#,
        );
        assert!(clarification.is_none());
    }

    #[test]
    fn normalize_tool_call_for_execution_strips_articles_from_git_status() {
        let normalized =
            ChatService::normalize_tool_call_for_execution(r#"[RUN_COMMAND the git status]"#);
        assert_eq!(normalized, "[RUN_COMMAND git status]");
    }

    #[test]
    fn ambiguous_run_that_placeholder_gets_clarification() {
        let clarification = ChatService::clarification_for_underspecified_tool_call(
            "run that",
            r#"[RUN_COMMAND that]"#,
        );
        assert!(clarification.is_some());
        assert!(clarification.unwrap().contains("which previous command"));
    }

    #[test]
    fn normalize_run_command_candidate_strips_articles_for_deterministic_path() {
        let normalized = ChatService::normalize_run_command_candidate("the git status");
        assert_eq!(normalized, "git status");
    }

    #[test]
    fn normalize_run_command_candidate_preserves_case_for_generic_commands() {
        let normalized = ChatService::normalize_run_command_candidate("cat README.md");
        assert_eq!(normalized, "cat README.md");
    }

    #[test]
    fn extract_followup_run_command_parses_shell_echoed_command_line() {
        let command = ChatService::extract_followup_run_command(
            "$ the git status sh: the: command not found",
        )
        .expect("shell echoed command");
        assert_eq!(command, "git status");
    }

    #[test]
    fn request_needs_authoring_flow_for_open_ended_repo_change() {
        assert!(ChatService::request_needs_authoring_flow(
            "refactor the planner flow in this repo"
        ));
    }

    #[test]
    fn request_does_not_need_authoring_flow_for_direct_write_intent() {
        assert!(!ChatService::request_needs_authoring_flow(
            "create hello.rs with fn main() { println!(\"Hi\"); }"
        ));
    }

    #[test]
    fn request_does_not_need_authoring_flow_for_direct_edit_intent() {
        assert!(!ChatService::request_needs_authoring_flow(
            "edit lib/harper-core/src/lib.rs with a new export"
        ));
    }

    #[test]
    fn request_requires_tool_for_open_ended_authoring_flow() {
        assert_eq!(
            ChatService::request_requires_tool("refactor the planner flow in this repo"),
            Some("codebase_investigator")
        );
    }

    #[test]
    fn authoring_tool_retry_prompt_requires_plan_before_first_edit() {
        let mut candidates = HashSet::new();
        candidates.insert(PathBuf::from("lib/harper-ui/src/interfaces/ui/widgets.rs"));

        let prompt = ChatService::authoring_tool_retry_prompt(
            "refactor the planner flow in this repo and then update the tui rendering too",
            r#"{"tool":"search_replace","args":{"path":"lib/harper-ui/src/interfaces/ui/widgets.rs","search":"old","replace":"new"}}"#,
            Some(&candidates),
            false,
            false,
            false,
            &HashSet::new(),
        )
        .expect("should require plan");

        assert!(prompt.contains("call update_plan"));
        assert!(prompt.contains("Do not edit any file yet"));
    }

    #[test]
    fn authoring_tool_retry_prompt_requires_inspection_before_edit() {
        let mut candidates = HashSet::new();
        candidates.insert(PathBuf::from("lib/harper-ui/src/interfaces/ui/widgets.rs"));

        let prompt = ChatService::authoring_tool_retry_prompt(
            "change the retry rendering behavior in the tui",
            r#"{"tool":"search_replace","args":{"path":"lib/harper-ui/src/interfaces/ui/widgets.rs","search":"old","replace":"new"}}"#,
            Some(&candidates),
            false,
            true,
            true,
            &HashSet::new(),
        )
        .expect("should require inspection");

        assert!(prompt.contains("requires inspection before the first write/search_replace"));
        assert!(prompt.contains("codebase_investigator or read_file"));
    }

    #[test]
    fn authoring_tool_retry_prompt_rejects_edit_outside_candidates() {
        let mut candidates = HashSet::new();
        candidates.insert(PathBuf::from("lib/harper-ui/src/interfaces/ui/widgets.rs"));

        let prompt = ChatService::authoring_tool_retry_prompt(
            "change the retry rendering behavior in the tui",
            r#"{"tool":"search_replace","args":{"path":"/Users/username/Documents/project/source/file.txt","search":"old","replace":"new"}}"#,
            Some(&candidates),
            true,
            true,
            true,
            &HashSet::new(),
        )
        .expect("should reject non-candidate edit");

        assert!(prompt.contains("must target files from the grounded candidate set"));
        assert!(prompt.contains("widgets.rs"));
    }

    #[test]
    fn parses_execution_strategy_values() {
        assert_eq!(
            ChatService::parse_execution_strategy("auto"),
            Some(ExecutionStrategy::Auto)
        );
        assert_eq!(
            ChatService::parse_execution_strategy("grounded"),
            Some(ExecutionStrategy::Grounded)
        );
        assert_eq!(
            ChatService::parse_execution_strategy("deterministic"),
            Some(ExecutionStrategy::Deterministic)
        );
        assert_eq!(
            ChatService::parse_execution_strategy("model"),
            Some(ExecutionStrategy::ModelOnly)
        );
        assert_eq!(ChatService::parse_execution_strategy("bogus"), None);
    }

    #[test]
    fn grounded_prefers_deterministic_routing_for_routable_repo_intents() {
        assert!(ChatService::should_prefer_deterministic_routing(
            ExecutionStrategy::Grounded,
            "read lib/harper-core/src/agent/chat.rs"
        ));
        assert!(ChatService::should_prefer_deterministic_routing(
            ExecutionStrategy::Grounded,
            "show git diff"
        ));
    }

    #[test]
    fn grounded_does_not_force_deterministic_routing_for_open_ended_requests() {
        assert!(!ChatService::should_prefer_deterministic_routing(
            ExecutionStrategy::Grounded,
            "explain whether the current retry architecture is a good long-term design"
        ));
    }

    #[test]
    fn auto_does_not_prefer_deterministic_routing_up_front() {
        assert!(!ChatService::should_prefer_deterministic_routing(
            ExecutionStrategy::Auto,
            "read lib/harper-core/src/agent/chat.rs"
        ));
    }

    #[test]
    fn decide_task_mode_prefers_direct_deterministic_for_grounded_routable_intent() {
        assert_eq!(
            ChatService::decide_task_mode(
                ExecutionStrategy::Grounded,
                "where is execution strategy used in this repo"
            ),
            TaskMode::DirectDeterministic
        );
    }

    #[test]
    fn decide_task_mode_uses_plan_then_act_for_multi_step_authoring() {
        assert_eq!(
            ChatService::decide_task_mode(
                ExecutionStrategy::Grounded,
                "refactor the planner flow in this repo and then update the tui followup rendering"
            ),
            TaskMode::PlanThenAct
        );
    }

    #[test]
    fn decide_task_mode_uses_respond_only_for_plain_question() {
        assert_eq!(
            ChatService::decide_task_mode(
                ExecutionStrategy::Auto,
                "what is the difference between grounded and deterministic"
            ),
            TaskMode::RespondOnly
        );
    }

    #[test]
    fn headless_turn_debug_normalizes_run_command_in_deterministic_mode() {
        let debug = debug_turn(ExecutionStrategy::Deterministic, &[], "run the git status");

        assert_eq!(debug.task_mode, TaskMode::DirectDeterministic);
        assert!(matches!(
            debug.deterministic_intent,
            Some(DeterministicIntent::GitStatus)
        ));
        assert!(debug.normalized_command.is_none());
        assert!(debug.clarification.is_none());
    }

    #[test]
    fn headless_turn_debug_resolves_followup_run_command() {
        let history = vec![Message {
            role: "assistant".to_string(),
            content: "Ran `git status`.\n```\nOn branch main\n```".to_string(),
        }];

        let debug = debug_turn(ExecutionStrategy::Deterministic, &history, "run that");

        assert_eq!(debug.task_mode, TaskMode::DirectDeterministic);
        assert!(matches!(
            debug.deterministic_intent,
            Some(DeterministicIntent::RunCommand(_))
        ));
        assert_eq!(debug.normalized_command.as_deref(), Some("git status"));
        assert!(debug.clarification.is_none());
    }

    #[test]
    fn headless_turn_debug_clarifies_unresolved_followup_run_command() {
        let debug = debug_turn(ExecutionStrategy::Deterministic, &[], "run that");

        assert_eq!(debug.task_mode, TaskMode::ToolAssisted);
        assert!(debug.deterministic_intent.is_none());
        assert!(debug.normalized_command.is_none());
        assert!(debug.clarification.is_none());
    }

    #[test]
    fn headless_turn_debug_grounded_routable_query_prefers_direct_deterministic() {
        let debug = debug_turn(
            ExecutionStrategy::Grounded,
            &[],
            "where is execution strategy used in this repo",
        );

        assert_eq!(debug.task_mode, TaskMode::DirectDeterministic);
        assert!(matches!(
            debug.deterministic_intent,
            Some(DeterministicIntent::CodebaseSearch(_))
        ));
    }

    #[test]
    fn headless_turn_debug_auto_greeting_is_respond_only() {
        let debug = debug_turn(ExecutionStrategy::Auto, &[], "hi");
        assert_eq!(debug.task_mode, TaskMode::RespondOnly);
        assert!(debug.deterministic_intent.is_none());
    }
}
