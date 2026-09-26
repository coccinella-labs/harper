// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_core::ResolvedAgents;
use harper_core::core::Message;
use harper_core::core::plan::{PlanLoopOutcome, PlanLoopStage};
use harper_core::memory::session_service::GlobalStats;
use harper_core::{ApprovalProfile, AuthSession, ExecutionStrategy, PlanState, SandboxProfile};
use ratatui::layout::Rect;
use ratatui::text::Line;
use serde::Deserialize;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

// Constants for sidebar entry limits
const MAX_SIDEBAR_PROBE_ENTRIES: usize = 5;
const MAX_SIDEBAR_GIT_ENTRIES: usize = 4;
const MAX_SIDEBAR_FILE_ENTRIES: usize = 6;
pub const MAIN_MENU_ITEM_COUNT: usize = 6;

#[derive(Clone, Debug, Deserialize)]
pub struct ReviewFindingState {
    pub title: String,
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReviewState {
    pub summary: String,
    pub findings: Vec<ReviewFindingState>,
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationFocus {
    Messages,
    Review,
    PlanSteps,
    PlanJobs,
    Agents,
}

#[derive(Clone)]
pub struct SidebarSection {
    pub title: String,
    pub entries: Vec<String>,
}

#[derive(Clone)]
pub struct RenderedMessageBlock {
    pub role: String,
    pub content: String,
    pub lines: Vec<Line<'static>>,
}

#[derive(Clone, Default)]
pub struct ChatLoopState {
    pub stage: Option<PlanLoopStage>,
    pub last_outcome: Option<PlanLoopOutcome>,
    pub last_feedback: Option<String>,
}

impl ChatLoopState {
    pub fn has_state(&self) -> bool {
        self.stage.is_some() || self.last_outcome.is_some() || self.last_feedback.is_some()
    }
}

#[derive(Clone)]
pub struct ChatState {
    pub session_id: String,
    pub messages: Vec<Message>,
    pub awaiting_response: bool,
    pub active_plan: Option<PlanState>,
    pub active_agents: Option<ResolvedAgents>,
    pub active_review: Option<ReviewState>,
    pub review_selected: usize,
    pub plan_step_selected: usize,
    pub plan_steps_expanded: bool,
    pub plan_job_selected: usize,
    pub plan_jobs_expanded: bool,
    pub plan_job_output_scroll: usize,
    pub navigation_focus: NavigationFocus,
    pub command_output: Option<CommandOutputState>,
    pub command_output_expanded: bool,
    pub command_output_scroll: usize,
    pub loop_state: ChatLoopState,
    pub agents_panel_expanded: bool,
    pub agents_scroll_offset: usize,
    pub input: String,
    pub web_search: bool,
    pub web_search_enabled: bool,
    pub completion_candidates: Vec<String>,
    pub completion_index: usize,
    pub scroll_offset: usize,
    pub completion_prefix: Option<String>,
    pub sidebar_visible: bool,
    pub sidebar_sections: Vec<SidebarSection>,
    pub rendered_message_cache: Vec<RenderedMessageBlock>,
    pub rendered_transcript_lines: Vec<Line<'static>>,
    pub render_cache_theme_key: String,
    pub messages_area: Cell<Option<Rect>>,
    pub command_output_area: Cell<Option<Rect>>,
    pub command_output_selection: Option<LineSelection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSelection {
    pub anchor: usize,
    pub focus: usize,
}

impl LineSelection {
    pub fn range(self) -> (usize, usize) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}

#[derive(Clone)]
pub struct CommandOutputState {
    pub command: String,
    pub content: String,
    pub has_error: bool,
    pub done: bool,
}

pub fn gather_sidebar_entries(chat_state: Option<&ChatState>) -> Vec<SidebarSection> {
    let mut command_entries = Vec::new();
    let mut seen_commands = BTreeSet::new();
    if let Some(chat) = chat_state {
        for msg in chat.messages.iter().rev() {
            for line in msg.content.lines().rev() {
                let trimmed = line.trim();
                if trimmed.starts_with("$ ")
                    && seen_commands.insert(trimmed.trim_start_matches("$ ").to_string())
                {
                    command_entries.push(trimmed.trim_start_matches("$ ").to_string());
                }
                if command_entries.len() >= MAX_SIDEBAR_PROBE_ENTRIES {
                    break;
                }
            }
            if command_entries.len() >= MAX_SIDEBAR_PROBE_ENTRIES {
                break;
            }
        }
    }

    let mut git_entries = Vec::new();
    if let Ok(status) = harper_core::tools::git::git_status() {
        git_entries = summarize_git_status(&status);
    }

    let mut file_entries = Vec::new();
    if let Ok(dir) = fs::read_dir(".") {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if should_skip_sidebar_workspace_entry(&name) {
                continue;
            }
            file_entries.push(name);
            if file_entries.len() >= MAX_SIDEBAR_FILE_ENTRIES {
                break;
            }
        }
    }

    build_sidebar_sections(command_entries, git_entries, file_entries)
}

/// Gather sidebar entries asynchronously
pub async fn gather_sidebar_entries_async(messages: &[Message]) -> Vec<SidebarSection> {
    let mut command_entries = Vec::new();
    let mut seen_commands = BTreeSet::new();
    for msg in messages.iter().rev() {
        for line in msg.content.lines().rev() {
            let trimmed = line.trim();
            if trimmed.starts_with("$ ")
                && seen_commands.insert(trimmed.trim_start_matches("$ ").to_string())
            {
                command_entries.push(trimmed.trim_start_matches("$ ").to_string());
            }
            if command_entries.len() >= MAX_SIDEBAR_PROBE_ENTRIES {
                break;
            }
        }
        if command_entries.len() >= MAX_SIDEBAR_PROBE_ENTRIES {
            break;
        }
    }

    let mut git_entries = Vec::new();
    if let Ok(status) = harper_core::tools::git::git_status_async().await {
        git_entries = summarize_git_status(&status);
    }

    let mut file_entries = Vec::new();
    let mut dir = match tokio::fs::read_dir(".").await {
        Ok(dir) => dir,
        Err(_) => {
            return build_sidebar_sections(command_entries, git_entries, file_entries);
        }
    };
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_skip_sidebar_workspace_entry(&name) {
            continue;
        }
        file_entries.push(name);
        if file_entries.len() >= MAX_SIDEBAR_FILE_ENTRIES {
            break;
        }
    }

    build_sidebar_sections(command_entries, git_entries, file_entries)
}

fn build_sidebar_sections(
    command_entries: Vec<String>,
    git_entries: Vec<String>,
    file_entries: Vec<String>,
) -> Vec<SidebarSection> {
    let mut sections = Vec::new();

    if !command_entries.is_empty() {
        sections.push(SidebarSection {
            title: "Recent commands".to_string(),
            entries: command_entries,
        });
    }

    if !git_entries.is_empty() {
        sections.push(SidebarSection {
            title: "Git status".to_string(),
            entries: git_entries,
        });
    }

    if !file_entries.is_empty() {
        sections.push(SidebarSection {
            title: "Top level".to_string(),
            entries: file_entries,
        });
    }

    if sections.is_empty() {
        sections.push(SidebarSection {
            title: "Context".to_string(),
            entries: vec!["No recent context".to_string()],
        });
    }

    sections
}

fn summarize_git_status(status: &str) -> Vec<String> {
    let mut unique_paths = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut modified = 0usize;
    let mut added = 0usize;
    let mut deleted = 0usize;
    let mut untracked = 0usize;

    for line in status.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Git status")
            || trimmed.starts_with("Git working")
        {
            continue;
        }

        let mut chars = trimmed.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');

        let path = trimmed
            .split_once(' ')
            .map(|(_, path)| path.trim())
            .unwrap_or(trimmed);

        if seen_paths.insert(path.to_string()) {
            unique_paths.push(path.to_string());

            match (x, y) {
                ('?', '?') => untracked += 1,
                ('A', _) | (_, 'A') => added += 1,
                ('D', _) | (_, 'D') => deleted += 1,
                _ => modified += 1,
            }
        }
    }

