// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_core::{ApprovalProfile, ExecutionStrategy, SandboxProfile};

use super::app::HeaderWidget;
use std::fs;
use std::io;
use std::path::Path;

pub struct ExecPolicySettings<'a> {
    pub approval: ApprovalProfile,
    pub execution_strategy: ExecutionStrategy,
    pub sandbox: SandboxProfile,
    pub retry_max_attempts: u32,
    pub allowed_commands: &'a [String],
    pub blocked_commands: &'a [String],
    pub header_widgets: &'a [HeaderWidget],
}

pub fn save_execution_policy_settings(settings: ExecPolicySettings<'_>) -> io::Result<()> {
    let path = Path::new("config/local.toml");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = upsert_exec_policy_settings(&existing, &settings);
    fs::write(path, updated)
}

pub fn save_appearance_settings(show_menu_logo: bool, mouse_capture: bool) -> io::Result<()> {
    let path = Path::new("config/local.toml");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = upsert_ui_setting(
        &existing,
        "show_menu_logo",
        &format!("show_menu_logo = {show_menu_logo}"),
    );
    let updated = upsert_ui_setting(
        &updated,
        "mouse_capture",
        &format!("mouse_capture = {mouse_capture}"),
    );
    fs::write(path, updated)
}

fn upsert_ui_setting(input: &str, key: &str, line: &str) -> String {
    let mut lines: Vec<String> = input.lines().map(str::to_string).collect();
    let section_start = lines.iter().position(|line| line.trim() == "[ui]");

    match section_start {
        Some(start) => {
            let section_end = lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, line)| line.trim_start().starts_with('['))
                .map(|(index, _)| index)
                .unwrap_or(lines.len());

            let mut section_body: Vec<String> = lines[start + 1..section_end]
                .iter()
                .filter(|existing| !existing.trim_start().starts_with(key))
                .cloned()
                .collect();
            section_body.push(line.to_string());
            lines.splice(start + 1..section_end, section_body);
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[ui]".to_string());
            lines.push(line.to_string());
        }
    }

    let mut output = lines.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn upsert_exec_policy_settings(input: &str, settings: &ExecPolicySettings<'_>) -> String {
    let approval_line = format!(
        "approval_profile = \"{}\"",
        approval_profile_name(settings.approval)
    );
    let strategy_line = format!(
        "execution_strategy = \"{}\"",
        execution_strategy_name(settings.execution_strategy)
    );
    let sandbox_line = format!(
        "sandbox_profile = \"{}\"",
        sandbox_profile_name(settings.sandbox)
    );
    let retry_line = format!("retry_max_attempts = {}", settings.retry_max_attempts);
    let allowed_line = format!(
        "allowed_commands = {}",
        serde_json::to_string(settings.allowed_commands).unwrap_or_else(|_| "[]".to_string())
    );
    let blocked_line = format!(
        "blocked_commands = {}",
        serde_json::to_string(settings.blocked_commands).unwrap_or_else(|_| "[]".to_string())
    );
    let header_widgets_line = format!(
        "header_widgets = {}",
        serde_json::to_string(&header_widget_names(settings.header_widgets))
            .unwrap_or_else(|_| "[]".to_string())
    );

    let mut lines: Vec<String> = input.lines().map(str::to_string).collect();
    let section_start = lines.iter().position(|line| line.trim() == "[exec_policy]");

    match section_start {
        Some(start) => {
            let section_end = lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, line)| line.trim_start().starts_with('['))
                .map(|(index, _)| index)
                .unwrap_or(lines.len());

            let mut section_body: Vec<String> = lines[start + 1..section_end]
                .iter()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    !(trimmed.starts_with("approval_profile")
                        || trimmed.starts_with("execution_strategy")
                        || trimmed.starts_with("sandbox_profile")
                        || trimmed.starts_with("retry_max_attempts")
                        || trimmed.starts_with("allowed_commands")
                        || trimmed.starts_with("blocked_commands"))
                })
                .cloned()
                .collect();
            section_body.insert(0, blocked_line);
            section_body.insert(0, allowed_line);
            section_body.insert(0, retry_line);
            section_body.insert(0, sandbox_line);
            section_body.insert(0, strategy_line);
            section_body.insert(0, approval_line);

            lines.splice(start + 1..section_end, section_body);
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[exec_policy]".to_string());
            lines.push(approval_line);
            lines.push(strategy_line);
            lines.push(sandbox_line);
            lines.push(retry_line);
            lines.push(allowed_line);
            lines.push(blocked_line);
        }
    }

    let ui_section_start = lines.iter().position(|line| line.trim() == "[ui]");

    match ui_section_start {
        Some(start) => {
            let section_end = lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, line)| line.trim_start().starts_with('['))
                .map(|(index, _)| index)
                .unwrap_or(lines.len());

            let mut section_body: Vec<String> = lines[start + 1..section_end]
                .iter()
                .filter(|line| !line.trim_start().starts_with("header_widgets"))
                .cloned()
                .collect();
            section_body.insert(0, header_widgets_line);
            lines.splice(start + 1..section_end, section_body);
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[ui]".to_string());
            lines.push(header_widgets_line);
        }
    }

    let mut output = lines.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

