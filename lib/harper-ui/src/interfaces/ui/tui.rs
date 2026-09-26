// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::app::{AppState, ApprovalState, ChatState, CommandOutputState, TuiApp};
use super::auth;
use super::events::{self, EventResult};
use super::settings;
use super::theme::Theme;
use super::widgets;
use harper_core::ExecutionStrategy;
use harper_core::agent::chat::ChatService;
use harper_core::core::ApiConfig;
use harper_core::core::io_traits::{RuntimeEventSink, UserApproval};
use harper_core::core::plan::{PlanLoopOutcome, PlanLoopStage};
use harper_core::memory::session_service::SessionService;
use harper_core::runtime::config::{ExecPolicyConfig, UiConfig};
use harper_core::{PlanState, ResolvedAgents, SessionStateView};
use rusqlite::Connection;

use async_trait::async_trait;
use harper_core::core::error::HarperResult;
use std::path::Path;

/// Type alias for the approval message sent via channels
type ApprovalMessage = (String, String, Arc<Mutex<Option<oneshot::Sender<bool>>>>);

/// Approval provider for TUI that uses channels to communicate with the UI loop
pub struct TuiApproval {
    approval_tx: mpsc::Sender<ApprovalMessage>,
}

pub struct TuiRuntimeEvents {
    ui_tx: mpsc::Sender<UiUpdate>,
}

pub struct TuiRunOptions {
    pub custom_commands: HashMap<String, String>,
    pub server_base_url: Option<String>,
}

#[async_trait]
impl UserApproval for TuiApproval {
    async fn approve(&self, prompt: &str, command: &str) -> HarperResult<bool> {
        let (tx, rx) = oneshot::channel();
        self.approval_tx
            .send((
                prompt.to_string(),
                command.to_string(),
                Arc::new(Mutex::new(Some(tx))),
            ))
            .await
            .map_err(|_| {
                harper_core::core::error::HarperError::Command(
                    "Failed to send approval request".to_string(),
                )
            })?;

        rx.await.map_err(|_| {
            harper_core::core::error::HarperError::Command(
                "Failed to receive approval response".to_string(),
            )
        })
    }
}

#[async_trait]
impl RuntimeEventSink for TuiRuntimeEvents {
    async fn plan_updated(&self, session_id: &str, plan: Option<PlanState>) -> HarperResult<()> {
        self.ui_tx
            .send(UiUpdate::PlanUpdated {
                session_id: session_id.to_string(),
                active_plan: plan,
            })
            .await
            .map_err(|_| {
                harper_core::core::error::HarperError::Command(
                    "Failed to send runtime plan update".to_string(),
                )
            })?;
        Ok(())
    }

    async fn agents_updated(
        &self,
        session_id: &str,
        agents: Option<ResolvedAgents>,
    ) -> HarperResult<()> {
        self.ui_tx
            .send(UiUpdate::AgentsUpdated {
                session_id: session_id.to_string(),
                active_agents: agents,
            })
            .await
            .map_err(|_| {
                harper_core::core::error::HarperError::Command(
                    "Failed to send runtime AGENTS update".to_string(),
                )
            })?;
        Ok(())
    }

    async fn activity_updated(&self, session_id: &str, status: Option<String>) -> HarperResult<()> {
        self.ui_tx
            .send(UiUpdate::ActivityUpdated {
                session_id: session_id.to_string(),
                status,
            })
            .await
            .map_err(|_| {
                harper_core::core::error::HarperError::Command(
                    "Failed to send runtime activity update".to_string(),
                )
            })?;
        Ok(())
    }

    async fn command_output_updated(
        &self,
        session_id: &str,
        command: String,
        chunk: String,
        is_error: bool,
        done: bool,
    ) -> HarperResult<()> {
        self.ui_tx
            .send(UiUpdate::CommandOutputUpdated {
                session_id: session_id.to_string(),
                command,
                chunk,
                is_error,
                done,
            })
            .await
            .map_err(|_| {
                harper_core::core::error::HarperError::Command(
                    "Failed to send runtime command output update".to_string(),
                )
            })?;
        Ok(())
    }
}

/// Messages sent to the background chat worker
enum WorkerMsg {
    SendMessage {
        user_msg: String,
        session_id: String,
        web_search: bool,
        auth_user_id: Option<String>,
    },
    ExecuteShellCommand {
        command: String,
        session_id: String,
        auth_user_id: Option<String>,
    },
    RetryPlanCommand {
        command: String,
        session_id: String,
    },
}

/// Messages sent from the background chat worker to the UI
enum UiUpdate {
    MessageProcessed(SessionStateView),
    ActivityUpdated {
        session_id: String,
        status: Option<String>,
    },
    CommandOutputUpdated {
        session_id: String,
        command: String,
        chunk: String,
        is_error: bool,
        done: bool,
    },
    PlanUpdated {
        session_id: String,
        active_plan: Option<PlanState>,
    },
    AgentsUpdated {
        session_id: String,
        active_agents: Option<ResolvedAgents>,
    },
    SidebarEntries {
        sections: Vec<super::app::SidebarSection>,
    },
    UpdateStatus {
        status: Result<Option<String>, String>,
        homebrew_path_fix: Option<crate::update::HomebrewPathFix>,
        manual: bool,
    },
    HomebrewPathFixApplied {
        result: Result<String, String>,
    },
    Error(String),
}
fn coalesce_ui_update(batch: &mut Vec<UiUpdate>, update: UiUpdate) {
    if let UiUpdate::CommandOutputUpdated {
        session_id,
        command,
        chunk,
        is_error,
        done,
    } = &update
    {
        if let Some(UiUpdate::CommandOutputUpdated {
            session_id: prev_session,
            command: prev_command,
            chunk: prev_chunk,
            is_error: prev_is_error,
            done: prev_done,
        }) = batch.last_mut()
        {
            if prev_session == session_id && prev_command == command {
                prev_chunk.push_str(chunk);
                *prev_is_error |= *is_error;
                *prev_done = *done;
                return;
            }
        }
        batch.push(update);
        return;
    }

    if matches!(update, UiUpdate::Error(_)) {
        batch.push(update);
        return;
    }

    if let Some(last) = batch.last() {
        if matches!(
            last,
            UiUpdate::CommandOutputUpdated { .. } | UiUpdate::Error(_)
        ) {
            batch.push(update);
            return;
        }
        if std::mem::discriminant(last) == std::mem::discriminant(&update) {
            *batch.last_mut().expect("batch is non-empty") = update;
            return;
        }
    }
    batch.push(update);
}

/// Helper function to spawn async sidebar gathering task
fn spawn_sidebar_gathering(chat_state: &ChatState, ui_tx: &mpsc::Sender<UiUpdate>) {
    let messages = chat_state.messages.clone();
    let ui_tx_clone = ui_tx.clone();
    tokio::spawn(async move {
        let sections = super::app::gather_sidebar_entries_async(&messages).await;
        let _ = ui_tx_clone
            .send(UiUpdate::SidebarEntries { sections })
            .await;
    });
}

fn infer_chat_loop_stage(status: &str) -> Option<PlanLoopStage> {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        s if s.starts_with("planning") => Some(PlanLoopStage::Planning),
        s if s.starts_with("inspecting") => Some(PlanLoopStage::Inspecting),
        s if s.starts_with("executing") || s.starts_with("running") => {
            Some(PlanLoopStage::Executing)
        }
        s if s.starts_with("summarizing") => Some(PlanLoopStage::Feedback),
        s if s.starts_with("responding")
            || s.starts_with("thinking")
            || s.starts_with("waiting for browser sign-in") =>
        {
            Some(PlanLoopStage::Responding)
        }
        s if s.starts_with("waiting approval")
            || s.starts_with("resuming")
            || s.starts_with("retrying") =>
        {
            Some(PlanLoopStage::Feedback)
        }
        _ => None,
    }
}