    if unique_paths.is_empty() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    let mut summary_parts = Vec::new();
    if modified > 0 {
        summary_parts.push(format!("{modified} modified"));
    }
    if added > 0 {
        summary_parts.push(format!("{added} added"));
    }
    if deleted > 0 {
        summary_parts.push(format!("{deleted} deleted"));
    }
    if untracked > 0 {
        summary_parts.push(format!("{untracked} untracked"));
    }

    if summary_parts.is_empty() {
        entries.push(format!("{} changed", unique_paths.len()));
    } else {
        entries.push(summary_parts.join(", "));
    }

    for path in unique_paths.into_iter().take(MAX_SIDEBAR_GIT_ENTRIES - 1) {
        entries.push(format_sidebar_path(&path));
    }
    entries
}

fn format_sidebar_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() <= 2 {
        return path.to_string();
    }

    let tail = parts[parts.len().saturating_sub(2)..].join("/");
    format!("…/{tail}")
}

fn should_skip_sidebar_workspace_entry(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "bazel-bin" | "bazel-out" | "bazel-testlogs" | "target" | "node_modules" | ".DS_Store"
        )
}

impl ChatState {
    fn has_command_output_display(&self) -> bool {
        self.command_output.is_some()
            || self
                .active_plan
                .as_ref()
                .and_then(|plan| plan.runtime.as_ref())
                .is_some_and(|runtime| !runtime.jobs.is_empty())
    }