pub fn approval_profile_name(profile: ApprovalProfile) -> &'static str {
    match profile {
        ApprovalProfile::Strict => "strict",
        ApprovalProfile::AllowListed => "allow_listed",
        ApprovalProfile::AllowAll => "allow_all",
    }
}

pub fn execution_strategy_name(strategy: ExecutionStrategy) -> &'static str {
    match strategy {
        ExecutionStrategy::Auto => "auto",
        ExecutionStrategy::Grounded => "grounded",
        ExecutionStrategy::Deterministic => "deterministic",
        ExecutionStrategy::ModelOnly => "model",
    }
}

pub fn sandbox_profile_name(profile: SandboxProfile) -> &'static str {
    match profile {
        SandboxProfile::Disabled => "disabled",
        SandboxProfile::Workspace => "workspace",
        SandboxProfile::NetworkedWorkspace => "networked_workspace",
    }
}

pub fn parse_approval_profile(value: &str) -> Option<ApprovalProfile> {
    match value.trim().to_ascii_lowercase().as_str() {
        "strict" => Some(ApprovalProfile::Strict),
        "allow_listed" | "allow-listed" | "allowlisted" => Some(ApprovalProfile::AllowListed),
        "allow_all" | "allow-all" | "allowall" => Some(ApprovalProfile::AllowAll),
        _ => None,
    }
}

pub fn parse_execution_strategy(value: &str) -> Option<ExecutionStrategy> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ExecutionStrategy::Auto),
        "grounded" => Some(ExecutionStrategy::Grounded),
        "deterministic" => Some(ExecutionStrategy::Deterministic),
        "model" | "model_only" | "model-only" => Some(ExecutionStrategy::ModelOnly),
        _ => None,
    }
}

pub fn parse_sandbox_profile(value: &str) -> Option<SandboxProfile> {
    match value.trim().to_ascii_lowercase().as_str() {
        "disabled" | "off" => Some(SandboxProfile::Disabled),
        "workspace" => Some(SandboxProfile::Workspace),
        "networked_workspace" | "networked-workspace" | "networked" => {
            Some(SandboxProfile::NetworkedWorkspace)
        }
        _ => None,
    }
}

pub fn next_approval_profile(profile: ApprovalProfile) -> ApprovalProfile {
    match profile {
        ApprovalProfile::Strict => ApprovalProfile::AllowListed,
        ApprovalProfile::AllowListed => ApprovalProfile::AllowAll,
        ApprovalProfile::AllowAll => ApprovalProfile::Strict,
    }
}