fn final_chat_loop_outcome(
    stage: Option<&PlanLoopStage>,
    current: Option<&PlanLoopOutcome>,
) -> PlanLoopOutcome {
    match stage {
        Some(PlanLoopStage::Responding) => PlanLoopOutcome::Responded,
        Some(
            PlanLoopStage::Planning
            | PlanLoopStage::Inspecting
            | PlanLoopStage::Executing
            | PlanLoopStage::Feedback,
        ) => PlanLoopOutcome::Succeeded,
        Some(PlanLoopStage::Interrupted) => current.cloned().unwrap_or(PlanLoopOutcome::Cancelled),
        Some(PlanLoopStage::RetryPending | PlanLoopStage::ReplanRequired) | None => {
            current.cloned().unwrap_or(PlanLoopOutcome::Responded)
        }
    }
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn load_cancel_session_view(
    conn: &Connection,
    session_id: &str,
    auth_user_id: Option<&str>,
) -> SessionStateView {
    let session_service = SessionService::new(conn);
    let fallback_history =
        || harper_core::memory::storage::load_history(conn, session_id).unwrap_or_default();
    match auth_user_id {
        Some(user_id) => session_service
            .load_session_state_view_for_user(session_id, user_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| SessionStateView {
                session_id: session_id.to_string(),
                user_id: Some(user_id.to_string()),
                messages: fallback_history(),
                plan: None,
                agents: None,
                agents_rendered: None,
                agents_effective_rendered: None,
            }),
        None => session_service
            .load_session_state_view(session_id)
            .unwrap_or_else(|_| SessionStateView {
                session_id: session_id.to_string(),
                user_id: None,
                messages: fallback_history(),
                plan: None,
                agents: None,
                agents_rendered: None,
                agents_effective_rendered: None,
            }),
    }
}

async fn finalize_worker_cancellation(
    conn: &Connection,
    session_id: &str,
    auth_user_id: Option<&str>,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    let _ = harper_core::tools::plan::set_plan_loop_stage(
        conn,
        session_id,
        PlanLoopStage::Interrupted,
        Some("cancelled by user".to_string()),
    );
    let _ = harper_core::tools::plan::record_plan_loop_outcome(
        conn,
        session_id,
        PlanLoopOutcome::Cancelled,
        Some("cancelled by user".to_string()),
    );
    let session_view = load_cancel_session_view(conn, session_id, auth_user_id);
    let _ = ui_tx.send(UiUpdate::MessageProcessed(session_view)).await;
}

fn display_command_info(app: &mut TuiApp, response: String) {
    app.set_persistent_info_message(response);
}

fn parse_strategy_command(
    input: &str,
    current: ExecutionStrategy,
) -> Option<Result<ExecutionStrategy, ExecutionStrategy>> {
    let trimmed = input.trim();
    let command = trimmed.strip_prefix("/strategy")?;
    let value = command.trim();
    if value.is_empty() {
        return Some(Err(current));
    }

    let strategy = match value.to_ascii_lowercase().as_str() {
        "auto" => ExecutionStrategy::Auto,
        "grounded" => ExecutionStrategy::Grounded,
        "deterministic" => ExecutionStrategy::Deterministic,
        "model" | "model-only" | "model_only" => ExecutionStrategy::ModelOnly,
        _ => return Some(Err(current)),
    };
    Some(Ok(strategy))
}

enum AgentsCommand {
    ShowStatus,
    Set(bool),
}

fn parse_agents_command(input: &str) -> Option<AgentsCommand> {
    let trimmed = input.trim();
    let command = trimmed.strip_prefix("/agents")?;
    let value = command.trim().to_ascii_lowercase();
    match value.as_str() {
        "" | "status" => Some(AgentsCommand::ShowStatus),
        "on" | "enable" | "enabled" => Some(AgentsCommand::Set(true)),
        "off" | "disable" | "disabled" => Some(AgentsCommand::Set(false)),
        _ => None,
    }
}

enum UpdateCommand {
    ShowStatus,
    Refresh,
}

fn spawn_update_status_refresh(ui_tx: &mpsc::Sender<UiUpdate>, manual: bool) {
    let update_ui_tx = ui_tx.clone();
    tokio::spawn(async move {
        let status = crate::update::fetch_update_status().await;
        let homebrew_path_fix = crate::update::detect_homebrew_path_fix();
        let _ = update_ui_tx
            .send(UiUpdate::UpdateStatus {
                status,
                homebrew_path_fix,
                manual,
            })
            .await;
    });
}

fn spawn_homebrew_path_fix(
    ui_tx: &mpsc::Sender<UiUpdate>,
    approval_tx: &mpsc::Sender<ApprovalMessage>,
    fix: crate::update::HomebrewPathFix,
) {
    let ui_tx = ui_tx.clone();
    let approval_tx = approval_tx.clone();
    tokio::spawn(async move {
        let command = format!(
            "move {} aside and link it to {}",
            fix.shadow_path.display(),
            fix.homebrew_path.display()
        );
        let prompt = "Fix Homebrew PATH?".to_string();
        let (tx, rx) = oneshot::channel();
        let tx = Arc::new(Mutex::new(Some(tx)));
        if approval_tx.send((prompt, command, tx)).await.is_err() {
            let _ = ui_tx
                .send(UiUpdate::HomebrewPathFixApplied {
                    result: Err("approval request could not be shown".to_string()),
                })
                .await;
            return;
        }

        match rx.await {
            Ok(true) => {
                let result = crate::update::apply_homebrew_path_fix(&fix).map(|backup_path| {
                    format!(
                        "Homebrew PATH fixed. Previous binary moved to {}.",
                        backup_path.display()
                    )
                });
                let _ = ui_tx
                    .send(UiUpdate::HomebrewPathFixApplied { result })
                    .await;
            }
            Ok(false) => {
                let _ = ui_tx
                    .send(UiUpdate::HomebrewPathFixApplied {
                        result: Ok("Homebrew PATH fix cancelled.".to_string()),
                    })
                    .await;
            }
            Err(_) => {
                let _ = ui_tx
                    .send(UiUpdate::HomebrewPathFixApplied {
                        result: Err("approval was cancelled".to_string()),
                    })
                    .await;
            }
        }
    });
}

fn clamp_execution_policy_selection(app: &mut TuiApp) {
    let row_count = app.execution_policy_row_count();
    if let AppState::ExecutionPolicy(selected) = &mut app.state {
        *selected = (*selected).min(row_count.saturating_sub(1));
    }
}

fn parse_update_command(input: &str) -> Option<UpdateCommand> {
    let trimmed = input.trim();
    let command = trimmed.strip_prefix("/update")?;
    let value = command.trim().to_ascii_lowercase();
    match value.as_str() {
        "" | "status" => Some(UpdateCommand::ShowStatus),
        "check" | "refresh" => Some(UpdateCommand::Refresh),
        _ => None,
    }
}

fn build_native_shell_context(
    app: &TuiApp,
    api_config: &ApiConfig,
    exec_policy: &ExecPolicyConfig,
    db_path: Option<&str>,
) -> harper_core::NativeShellContext {
    let auth = app.auth_session.as_ref().map(|session| {
        let user = session
            .user
            .email
            .clone()
            .or_else(|| session.user.display_name.clone())
            .unwrap_or_else(|| session.user.user_id.clone());
        harper_core::AuthShellContext {
            status: format!("signed in as {}", user),
        }
    });

    harper_core::NativeShellContext {
        auth,
        config: Some(harper_core::ConfigShellContext {
            provider: api_config.provider.to_string(),
            model: api_config.model_name.clone(),
            base_url: api_config.base_url.clone(),
            database_path: db_path.unwrap_or(":memory:").to_string(),
            approval: format!("{:?}", exec_policy.effective_approval_profile()),
            strategy: format!("{:?}", exec_policy.effective_execution_strategy()),
            sandbox: format!("{:?}", exec_policy.effective_sandbox_profile()),
        }),
    }
}

fn apply_tui_config_set(app: &mut TuiApp, key: &str, value: &str) -> Result<String, String> {
    match key.trim().to_ascii_lowercase().as_str() {
        "approval" | "approval_profile" => {
            app.approval_profile = settings::parse_approval_profile(value)
                .ok_or("approval must be one of: strict, allow_listed, allow_all")?;
        }
        "strategy" | "execution_strategy" => {
            app.execution_strategy = settings::parse_execution_strategy(value)
                .ok_or("strategy must be one of: auto, grounded, deterministic, model")?;
        }
        "sandbox" | "sandbox_profile" => {
            app.sandbox_profile = settings::parse_sandbox_profile(value)
                .ok_or("sandbox must be one of: disabled, workspace, networked_workspace")?;
        }
        "retries" | "retry_max_attempts" => {
            app.retry_max_attempts = value
                .trim()
                .parse::<u32>()
                .map_err(|_| "retries must be a non-negative integer")?;
        }
        other => return Err(format!("unsupported config key '{}'", other)),
    }

    settings::save_execution_policy_settings(settings::ExecPolicySettings {
        approval: app.approval_profile,
        execution_strategy: app.execution_strategy,
        sandbox: app.sandbox_profile,
        retry_max_attempts: app.retry_max_attempts,
        allowed_commands: &app.allowed_commands,
        blocked_commands: &app.blocked_commands,
        header_widgets: &app.header_widgets,
    })
    .map_err(|err| format!("Failed to save config: {}", err))?;

    Ok(format!(
        "Saved config {} = {} in config/local.toml",
        key, value
    ))
}

fn sync_mouse_capture<W: Write>(
    backend: &mut CrosstermBackend<W>,
    enabled: &mut bool,
    desired: bool,
) -> std::io::Result<()> {
    if *enabled == desired {
        return Ok(());
    }

    if desired {
        execute!(backend, EnableMouseCapture)?;
    } else {
        execute!(backend, DisableMouseCapture)?;
    }
    *enabled = desired;
    Ok(())
}

fn sync_exec_policy_from_app(app: &TuiApp, exec_policy: &Arc<Mutex<ExecPolicyConfig>>) {
    if let Ok(mut policy) = exec_policy.lock() {
        policy.approval_profile = Some(app.approval_profile);
        policy.execution_strategy = Some(app.execution_strategy);
        policy.sandbox_profile = Some(app.sandbox_profile);
        policy.allowed_commands = if app.allowed_commands.is_empty() {
            None
        } else {
            Some(app.allowed_commands.clone())
        };
        policy.blocked_commands = if app.blocked_commands.is_empty() {
            None
        } else {
            Some(app.blocked_commands.clone())
        };
        policy.retry_max_attempts = Some(app.retry_max_attempts);
    }
}

async fn start_tui_auth_login(app: &mut TuiApp, auth_client: &reqwest::Client, provider: &str) {
    let Some(base_url) = app.auth_server_base_url.clone() else {
        app.set_error_message("TUI auth requires the Harper server to be enabled".to_string());
        return;
    };
    match auth::start_tui_auth_flow(auth_client, &base_url, provider).await {
        Ok(flow) => {
            app.auth_flow_id = Some(flow.flow_id.clone());
            app.auth_last_poll_at = None;
            match auth::launch_browser(&flow.login_url) {
                Ok(_) => app.set_status_message(format!("Opened browser for {} sign-in", provider)),
                Err(_) => {
                    app.set_info_message(format!("Open this URL to sign in:\n{}", flow.login_url))
                }
            }
            app.set_activity_status(Some("waiting for browser sign-in".to_string()));
        }
        Err(err) => app.set_error_message(format!("Auth login failed: {}", err)),
    }
}

/// Restore the terminal if the TUI panics, so the user's shell is never
/// left bricked (hidden cursor, raw mode, alternate screen). Every step is
/// best-effort: a hook must not panic itself.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = execute!(
            stdout,
            LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
            cursor::Show
        );
        default_hook(info);
    }));
}