    pub fn follow_latest_messages(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn refresh_plan_state(&mut self) {
        if let Some(plan) = &self.active_plan {
            let max_steps = plan.items.len().saturating_sub(1);
            self.plan_step_selected = self.plan_step_selected.min(max_steps);
            let max_jobs = plan
                .runtime
                .as_ref()
                .map(|runtime| runtime.jobs.len().saturating_sub(1))
                .unwrap_or(0);
            self.plan_job_selected = self.plan_job_selected.min(max_jobs);
        } else {
            self.plan_step_selected = 0;
            self.plan_steps_expanded = false;
            self.plan_job_selected = 0;
            self.plan_jobs_expanded = false;
            self.plan_job_output_scroll = 0;
        }
        self.normalize_navigation_focus();
    }

    pub fn reset_completion(&mut self) {
        self.completion_candidates.clear();
        self.completion_index = 0;
        self.completion_prefix = None;
    }

    pub fn set_loop_stage(&mut self, stage: PlanLoopStage, feedback: Option<String>) {
        self.loop_state.stage = Some(stage);
        if let Some(feedback) = feedback {
            self.loop_state.last_feedback = Some(feedback);
        }
    }

    pub fn record_loop_outcome(&mut self, outcome: PlanLoopOutcome, feedback: Option<String>) {
        self.loop_state.last_outcome = Some(outcome);
        if let Some(feedback) = feedback {
            self.loop_state.last_feedback = Some(feedback);
        }
    }

    pub fn set_navigation_focus(&mut self, focus: NavigationFocus) {
        self.navigation_focus = focus;
        self.normalize_navigation_focus();
    }

    pub fn normalize_navigation_focus(&mut self) {
        let has_review = self
            .active_review
            .as_ref()
            .is_some_and(|review| !review.findings.is_empty());
        let has_steps = self
            .active_plan
            .as_ref()
            .is_some_and(|plan| !plan.items.is_empty());
        let has_jobs = self
            .active_plan
            .as_ref()
            .and_then(|plan| plan.runtime.as_ref())
            .is_some_and(|runtime| !runtime.jobs.is_empty());
        let has_agents = self.agents_panel_expanded
            && self
                .active_agents
                .as_ref()
                .is_some_and(|agents| !agents.effective_rule_sections.is_empty());

        self.navigation_focus = match self.navigation_focus {
            NavigationFocus::Review if has_review => NavigationFocus::Review,
            NavigationFocus::PlanSteps if has_steps => NavigationFocus::PlanSteps,
            NavigationFocus::PlanJobs if has_jobs => NavigationFocus::PlanJobs,
            NavigationFocus::Agents if has_agents => NavigationFocus::Agents,
            _ => NavigationFocus::Messages,
        };
    }

    pub fn navigation_focus_label(&self) -> &'static str {
        match self.navigation_focus {
            NavigationFocus::Messages => "messages",
            NavigationFocus::Review => "findings",
            NavigationFocus::PlanSteps => "plan",
            NavigationFocus::PlanJobs => "jobs",
            NavigationFocus::Agents => "agents",
        }
    }

    pub fn refresh_review_state(&mut self) {
        self.active_review = derive_review_state(&self.messages);
        if let Some(review) = &self.active_review {
            if review.findings.is_empty() {
                self.review_selected = 0;
            } else {
                self.review_selected = self.review_selected.min(review.findings.len() - 1);
            }
        } else {
            self.review_selected = 0;
        }
        self.refresh_plan_state();
    }
}