pub fn next_execution_strategy(strategy: ExecutionStrategy) -> ExecutionStrategy {
    match strategy {
        ExecutionStrategy::Auto => ExecutionStrategy::Grounded,
        ExecutionStrategy::Grounded => ExecutionStrategy::Deterministic,
        ExecutionStrategy::Deterministic => ExecutionStrategy::ModelOnly,
        ExecutionStrategy::ModelOnly => ExecutionStrategy::Auto,
    }
}

pub fn next_sandbox_profile(profile: SandboxProfile) -> SandboxProfile {
    match profile {
        SandboxProfile::Disabled => SandboxProfile::Workspace,
        SandboxProfile::Workspace => SandboxProfile::NetworkedWorkspace,
        SandboxProfile::NetworkedWorkspace => SandboxProfile::Disabled,
    }
}

pub fn next_retry_max_attempts(value: u32) -> u32 {
    (value + 1) % 4
}

pub fn header_widget_name(widget: HeaderWidget) -> &'static str {
    match widget {
        HeaderWidget::Session => "session",
        HeaderWidget::Plan => "plan",
        HeaderWidget::Agents => "agents",
        HeaderWidget::Web => "web",
        HeaderWidget::Auth => "auth",
        HeaderWidget::Focus => "focus",
        HeaderWidget::Model => "model",
        HeaderWidget::Cwd => "cwd",
        HeaderWidget::Strategy => "strategy",
        HeaderWidget::Approval => "approval",
        HeaderWidget::Update => "update",
        HeaderWidget::Activity => "activity",
    }
}

pub fn header_widget_names(widgets: &[HeaderWidget]) -> Vec<&'static str> {
    widgets.iter().map(|w| header_widget_name(*w)).collect()
}

pub fn available_header_widgets() -> &'static [HeaderWidget] {
    &[
        HeaderWidget::Session,
        HeaderWidget::Plan,
        HeaderWidget::Agents,
        HeaderWidget::Web,
        HeaderWidget::Auth,
        HeaderWidget::Focus,
        HeaderWidget::Model,
        HeaderWidget::Cwd,
        HeaderWidget::Strategy,
        HeaderWidget::Approval,
        HeaderWidget::Update,
        HeaderWidget::Activity,
    ]
}

pub fn parse_header_widgets(values: &[String]) -> Vec<HeaderWidget> {
    let mut parsed = Vec::new();
    for value in values {
        let widget = match value.trim().to_ascii_lowercase().as_str() {
            "session" => Some(HeaderWidget::Session),
            "plan" => Some(HeaderWidget::Plan),
            "agents" => Some(HeaderWidget::Agents),
            "web" => Some(HeaderWidget::Web),
            "auth" => Some(HeaderWidget::Auth),
            "focus" => Some(HeaderWidget::Focus),
            "model" => Some(HeaderWidget::Model),
            "cwd" => Some(HeaderWidget::Cwd),
            "strategy" => Some(HeaderWidget::Strategy),
            "approval" => Some(HeaderWidget::Approval),
            "update" => Some(HeaderWidget::Update),
            "activity" => Some(HeaderWidget::Activity),
            _ => None,
        };
        if let Some(widget) = widget {
            if !parsed.contains(&widget) {
                parsed.push(widget);
            }
        }
    }
    if !parsed.contains(&HeaderWidget::Model) {
        parsed.insert(0, HeaderWidget::Model);
    }
    if parsed.is_empty() {
        vec![HeaderWidget::Model]
    } else {
        parsed
    }
}

pub fn header_widgets_summary(widgets: &[HeaderWidget]) -> String {
    header_widget_names(widgets).join(", ")
}

#[cfg(test)]
mod tests {
    use super::{
        ExecPolicySettings, HeaderWidget, approval_profile_name, execution_strategy_name,
        next_approval_profile, next_execution_strategy, next_retry_max_attempts,
        next_sandbox_profile, sandbox_profile_name, upsert_exec_policy_settings, upsert_ui_setting,
    };
    use harper_core::{ApprovalProfile, ExecutionStrategy, SandboxProfile};

