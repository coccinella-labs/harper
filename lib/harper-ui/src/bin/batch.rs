// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use async_trait::async_trait;
use harper_core::core::io_traits::{DenyApproval, RuntimeEventSink, StdinApproval};
use harper_core::runtime::config::HarperConfig;
use harper_core::{
    ApiConfig, ConfigShellContext, HarperError, Message, NativeShellContext, NativeShellOutcome,
    PlanState, ResolvedAgents,
    agent::chat::{ChatService, ChatTurnDebugSummary},
    create_connection, execute_native_shell_command_with_context, init_db,
    parse_native_shell_command, resolve_session_target,
};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::io::{self, IsTerminal};
use std::sync::{Arc, Mutex};
use turul_mcp_client::McpClient;

#[derive(Debug, Default, Clone, Serialize)]
struct TurnDebugOutput {
    activity: Vec<String>,
    command: Option<String>,
    command_output: Option<String>,
    command_error: bool,
    command_done: bool,
}

#[derive(Default)]
struct BatchRuntimeEvents {
    per_session: Mutex<HashMap<String, TurnDebugOutput>>,
}

impl BatchRuntimeEvents {
    fn take_turn(&self, session_id: &str) -> TurnDebugOutput {
        self.per_session
            .lock()
            .expect("batch runtime events lock")
            .remove(session_id)
            .unwrap_or_default()
    }
}

#[async_trait]
impl RuntimeEventSink for BatchRuntimeEvents {
    async fn plan_updated(
        &self,
        _session_id: &str,
        _plan: Option<PlanState>,
    ) -> Result<(), HarperError> {
        Ok(())
    }

    async fn agents_updated(
        &self,
        _session_id: &str,
        _agents: Option<ResolvedAgents>,
    ) -> Result<(), HarperError> {
        Ok(())
    }

    async fn activity_updated(
        &self,
        session_id: &str,
        status: Option<String>,
    ) -> Result<(), HarperError> {
        if let Some(status) = status {
            let mut sessions = self.per_session.lock().expect("batch runtime events lock");
            let entry = sessions.entry(session_id.to_string()).or_default();
            if entry.activity.last() != Some(&status) {
                entry.activity.push(status);
            }
        }
        Ok(())
    }

    async fn command_output_updated(
        &self,
        session_id: &str,
        command: String,
        chunk: String,
        is_error: bool,
        done: bool,
    ) -> Result<(), HarperError> {
        let mut sessions = self.per_session.lock().expect("batch runtime events lock");
        let entry = sessions.entry(session_id.to_string()).or_default();
        entry.command = Some(command);
        entry.command_error = is_error;
        entry.command_done = done;
        match entry.command_output.as_mut() {
            Some(output) => output.push_str(&chunk),
            None => entry.command_output = Some(chunk),
        }
        Ok(())
    }
}

#[derive(Default)]
struct BatchArgs {
    prompts: Vec<String>,
    strategy: Option<String>,
    json: bool,
    web: bool,
    help: bool,
}

#[derive(Serialize)]
struct TurnResult {
    prompt: String,
    response: String,
    routing: ChatTurnDebugSummary,
    debug: TurnDebugOutput,
}

