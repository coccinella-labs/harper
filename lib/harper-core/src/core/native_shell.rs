// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::core::error::{HarperError, HarperResult};
use crate::core::plan::{PlanItem, PlanState, PlanStepStatus};
use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeShellCommand {
    Ask(String),
    Auth(AuthShellCommand),
    Config(ConfigShellCommand),
    Help,
    History(HistoryShellCommand),
    Session(SessionShellCommand),
    Status,
    Update(UpdateShellCommand),
    Run(String),
    Plan(PlanShellCommand),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeShellContext {
    pub auth: Option<AuthShellContext>,
    pub config: Option<ConfigShellContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthShellContext {
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigShellContext {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub database_path: String,
    pub approval: String,
    pub strategy: String,
    pub sandbox: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthShellCommand {
    Login { provider: String },
    Status,
    Logout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigShellCommand {
    Show,
    Set { key: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryShellCommand {
    Show(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionShellCommand {
    List,
    Show(String),
    Open(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateShellCommand {
    Apply,
    Check,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanShellCommand {
    Show,
    Add(String),
    Done(usize),
    Start(usize),
    Block {
        index: usize,
        reason: Option<String>,
    },
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeShellOutcome {
    Handled(String),
    Ask(String),
    Run(String),
    UpdateCheck,
    UpdateApply,
    UpdateStatus,
    ConfigSet { key: String, value: String },
    AuthLogin { provider: String },
    AuthLogout,
    OpenSession { target: String, preview: bool },
}

pub fn parse_native_shell_command(input: &str) -> HarperResult<Option<NativeShellCommand>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let has_slash = trimmed.starts_with('/');
    let normalized = trimmed.strip_prefix('/').unwrap_or(trimmed);
    let tokens = tokenize(normalized)?;
    let Some(command) = tokens.first().map(|token| token.as_str()) else {
        return Ok(None);
    };

    match command {
        "help" if has_slash || tokens.len() == 1 => Ok(Some(NativeShellCommand::Help)),
        "status" if has_slash || tokens.len() == 1 => Ok(Some(NativeShellCommand::Status)),
        "auth" => parse_auth_command(&tokens, has_slash)
            .map(|command| command.map(NativeShellCommand::Auth)),
        "config" => parse_config_command(&tokens, has_slash)
            .map(|command| command.map(NativeShellCommand::Config)),
        "history" => parse_history_command(&tokens, has_slash)
            .map(|command| command.map(NativeShellCommand::History)),
        "session" | "sessions" => parse_session_command(&tokens, has_slash)
            .map(|command| command.map(NativeShellCommand::Session)),
        "update" => parse_update_command(&tokens, has_slash)
            .map(|command| command.map(NativeShellCommand::Update)),
        "ask" if has_slash || looks_like_quoted_command_arg(normalized, "ask") => {
            let prompt = rest_after_command(normalized, "ask");
            if prompt.is_empty() {
                return Err(HarperError::Validation(
                    "ask requires a message".to_string(),
                ));
            }
            Ok(Some(NativeShellCommand::Ask(unquote_if_wrapped(prompt))))
        }
        "run" if has_slash || looks_like_shell_command_arg(normalized, "run") => {
            let command = rest_after_command(normalized, "run");
            if command.is_empty() {
                return Err(HarperError::Validation(
                    "run requires a command".to_string(),
                ));
            }
            Ok(Some(NativeShellCommand::Run(command.to_string())))
        }
        "plan" => parse_plan_command(&tokens, has_slash)
            .map(|command| command.map(NativeShellCommand::Plan)),
        _ => Ok(None),
    }
}

pub fn execute_native_shell_command(
    conn: &Connection,
    session_id: &str,
    command: NativeShellCommand,
) -> HarperResult<NativeShellOutcome> {
    execute_native_shell_command_with_context(
        conn,
        session_id,
        command,
        &NativeShellContext::default(),
    )
}

pub fn execute_native_shell_command_with_context(
    conn: &Connection,
    session_id: &str,
    command: NativeShellCommand,
    context: &NativeShellContext,
) -> HarperResult<NativeShellOutcome> {
    match command {
        NativeShellCommand::Ask(prompt) => Ok(NativeShellOutcome::Ask(prompt)),
        NativeShellCommand::Run(command) => Ok(NativeShellOutcome::Run(command)),
        NativeShellCommand::Config(ConfigShellCommand::Set { key, value }) => {
            Ok(NativeShellOutcome::ConfigSet { key, value })
        }
        NativeShellCommand::Update(UpdateShellCommand::Apply) => {
            Ok(NativeShellOutcome::UpdateApply)
        }
        NativeShellCommand::Update(UpdateShellCommand::Check) => {
            Ok(NativeShellOutcome::UpdateCheck)
        }
        NativeShellCommand::Update(UpdateShellCommand::Status) => {
            Ok(NativeShellOutcome::UpdateStatus)
        }
        NativeShellCommand::Auth(AuthShellCommand::Login { provider }) => {
            Ok(NativeShellOutcome::AuthLogin { provider })
        }
        NativeShellCommand::Auth(AuthShellCommand::Logout) => Ok(NativeShellOutcome::AuthLogout),
        NativeShellCommand::Auth(AuthShellCommand::Status) => Ok(NativeShellOutcome::Handled(
            context
                .auth
                .as_ref()
                .map(|auth| format!("Auth status: {}", auth.status))
                .unwrap_or_else(|| "Auth status: not signed in".to_string()),
        )),
        NativeShellCommand::Config(ConfigShellCommand::Show) => Ok(NativeShellOutcome::Handled(
            format_config_context(context.config.as_ref()),
        )),
        NativeShellCommand::History(HistoryShellCommand::Show(target)) => {
            let target_session_id = match target {
                Some(target) => resolve_session_target(conn, &target)?,
                None => session_id.to_string(),
            };
            Ok(NativeShellOutcome::Handled(format_history(
                conn,
                &target_session_id,
            )?))
        }
        NativeShellCommand::Session(SessionShellCommand::List) => {
            Ok(NativeShellOutcome::Handled(format_sessions(conn)?))
        }
        NativeShellCommand::Session(SessionShellCommand::Show(target)) => {
            Ok(NativeShellOutcome::OpenSession {
                target,
                preview: true,
            })
        }
        NativeShellCommand::Session(SessionShellCommand::Open(target)) => {
            Ok(NativeShellOutcome::OpenSession {
                target,
                preview: false,
            })
        }
        NativeShellCommand::Help => Ok(NativeShellOutcome::Handled(help_text())),
        NativeShellCommand::Status => Ok(NativeShellOutcome::Handled(
            "Harper native shell is available. Use help for commands.".to_string(),
        )),
        NativeShellCommand::Plan(command) => {
            execute_plan_command(conn, session_id, command).map(NativeShellOutcome::Handled)
        }
    }
}

fn parse_auth_command(tokens: &[String], strict: bool) -> HarperResult<Option<AuthShellCommand>> {
    match tokens
        .get(1)
        .map(|token| token.as_str())
        .unwrap_or("status")
    {
        "login" => Ok(Some(AuthShellCommand::Login {
            provider: tokens
                .get(2)
                .cloned()
                .unwrap_or_else(|| "github".to_string()),
        })),
        "status" => Ok(Some(AuthShellCommand::Status)),
        "logout" => Ok(Some(AuthShellCommand::Logout)),
        _ if !strict => Ok(None),
        subcommand => Err(HarperError::Validation(format!(
            "unknown auth command '{}'",
            subcommand
        ))),
    }
}

fn parse_config_command(
    tokens: &[String],
    strict: bool,
) -> HarperResult<Option<ConfigShellCommand>> {
    match tokens.get(1).map(|token| token.as_str()).unwrap_or("show") {
        "show" => Ok(Some(ConfigShellCommand::Show)),
        "set" => {
            let key = tokens
                .get(2)
                .cloned()
                .ok_or_else(|| HarperError::Validation("config set requires a key".to_string()))?;
            let value = join_args(&tokens[3..]);
            if value.is_empty() {
                return Err(HarperError::Validation(
                    "config set requires a value".to_string(),
                ));
            }
            Ok(Some(ConfigShellCommand::Set { key, value }))
        }
        _ if !strict => Ok(None),
        subcommand => Err(HarperError::Validation(format!(
            "unknown config command '{}'",
            subcommand
        ))),
    }
}

fn parse_history_command(
    tokens: &[String],
    strict: bool,
) -> HarperResult<Option<HistoryShellCommand>> {
    match tokens.get(1).map(|token| token.as_str()).unwrap_or("show") {
        "show" | "list" => Ok(Some(HistoryShellCommand::Show(tokens.get(2).cloned()))),
        _ if !strict => Ok(None),
        subcommand => Err(HarperError::Validation(format!(
            "unknown history command '{}'",
            subcommand
        ))),
    }
}

fn parse_session_command(
    tokens: &[String],
    strict: bool,
) -> HarperResult<Option<SessionShellCommand>> {
    match tokens.get(1).map(|token| token.as_str()).unwrap_or("list") {
        "list" | "ls" => Ok(Some(SessionShellCommand::List)),
        "show" | "view" => {
            let target = tokens.get(2).cloned().ok_or_else(|| {
                HarperError::Validation("session show requires a session number or id".to_string())
            })?;
            Ok(Some(SessionShellCommand::Show(target)))
        }
        "open" => {
            let target = tokens.get(2).cloned().ok_or_else(|| {
                HarperError::Validation("session open requires a session number or id".to_string())
            })?;
            Ok(Some(SessionShellCommand::Open(target)))
        }
        _ if !strict => Ok(None),
        subcommand => Err(HarperError::Validation(format!(
            "unknown session command '{}'",
            subcommand
        ))),
    }
}

fn parse_update_command(
    tokens: &[String],
    strict: bool,
) -> HarperResult<Option<UpdateShellCommand>> {
    match tokens
        .get(1)
        .map(|token| token.as_str())
        .unwrap_or("status")
    {
        "apply" => Ok(Some(UpdateShellCommand::Apply)),
        "check" => Ok(Some(UpdateShellCommand::Check)),
        "status" => Ok(Some(UpdateShellCommand::Status)),
        _ if !strict => Ok(None),
        subcommand => Err(HarperError::Validation(format!(
            "unknown update command '{}'",
            subcommand
        ))),
    }
}

fn parse_plan_command(tokens: &[String], strict: bool) -> HarperResult<Option<PlanShellCommand>> {
    let Some(subcommand) = tokens.get(1).map(|token| token.as_str()) else {
        return Ok(Some(PlanShellCommand::Show));
    };

    let command = match subcommand {
        "show" | "list" | "ls" => PlanShellCommand::Show,
        "add" => {
            let step = join_args(&tokens[2..]);
            if step.trim().is_empty() {
                return Err(HarperError::Validation(
                    "plan add requires a step".to_string(),
                ));
            }
            PlanShellCommand::Add(step)
        }
        "done" => PlanShellCommand::Done(parse_one_based_index(tokens.get(2), "plan done")?),
        "start" => PlanShellCommand::Start(parse_one_based_index(tokens.get(2), "plan start")?),
        "block" => {
            let index = parse_one_based_index(tokens.get(2), "plan block")?;
            let reason = (!tokens[3..].is_empty()).then(|| join_args(&tokens[3..]));
            PlanShellCommand::Block { index, reason }
        }
        "clear" => PlanShellCommand::Clear,
        _ if !strict => return Ok(None),
        _ => {
            return Err(HarperError::Validation(format!(
                "unknown plan command '{}'",
                subcommand
            )));
        }
    };
    Ok(Some(command))
}

fn execute_plan_command(
    conn: &Connection,
    session_id: &str,
    command: PlanShellCommand,
) -> HarperResult<String> {
    match command {
        PlanShellCommand::Show => Ok(format_plan_state(
            crate::memory::storage::load_plan_state(conn, session_id)?.as_ref(),
        )),
        PlanShellCommand::Add(step) => {
            let mut plan = crate::memory::storage::load_plan_state(conn, session_id)?
                .unwrap_or_else(|| PlanState {
                    explanation: Some("Updated from Harper native shell".to_string()),
                    items: Vec::new(),
                    runtime: None,
                    updated_at: None,
                });
            plan.items.push(PlanItem {
                step,
                status: PlanStepStatus::Pending,
                job_id: None,
            });
            crate::memory::storage::save_plan_state(conn, session_id, &plan)?;
            Ok(format!("Plan updated: {} steps.", plan.items.len()))
        }
        PlanShellCommand::Done(index) => {
            ensure_plan_step_exists(conn, session_id, index)?;
            crate::tools::plan::set_plan_step_status(
                conn,
                session_id,
                index,
                PlanStepStatus::Completed,
            )?;
            Ok(format!("Plan step {} marked completed.", index + 1))
        }
        PlanShellCommand::Start(index) => {
            ensure_plan_step_exists(conn, session_id, index)?;
            crate::tools::plan::set_plan_step_status(
                conn,
                session_id,
                index,
                PlanStepStatus::InProgress,
            )?;
            Ok(format!("Plan step {} marked in progress.", index + 1))
        }
        PlanShellCommand::Block { index, reason } => {
            ensure_plan_step_exists(conn, session_id, index)?;
            crate::tools::plan::set_plan_step_status(
                conn,
                session_id,
                index,
                PlanStepStatus::Blocked,
            )?;
            match reason {
                Some(reason) => Ok(format!("Plan step {} blocked: {}", index + 1, reason)),
                None => Ok(format!("Plan step {} marked blocked.", index + 1)),
            }
        }
        PlanShellCommand::Clear => {
            crate::tools::plan::clear_plan_state(conn, session_id)?;
            Ok("Plan cleared.".to_string())
        }
    }
}

fn ensure_plan_step_exists(conn: &Connection, session_id: &str, index: usize) -> HarperResult<()> {
    let Some(plan) = crate::memory::storage::load_plan_state(conn, session_id)? else {
        return Err(HarperError::Validation("No active plan.".to_string()));
    };
    if index >= plan.items.len() {
        return Err(HarperError::Validation(format!(
            "plan step {} is out of bounds",
            index + 1
        )));
    }
    Ok(())
}

fn format_plan_state(plan: Option<&PlanState>) -> String {
    let Some(plan) = plan else {
        return "No active plan.".to_string();
    };
    if plan.items.is_empty() {
        return "Plan is empty.".to_string();
    }

    let mut lines = Vec::new();
    if let Some(explanation) = plan
        .explanation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(explanation.to_string());
    }
    for (index, item) in plan.items.iter().enumerate() {
        lines.push(format!(
            "{}. [{}] {}",
            index + 1,
            status_label(item.status),
            item.step
        ));
    }
    lines.join("\n")
}

fn format_sessions(conn: &Connection) -> HarperResult<String> {
    let sessions =
        crate::memory::session_service::SessionService::new(conn).list_sessions_data()?;
    if sessions.is_empty() {
        return Ok("No previous sessions found.".to_string());
    }

    let mut lines = vec!["Sessions:".to_string()];
    for (index, session) in sessions.iter().take(20).enumerate() {
        let title = session
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&session.id);
        lines.push(format!("{}. {} ({})", index + 1, title, session.created_at));
    }
    if sessions.len() > 20 {
        lines.push(format!("{} more sessions", sessions.len() - 20));
    }
    Ok(lines.join("\n"))
}

pub fn resolve_session_target(conn: &Connection, target: &str) -> HarperResult<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err(HarperError::Validation(
            "session target cannot be empty".to_string(),
        ));
    }

    if let Ok(index) = trimmed.parse::<usize>() {
        let zero_based = index
            .checked_sub(1)
            .ok_or_else(|| HarperError::Validation("session numbers start at 1".to_string()))?;
        let sessions =
            crate::memory::session_service::SessionService::new(conn).list_sessions_data()?;
        let session = sessions.get(zero_based).ok_or_else(|| {
            HarperError::Validation(format!("session {} is out of bounds", index))
        })?;
        return Ok(session.id.clone());
    }

    Ok(trimmed.to_string())
}

fn format_history(conn: &Connection, session_id: &str) -> HarperResult<String> {
    let history = crate::memory::storage::load_history(conn, session_id)?;
    if history.is_empty() {
        return Ok("No history for this session yet.".to_string());
    }

    let mut lines = vec!["History:".to_string()];
    for (index, message) in history.iter().rev().take(10).enumerate() {
        let preview = message
            .content
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect::<String>();
        lines.push(format!("{}. {}: {}", index + 1, message.role, preview));
    }
    Ok(lines.join("\n"))
}

fn format_config_context(config: Option<&ConfigShellContext>) -> String {
    let Some(config) = config else {
        return "Config summary unavailable.".to_string();
    };
    [
        "Config:".to_string(),
        format!("provider: {}", config.provider),
        format!("model: {}", config.model),
        format!("base_url: {}", config.base_url),
        format!("database: {}", config.database_path),
        format!("approval: {}", config.approval),
        format!("strategy: {}", config.strategy),
        format!("sandbox: {}", config.sandbox),
    ]
    .join("\n")
}

fn status_label(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "pending",
        PlanStepStatus::InProgress => "in_progress",
        PlanStepStatus::Completed => "completed",
        PlanStepStatus::Blocked => "blocked",
    }
}

fn help_text() -> String {
    [
        "Harper native shell commands:",
        "  ask \"message\"",
        "  plan show",
        "  plan list",
        "  plan add \"step\"",
        "  plan start <number>",
        "  plan done <number>",
        "  plan block <number> \"reason\"",
        "  plan clear",
        "  session list",
        "  session ls",
        "  session show <number|id>",
        "  session open <number|id>",
        "  history show [number|id]",
        "  history list [number|id]",
        "  auth status",
        "  auth login [provider]",
        "  auth logout",
        "  config show",
        "  config set approval|strategy|sandbox|retries <value>",
        "  update check",
        "  update apply",
        "  status",
        "  run <command>",
        "",
        "Existing slash commands also remain available:",
        "  /strategy auto|grounded|deterministic|model",
        "  /agents on|off|status",
        "  /audit [limit]",
        "  /clear",
        "  /exit",
    ]
    .join("\n")
}

fn parse_one_based_index(value: Option<&String>, command: &str) -> HarperResult<usize> {
    let raw = value
        .ok_or_else(|| HarperError::Validation(format!("{} requires a step number", command)))?;
    let parsed = raw.parse::<usize>().map_err(|_| {
        HarperError::Validation(format!("{} requires a numeric step number", command))
    })?;
    parsed
        .checked_sub(1)
        .ok_or_else(|| HarperError::Validation("plan step numbers start at 1".to_string()))
}

fn tokenize(input: &str) -> HarperResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(active), current_ch) if current_ch == active => quote = None,
            (Some(_), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (Some(_), current_ch) => current.push(current_ch),
            (None, '"' | '\'') => quote = Some(ch),
            (None, current_ch) if current_ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            (None, current_ch) => current.push(current_ch),
        }
    }

    if quote.is_some() {
        return Err(HarperError::Validation(
            "unterminated quoted string".to_string(),
        ));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn join_args(args: &[String]) -> String {
    args.join(" ").trim().to_string()
}

fn rest_after_command<'a>(input: &'a str, command: &str) -> &'a str {
    input.strip_prefix(command).unwrap_or(input).trim_start()
}

fn looks_like_quoted_command_arg(input: &str, command: &str) -> bool {
    let arg = rest_after_command(input, command).trim_start();
    arg.starts_with('"') || arg.starts_with('\'')
}

fn looks_like_shell_command_arg(input: &str, command: &str) -> bool {
    let first_arg = rest_after_command(input, command)
        .split_whitespace()
        .next()
        .unwrap_or_default();

    if first_arg.is_empty() {
        return true;
    }

    first_arg.contains('/')
        || first_arg.starts_with('.')
        || matches!(
            first_arg,
            "awk"
                | "bash"
                | "cargo"
                | "cat"
                | "cd"
                | "cp"
                | "curl"
                | "echo"
                | "find"
                | "git"
                | "grep"
                | "ls"
                | "make"
                | "mkdir"
                | "node"
                | "npm"
                | "pnpm"
                | "pwd"
                | "rg"
                | "sed"
                | "sh"
                | "touch"
                | "yarn"
        )
}

fn unquote_if_wrapped(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open db");
        crate::memory::storage::init_db(&conn).expect("init db");
        conn
    }

    #[test]
    fn parses_plan_add_with_quoted_step() {
        let parsed = parse_native_shell_command(r#"plan add "Inspect files""#)
            .expect("parse")
            .expect("command");
        assert_eq!(
            parsed,
            NativeShellCommand::Plan(PlanShellCommand::Add("Inspect files".to_string()))
        );
    }

    #[test]
    fn parses_slash_plan_done_as_one_based_index() {
        let parsed = parse_native_shell_command("/plan done 2")
            .expect("parse")
            .expect("command");
        assert_eq!(parsed, NativeShellCommand::Plan(PlanShellCommand::Done(1)));
        assert_eq!(
            parse_native_shell_command("plan list")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Plan(PlanShellCommand::Show)
        );
        assert_eq!(
            parse_native_shell_command("plan ls")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Plan(PlanShellCommand::Show)
        );
    }

    #[test]
    fn parses_read_only_service_commands() {
        assert_eq!(
            parse_native_shell_command("session list")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Session(SessionShellCommand::List)
        );
        assert_eq!(
            parse_native_shell_command("session ls")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Session(SessionShellCommand::List)
        );
        assert_eq!(
            parse_native_shell_command("history show")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::History(HistoryShellCommand::Show(None))
        );
        assert_eq!(
            parse_native_shell_command("history show 2")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::History(HistoryShellCommand::Show(Some("2".to_string())))
        );
        assert_eq!(
            parse_native_shell_command("session open 2")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Session(SessionShellCommand::Open("2".to_string()))
        );
        assert_eq!(
            parse_native_shell_command("session show abc")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Session(SessionShellCommand::Show("abc".to_string()))
        );
        assert_eq!(
            parse_native_shell_command("history list")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::History(HistoryShellCommand::Show(None))
        );
        assert_eq!(
            parse_native_shell_command("auth login")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Auth(AuthShellCommand::Login {
                provider: "github".to_string()
            })
        );
        assert_eq!(
            parse_native_shell_command("auth login google")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Auth(AuthShellCommand::Login {
                provider: "google".to_string()
            })
        );
        assert_eq!(
            parse_native_shell_command("auth status")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Auth(AuthShellCommand::Status)
        );
        assert_eq!(
            parse_native_shell_command("auth logout")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Auth(AuthShellCommand::Logout)
        );
        assert_eq!(
            parse_native_shell_command("config show")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Config(ConfigShellCommand::Show)
        );
        assert_eq!(
            parse_native_shell_command("config set strategy deterministic")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Config(ConfigShellCommand::Set {
                key: "strategy".to_string(),
                value: "deterministic".to_string()
            })
        );
        assert_eq!(
            parse_native_shell_command("/config set retries 2")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Config(ConfigShellCommand::Set {
                key: "retries".to_string(),
                value: "2".to_string()
            })
        );
        assert_eq!(
            parse_native_shell_command("update check")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Update(UpdateShellCommand::Check)
        );
        assert_eq!(
            parse_native_shell_command("update apply")
                .expect("parse")
                .expect("command"),
            NativeShellCommand::Update(UpdateShellCommand::Apply)
        );
    }

    #[test]
    fn help_mentions_native_and_slash_commands() {
        let output =
            execute_native_shell_command(&setup_conn(), "session-a", NativeShellCommand::Help)
                .expect("help");

        assert!(
            matches!(output, NativeShellOutcome::Handled(text) if text.contains("auth login [provider]") && text.contains("/strategy auto|grounded|deterministic|model"))
        );
    }

    #[test]
    fn execution_returns_structured_outcomes_for_routed_commands() {
        let conn = setup_conn();

        assert_eq!(
            execute_native_shell_command(
                &conn,
                "session-a",
                NativeShellCommand::Auth(AuthShellCommand::Login {
                    provider: "github".to_string()
                })
            )
            .expect("auth login"),
            NativeShellOutcome::AuthLogin {
                provider: "github".to_string()
            }
        );
        assert_eq!(
            execute_native_shell_command(
                &conn,
                "session-a",
                NativeShellCommand::Run("pwd".to_string())
            )
            .expect("run"),
            NativeShellOutcome::Run("pwd".to_string())
        );
        assert_eq!(
            execute_native_shell_command(
                &conn,
                "session-a",
                NativeShellCommand::Config(ConfigShellCommand::Set {
                    key: "strategy".to_string(),
                    value: "deterministic".to_string()
                })
            )
            .expect("config set"),
            NativeShellOutcome::ConfigSet {
                key: "strategy".to_string(),
                value: "deterministic".to_string()
            }
        );
    }

    #[test]
    fn unknown_text_is_not_a_shell_command() {
        assert_eq!(
            parse_native_shell_command("please inspect this repo").expect("parse"),
            None
        );
        assert_eq!(
            parse_native_shell_command("help me debug this").expect("parse"),
            None
        );
        assert_eq!(
            parse_native_shell_command("plan the migration").expect("parse"),
            None
        );
        assert_eq!(
            parse_native_shell_command("run the tests").expect("parse"),
            None
        );
        assert_eq!(
            parse_native_shell_command("ask how updates work").expect("parse"),
            None
        );
    }

    #[test]
    fn slash_commands_remain_strict() {
        let err = parse_native_shell_command("/plan the migration").expect_err("strict slash");
        assert!(err.to_string().contains("unknown plan command 'the'"));
        assert!(
            parse_native_shell_command("/ask how updates work")
                .expect("slash ask parses")
                .is_some()
        );
        assert!(
            parse_native_shell_command("/run the tests")
                .expect("slash run parses")
                .is_some()
        );
    }

    #[test]
    fn update_status_returns_structured_outcome() {
        let conn = setup_conn();
        let output = execute_native_shell_command(
            &conn,
            "session-a",
            NativeShellCommand::Update(UpdateShellCommand::Status),
        )
        .expect("update status");

        assert_eq!(output, NativeShellOutcome::UpdateStatus);
    }

    #[test]
    fn plan_commands_update_persisted_state() {
        let conn = setup_conn();
        execute_native_shell_command(
            &conn,
            "session-a",
            NativeShellCommand::Plan(PlanShellCommand::Add("Inspect files".to_string())),
        )
        .expect("add step");
        execute_native_shell_command(
            &conn,
            "session-a",
            NativeShellCommand::Plan(PlanShellCommand::Start(0)),
        )
        .expect("start step");

        let output = execute_native_shell_command(
            &conn,
            "session-a",
            NativeShellCommand::Plan(PlanShellCommand::Show),
        )
        .expect("show");
        assert!(
            matches!(output, NativeShellOutcome::Handled(text) if text.contains("[in_progress] Inspect files"))
        );
    }

    #[test]
    fn plan_done_requires_existing_plan() {
        let conn = setup_conn();
        let err = execute_native_shell_command(
            &conn,
            "session-a",
            NativeShellCommand::Plan(PlanShellCommand::Done(0)),
        )
        .expect_err("missing plan should fail");

        assert!(err.to_string().contains("No active plan"));
    }
}