pub struct ApprovalState {
    pub prompt: String,
    pub command: String,
    pub tx: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    pub scroll_offset: u16,
}

impl Clone for ApprovalState {
    fn clone(&self) -> Self {
        Self {
            prompt: self.prompt.clone(),
            command: self.command.clone(),
            tx: self.tx.clone(),
            scroll_offset: self.scroll_offset,
        }
    }
}

#[derive(Clone)]
pub enum AppState {
    Menu(usize),
    Chat(Box<ChatState>),
    Sessions(Vec<SessionInfo>, usize),       // sessions, selected
    ExportSessions(Vec<SessionInfo>, usize), // sessions, selected for export
    Settings(usize),                         // selected settings row
    Profile(usize),
    Appearance(usize),
    ExecutionPolicy(usize),
    ViewSession(String, Vec<Message>, usize), // name, messages, selected
    Stats(GlobalStats),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionPolicyListField {
    HeaderWidgets,
    AllowedCommands,
    BlockedCommands,
}

#[derive(Clone, Debug)]
pub struct ExecutionPolicyEditorState {
    pub field: ExecutionPolicyListField,
    pub input: String,
    pub selected_index: usize,
    pub text_input_focused: bool,
}

#[derive(Clone)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub enum MessageType {
    Error,
    Help,
    Status,
    Info,
}