async fn execute_batch_run_command(
    command: &str,
    api_config: &ApiConfig,
    exec_policy: &harper_core::ExecPolicyConfig,
    approver: Arc<dyn harper_core::core::io_traits::UserApproval>,
    runtime_events: Arc<BatchRuntimeEvents>,
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<String, HarperError> {
    let audit_ctx = harper_core::tools::shell::CommandAuditContext {
        conn,
        session_id: Some(session_id),
        source: "native_shell_batch",
    };
    let response = format!(
        r#"[RUN_COMMAND {{"command":{}}}]"#,
        serde_json::to_string(command)
            .map_err(|err| HarperError::Validation(format!("Failed to encode command: {}", err)))?
    );
    harper_core::tools::shell::execute_command(
        &response,
        api_config,
        exec_policy,
        None,
        Some(&audit_ctx),
        Some(approver),
        Some(runtime_events),
    )
    .await
}

fn format_batch_run_response(tool_response: &str, debug: &TurnDebugOutput) -> String {
    if debug
        .activity
        .iter()
        .any(|status| status == "approval rejected")
        || tool_response
            .trim()
            .eq_ignore_ascii_case("Command execution cancelled by user")
    {
        return "Command was not run: approval rejected.".to_string();
    }

    if debug.command_error {
        return "Command failed.".to_string();
    }

    let output = tool_response.trim_end();
    if output.is_empty() {
        "Command finished.".to_string()
    } else {
        output.to_string()
    }
}

fn print_help() {
    println!("Harper Batch Processor");
    println!();
    println!("Usage:");
    println!("  harper-batch --prompt \"...\" [--prompt \"...\"] [options]");
    println!("  printf 'prompt one\\nprompt two\\n' | harper-batch [options]");
    println!();
    println!("Options:");
    println!("  --prompt <text>          Add a prompt to run in the same session");
    println!("  --strategy <mode>       One of: auto, grounded, deterministic, model");
    println!("  --json                  Print JSON output");
    println!("  --web                   Enable web search for the session");
    println!("  --help                  Show this help");
}

fn parse_args(args: &[String]) -> Result<BatchArgs, HarperError> {
    let mut parsed = BatchArgs::default();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--prompt" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(HarperError::Validation(
                        "--prompt requires a value".to_string(),
                    ));
                };
                parsed.prompts.push(value.clone());
            }
            "--strategy" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(HarperError::Validation(
                        "--strategy requires a value".to_string(),
                    ));
                };
                parsed.strategy = Some(value.clone());
            }
            "--json" => parsed.json = true,
            "--web" => parsed.web = true,
            "--help" | "-h" => parsed.help = true,
            other => {
                return Err(HarperError::Validation(format!(
                    "Unknown batch argument: {}",
                    other
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

fn get_api_key(config: &HarperConfig) -> String {
    let mut api_key = config.api.api_key.clone();
    if config.api.provider == "Gemini" {
        if let Ok(env_key) = env::var("GEMINI_API_KEY") {
            api_key = env_key;
        }
    } else if config.api.provider == "OpenAI" {
        if let Ok(env_key) = env::var("OPENAI_API_KEY") {
            api_key = env_key;
        }
    } else if config.api.provider == "Sambanova" {
        if let Ok(env_key) = env::var("SAMBASTUDIO_API_KEY") {
            api_key = env_key;
        }
    }
    api_key
}

fn read_stdin_prompts() -> Result<Vec<String>, HarperError> {
    if io::stdin().is_terminal() {
        return Ok(Vec::new());
    }
    let input = io::read_to_string(io::stdin())
        .map_err(|e| HarperError::Io(format!("Failed to read stdin prompts: {}", e)))?;
    Ok(input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn build_api_config(config: &HarperConfig) -> Result<ApiConfig, HarperError> {
    Ok(ApiConfig {
        provider: config.api.get_provider()?,
        api_key: get_api_key(config),
        base_url: config.api.base_url.clone(),
        model_name: config.api.model_name.clone(),
    })
}

fn build_native_shell_context(config: &HarperConfig) -> NativeShellContext {
    NativeShellContext {
        auth: None,
        config: Some(ConfigShellContext {
            provider: config.api.provider.clone(),
            model: config.api.model_name.clone(),
            base_url: config.api.base_url.clone(),
            database_path: config.database.path.clone(),
            approval: format!("{:?}", config.exec_policy.effective_approval_profile()),
            strategy: format!("{:?}", config.exec_policy.effective_execution_strategy()),
            sandbox: format!("{:?}", config.exec_policy.effective_sandbox_profile()),
        }),
    }
}

async fn init_mcp_client(config: &HarperConfig) -> Option<McpClient> {
    if !config.mcp.enabled {
        return None;
    }

    let transport = turul_mcp_client::transport::HttpTransport::new(&config.mcp.server_url).ok()?;
    let client = turul_mcp_client::McpClientBuilder::new()
        .with_transport(Box::new(transport))
        .build();
    client.connect().await.ok()?;
    Some(client)
}

fn latest_assistant_response(history: &[Message]) -> String {
    history
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.content.clone())
        .unwrap_or_default()
}

fn format_session_messages(messages: &[Message]) -> String {
    if messages.is_empty() {
        return "Session has no messages.".to_string();
    }
    let mut lines = Vec::new();
    for (index, message) in messages.iter().enumerate().take(20) {
        let preview = message
            .content
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(160)
            .collect::<String>();
        lines.push(format!("{}. {}: {}", index + 1, message.role, preview));
    }
    if messages.len() > 20 {
        lines.push(format!("{} more messages", messages.len() - 20));
    }
    lines.join("\n")
}

fn apply_config_set(config: &HarperConfig, key: &str, value: &str) -> Result<String, HarperError> {
    let mut approval = config.exec_policy.effective_approval_profile();
    let mut execution_strategy = config.exec_policy.effective_execution_strategy();
    let mut sandbox = config.exec_policy.effective_sandbox_profile();
    let mut retry_max_attempts = config.exec_policy.effective_retry_max_attempts();

    match key.trim().to_ascii_lowercase().as_str() {
        "approval" | "approval_profile" => {
            approval = harper_ui::interfaces::ui::settings::parse_approval_profile(value)
                .ok_or_else(|| {
                    HarperError::Validation(
                        "approval must be one of: strict, allow_listed, allow_all".to_string(),
                    )
                })?;
        }
        "strategy" | "execution_strategy" => {
            execution_strategy = harper_ui::interfaces::ui::settings::parse_execution_strategy(
                value,
            )
            .ok_or_else(|| {
                HarperError::Validation(
                    "strategy must be one of: auto, grounded, deterministic, model".to_string(),
                )
            })?;
        }
        "sandbox" | "sandbox_profile" => {
            sandbox = harper_ui::interfaces::ui::settings::parse_sandbox_profile(value)
                .ok_or_else(|| {
                    HarperError::Validation(
                        "sandbox must be one of: disabled, workspace, networked_workspace"
                            .to_string(),
                    )
                })?;
        }
        "retries" | "retry_max_attempts" => {
            retry_max_attempts = value.trim().parse::<u32>().map_err(|_| {
                HarperError::Validation("retries must be a non-negative integer".to_string())
            })?;
        }
        other => {
            return Err(HarperError::Validation(format!(
                "unsupported config key '{}'",
                other
            )));
        }
    }

    let header_widgets = harper_ui::interfaces::ui::settings::parse_header_widgets(
        &config.ui.effective_header_widgets(),
    );
    harper_ui::interfaces::ui::settings::save_execution_policy_settings(
        harper_ui::interfaces::ui::settings::ExecPolicySettings {
            approval,
            execution_strategy,
            sandbox,
            retry_max_attempts,
            allowed_commands: config
                .exec_policy
                .allowed_commands
                .as_deref()
                .unwrap_or(&[]),
            blocked_commands: config
                .exec_policy
                .blocked_commands
                .as_deref()
                .unwrap_or(&[]),
            header_widgets: &header_widgets,
        },
    )
    .map_err(|err| HarperError::Io(format!("Failed to save config: {}", err)))?;

    Ok(format!(
        "Saved config {} = {} in config/local.toml",
        key, value
    ))
}

#[tokio::main]
async fn main() -> Result<(), HarperError> {
    let _ = dotenvy::dotenv();

    let raw_args: Vec<String> = env::args().collect();
    let mut args = parse_args(&raw_args)?;
    if args.help {
        print_help();
        return Ok(());
    }

    args.prompts.extend(read_stdin_prompts()?);
    if args.prompts.is_empty() {
        return Err(HarperError::Validation(
            "No prompts provided. Use --prompt or pipe prompts on stdin.".to_string(),
        ));
    }

    let mut config = HarperConfig::new()?;
    let api_config = build_api_config(&config)?;
    let conn = create_connection(&config.database.path)?;
    init_db(&conn)?;
    let mcp_client = init_mcp_client(&config).await;
    let runtime_events = Arc::new(BatchRuntimeEvents::default());

    let mut chat_service = ChatService::new(
        &conn,
        &api_config,
        mcp_client.as_ref(),
        None,
        Some(uuid::Uuid::new_v4().to_string()),
        config.custom_commands.commands.clone().unwrap_or_default(),
        config.exec_policy.clone(),
    )
    .with_runtime_events(runtime_events.clone());

    let approver: Arc<dyn harper_core::core::io_traits::UserApproval> = if io::stdin().is_terminal()
    {
        Arc::new(StdinApproval)
    } else {
        Arc::new(DenyApproval)
    };

    chat_service = chat_service.with_approver(approver.clone());

    let (mut history, session_id) = chat_service.create_session(args.web).await?;
    let mut shell_context = build_native_shell_context(&config);

    if let Some(strategy) = args.strategy.as_deref() {
        let strategy_command = format!("/strategy {}", strategy);
        chat_service
            .send_message(&strategy_command, &mut history, args.web, &session_id)
            .await?;
        let _ = runtime_events.take_turn(&session_id);
    }

    let mut results = Vec::new();
    for prompt in args.prompts {
        let mut prompt = prompt;
        let routing = chat_service.debug_turn_summary(&history, &prompt);
        if let Some(command) = parse_native_shell_command(&prompt)? {
            match execute_native_shell_command_with_context(
                &conn,
                &session_id,
                command,
                &shell_context,
            )? {
                NativeShellOutcome::Handled(response) => {
                    results.push(TurnResult {
                        prompt,
                        response,
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::Ask(message) => {
                    prompt = message;
                }
                NativeShellOutcome::Run(command) => {
                    let tool_response = execute_batch_run_command(
                        &command,
                        &api_config,
                        &config.exec_policy,
                        approver.clone(),
                        runtime_events.clone(),
                        &conn,
                        &session_id,
                    )
                    .await?;
                    let debug = runtime_events.take_turn(&session_id);
                    let response = format_batch_run_response(&tool_response, &debug);
                    results.push(TurnResult {
                        prompt,
                        response,
                        routing,
                        debug,
                    });
                    continue;
                }
                NativeShellOutcome::UpdateCheck => {
                    let response = harper_ui::update::fetch_update_status()
                        .await
                        .map(|status| status.unwrap_or_else(|| "No update status.".to_string()))
                        .unwrap_or_else(|err| format!("Update check failed: {}", err));
                    results.push(TurnResult {
                        prompt,
                        response,
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::UpdateStatus => {
                    results.push(TurnResult {
                        prompt,
                        response: "No update status is cached in batch. Use update check."
                            .to_string(),
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::UpdateApply => {
                    results.push(TurnResult {
                        prompt,
                        response: "Update apply is intentionally explicit. Run: harper self-update"
                            .to_string(),
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::ConfigSet { key, value } => {
                    let response = apply_config_set(&config, &key, &value)?;
                    config = HarperConfig::new()?;
                    shell_context = build_native_shell_context(&config);
                    results.push(TurnResult {
                        prompt,
                        response,
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::AuthLogin { provider } => {
                    results.push(TurnResult {
                        prompt,
                        response: format!(
                            "Auth login for '{}' is interactive. Run Harper in the TUI and use: auth login {}",
                            provider, provider
                        ),
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::AuthLogout => {
                    results.push(TurnResult {
                        prompt,
                        response: "Auth logout is only available in the interactive TUI."
                            .to_string(),
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
                NativeShellOutcome::OpenSession { target, .. } => {
                    let response =
                        match resolve_session_target(&conn, &target).and_then(|session_id| {
                            harper_core::memory::session_service::SessionService::new(&conn)
                                .view_session_data(&session_id)
                        }) {
                            Ok(messages) => format_session_messages(&messages),
                            Err(err) => format!("Session open failed: {}", err),
                        };
                    results.push(TurnResult {
                        prompt,
                        response,
                        routing,
                        debug: TurnDebugOutput::default(),
                    });
                    continue;
                }
            }
        }
        chat_service
            .send_message(&prompt, &mut history, args.web, &session_id)
            .await?;
        let mut debug = runtime_events.take_turn(&session_id);
        let response = latest_assistant_response(&history);
        if routing.task_mode == "respond_only"
            && result_looks_like_backend_unavailable(&response)
            && routing.deterministic_intent.is_none()
            && routing.normalized_command.is_none()
            && routing.clarification.is_none()
        {
            debug.activity.retain(|status| status == "responding");
        }
        results.push(TurnResult {
            prompt,
            response,
            routing,
            debug,
        });
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&results)
                .map_err(|e| HarperError::Validation(format!("Failed to serialize JSON: {}", e)))?
        );
        return Ok(());
    }

    for result in results {
        println!("USER: {}", result.prompt);
        println!(
            "ROUTING: strategy={} task_mode={}",
            result.routing.strategy, result.routing.task_mode
        );
        if let Some(intent) = &result.routing.deterministic_intent {
            println!("DETERMINISTIC INTENT: {}", intent);
        }
        if let Some(command) = &result.routing.normalized_command {
            println!("NORMALIZED COMMAND: {}", command);
        }
        if let Some(clarification) = &result.routing.clarification {
            println!("CLARIFICATION: {}", clarification);
        }
        if !result.debug.activity.is_empty() {
            println!("ACTIVITY:");
            for status in &result.debug.activity {
                println!("  - {}", status);
            }
        }
        if let Some(command) = &result.debug.command {
            println!("COMMAND: {}", command);
        }
        if let Some(output) = &result.debug.command_output {
            if result.debug.command_error {
                println!("COMMAND OUTPUT (error):");
            } else {
                println!("COMMAND OUTPUT:");
            }
            println!("{}", output.trim_end());
        }
        println!("ASSISTANT: {}", result.response);
        println!();
    }

    Ok(())
}

fn result_looks_like_backend_unavailable(response: &str) -> bool {
    response.trim().starts_with("Model backend unavailable.")
}

#[cfg(test)]
mod tests {
    use super::{TurnDebugOutput, format_batch_run_response};

    #[test]
    fn batch_run_response_reports_rejected_approval() {
        let debug = TurnDebugOutput {
            activity: vec![
                "waiting approval: pwd".to_string(),
                "approval rejected".to_string(),
            ],
            ..TurnDebugOutput::default()
        };

        assert_eq!(
            format_batch_run_response("Command execution cancelled by user", &debug),
            "Command was not run: approval rejected."
        );
    }

    #[test]
    fn batch_run_response_returns_successful_output() {
        let debug = TurnDebugOutput::default();

        assert_eq!(format_batch_run_response("hello\n", &debug), "hello");
    }
}