pub async fn run_tui(
    conn: &Connection,
    api_config: &ApiConfig,
    session_service: &SessionService<'_>,
    theme: &Theme,
    exec_policy: &ExecPolicyConfig,
    ui_config: &UiConfig,
    options: TuiRunOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    // Set up terminal
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        cursor::Hide
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new();
    app.show_menu_logo = ui_config.show_menu_logo.unwrap_or(true);
    app.mouse_capture = ui_config.mouse_capture.unwrap_or(false);
    let mut mouse_capture_enabled = false;
    sync_mouse_capture(
        terminal.backend_mut(),
        &mut mouse_capture_enabled,
        app.mouse_capture,
    )?;
    app.model_label = format!("{:?} / {}", api_config.provider, api_config.model_name);
    app.current_working_dir = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    app.auth_session = auth::load_auth_session();
    app.auth_server_base_url = options.server_base_url.clone();
    app.approval_profile = exec_policy.effective_approval_profile();
    app.execution_strategy = exec_policy.effective_execution_strategy();
    let configured_widgets = ui_config.effective_header_widgets();
    if !configured_widgets.is_empty() {
        app.header_widgets = settings::parse_header_widgets(&configured_widgets);
    }
    app.sandbox_profile = exec_policy.effective_sandbox_profile();
    app.retry_max_attempts = exec_policy.effective_retry_max_attempts();
    app.allowed_commands = exec_policy.allowed_commands.clone().unwrap_or_default();
    app.blocked_commands = exec_policy.blocked_commands.clone().unwrap_or_default();
    let auth_client = reqwest::Client::new();

    // Set up channels for background worker
    let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerMsg>(10);
    let (ui_tx, mut ui_rx) = mpsc::channel::<UiUpdate>(10);
    let (approval_tx, mut approval_rx) = mpsc::channel::<ApprovalMessage>(1);

    spawn_update_status_refresh(&ui_tx, false);

    // Clone data for worker
    let worker_api_config = api_config.clone();
    let worker_custom_commands = options.custom_commands.clone();
    let worker_exec_policy = Arc::new(Mutex::new(exec_policy.clone()));
    let ui_exec_policy = worker_exec_policy.clone();
    let db_path = conn
        .path()
        .and_then(|p| Path::new(p).to_str().map(|s| s.to_string()));
    let db_path_for_shell = db_path.clone();

    // Wrap MCP client in Arc if present
    // Note: This requires McpClient to be thread-safe (Send + Sync)
    // We assume McpClient is Arc-cloneable or we wrap it.
    // For now, if McpClient is not cloneable, we'd need a proxy.
    // Let's assume the user can provide an Arc or we wrap it here if we had access to the type.
    // Since we only have Option<&McpClient>, we can't easily Arc it for the worker thread.
    // ARCHITECTURAL NOTE: To support MCP in TUI worker, McpClient should be passed as Arc<McpClient>.

    // Spawn background worker in a separate thread to handle non-Send Connection
    let ui_tx_clone = ui_tx.clone();
    let worker_approval_tx = approval_tx.clone();
    let worker_cancel = app.cancel_requested.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build worker runtime");

        rt.block_on(async {
            // Open worker connection
            let worker_conn = if let Some(path) = db_path {
                Connection::open(path).expect("Worker failed to open database")
            } else {
                Connection::open_in_memory().expect("Worker failed to open in-memory database")
            };

            let mut api_cache = harper_core::core::cache::new_api_cache();
            let approver = Arc::new(TuiApproval {
                approval_tx: worker_approval_tx,
            });
            let runtime_events = Arc::new(TuiRuntimeEvents {
                ui_tx: ui_tx_clone.clone(),
            });

            while let Some(msg) = worker_rx.recv().await {
                match msg {
                    WorkerMsg::SendMessage {
                        user_msg,
                        session_id,
                        web_search,
                        auth_user_id,
                    } => {
                        tokio::select! {
                            () = wait_for_cancel(&worker_cancel) => {
                                finalize_worker_cancellation(
                                    &worker_conn,
                                    &session_id,
                                    auth_user_id.as_deref(),
                                    &ui_tx_clone,
                                )
                                .await;
                            }
                            () = async {
                        let mut chat_service = ChatService::new(
                            &worker_conn,
                            &worker_api_config,
                            None, // TODO: Support MCP in worker thread
                            Some(&mut api_cache),
                            None,
                            worker_custom_commands.clone(),
                            worker_exec_policy
                                .lock()
                                .expect("worker exec policy lock")
                                .clone(),
                        )
                        .with_approver(approver.clone())
                        .with_runtime_events(runtime_events.clone());

                        // Load existing history
                        let mut history =
                            harper_core::memory::storage::load_history(&worker_conn, &session_id)
                                .unwrap_or_default();

                        if let Some(user_id) = auth_user_id.as_deref() {
                            let _ = harper_core::memory::storage::save_session_for_user(
                                &worker_conn,
                                &session_id,
                                user_id,
                            );
                        }

                        match chat_service
                            .send_message(&user_msg, &mut history, web_search, &session_id)
                            .await
                        {
                            Ok(_) => {
                                let session_service = SessionService::new(&worker_conn);
                                let session_view = match auth_user_id.as_deref() {
                                    Some(user_id) => session_service
                                        .load_session_state_view_for_user(&session_id, user_id)
                                        .ok()
                                        .flatten()
                                        .unwrap_or_else(|| SessionStateView {
                                            session_id: session_id.clone(),
                                            user_id: Some(user_id.to_string()),
                                            messages: history.clone(),
                                            plan: None,
                                            agents: None,
                                            agents_rendered: None,
                                            agents_effective_rendered: None,
                                        }),
                                    None => session_service
                                        .load_session_state_view(&session_id)
                                        .unwrap_or_else(|_| SessionStateView {
                                            session_id: session_id.clone(),
                                            user_id: None,
                                            messages: history.clone(),
                                            plan: None,
                                            agents: None,
                                            agents_rendered: None,
                                            agents_effective_rendered: None,
                                        }),
                                };
                                let _ = ui_tx_clone
                                    .send(UiUpdate::MessageProcessed(session_view))
                                    .await;
                            }
                            Err(e) => {
                                let _ = ui_tx_clone.send(UiUpdate::Error(e.to_string())).await;
                            }
                        }
                            } => {}
                        }
                    }
                    WorkerMsg::RetryPlanCommand {
                        command,
                        session_id,
                    } => {
                        tokio::select! {
                            () = wait_for_cancel(&worker_cancel) => {
                                finalize_worker_cancellation(
                                    &worker_conn,
                                    &session_id,
                                    None,
                                    &ui_tx_clone,
                                )
                                .await;
                            }
                            () = async {
                        let exec_policy = worker_exec_policy
                            .lock()
                            .expect("worker exec policy lock")
                            .clone();
                        let audit_ctx = harper_core::tools::shell::CommandAuditContext {
                            conn: &worker_conn,
                            session_id: Some(&session_id),
                            source: "ui_plan_retry",
                        };
                        let response = format!(
                            r#"[RUN_COMMAND {{"command":{}}}]"#,
                            serde_json::to_string(&command)
                                .expect("serialize retry command payload")
                        );
                        let result = harper_core::tools::shell::execute_command(
                            &response,
                            &worker_api_config,
                            &exec_policy,
                            None,
                            Some(&audit_ctx),
                            Some(approver.clone()),
                            Some(runtime_events.clone()),
                        )
                        .await;
                        if let Err(err) = result {
                            let _ = ui_tx_clone.send(UiUpdate::Error(err.to_string())).await;
                        }
                            } => {}
                        }
                    }
                    WorkerMsg::ExecuteShellCommand {
                        command,
                        session_id,
                        auth_user_id,
                    } => {
                        tokio::select! {
                            () = wait_for_cancel(&worker_cancel) => {
                                finalize_worker_cancellation(
                                    &worker_conn,
                                    &session_id,
                                    auth_user_id.as_deref(),
                                    &ui_tx_clone,
                                )
                                .await;
                            }
                            () = async {
                        let exec_policy = worker_exec_policy
                            .lock()
                            .expect("worker exec policy lock")
                            .clone();
                        let audit_ctx = harper_core::tools::shell::CommandAuditContext {
                            conn: &worker_conn,
                            session_id: Some(&session_id),
                            source: "native_shell_tui",
                        };
                        let response = format!(
                            r#"[RUN_COMMAND {{"command":{}}}]"#,
                            serde_json::to_string(&command)
                                .expect("serialize native shell command payload")
                        );
                        let result = harper_core::tools::shell::execute_command(
                            &response,
                            &worker_api_config,
                            &exec_policy,
                            None,
                            Some(&audit_ctx),
                            Some(approver.clone()),
                            Some(runtime_events.clone()),
                        )
                        .await;
                        match result {
                            Ok(_) => {
                                let session_service = SessionService::new(&worker_conn);
                                let session_view = match auth_user_id.as_deref() {
                                    Some(user_id) => session_service
                                        .load_session_state_view_for_user(&session_id, user_id)
                                        .ok()
                                        .flatten()
                                        .unwrap_or_else(|| SessionStateView {
                                            session_id: session_id.clone(),
                                            user_id: Some(user_id.to_string()),
                                            messages: harper_core::memory::storage::load_history(
                                                &worker_conn,
                                                &session_id,
                                            )
                                            .unwrap_or_default(),
                                            plan: None,
                                            agents: None,
                                            agents_rendered: None,
                                            agents_effective_rendered: None,
                                        }),
                                    None => session_service
                                        .load_session_state_view(&session_id)
                                        .unwrap_or_else(|_| SessionStateView {
                                            session_id: session_id.clone(),
                                            user_id: None,
                                            messages: harper_core::memory::storage::load_history(
                                                &worker_conn,
                                                &session_id,
                                            )
                                            .unwrap_or_default(),
                                            plan: None,
                                            agents: None,
                                            agents_rendered: None,
                                            agents_effective_rendered: None,
                                        }),
                                };
                                let _ = ui_tx_clone
                                    .send(UiUpdate::MessageProcessed(session_view))
                                    .await;
                            }
                            Err(err) => {
                                let _ = ui_tx_clone.send(UiUpdate::Error(err.to_string())).await;
                            }
                        }
                            } => {}
                        }
                    }
                }
            }
        });
    });

    loop {
        app.refresh_activity_status();
        app.refresh_message();
        events::apply_drag_auto_scroll(&mut app);
        if let AppState::Chat(chat_state) = &mut app.state {
            crate::interfaces::ui::widgets::refresh_chat_render_cache(chat_state, theme);
        }
        terminal.draw(|f| widgets::draw(f, &app, theme))?;

        // Handle both UI events and worker updates
        tokio::select! {
            // UI Events - Bubble up errors from terminal IO
            event_res = async {
                if crossterm::event::poll(std::time::Duration::from_millis(100))? {
                    Ok::<_, std::io::Error>(Some(crossterm::event::read()?))
                } else {
                    Ok(None)
                }
            } => {
                let event = event_res?; // Proper error propagation
                if let Some(event) = event {
                    match events::handle_event(event, &mut app, session_service) {
                        EventResult::Quit => break,
                        EventResult::SendMessage(mut msg) => {
                            let shell_context = build_native_shell_context(
                                &app,
                                api_config,
                                exec_policy,
                                db_path_for_shell.as_deref(),
                            );
                            if let AppState::Chat(chat_state) = &mut app.state {
                                match harper_core::parse_native_shell_command(&msg) {
                                    Ok(Some(command)) => {
                                        let session_id = chat_state.session_id.clone();
                                        match harper_core::execute_native_shell_command_with_context(
                                            conn,
                                            &session_id,
                                            command,
                                            &shell_context,
                                        ) {
                                            Ok(harper_core::NativeShellOutcome::Handled(response)) => {
                                                chat_state.active_plan = harper_core::memory::storage::load_plan_state(conn, &session_id).ok().flatten();
                                                display_command_info(&mut app, response);
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::Ask(prompt)) => {
                                                msg = prompt;
                                            }
                                            Ok(harper_core::NativeShellOutcome::Run(command)) => {
                                                chat_state.command_output = None;
                                                chat_state.command_output_expanded = false;
                                                chat_state.command_output_scroll = 0;
                                                chat_state.command_output_selection = None;
                                                app.set_activity_status(Some(format!("running: {}", command)));
                                                app.cancel_requested.store(false, Ordering::SeqCst);
                                                let _ = worker_tx.send(WorkerMsg::ExecuteShellCommand {
                                                    command,
                                                    session_id,
                                                    auth_user_id: app
                                                        .auth_session
                                                        .as_ref()
                                                        .map(|session| session.user.user_id.clone()),
                                                }).await;
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::UpdateCheck) => {
                                                app.set_activity_status(Some("checking updates".to_string()));
                                                spawn_update_status_refresh(&ui_tx, true);
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::UpdateStatus) => {
                                                if let Some(status) = app.update_status.clone() {
                                                    display_command_info(&mut app, format!("Current {}", status));
                                                } else {
                                                    display_command_info(
                                                        &mut app,
                                                        "No update status is cached yet. Use /update check."
                                                            .to_string(),
                                                    );
                                                }
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::UpdateApply) => {
                                                display_command_info(
                                                    &mut app,
                                                    "Update apply is intentionally explicit. Run: harper self-update".to_string(),
                                                );
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::ConfigSet { key, value }) => {
                                                match apply_tui_config_set(&mut app, &key, &value) {
                                                    Ok(message) => {
                                                        sync_exec_policy_from_app(&app, &ui_exec_policy);
                                                        app.set_status_message(message);
                                                    }
                                                    Err(err) => app.set_error_message(err),
                                                }
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::AuthLogin { provider }) => {
                                                start_tui_auth_login(&mut app, &auth_client, &provider).await;
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::AuthLogout) => {
                                                if let Err(err) = auth::clear_auth_session() {
                                                    app.set_error_message(format!("Auth logout failed: {}", err));
                                                } else {
                                                    app.auth_session = None;
                                                    app.auth_flow_id = None;
                                                    app.auth_last_poll_at = None;
                                                    app.set_status_message("Cleared local TUI auth session".to_string());
                                                }
                                                continue;
                                            }
                                            Ok(harper_core::NativeShellOutcome::OpenSession { target, preview }) => {
                                                match harper_core::resolve_session_target(conn, &target) {
                                                    Ok(target_session_id) => {
                                                        if preview {
                                                            match session_service.view_session_data(&target_session_id) {
                                                                Ok(messages) => {
                                                                    app.state = AppState::ViewSession(target_session_id, messages, 0);
                                                                }
                                                                Err(err) => app.set_error_message(format!("Error loading session preview: {}", err)),
                                                            }
                                                        } else {
                                                            match session_service.load_session_state_view(&target_session_id) {
                                                                Ok(session_view) => {
                                                                    app.state = AppState::Chat(Box::new(events::create_chat_state(
                                                                        session_view.session_id,
                                                                        session_view.messages,
                                                                        session_view.plan,
                                                                        session_view.agents,
                                                                        app.agents_context_enabled,
                                                                    )));
                                                                    if let AppState::Chat(chat_state) = &mut app.state {
                                                                        spawn_sidebar_gathering(chat_state, &ui_tx);
                                                                    }
                                                                }
                                                                Err(err) => app.set_error_message(format!("Error loading session: {}", err)),
                                                            }
                                                        }
                                                    }
                                                    Err(err) => app.set_error_message(err.to_string()),
                                                }
                                                continue;
                                            }
                                            Err(err) => {
                                                app.set_error_message(err.to_string());
                                                continue;
                                            }
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(err) => {
                                        app.set_error_message(err.to_string());
                                        continue;
                                    }
                                }
                                if let Some(strategy) = parse_strategy_command(&msg, app.execution_strategy) {
                                    match strategy {
                                        Ok(new_strategy) => {
                                            app.execution_strategy = new_strategy;
                                            if let Ok(mut policy) = ui_exec_policy.lock() {
                                                policy.execution_strategy = Some(new_strategy);
                                            }
                                            app.set_status_message(format!(
                                                "Execution strategy set to {}",
                                                settings::execution_strategy_name(new_strategy)
                                            ));
                                        }
                                        Err(current_only) => {
                                            display_command_info(&mut app, format!(
                                                "Current execution strategy: {}",
                                                settings::execution_strategy_name(current_only)
                                            ));
                                        }
                                    }
                                    continue;
                                }
                                if let Some(command) = parse_agents_command(&msg) {
                                    match command {
                                        AgentsCommand::ShowStatus => {
                                            let status = if app.agents_context_enabled {
                                                "on"
                                            } else {
                                                "off"
                                            };
                                            display_command_info(&mut app, format!("AGENTS context: {}", status));
                                        }
                                        AgentsCommand::Set(enabled) => {
                                            app.agents_context_enabled = enabled;
                                            if enabled {
                                                chat_state.active_agents = std::env::current_dir()
                                                    .ok()
                                                    .and_then(|cwd| harper_core::core::agents::resolve_agents_for_dir(&cwd).ok());
                                                app.set_status_message("AGENTS context enabled".to_string());
                                            } else {
                                                chat_state.active_agents = None;
                                                chat_state.agents_panel_expanded = false;
                                                chat_state.agents_scroll_offset = 0;
                                                chat_state.set_navigation_focus(super::app::NavigationFocus::Messages);
                                                app.set_status_message("AGENTS context disabled".to_string());
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if let Some(command) = parse_update_command(&msg) {
                                    match command {
                                        UpdateCommand::ShowStatus => {
                                            if let Some(status) = app.update_status.clone() {
                                                display_command_info(&mut app, format!("Current {}", status));
                                            } else {
                                                display_command_info(
                                                    &mut app,
                                                    "No update status is cached yet. Use /update check."
                                                        .to_string(),
                                                );
                                            }
                                        }
                                        UpdateCommand::Refresh => {
                                            app.set_activity_status(Some("checking updates".to_string()));
                                            spawn_update_status_refresh(&ui_tx, true);
                                        }
                                    }
                                    continue;
                                }
                                if let Some(command) = auth::parse_tui_auth_command(&msg) {
                                    match command {
                                        auth::TuiAuthCommand::Login { provider } => {
                                            start_tui_auth_login(&mut app, &auth_client, &provider).await;
                                            continue;
                                        }
                                        auth::TuiAuthCommand::Logout => {
                                            if let Err(err) = auth::clear_auth_session() {
                                                app.set_error_message(format!("Auth logout failed: {}", err));
                                            } else {
                                                app.auth_session = None;
                                                app.auth_flow_id = None;
                                                app.auth_last_poll_at = None;
                                                app.set_status_message("Cleared local TUI auth session".to_string());
                                            }
                                            continue;
                                        }
                                        auth::TuiAuthCommand::Status => {
                                            let status = app.auth_session.as_ref().map(|session| {
                                                session.user.email.clone().unwrap_or_else(|| session.user.user_id.clone())
                                            }).unwrap_or_else(|| "not signed in".to_string());
                                            display_command_info(&mut app, format!("TUI auth status: {}", status));
                                            continue;
                                        }
                                    }
                                }
                                let session_id = chat_state.session_id.clone();
                                let web_search = chat_state.web_search_enabled;

                                // optimistic update
                                chat_state.messages.push(harper_core::core::Message {
                                    role: "user".to_string(),
                                    content: msg.clone(),
                                });
                                chat_state.follow_latest_messages();
                                chat_state.awaiting_response = true;
                                chat_state.command_output = None;
                                chat_state.command_output_expanded = false;
                                chat_state.command_output_scroll = 0;
                                chat_state.command_output_selection = None;
                                app.set_activity_status(Some("thinking".to_string()));
                                app.cancel_requested.store(false, Ordering::SeqCst);

                                let _ = worker_tx.send(WorkerMsg::SendMessage {
                                    user_msg: msg,
                                    session_id,
                                    web_search,
                                    auth_user_id: app
                                        .auth_session
                                        .as_ref()
                                        .map(|session| session.user.user_id.clone()),
                                }).await;
                            }
                        }
                        EventResult::LoadSessions => {
                            if let (Some(base_url), Some(session)) = (
                                app.auth_server_base_url.clone(),
                                app.auth_session.clone(),
                            ) {
                                let mut session = session;
                                match auth::fetch_remote_sessions(&auth_client, &base_url, &mut session).await {
                                    Ok(sessions) => {
                                        app.auth_session = Some(session);
                                        let session_infos = sessions
                                            .into_iter()
                                            .map(|s| super::app::SessionInfo {
                                                id: s.id.clone(),
                                                name: s.title.unwrap_or(s.id),
                                                created_at: s.created_at,
                                            })
                                            .collect();
                                        app.state = AppState::Sessions(session_infos, 0);
                                    }
                                    Err(err) => app.set_error_message(format!("Error loading remote sessions: {}", err)),
                                }
                            } else {
                                match session_service.list_sessions_data() {
                                    Ok(sessions) => {
                                        let session_infos = sessions
                                            .into_iter()
                                            .map(|s| super::app::SessionInfo {
                                                id: s.id.clone(),
                                                name: s.title.unwrap_or(s.id),
                                                created_at: s.created_at,
                                            })
                                            .collect();
                                        app.state = AppState::Sessions(session_infos, 0);
                                    }
                                    Err(e) => app.set_error_message(format!("Error loading sessions: {}", e)),
                                }
                            }
                        }
                        EventResult::OpenSession { session_id, preview } => {
                            if let (Some(base_url), Some(session)) = (
                                app.auth_server_base_url.clone(),
                                app.auth_session.clone(),
                            ) {
                                let mut session = session;
                                match auth::fetch_remote_session_state(&auth_client, &base_url, &mut session, &session_id).await {
                                    Ok(session_view) => {
                                        app.auth_session = Some(session);
                                        if preview {
                                            app.state = AppState::ViewSession(
                                                session_view.session_id,
                                                session_view.messages,
                                                0,
                                            );
                                        } else {
                                            app.state = AppState::Chat(Box::new(events::create_chat_state(
                                                session_view.session_id,
                                                session_view.messages,
                                                session_view.plan,
                                                session_view.agents,
                                                app.agents_context_enabled,
                                            )));
                                            if let AppState::Chat(chat_state) = &mut app.state {
                                                spawn_sidebar_gathering(chat_state, &ui_tx);
                                            }
                                        }
                                    }
                                    Err(err) => app.set_error_message(format!("Error loading remote session: {}", err)),
                                }
                            } else if preview {
                                if let Ok(messages) = session_service.view_session_data(&session_id) {
                                    app.state = AppState::ViewSession(session_id, messages, 0);
                                }
                            } else {
                                match session_service.load_session_state_view(&session_id) {
                                    Ok(session_view) => {
                                        app.state = AppState::Chat(Box::new(events::create_chat_state(
                                            session_view.session_id,
                                            session_view.messages,
                                            session_view.plan,
                                            session_view.agents,
                                            app.agents_context_enabled,
                                        )));
                                        if let AppState::Chat(chat_state) = &mut app.state {
                                            spawn_sidebar_gathering(chat_state, &ui_tx);
                                        }
                                    }
                                    Err(e) => app.set_error_message(format!("Error loading session: {}", e)),
                                }
                            }
                        }
                        EventResult::DeleteSession {
                            session_id,
                            remote,
                            export_view,
                            selected_index,
                        } => {
                            if remote {
                                if let (Some(base_url), Some(session)) = (
                                    app.auth_server_base_url.clone(),
                                    app.auth_session.clone(),
                                ) {
                                    let mut session = session;
                                    match auth::delete_remote_session(
                                        &auth_client,
                                        &base_url,
                                        &mut session,
                                        &session_id,
                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            app.auth_session = Some(session);
                                            app.set_status_message(
                                                "Remote session deleted".to_string(),
                                            );
                                            match auth::fetch_remote_sessions(
                                                &auth_client,
                                                &base_url,
                                                app.auth_session.as_mut().expect("session set"),
                                            )
                                            .await
                                            {
                                                Ok(sessions) => {
                                                    let session_infos = sessions
                                                        .into_iter()
                                                        .map(|s| super::app::SessionInfo {
                                                            id: s.id.clone(),
                                                            name: s.title.unwrap_or(s.id),
                                                            created_at: s.created_at,
                                                        })
                                                        .collect::<Vec<_>>();
                                                    let next_selected = selected_index
                                                        .min(session_infos.len().saturating_sub(1));
                                                    app.state =
                                                        AppState::Sessions(session_infos, next_selected);
                                                }
                                                Err(err) => app.set_error_message(format!(
                                                    "Error reloading remote sessions: {}",
                                                    err
                                                )),
                                            }
                                        }
                                        Err(err) => app.set_error_message(format!(
                                            "Error deleting remote session: {}",
                                            err
                                        )),
                                    }
                                }
                            } else {
                                let delete_result = if export_view {
                                    if let Some(auth_session) = app.auth_session.as_ref() {
                                        session_service.delete_session_for_user(
                                            &session_id,
                                            &auth_session.user.user_id,
                                        )
                                    } else {
                                        session_service.delete_session(&session_id)
                                    }
                                } else {
                                    session_service.delete_session(&session_id)
                                };
                                match delete_result {
                                    Ok(true) => {
                                        app.set_status_message("Local session deleted".to_string());
                                        let sessions_result = if export_view {
                                            if let Some(auth_session) = app.auth_session.as_ref() {
                                                session_service.list_sessions_data_for_user(
                                                    &auth_session.user.user_id,
                                                )
                                            } else {
                                                session_service.list_sessions_data()
                                            }
                                        } else {
                                            session_service.list_sessions_data()
                                        };
                                        match sessions_result {
                                            Ok(sessions) => {
                                                let session_infos = sessions
                                                    .into_iter()
                                                    .map(|s| super::app::SessionInfo {
                                                        id: s.id.clone(),
                                                        name: s.title.unwrap_or(s.id),
                                                        created_at: s.created_at,
                                                    })
                                                    .collect::<Vec<_>>();
                                                let next_selected = selected_index
                                                    .min(session_infos.len().saturating_sub(1));
                                                app.state = if export_view {
                                                    AppState::ExportSessions(
                                                        session_infos,
                                                        next_selected,
                                                    )
                                                } else {
                                                    AppState::Sessions(session_infos, next_selected)
                                                };
                                            }
                                            Err(e) => app.set_error_message(format!(
                                                "Error reloading sessions: {}",
                                                e
                                            )),
                                        }
                                    }
                                    Ok(false) => app.set_error_message(
                                        "Session not found for deletion".to_string(),
                                    ),
                                    Err(err) => app.set_error_message(format!(
                                        "Error deleting local session: {}",
                                        err
                                    )),
                                }
                            }
                        }
                        EventResult::RefreshAuthSession => {
                            if let (Some(base_url), Some(session)) = (
                                app.auth_server_base_url.clone(),
                                app.auth_session.clone(),
                            ) {
                                let mut session = session;
                                match auth::refresh_auth_session(&auth_client, &base_url, &mut session).await {
                                    Ok(()) => {
                                        let status = session
                                            .user
                                            .email
                                            .clone()
                                            .unwrap_or_else(|| session.user.user_id.clone());
                                        app.auth_session = Some(session);
                                        app.set_status_message(format!(
                                            "Auth session refreshed for {}",
                                            status
                                        ));
                                    }
                                    Err(err) => app.set_error_message(format!(
                                        "Auth session refresh failed: {}",
                                        err
                                    )),
                                }
                            } else {
                                app.set_error_message("No signed-in auth session to refresh".to_string());
                            }
                        }
                        EventResult::SaveExecutionPolicy => {
                            match settings::save_execution_policy_settings(
                                settings::ExecPolicySettings {
                                    approval: app.approval_profile,
                                    execution_strategy: app.execution_strategy,
                                    sandbox: app.sandbox_profile,
                                    retry_max_attempts: app.retry_max_attempts,
                                    allowed_commands: &app.allowed_commands,
                                    blocked_commands: &app.blocked_commands,
                                    header_widgets: &app.header_widgets,
                                },
                            ) {
                                Ok(()) => {
                                    if let Ok(mut policy) = ui_exec_policy.lock() {
                                        policy.approval_profile = Some(app.approval_profile);
                                        policy.execution_strategy = Some(app.execution_strategy);
                                        policy.sandbox_profile = Some(app.sandbox_profile);
                                        policy.allowed_commands = if app.allowed_commands.is_empty()
                                        {
                                            None
                                        } else {
                                            Some(app.allowed_commands.clone())
                                        };
                                        policy.blocked_commands = if app.blocked_commands.is_empty()
                                        {
                                            None
                                        } else {
                                            Some(app.blocked_commands.clone())
                                        };
                                        policy.retry_max_attempts = Some(app.retry_max_attempts);
                                    }
                                    app.set_status_message(
                                        "Saved execution policy to config/local.toml".to_string(),
                                    );
                                }
                                Err(err) => app.set_error_message(format!(
                                    "Failed to save execution policy: {}",
                                    err
                                )),
                            }
                        }
                        EventResult::SaveAppearance => {
                            match settings::save_appearance_settings(
                                app.show_menu_logo,
                                app.mouse_capture,
                            ) {
                                Ok(()) => app.set_status_message(
                                    "Saved appearance settings to config/local.toml".to_string(),
                                ),
                                Err(err) => app.set_error_message(format!(
                                    "Failed to save appearance settings: {}",
                                    err
                                )),
                            }
                        }
                        EventResult::CheckForUpdates => {
                            app.set_activity_status(Some("checking updates".to_string()));
                            spawn_update_status_refresh(&ui_tx, true);
                        }
                        EventResult::ConfirmHomebrewPathFix => {
                            if let Some(fix) = crate::update::detect_homebrew_path_fix() {
                                app.set_activity_status(Some(
                                    "waiting Homebrew PATH approval".to_string(),
                                ));
                                spawn_homebrew_path_fix(&ui_tx, &approval_tx, fix);
                            } else {
                                app.homebrew_path_fix = None;
                                app.set_info_message("No Homebrew PATH fix is needed.".to_string());
                            }
                        }
                        EventResult::SetPlanStepStatus {
                            session_id,
                            step_index,
                            status,
                        } => {
                            match harper_core::tools::plan::set_plan_step_status(
                                conn,
                                &session_id,
                                step_index,
                                status,
                            ) {
                                Ok(()) => {
                                    if let AppState::Chat(chat_state) = &mut app.state {
                                        if chat_state.session_id == session_id {
                                            match harper_core::memory::storage::load_plan_state(
                                                conn,
                                                &session_id,
                                            ) {
                                                Ok(plan) => {
                                                    chat_state.active_plan = plan;
                                                    chat_state.refresh_plan_state();
                                                }
                                                Err(err) => app.set_error_message(format!(
                                                    "Failed to reload plan: {}",
                                                    err
                                                )),
                                            }
                                        }
                                    }
                                    app.set_status_message("Plan step updated".to_string());
                                }
                                Err(err) => app
                                    .set_error_message(format!("Failed to update plan step: {}", err)),
                            }
                        }
                        EventResult::ClearPlan { session_id } => {
                            match harper_core::tools::plan::clear_plan_state(conn, &session_id) {
                                Ok(()) => {
                                    if let AppState::Chat(chat_state) = &mut app.state {
                                        if chat_state.session_id == session_id {
                                            chat_state.active_plan = None;
                                            chat_state.refresh_plan_state();
                                        }
                                    }
                                    app.set_status_message("Plan cleared".to_string());
                                }
                                Err(err) => {
                                    app.set_error_message(format!("Failed to clear plan: {}", err))
                                }
                            }
                        }
                        EventResult::RetryPlanFollowup {
                            session_id,
                            command,
                        } => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                if chat_state.session_id == session_id {
                                    chat_state.command_output = None;
                                    chat_state.command_output_expanded = false;
                                    chat_state.command_output_scroll = 0;
                                    chat_state.command_output_selection = None;
                                }
                            }
                            app.set_activity_status(Some(format!("retrying: {}", command)));
                            app.cancel_requested.store(false, Ordering::SeqCst);
                            let _ = worker_tx
                                .send(WorkerMsg::RetryPlanCommand {
                                    command,
                                    session_id,
                                })
                                .await;
                        }
                        EventResult::ClearPlanFollowup { session_id } => {
                            match harper_core::tools::plan::clear_plan_followup(conn, &session_id) {
                                Ok(()) => {
                                    if let AppState::Chat(chat_state) = &mut app.state {
                                        if chat_state.session_id == session_id {
                                            match harper_core::memory::storage::load_plan_state(
                                                conn,
                                                &session_id,
                                            ) {
                                                Ok(plan) => {
                                                    chat_state.active_plan = plan;
                                                    chat_state.refresh_plan_state();
                                                }
                                                Err(err) => app.set_error_message(format!(
                                                    "Failed to reload plan: {}",
                                                    err
                                                )),
                                            }
                                        }
                                    }
                                    app.set_status_message("Planner followup cleared".to_string());
                                }
                                Err(err) => app.set_error_message(format!(
                                    "Failed to clear planner followup: {}",
                                    err
                                )),
                            }
                        }
                        EventResult::RequestPlanReplan {
                            session_id,
                            step_index,
                            step,
                        } => {
                            match harper_core::tools::plan::replan_blocked_step(
                                conn,
                                &session_id,
                                step_index,
                            ) {
                                Ok(()) => {
                                    if let AppState::Chat(chat_state) = &mut app.state {
                                        if chat_state.session_id == session_id {
                                            match harper_core::memory::storage::load_plan_state(
                                                conn,
                                                &session_id,
                                            ) {
                                                Ok(plan) => {
                                                    chat_state.active_plan = plan;
                                                    chat_state.refresh_plan_state();
                                                }
                                                Err(err) => app.set_error_message(format!(
                                                    "Failed to reload plan: {}",
                                                    err
                                                )),
                                            }
                                        }
                                    }
                                    app.set_status_message(format!(
                                        "Planner replan created for: {}",
                                        step
                                    ));
                                }
                                Err(err) => {
                                    app.set_error_message(format!("Failed to replan step: {}", err))
                                }
                            }
                        }
                        EventResult::StartProfileLogin { provider } => {
                            start_tui_auth_login(&mut app, &auth_client, &provider).await;
                        }
                        EventResult::Continue => {}
                        EventResult::GatherSidebarEntries => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                spawn_sidebar_gathering(chat_state, &ui_tx);
                            }
                        }
                    }
                    sync_mouse_capture(
                        terminal.backend_mut(),
                        &mut mouse_capture_enabled,
                        app.mouse_capture,
                    )?;
                }
            }

            // Worker Updates
            update = ui_rx.recv() => {
                if let Some(first) = update {
                    let mut batch = vec![first];
                    while let Ok(next) = ui_rx.try_recv() {
                        coalesce_ui_update(&mut batch, next);
                    }
                    for update in batch {
                    match update {
                        UiUpdate::MessageProcessed(session_view) => {
                            let mut should_clear_activity = false;
                            if let AppState::Chat(chat_state) = &mut app.state {
                                if chat_state.session_id == session_view.session_id {
                                    chat_state.messages = session_view.messages;
                                    chat_state.follow_latest_messages();
                                    chat_state.awaiting_response = false;
                                    chat_state.active_plan = session_view.plan;
                                    chat_state.active_agents = session_view.agents;
                                    chat_state.refresh_plan_state();
                                    chat_state.refresh_review_state();
                                    if chat_state.active_plan.is_none() {
                                        let outcome = final_chat_loop_outcome(
                                            chat_state.loop_state.stage.as_ref(),
                                            chat_state.loop_state.last_outcome.as_ref(),
                                        );
                                        let feedback = if matches!(outcome, PlanLoopOutcome::Responded) {
                                            None
                                        } else {
                                            chat_state
                                                .messages
                                                .iter()
                                                .rev()
                                                .find(|message| message.role == "assistant")
                                                .and_then(|message| {
                                                    message
                                                        .content
                                                        .lines()
                                                        .find(|line| !line.trim().is_empty())
                                                })
                                                .map(str::trim)
                                                .map(ToOwned::to_owned)
                                                .or_else(|| {
                                                    chat_state
                                                        .loop_state
                                                        .last_feedback
                                                        .clone()
                                                })
                                        };
                                        chat_state.record_loop_outcome(outcome, feedback);
                                    }
                                    should_clear_activity = true;
                                    if chat_state.sidebar_visible {
                                        spawn_sidebar_gathering(chat_state, &ui_tx);
                                    }
                                }
                            }
                            if should_clear_activity {
                                app.set_activity_status(None);
                            }
                        }
                        UiUpdate::ActivityUpdated { session_id, status } => {
                            let matches_session = if let AppState::Chat(chat_state) = &mut app.state {
                                if chat_state.session_id == session_id {
                                    if let Some(status_text) = status.as_deref() {
                                        if let Some(stage) =
                                            infer_chat_loop_stage(status_text)
                                        {
                                            chat_state.set_loop_stage(
                                                stage,
                                                Some(status_text.to_string()),
                                            );
                                        } else {
                                            chat_state.loop_state.last_feedback =
                                                Some(status_text.to_string());
                                        }
                                    }
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            if matches_session {
                                app.set_activity_status(status);
                            }
                        }
                        UiUpdate::CommandOutputUpdated {
                            session_id,
                            command,
                            chunk,
                            is_error,
                            done,
                        } => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                if chat_state.session_id == session_id {
                                    let (current_command, has_error_output, is_done) = {
                                        let state = chat_state.command_output.get_or_insert(
                                            CommandOutputState {
                                                command: command.clone(),
                                                content: String::new(),
                                                has_error: false,
                                                done: false,
                                            },
                                        );
                                        if state.command != command {
                                            *state = CommandOutputState {
                                                command,
                                                content: String::new(),
                                                has_error: is_error,
                                                done,
                                            };
                                            chat_state.command_output_selection = None;
                                        }
                                        state.content.push_str(&chunk);
                                        state.has_error |= is_error;
                                        state.done = done;
                                        (state.command.clone(), state.has_error, state.done)
                                    };
                                    chat_state.set_loop_stage(
                                        PlanLoopStage::Executing,
                                        Some(format!("running {}", current_command)),
                                    );
                                    if is_done {
                                        let outcome = if has_error_output {
                                            PlanLoopOutcome::Failed
                                        } else {
                                            PlanLoopOutcome::Succeeded
                                        };
                                        chat_state.record_loop_outcome(
                                            outcome,
                                            Some(format!("{} finished", current_command)),
                                        );
                                    }
                                }
                            }
                        }
                        UiUpdate::PlanUpdated { session_id, active_plan } => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                if chat_state.session_id == session_id {
                                    chat_state.active_plan = active_plan;
                                    chat_state.refresh_plan_state();
                                }
                            }
                        }
                        UiUpdate::AgentsUpdated {
                            session_id,
                            active_agents,
                        } => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                if chat_state.session_id == session_id && app.agents_context_enabled {
                                    chat_state.active_agents = active_agents;
                                }
                            }
                        }
                        UiUpdate::SidebarEntries { sections } => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                chat_state.sidebar_sections = sections;
                            }
                        }
                        UiUpdate::UpdateStatus {
                            status,
                            homebrew_path_fix,
                            manual,
                        } => {
                            app.homebrew_path_fix =
                                homebrew_path_fix.map(|fix| fix.shadow_path);
                            clamp_execution_policy_selection(&mut app);
                            match status {
                                Ok(status) => {
                                    app.update_status = status;
                                    if manual {
                                        app.set_activity_status(None);
                                        if let Some(status) = &app.update_status {
                                            app.set_status_message(format!("Checked {}", status));
                                        } else {
                                            app.set_info_message(
                                                "Update status is unavailable.".to_string(),
                                            );
                                        }
                                    } else if let Some(status) = &app.update_status {
                                        if status != "update: latest"
                                            && status != "update: local newer"
                                        {
                                            app.set_info_message(format!("Startup {}", status));
                                        }
                                    }
                                }
                                Err(err) => {
                                    app.update_status = None;
                                    if manual {
                                        app.set_activity_status(None);
                                        app.set_info_message(format!(
                                            "Update check failed: {}",
                                            err
                                        ));
                                    }
                                }
                            }
                        }
                        UiUpdate::HomebrewPathFixApplied { result } => {
                            app.set_activity_status(None);
                            match result {
                                Ok(message) => {
                                    app.homebrew_path_fix =
                                        crate::update::detect_homebrew_path_fix()
                                            .map(|fix| fix.shadow_path);
                                    clamp_execution_policy_selection(&mut app);
                                    app.set_info_message(message);
                                }
                                Err(err) => app.set_error_message(format!(
                                    "Failed to fix Homebrew PATH: {}",
                                    err
                                )),
                            }
                        }
                        UiUpdate::Error(err) => {
                            if let AppState::Chat(chat_state) = &mut app.state {
                                chat_state.awaiting_response = false;
                                chat_state.record_loop_outcome(
                                    PlanLoopOutcome::Failed,
                                    Some(err.clone()),
                                );
                            }
                            app.set_activity_status(None);
                            app.set_error_message(err);
                        }
                    }
                    }
                }
            }

            // Approval Requests
            approval = approval_rx.recv() => {
                if let Some((prompt, command, tx)) = approval {
                    // Use the new pending_approval overlay instead of replacing state
                    app.set_activity_status(Some(format!("waiting approval: {}", command)));
                    app.pending_approval = Some(ApprovalState {
                        prompt,
                        command,
                        tx,
                        scroll_offset: 0,
                    });
                }
            }
        }

        if let (Some(base_url), Some(flow_id)) =
            (app.auth_server_base_url.clone(), app.auth_flow_id.clone())
        {
            let should_poll = app
                .auth_last_poll_at
                .is_none_or(|last| last.elapsed() >= std::time::Duration::from_millis(750));
            if should_poll {
                app.auth_last_poll_at = Some(std::time::Instant::now());
                match auth::poll_tui_auth_flow(&auth_client, &base_url, &flow_id).await {
                    Ok(flow) if flow.status == "complete" => {
                        if let Some(session) = flow.session {
                            match auth::save_auth_session(&session) {
                                Ok(_) => {
                                    let status = session
                                        .user
                                        .email
                                        .clone()
                                        .unwrap_or_else(|| session.user.user_id.clone());
                                    app.auth_session = Some(session);
                                    app.auth_flow_id = None;
                                    app.auth_last_poll_at = None;
                                    app.set_activity_status(None);
                                    app.set_status_message(format!("Signed in as {}", status));
                                }
                                Err(err) => app.set_error_message(format!(
                                    "Failed to save TUI auth session: {}",
                                    err
                                )),
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        app.auth_flow_id = None;
                        app.auth_last_poll_at = None;
                        app.set_activity_status(None);
                        app.set_error_message(format!("Auth polling failed: {}", err));
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    if mouse_capture_enabled {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen,
        cursor::Show
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        display_command_info, final_chat_loop_outcome, infer_chat_loop_stage, sync_mouse_capture,
    };
    use crate::interfaces::ui::app::{MessageType, TuiApp};
    use harper_core::core::plan::{PlanLoopOutcome, PlanLoopStage};
    use ratatui::backend::CrosstermBackend;

    #[test]
    fn summarizing_maps_to_feedback_stage() {
        assert_eq!(
            infer_chat_loop_stage("summarizing result"),
            Some(PlanLoopStage::Feedback)
        );
    }

    #[test]
    fn deterministic_non_plan_completion_maps_to_succeeded() {
        assert_eq!(
            final_chat_loop_outcome(Some(&PlanLoopStage::Feedback), None),
            PlanLoopOutcome::Succeeded
        );
        assert_eq!(
            final_chat_loop_outcome(Some(&PlanLoopStage::Responding), None),
            PlanLoopOutcome::Responded
        );
    }

    #[test]
    fn native_shell_help_uses_persistent_info_message() {
        let mut app = TuiApp::new();

        display_command_info(&mut app, "Help".to_string());

        let message = app.message.as_ref().expect("help message");
        assert!(matches!(message.message_type, MessageType::Info));
        assert!(message.expires_at.is_none());
    }

    #[test]
    fn command_info_messages_do_not_expire() {
        let mut app = TuiApp::new();

        display_command_info(&mut app, "Current execution strategy: auto".to_string());

        let message = app.message.as_ref().expect("command info message");
        assert!(matches!(message.message_type, MessageType::Info));
        assert!(message.expires_at.is_none());
    }

    #[test]
    fn mouse_capture_sync_tracks_terminal_state() {
        let mut backend = CrosstermBackend::new(Vec::<u8>::new());
        let mut enabled = false;

        sync_mouse_capture(&mut backend, &mut enabled, true).expect("enable mouse capture");
        assert!(enabled);

        sync_mouse_capture(&mut backend, &mut enabled, true).expect("keep mouse capture enabled");
        assert!(enabled);

        sync_mouse_capture(&mut backend, &mut enabled, false).expect("disable mouse capture");
        assert!(!enabled);
    }

    #[tokio::test]
    async fn wait_for_cancel_returns_immediately_when_already_requested() {
        use super::wait_for_cancel;
        use std::sync::atomic::{AtomicBool, Ordering};

        let cancel = AtomicBool::new(true);
        wait_for_cancel(&cancel).await;
        assert!(cancel.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn finalize_worker_cancellation_marks_loop_stage_interrupted() {
        use super::{UiUpdate, finalize_worker_cancellation};

        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        harper_core::memory::storage::init_db(&conn).expect("init db");
        let args = serde_json::json!({
            "items": [{"step": "Inspect files", "status": "in_progress"}]
        });
        harper_core::tools::plan::update_plan(&conn, "cancel-session", &args).expect("seed plan");

        let (ui_tx, mut ui_rx) = tokio::sync::mpsc::channel::<UiUpdate>(1);
        finalize_worker_cancellation(&conn, "cancel-session", None, &ui_tx).await;

        let plan = harper_core::memory::storage::load_plan_state(&conn, "cancel-session")
            .expect("load plan")
            .expect("plan present");
        let runtime = plan.runtime.expect("runtime present");
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Interrupted));
        assert_eq!(runtime.last_feedback.as_deref(), Some("cancelled by user"));
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::Cancelled));

        match ui_rx.try_recv() {
            Ok(UiUpdate::MessageProcessed(view)) => {
                assert_eq!(view.session_id, "cancel-session");
            }
            Ok(_) => panic!("expected MessageProcessed update"),
            Err(err) => panic!("expected MessageProcessed update, got {err}"),
        }
    }

    #[test]
    fn coalesce_ui_update_keeps_last_consecutive_activity_update() {
        use super::{UiUpdate, coalesce_ui_update};

        let mut batch = vec![UiUpdate::ActivityUpdated {
            session_id: "s".to_string(),
            status: Some("thinking".to_string()),
        }];
        coalesce_ui_update(
            &mut batch,
            UiUpdate::ActivityUpdated {
                session_id: "s".to_string(),
                status: Some("running cargo test".to_string()),
            },
        );

        assert_eq!(batch.len(), 1);
        match &batch[0] {
            UiUpdate::ActivityUpdated { status, .. } => {
                assert_eq!(status.as_deref(), Some("running cargo test"));
            }
            _ => panic!("expected ActivityUpdated"),
        }
    }

    #[test]
    fn coalesce_ui_update_merges_consecutive_command_output_chunks() {
        use super::{UiUpdate, coalesce_ui_update};

        let mut batch = vec![UiUpdate::CommandOutputUpdated {
            session_id: "s".to_string(),
            command: "cargo test".to_string(),
            chunk: "line 1\n".to_string(),
            is_error: false,
            done: false,
        }];
        coalesce_ui_update(
            &mut batch,
            UiUpdate::CommandOutputUpdated {
                session_id: "s".to_string(),
                command: "cargo test".to_string(),
                chunk: "line 2\n".to_string(),
                is_error: true,
                done: true,
            },
        );

        assert_eq!(batch.len(), 1);
        match &batch[0] {
            UiUpdate::CommandOutputUpdated {
                chunk,
                is_error,
                done,
                ..
            } => {
                assert_eq!(chunk, "line 1\nline 2\n");
                assert!(*is_error);
                assert!(*done);
            }
            _ => panic!("expected CommandOutputUpdated"),
        }
    }

    #[test]
    fn coalesce_ui_update_keeps_distinct_update_types_and_errors() {
        use super::{UiUpdate, coalesce_ui_update};

        let mut batch = vec![UiUpdate::ActivityUpdated {
            session_id: "s".to_string(),
            status: Some("thinking".to_string()),
        }];
        coalesce_ui_update(&mut batch, UiUpdate::Error("first".to_string()));
        coalesce_ui_update(&mut batch, UiUpdate::Error("second".to_string()));
        assert_eq!(batch.len(), 3);

        let mut split_commands = vec![UiUpdate::CommandOutputUpdated {
            session_id: "s".to_string(),
            command: "cargo test".to_string(),
            chunk: "a".to_string(),
            is_error: false,
            done: false,
        }];
        coalesce_ui_update(
            &mut split_commands,
            UiUpdate::CommandOutputUpdated {
                session_id: "s".to_string(),
                command: "cargo fmt".to_string(),
                chunk: "b".to_string(),
                is_error: false,
                done: false,
            },
        );
        assert_eq!(split_commands.len(), 2);
    }
}