#[derive(Clone)]
pub struct UiMessage {
    pub content: String,
    pub message_type: MessageType,
    pub expires_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderWidget {
    Session,
    Plan,
    Agents,
    Web,
    Auth,
    Focus,
    Model,
    Cwd,
    Strategy,
    Approval,
    Update,
    Activity,
}

#[derive(Clone)]
pub struct TuiApp {
    pub state: AppState,
    pub message: Option<UiMessage>,
    pub help_selected: usize,
    pub pending_approval: Option<ApprovalState>,
    pub activity_status: Option<String>,
    pub activity_started_at: Option<Instant>,
    pub activity_clear_pending: bool,
    pub cut_buffer: String,
    pub approval_history: Vec<String>,
    pub model_label: String,
    pub current_working_dir: String,
    pub auth_session: Option<AuthSession>,
    pub auth_flow_id: Option<String>,
    pub auth_server_base_url: Option<String>,
    pub auth_last_poll_at: Option<Instant>,
    pub approval_profile: ApprovalProfile,
    pub execution_strategy: ExecutionStrategy,
    pub sandbox_profile: SandboxProfile,
    pub retry_max_attempts: u32,
    pub agents_context_enabled: bool,
    pub allowed_commands: Vec<String>,
    pub blocked_commands: Vec<String>,
    pub execution_policy_editor: Option<ExecutionPolicyEditorState>,
    pub header_widgets: Vec<HeaderWidget>,
    pub update_status: Option<String>,
    pub homebrew_path_fix: Option<PathBuf>,
    pub show_menu_logo: bool,
    pub mouse_capture: bool,
    pub drag_scroll: Option<DragScrollState>,
    pub cancel_requested: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragScrollDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragScrollTarget {
    Messages,
    CommandOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragScrollState {
    pub target: DragScrollTarget,
    pub direction: DragScrollDirection,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self {
            state: AppState::Menu(0),
            message: None,
            help_selected: 0,
            pending_approval: None,
            activity_status: None,
            activity_started_at: None,
            activity_clear_pending: false,
            cut_buffer: String::new(),
            approval_history: Vec::new(),
            model_label: String::new(),
            current_working_dir: String::new(),
            auth_session: None,
            auth_flow_id: None,
            auth_server_base_url: None,
            auth_last_poll_at: None,
            approval_profile: ApprovalProfile::AllowListed,
            execution_strategy: ExecutionStrategy::Auto,
            sandbox_profile: SandboxProfile::Disabled,
            retry_max_attempts: 1,
            agents_context_enabled: true,
            allowed_commands: Vec::new(),
            blocked_commands: Vec::new(),
            execution_policy_editor: None,
            header_widgets: vec![
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
            ],
            update_status: None,
            homebrew_path_fix: None,
            show_menu_logo: true,
            mouse_capture: false,
            drag_scroll: None,
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl TuiApp {
    fn set_message(
        &mut self,
        content: String,
        message_type: MessageType,
        expires_after: Option<Duration>,
    ) {
        if content.trim().is_empty() {
            self.message = None;
            return;
        }
        self.message = Some(UiMessage {
            content,
            message_type,
            expires_at: expires_after.map(|duration| Instant::now() + duration),
        });
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn profile_action_count(&self) -> usize {
        if self.auth_session.is_some() { 2 } else { 3 }
    }

    pub fn execution_policy_row_count(&self) -> usize {
        if self.homebrew_path_fix.is_some() {
            10
        } else {
            9
        }
    }

    pub fn set_error_message(&mut self, content: String) {
        self.set_message(content, MessageType::Error, None);
    }

    pub fn set_help_message(&mut self, content: String) {
        self.help_selected = 0;
        self.set_message(content, MessageType::Help, None);
    }

    pub fn set_status_message(&mut self, content: String) {
        self.set_message(content, MessageType::Status, Some(Duration::from_secs(2)));
    }

    pub fn set_info_message(&mut self, content: String) {
        self.set_message(content, MessageType::Info, Some(Duration::from_secs(3)));
    }

    pub fn set_persistent_info_message(&mut self, content: String) {
        self.set_message(content, MessageType::Info, None);
    }

    pub fn clear_message(&mut self) {
        self.message = None;
        self.help_selected = 0;
    }

    pub fn refresh_message(&mut self) {
        if self
            .message
            .as_ref()
            .and_then(|message| message.expires_at)
            .is_some_and(|expires_at| Instant::now() >= expires_at)
        {
            self.message = None;
        }
    }

    pub fn auth_status_label(&self) -> String {
        match &self.auth_session {
            Some(session) => session
                .user
                .email
                .clone()
                .unwrap_or_else(|| format!("signed in: {}", session.user.user_id)),
            None => "auth: signed out".to_string(),
        }
    }

    pub fn set_activity_status(&mut self, status: Option<String>) {
        const MIN_ACTIVITY_VISIBLE: Duration = Duration::from_millis(700);

        match status {
            Some(status) => {
                self.activity_status = Some(status);
                self.activity_started_at = Some(Instant::now());
                self.activity_clear_pending = false;
            }
            None => {
                let should_clear_now = self
                    .activity_started_at
                    .map(|started| started.elapsed() >= MIN_ACTIVITY_VISIBLE)
                    .unwrap_or(true);
                if should_clear_now {
                    self.activity_status = None;
                    self.activity_started_at = None;
                    self.activity_clear_pending = false;
                } else {
                    self.activity_clear_pending = true;
                }
            }
        }
    }

    pub fn refresh_activity_status(&mut self) {
        if !self.activity_clear_pending {
            return;
        }
        if self
            .activity_started_at
            .map(|started| started.elapsed() >= Duration::from_millis(700))
            .unwrap_or(true)
        {
            self.activity_status = None;
            self.activity_started_at = None;
            self.activity_clear_pending = false;
        }
    }

    pub fn next(&mut self) {
        if let Some(approval) = &mut self.pending_approval {
            approval.scroll_offset = approval.scroll_offset.saturating_add(1);
            return;
        }

        let profile_action_count = self.profile_action_count();
        let execution_policy_row_count = self.execution_policy_row_count();
        match &mut self.state {
            AppState::Menu(sel) => *sel = (*sel + 1) % MAIN_MENU_ITEM_COUNT,
            AppState::Chat(chat_state) => {
                if chat_state.command_output_expanded {
                    if chat_state.has_command_output_display() {
                        chat_state.command_output_scroll =
                            chat_state.command_output_scroll.saturating_add(1);
                        return;
                    }
                    chat_state.command_output_expanded = false;
                    chat_state.command_output_scroll = 0;
                }
                chat_state.normalize_navigation_focus();
                match chat_state.navigation_focus {
                    NavigationFocus::Review => {
                        if let Some(review) = &chat_state.active_review {
                            if !review.findings.is_empty() {
                                chat_state.review_selected =
                                    (chat_state.review_selected + 1).min(review.findings.len() - 1);
                            }
                        }
                    }
                    NavigationFocus::PlanSteps => {
                        if let Some(plan) = &chat_state.active_plan {
                            if !plan.items.is_empty() {
                                chat_state.plan_step_selected =
                                    (chat_state.plan_step_selected + 1).min(plan.items.len() - 1);
                            }
                        }
                    }
                    NavigationFocus::PlanJobs => {
                        if chat_state.plan_jobs_expanded {
                            chat_state.plan_job_output_scroll =
                                chat_state.plan_job_output_scroll.saturating_add(1);
                        } else if let Some(runtime) = chat_state
                            .active_plan
                            .as_ref()
                            .and_then(|plan| plan.runtime.as_ref())
                        {
                            if !runtime.jobs.is_empty() {
                                chat_state.plan_job_selected =
                                    (chat_state.plan_job_selected + 1).min(runtime.jobs.len() - 1);
                                chat_state.plan_job_output_scroll = 0;
                            }
                        }
                    }
                    NavigationFocus::Agents => {
                        chat_state.agents_scroll_offset =
                            chat_state.agents_scroll_offset.saturating_add(1);
                    }
                    NavigationFocus::Messages => {
                        chat_state.scroll_offset = chat_state.scroll_offset.saturating_sub(1);
                    }
                }
            }
            AppState::Sessions(sessions, sel) => {
                if !sessions.is_empty() {
                    *sel = (*sel + 1) % sessions.len();
                }
            }
            AppState::Settings(sel) => *sel = (*sel + 1) % 6,
            AppState::Profile(sel) => *sel = (*sel + 1) % profile_action_count,
            AppState::Appearance(sel) => *sel = (*sel + 1) % 3,
            AppState::ExecutionPolicy(sel) => *sel = (*sel + 1) % execution_policy_row_count,
            AppState::ExportSessions(sessions, sel) => {
                if !sessions.is_empty() {
                    *sel = (*sel + 1) % sessions.len();
                }
            }
            AppState::ViewSession(_, messages, sel) => {
                if !messages.is_empty() {
                    *sel = sel.saturating_add(1);
                }
            }
            AppState::Stats(_) => {}
        }
    }

    pub fn previous(&mut self) {
        if let Some(approval) = &mut self.pending_approval {
            approval.scroll_offset = approval.scroll_offset.saturating_sub(1);
            return;
        }

        let profile_action_count = self.profile_action_count();
        let execution_policy_row_count = self.execution_policy_row_count();
        match &mut self.state {
            AppState::Menu(sel) => {
                *sel = if *sel == 0 {
                    MAIN_MENU_ITEM_COUNT - 1
                } else {
                    *sel - 1
                }
            }
            AppState::Chat(chat_state) => {
                if chat_state.command_output_expanded {
                    if chat_state.has_command_output_display() {
                        chat_state.command_output_scroll =
                            chat_state.command_output_scroll.saturating_sub(1);
                        return;
                    }
                    chat_state.command_output_expanded = false;
                    chat_state.command_output_scroll = 0;
                }
                chat_state.normalize_navigation_focus();
                match chat_state.navigation_focus {
                    NavigationFocus::Review => {
                        chat_state.review_selected = chat_state.review_selected.saturating_sub(1);
                    }
                    NavigationFocus::PlanSteps => {
                        chat_state.plan_step_selected =
                            chat_state.plan_step_selected.saturating_sub(1);
                    }
                    NavigationFocus::PlanJobs => {
                        if chat_state.plan_jobs_expanded {
                            chat_state.plan_job_output_scroll =
                                chat_state.plan_job_output_scroll.saturating_sub(1);
                        } else {
                            chat_state.plan_job_selected =
                                chat_state.plan_job_selected.saturating_sub(1);
                            chat_state.plan_job_output_scroll = 0;
                        }
                    }
                    NavigationFocus::Agents => {
                        chat_state.agents_scroll_offset =
                            chat_state.agents_scroll_offset.saturating_sub(1);
                    }
                    NavigationFocus::Messages => {
                        chat_state.scroll_offset = chat_state.scroll_offset.saturating_add(1);
                    }
                }
            }
            AppState::Sessions(sessions, sel) => {
                if !sessions.is_empty() {
                    *sel = if *sel == 0 {
                        sessions.len() - 1
                    } else {
                        *sel - 1
                    };
                }
            }
            AppState::Settings(sel) => *sel = if *sel == 0 { 5 } else { *sel - 1 },
            AppState::Profile(sel) => {
                *sel = if *sel == 0 {
                    profile_action_count - 1
                } else {
                    *sel - 1
                };
            }
            AppState::Appearance(sel) => *sel = if *sel == 0 { 2 } else { *sel - 1 },
            AppState::ExecutionPolicy(sel) => {
                *sel = if *sel == 0 {
                    execution_policy_row_count - 1
                } else {
                    *sel - 1
                }
            }
            AppState::ExportSessions(sessions, sel) => {
                if !sessions.is_empty() {
                    *sel = if *sel == 0 {
                        sessions.len() - 1
                    } else {
                        *sel - 1
                    };
                }
            }
            AppState::ViewSession(_, messages, sel) => {
                if !messages.is_empty() {
                    *sel = sel.saturating_sub(1);
                }
            }
            AppState::Stats(_) => {}
        }
    }
}

fn derive_review_state(messages: &[Message]) -> Option<ReviewState> {
    messages
        .iter()
        .rev()
        .find_map(|message| parse_review_state(&message.content))
}

fn parse_review_state(content: &str) -> Option<ReviewState> {
    let trimmed = content.trim();
    let cleaned = if let Some(stripped) = trimmed.strip_prefix("```") {
        let without_lang = stripped
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(stripped);
        without_lang
            .strip_suffix("```")
            .unwrap_or(without_lang)
            .trim()
    } else {
        trimmed
    };

    let value: serde_json::Value = serde_json::from_str(cleaned).ok()?;
    if value.get("summary").is_none() || value.get("findings").is_none() {
        return None;
    }
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::{AppState, ChatState, MessageType, NavigationFocus, TuiApp, UiMessage};
    use harper_core::core::Message;
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    #[test]
    fn status_messages_expire() {
        let mut app = TuiApp::new();
        app.set_status_message("Signed in".to_string());

        assert!(matches!(
            app.message.as_ref().map(|message| &message.message_type),
            Some(MessageType::Status)
        ));
        assert!(
            app.message
                .as_ref()
                .and_then(|message| message.expires_at)
                .is_some()
        );

        if let Some(message) = &mut app.message {
            message.expires_at = Some(Instant::now() - Duration::from_millis(1));
        }

        app.refresh_message();
        assert!(app.message.is_none());
    }

    #[test]
    fn help_messages_do_not_expire() {
        let mut app = TuiApp::new();
        app.message = Some(UiMessage {
            content: "Help".to_string(),
            message_type: MessageType::Help,
            expires_at: None,
        });

        app.refresh_message();
        assert!(app.message.is_some());
    }

    #[test]
    fn persistent_info_messages_do_not_expire() {
        let mut app = TuiApp::new();
        app.set_persistent_info_message("Help".to_string());

        let message = app.message.as_ref().expect("info message");
        assert!(matches!(message.message_type, MessageType::Info));
        assert!(message.expires_at.is_none());

        app.refresh_message();
        assert!(app.message.is_some());
    }

    #[test]
    fn stale_command_output_expansion_does_not_trap_scrolling() {
        let mut app = TuiApp::new();
        app.state = AppState::Chat(Box::new(ChatState {
            session_id: "session".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            awaiting_response: false,
            active_plan: None,
            active_agents: None,
            active_review: None,
            review_selected: 0,
            plan_step_selected: 0,
            plan_steps_expanded: false,
            plan_job_selected: 0,
            plan_jobs_expanded: false,
            plan_job_output_scroll: 0,
            navigation_focus: NavigationFocus::Messages,
            command_output: None,
            command_output_expanded: true,
            command_output_scroll: 5,
            loop_state: super::ChatLoopState::default(),
            agents_panel_expanded: false,
            agents_scroll_offset: 0,
            input: String::new(),
            web_search: false,
            web_search_enabled: false,
            completion_candidates: Vec::new(),
            completion_index: 0,
            scroll_offset: 2,
            completion_prefix: None,
            sidebar_visible: false,
            sidebar_sections: Vec::new(),
            rendered_message_cache: Vec::new(),
            rendered_transcript_lines: Vec::new(),
            render_cache_theme_key: String::new(),
            messages_area: Cell::new(None),
            command_output_area: Cell::new(None),
            command_output_selection: None,
        }));

        app.next();

        let AppState::Chat(chat_state) = &app.state else {
            panic!("expected chat state");
        };
        assert!(!chat_state.command_output_expanded);
        assert_eq!(chat_state.command_output_scroll, 0);
        assert_eq!(chat_state.scroll_offset, 1);
    }
}