    #[test]
    fn upsert_exec_policy_profiles_creates_section() {
        let output = upsert_exec_policy_settings(
            "",
            &ExecPolicySettings {
                approval: ApprovalProfile::AllowListed,
                execution_strategy: ExecutionStrategy::Grounded,
                sandbox: SandboxProfile::Workspace,
                retry_max_attempts: 2,
                allowed_commands: &["git".to_string()],
                blocked_commands: &["rm".to_string()],
                header_widgets: &[HeaderWidget::Model, HeaderWidget::Strategy],
            },
        );
        assert!(output.contains("[exec_policy]"));
        assert!(output.contains("approval_profile = \"allow_listed\""));
        assert!(output.contains("execution_strategy = \"grounded\""));
        assert!(output.contains("sandbox_profile = \"workspace\""));
        assert!(output.contains("retry_max_attempts = 2"));
        assert!(output.contains("allowed_commands = [\"git\"]"));
        assert!(output.contains("blocked_commands = [\"rm\"]"));
    }

    #[test]
    fn upsert_exec_policy_profiles_replaces_existing_values() {
        let input = "[exec_policy]\napproval_profile = \"strict\"\nsandbox_profile = \"disabled\"\nallowed_commands = [\"git\"]\nblocked_commands = [\"rm\"]\n";
        let output = upsert_exec_policy_settings(
            input,
            &ExecPolicySettings {
                approval: ApprovalProfile::AllowAll,
                execution_strategy: ExecutionStrategy::Deterministic,
                sandbox: SandboxProfile::NetworkedWorkspace,
                retry_max_attempts: 3,
                allowed_commands: &["ls".to_string()],
                blocked_commands: &["sudo".to_string()],
                header_widgets: &[HeaderWidget::Model, HeaderWidget::Cwd],
            },
        );
        assert!(output.contains("approval_profile = \"allow_all\""));
        assert!(output.contains("execution_strategy = \"deterministic\""));
        assert!(output.contains("sandbox_profile = \"networked_workspace\""));
        assert!(output.contains("retry_max_attempts = 3"));
        assert!(output.contains("allowed_commands = [\"ls\"]"));
        assert!(output.contains("blocked_commands = [\"sudo\"]"));
        assert!(!output.contains("approval_profile = \"strict\""));
    }

    #[test]
    fn upsert_ui_setting_replaces_menu_logo_value() {
        let input = "[ui]\ntheme = \"minimal\"\nshow_menu_logo = true\n";
        let output = upsert_ui_setting(input, "show_menu_logo", "show_menu_logo = false");

        assert!(output.contains("[ui]"));
        assert!(output.contains("theme = \"minimal\""));
        assert!(output.contains("show_menu_logo = false"));
        assert!(!output.contains("show_menu_logo = true"));
    }

    #[test]
    fn profile_name_helpers_match_config_values() {
        assert_eq!(
            approval_profile_name(ApprovalProfile::AllowListed),
            "allow_listed"
        );
        assert_eq!(
            execution_strategy_name(ExecutionStrategy::Grounded),
            "grounded"
        );
        assert_eq!(
            sandbox_profile_name(SandboxProfile::NetworkedWorkspace),
            "networked_workspace"
        );
    }

    #[test]
    fn profile_cycle_helpers_advance() {
        assert_eq!(
            next_approval_profile(ApprovalProfile::Strict),
            ApprovalProfile::AllowListed
        );
        assert_eq!(
            next_execution_strategy(ExecutionStrategy::Auto),
            ExecutionStrategy::Grounded
        );
        assert_eq!(
            next_sandbox_profile(SandboxProfile::Workspace),
            SandboxProfile::NetworkedWorkspace
        );
        assert_eq!(next_retry_max_attempts(0), 1);
        assert_eq!(next_retry_max_attempts(3), 0);
    }
}
