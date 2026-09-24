// Copyright 2026 coccinella-labs
//
// Use of this source code is governed by an MIT-style
// license that can be found in the LICENSE-MIT file.

//! Deterministic replay harness for the agent tool loop.
//!
//! Provides the two additive seams needed to run `process_message`
//! offline against an in-memory SQLite database with no network access:
//!
//! * [`LlmCompleter`] owns the model-completion boundary. The real
//!   implementation wraps [`crate::core::llm_client::call_llm`]; scripted
//!   fakes supply an ordered response sequence.
//! * [`ToolDispatcher`] owns the tool-execution boundary. The real
//!   implementation delegates to [`crate::tools::ToolService`]; scripted
//!   fakes record the calls and return canned results.
//!
//! The seams are injected with the same builder style as the existing
//! `with_approver` / `with_runtime_events`, and the loop itself is unchanged
//! when no scripted components are used.
//!
//! Runtime event ordering in the current loop surface is asserted at the seam
//! level: a replay interleaves exactly one completion per model call with the
//! recorded dispatch sequence. [`ReplayHarness::replay_with_sink`] is provided
//! for UI-facing status events once exercise paths emit them deterministically.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reqwest::Client;
use rusqlite::Connection;

use crate::core::error::{HarperError, HarperResult};
use crate::core::io_traits::{RuntimeEventSink, UserApproval};
use crate::core::plan::PlanRuntime;
use crate::core::tool_call::{ToolCall, ToolCallSource};
use crate::core::{ApiConfig, ApiProvider, Message};
use crate::runtime::config::ExecPolicyConfig;
use crate::tools::ToolExecOutcome;

/// Model-completion boundary.
#[async_trait]
pub trait LlmCompleter: Send + Sync {
    /// Returns the assistant text for a full chat history.
    async fn complete(
        &self,
        client: &Client,
        config: &ApiConfig,
        history: &[Message],
    ) -> Result<String, HarperError>;
}

/// Real model completion: forwards to `llm_client::call_llm`.
pub struct RealLlmCompleter;

#[async_trait]
impl LlmCompleter for RealLlmCompleter {
    async fn complete(
        &self,
        client: &Client,
        config: &ApiConfig,
        history: &[Message],
    ) -> Result<String, HarperError> {
        crate::core::llm_client::call_llm(client, config, history).await
    }
}

/// Scripted completion that pops responses in call order.
pub struct ScriptedCompleter {
    script: Mutex<VecDeque<Result<String, HarperError>>>,
}

impl ScriptedCompleter {
    pub fn new(script: Vec<Result<String, HarperError>>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(script)),
        }
    }
}

#[async_trait]
impl LlmCompleter for ScriptedCompleter {
    async fn complete(
        &self,
        _client: &Client,
        _config: &ApiConfig,
        _history: &[Message],
    ) -> Result<String, HarperError> {
        self.script
            .lock()
            .expect("script lock")
            .pop_front()
            .expect("ScriptedCompleter ran out of scripted responses")
    }
}

/// Everything [`ToolDispatcher`] needs to construct the real tool service.
pub struct ToolDispatchContext<'a> {
    pub conn: &'a Connection,
    pub config: &'a ApiConfig,
    pub exec_policy: &'a ExecPolicyConfig,
    pub session_id: Option<&'a str>,
    pub mcp_client: Option<&'a turul_mcp_client::McpClient>,
    pub approver: Option<Arc<dyn UserApproval>>,
    pub runtime_events: Option<Arc<dyn RuntimeEventSink>>,
}

/// Tool-execution boundary.
#[async_trait(?Send)]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        ctx: ToolDispatchContext<'_>,
        client: &Client,
        history: &[Message],
        tool_call: &ToolCall,
        web_search_enabled: bool,
    ) -> Result<ToolExecOutcome, HarperError>;
}

/// Real tool execution: delegates to `ToolService::handle_tool_use`.
pub struct RealToolDispatcher;

#[async_trait(?Send)]
impl ToolDispatcher for RealToolDispatcher {
    async fn dispatch(
        &self,
        ctx: ToolDispatchContext<'_>,
        client: &Client,
        history: &[Message],
        tool_call: &ToolCall,
        web_search_enabled: bool,
    ) -> Result<ToolExecOutcome, HarperError> {
        let mut tool_service = crate::tools::ToolService::new(
            ctx.conn,
            ctx.config,
            ctx.exec_policy,
            ctx.mcp_client,
            ctx.session_id,
        );
        if let Some(approver) = ctx.approver {
            tool_service = tool_service.with_approver(approver);
        }
        if let Some(runtime_events) = ctx.runtime_events {
            tool_service = tool_service.with_runtime_events(runtime_events);
        }
        tool_service
            .handle_tool_use(client, history, tool_call, web_search_enabled)
            .await
    }
}

/// A single recorded tool-call event from a [`ScriptedToolDispatcher`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Scripted tool execution that records calls and returns canned results.
#[allow(clippy::type_complexity)]
pub struct ScriptedToolDispatcher {
    script: Mutex<VecDeque<Result<ToolExecOutcome, HarperError>>>,
    calls: Mutex<Vec<RecordedToolCall>>,
}

impl ScriptedToolDispatcher {
    pub fn new(script: Vec<Result<ToolExecOutcome, HarperError>>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(script)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Executed tool calls, in order.
    pub fn recorded_calls(&self) -> Vec<RecordedToolCall> {
        self.calls.lock().expect("calls lock").clone()
    }
}

#[async_trait(?Send)]
impl ToolDispatcher for ScriptedToolDispatcher {
    async fn dispatch(
        &self,
        _ctx: ToolDispatchContext<'_>,
        _client: &Client,
        _history: &[Message],
        tool_call: &ToolCall,
        _web_search_enabled: bool,
    ) -> Result<ToolExecOutcome, HarperError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedToolCall {
                name: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
            });
        self.script
            .lock()
            .expect("script lock")
            .pop_front()
            .expect("ScriptedToolDispatcher ran out of scripted responses")
    }
}

/// Replays a session offline against an in-memory database.
pub struct ReplayHarness {
    conn: Connection,
    provider: ApiProvider,
    tool_call_source: Option<ToolCallSource>,
}

impl ReplayHarness {
    pub fn new() -> Self {
        Self::with_provider(ApiProvider::OpenAI)
    }

    pub fn with_provider(provider: ApiProvider) -> Self {
        let conn = Connection::open_in_memory().expect("in-memory connection");
        crate::memory::storage::init_db(&conn).expect("init in-memory db");
        Self {
            conn,
            provider,
            tool_call_source: None,
        }
    }

    pub fn with_tool_call_source(mut self, tool_call_source: ToolCallSource) -> Self {
        self.tool_call_source = Some(tool_call_source);
        self
    }

    /// Drives `process_message` with injected seams and returns the final
    /// response plus the persisted [`PlanRuntime`], regardless of outcome.
    pub async fn replay_with_sink(
        &self,
        completer: Arc<dyn LlmCompleter>,
        dispatcher: Arc<dyn ToolDispatcher>,
        events: Arc<dyn RuntimeEventSink>,
        policy: ExecPolicyConfig,
        session_id: &str,
        history: &mut Vec<Message>,
    ) -> (HarperResult<String>, Option<PlanRuntime>) {
        let seeded = crate::memory::storage::save_session(&self.conn, session_id)
            .and_then(|_| {
                crate::memory::storage::save_plan_state(
                    &self.conn,
                    session_id,
                    &crate::core::plan::PlanState::default(),
                )
            })
            .is_ok();
        assert!(seeded, "failed to seed in-memory session");

        let config = ApiConfig {
            provider: self.provider,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-4o-mini".to_string(),
        };

        let mut chat = crate::agent::chat::ChatService::new(
            &self.conn,
            &config,
            None,
            None,
            Some(session_id.to_string()),
            HashMap::new(),
            policy,
        )
        .with_completer(completer)
        .with_dispatcher(dispatcher)
        .with_runtime_events(events);
        if let Some(tool_call_source) = self.tool_call_source {
            chat = chat.with_tool_call_source(tool_call_source);
        }

        let response = chat.process_message(history, false, session_id).await;
        let runtime = crate::memory::storage::load_plan_state(&self.conn, session_id)
            .ok()
            .flatten()
            .and_then(|plan| plan.runtime)
            .filter(|runtime| !runtime.is_empty());
        (response, runtime)
    }

    /// [`ReplayHarness::replay_with_sink`] with a no-op event sink.
    pub async fn replay(
        &self,
        completer: Arc<dyn LlmCompleter>,
        dispatcher: Arc<dyn ToolDispatcher>,
        policy: ExecPolicyConfig,
        session_id: &str,
        history: &mut Vec<Message>,
    ) -> (HarperResult<String>, Option<PlanRuntime>) {
        self.replay_with_sink(
            completer,
            dispatcher,
            Arc::new(crate::core::io_traits::NoopRuntimeEventSink),
            policy,
            session_id,
            history,
        )
        .await
    }
}

impl Default for ReplayHarness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agents::ResolvedAgents;
    use crate::core::io_traits::RuntimeEventSink;
    use crate::core::plan::PlanState;
    use crate::core::plan::{PlanLoopOutcome, PlanLoopStage};
    use crate::runtime::config::{ApprovalProfile, ExecutionStrategy};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn user_message(content: &str) -> Message {
        Message {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }

    fn openai_tool_call(name: &str, arguments: &str) -> String {
        let escaped = arguments.replace('"', "\\\"");
        format!(
            r#"[{{"id":"{name}","function":{{"name":"{name}","arguments":"{escaped}"}}}}]"#,
            name = name,
            escaped = escaped
        )
    }

    fn default_policy() -> ExecPolicyConfig {
        ExecPolicyConfig::default()
    }

    fn model_only_policy() -> ExecPolicyConfig {
        ExecPolicyConfig {
            execution_strategy: Some(ExecutionStrategy::ModelOnly),
            ..ExecPolicyConfig::default()
        }
    }

    fn allow_all_policy() -> ExecPolicyConfig {
        ExecPolicyConfig {
            approval_profile: Some(ApprovalProfile::AllowAll),
            ..ExecPolicyConfig::default()
        }
    }

    fn join_openai_tool_calls(calls: &[String]) -> String {
        let items = calls
            .iter()
            .map(|call| call.trim().trim_start_matches('[').trim_end_matches(']'))
            .collect::<Vec<_>>();
        format!("[{}]", items.join(","))
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedRuntimeEvent {
        Activity(Option<String>),
        PlanUpdated,
        AgentsUpdated,
        CommandOutput {
            command: String,
            chunk: String,
            is_error: bool,
            done: bool,
        },
    }

    #[derive(Default)]
    struct RecordingRuntimeEventSink {
        events: Mutex<Vec<RecordedRuntimeEvent>>,
    }

    impl RecordingRuntimeEventSink {
        fn events(&self) -> Vec<RecordedRuntimeEvent> {
            self.events.lock().expect("events lock").clone()
        }
    }

    #[async_trait]
    impl RuntimeEventSink for RecordingRuntimeEventSink {
        async fn plan_updated(
            &self,
            _session_id: &str,
            _plan: Option<PlanState>,
        ) -> HarperResult<()> {
            self.events
                .lock()
                .expect("events lock")
                .push(RecordedRuntimeEvent::PlanUpdated);
            Ok(())
        }

        async fn agents_updated(
            &self,
            _session_id: &str,
            _agents: Option<ResolvedAgents>,
        ) -> HarperResult<()> {
            self.events
                .lock()
                .expect("events lock")
                .push(RecordedRuntimeEvent::AgentsUpdated);
            Ok(())
        }

        async fn activity_updated(
            &self,
            _session_id: &str,
            status: Option<String>,
        ) -> HarperResult<()> {
            self.events
                .lock()
                .expect("events lock")
                .push(RecordedRuntimeEvent::Activity(status));
            Ok(())
        }

        async fn command_output_updated(
            &self,
            _session_id: &str,
            command: String,
            chunk: String,
            is_error: bool,
            done: bool,
        ) -> HarperResult<()> {
            self.events
                .lock()
                .expect("events lock")
                .push(RecordedRuntimeEvent::CommandOutput {
                    command,
                    chunk,
                    is_error,
                    done,
                });
            Ok(())
        }
    }

    #[tokio::test]
    async fn normal_tool_round_runs_one_tool_then_final_prose() {
        let harness = ReplayHarness::new();
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![Ok(
            ToolExecOutcome::recoverable("Tool executed".to_string(), "all tests pass".to_string()),
        )]));
        let completer = Arc::new(ScriptedCompleter::new(vec![
            Ok(openai_tool_call(
                "run_command",
                r#"{"command":"cargo test"}"#,
            )),
            Ok("All tests pass.".to_string()),
        ]));

        let mut history = vec![user_message("run cargo test")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher.clone(),
                default_policy(),
                "session-normal",
                &mut history,
            )
            .await;

        assert!(matches!(response.as_deref(), Ok("All tests pass.")));
        assert_eq!(
            dispatcher.recorded_calls(),
            vec![RecordedToolCall {
                name: "run_command".to_string(),
                arguments: serde_json::json!({"command": "cargo test"}),
            }]
        );
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Feedback));
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::Responded));
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn repeated_tool_call_is_silenced_with_retry_guidance() {
        let harness = ReplayHarness::new();
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![Ok(
            ToolExecOutcome::recoverable("Tool executed".to_string(), "all tests pass".to_string()),
        )]));
        let twice = openai_tool_call("run_command", r#"{"command":"cargo test"}"#);
        let completer = Arc::new(ScriptedCompleter::new(vec![
            Ok("Running the test suite is a good idea.".to_string()),
            Ok(join_openai_tool_calls(&[twice.clone(), twice])),
        ]));

        let mut history = vec![user_message("run cargo test")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher.clone(),
                default_policy(),
                "session-dedup",
                &mut history,
            )
            .await;

        assert!(matches!(
            response.as_deref(),
            Ok("This tool was already executed this round, consider a different tool.")
        ));
        assert_eq!(dispatcher.recorded_calls().len(), 1);
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::Duplicate));
    }

    #[tokio::test]
    async fn persisted_dedup_keys_silence_tools_across_resumed_sessions() {
        let harness = ReplayHarness::new();
        let first_dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![Ok(
            ToolExecOutcome::recoverable("Tool executed".to_string(), "all tests pass".to_string()),
        )]));
        let first_completer = Arc::new(ScriptedCompleter::new(vec![
            Ok(openai_tool_call(
                "run_command",
                r#"{"command":"cargo test"}"#,
            )),
            Ok("All tests pass.".to_string()),
        ]));

        let mut first_history = vec![user_message("run cargo test")];
        let (first_response, _) = harness
            .replay(
                first_completer,
                first_dispatcher.clone(),
                default_policy(),
                "session-resume",
                &mut first_history,
            )
            .await;
        assert!(first_response.is_ok());
        assert_eq!(first_dispatcher.recorded_calls().len(), 1);

        let second_dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![]));
        let second_completer = Arc::new(ScriptedCompleter::new(vec![Ok(openai_tool_call(
            "run_command",
            r#"{"command":"cargo test"}"#,
        ))]));

        let mut second_history = vec![user_message("run cargo test again")];
        let (second_response, second_runtime) = harness
            .replay(
                second_completer,
                second_dispatcher.clone(),
                default_policy(),
                "session-resume",
                &mut second_history,
            )
            .await;

        assert!(second_response.is_ok());
        assert!(
            second_dispatcher.recorded_calls().is_empty(),
            "persisted dedupe keys must suppress re-execution across resumed sessions"
        );
        let second_runtime = second_runtime.expect("plan runtime persisted");
        assert_eq!(
            second_runtime.last_outcome,
            Some(PlanLoopOutcome::Duplicate)
        );
    }

    #[tokio::test]
    async fn approval_rejection_persists_rejected_outcome() {
        let harness = ReplayHarness::new();
        let rejection = "Command execution cancelled by user".to_string();
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![Ok(
            ToolExecOutcome::recoverable(rejection.clone(), rejection),
        )]));
        let completer = Arc::new(ScriptedCompleter::new(vec![Ok(openai_tool_call(
            "run_command",
            r#"{"command":"rm -rf /tmp/harper-test"}"#,
        ))]));

        let mut history = vec![user_message("delete something")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher,
                default_policy(),
                "session-rejected",
                &mut history,
            )
            .await;

        assert!(response.is_ok());
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::Rejected));
        assert_eq!(runtime.last_feedback.as_deref(), Some("approval rejected"));
    }

    #[tokio::test]
    async fn underspecified_tool_call_forces_clarification() {
        let harness = ReplayHarness::new().with_tool_call_source(ToolCallSource::BracketLegacy);
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![]));
        let completer = Arc::new(ScriptedCompleter::new(vec![Ok(
            "[RUN_COMMAND that]".to_string()
        )]));

        let mut history = vec![user_message("x")];
        let (response, _runtime) = harness
            .replay(
                completer,
                dispatcher.clone(),
                default_policy(),
                "session-clarify",
                &mut history,
            )
            .await;

        assert!(
            response
                .as_deref()
                .map(|text| text.contains("which previous command"))
                .unwrap_or(false)
        );
        assert!(dispatcher.recorded_calls().is_empty());
    }

    #[tokio::test]
    async fn model_only_strategy_returns_backend_unavailable_reply() {
        let harness = ReplayHarness::new();
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![]));
        let completer = Arc::new(ScriptedCompleter::new(vec![Err(HarperError::Api(
            "backend unavailable".to_string(),
        ))]));

        let mut history = vec![user_message("hello")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher,
                model_only_policy(),
                "session-backend",
                &mut history,
            )
            .await;

        assert!(matches!(response, Ok(text) if text.contains("backend")));
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Feedback));
        assert_eq!(
            runtime.last_outcome,
            Some(PlanLoopOutcome::BackendUnavailable)
        );
    }

    #[tokio::test]
    async fn loop_runs_first_tool_round_plus_one_forced_retry_then_responds() {
        let harness = ReplayHarness::new();
        let results = (0..2)
            .map(|i| {
                Ok(ToolExecOutcome::recoverable(
                    format!("result-{i}"),
                    format!("content-{i}"),
                ))
            })
            .collect();
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(results));
        let tools = [
            ("run_command", r#"{"command":"cargo test"}"#),
            ("read_file", r#"{"path":"src/main.rs"}"#),
            // The read_file call triggers agents-guidance injection (its
            // target path now resolves for OpenAI sources), which costs one
            // extra model turn; the model then proceeds with the same call.
            ("read_file", r#"{"path":"src/main.rs"}"#),
        ];
        let completer = Arc::new(ScriptedCompleter::new(
            tools
                .iter()
                .map(|(name, arguments)| Ok(openai_tool_call(name, arguments)))
                .collect(),
        ));

        let mut history = vec![user_message("run cargo test")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher.clone(),
                default_policy(),
                "session-exhaust",
                &mut history,
            )
            .await;

        assert!(matches!(response.as_deref(), Ok("result-1")));
        assert_eq!(dispatcher.recorded_calls().len(), 2);
        assert_eq!(
            dispatcher
                .recorded_calls()
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            vec!["run_command", "read_file"]
        );
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Feedback));
        assert_eq!(
            runtime.last_outcome,
            Some(PlanLoopOutcome::Responded),
            "guidance turns must not exhaust the tool-round budget"
        );
    }

    #[tokio::test]
    async fn agents_guidance_does_not_consume_tool_rounds() {
        let harness = ReplayHarness::new();
        let calls = [
            openai_tool_call("read_file", r#"{"path":"src/main.rs"}"#),
            openai_tool_call("run_command", r#"{"command":"echo 1"}"#),
            openai_tool_call("run_command", r#"{"command":"echo 2"}"#),
            openai_tool_call("run_command", r#"{"command":"echo 3"}"#),
            openai_tool_call("run_command", r#"{"command":"echo 4"}"#),
        ];
        // One agents-guidance turn for the first path-targeting call, then four
        // tool rounds must still dispatch. Completer scripts cover the initial
        // call plus the guidance re-prompt; each dispatch returns the next
        // distinct tool call as `tool_result` so the loop continues without
        // extra completions or a duplicate guard, then the fifth iteration hits
        // the tool-round ceiling.
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(
            calls[1..]
                .iter()
                .map(|call| {
                    Ok(ToolExecOutcome::recoverable(
                        call.clone(),
                        "content".to_string(),
                    ))
                })
                .collect(),
        ));
        let completer = Arc::new(ScriptedCompleter::new(vec![
            Ok(calls[0].clone()),
            Ok(calls[0].clone()),
        ]));

        let mut history = vec![user_message("inspect the project layout")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher.clone(),
                default_policy(),
                "session-guidance-budget",
                &mut history,
            )
            .await;

        assert!(response.is_ok());
        assert_eq!(dispatcher.recorded_calls().len(), 4);
        assert_eq!(
            dispatcher
                .recorded_calls()
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            vec!["read_file", "run_command", "run_command", "run_command"]
        );
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Feedback));
        assert_eq!(
            runtime.last_outcome,
            Some(PlanLoopOutcome::MaxToolRounds),
            "four tool rounds remain available after one agents-guidance turn"
        );
    }

    #[tokio::test]
    async fn loop_exhausts_after_four_tool_rounds() {
        let harness = ReplayHarness::new();
        let calls = (0..5)
            .map(|i| openai_tool_call("run_command", &format!(r#"{{"command":"echo {i}"}}"#)))
            .collect::<Vec<_>>();
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(
            calls
                .iter()
                .skip(1)
                .map(|call| {
                    Ok(ToolExecOutcome::recoverable(
                        call.clone(),
                        "next".to_string(),
                    ))
                })
                .collect(),
        ));
        let completer = Arc::new(ScriptedCompleter::new(vec![Ok(calls[0].clone())]));

        let mut history = vec![user_message("use tools")];
        let (response, runtime) = harness
            .replay(
                completer,
                dispatcher.clone(),
                default_policy(),
                "session-four-rounds",
                &mut history,
            )
            .await;

        assert!(response.is_ok());
        assert_eq!(
            dispatcher
                .recorded_calls()
                .iter()
                .map(|call| call.arguments["command"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["echo 0", "echo 1", "echo 2", "echo 3"]
        );
        let runtime = runtime.expect("plan runtime persisted");
        assert_eq!(runtime.loop_stage, Some(PlanLoopStage::Feedback));
        assert_eq!(runtime.last_outcome, Some(PlanLoopOutcome::MaxToolRounds));
    }

    #[tokio::test]
    async fn runtime_activity_events_keep_loop_order() {
        let harness = ReplayHarness::new();
        let events = Arc::new(RecordingRuntimeEventSink::default());
        let dispatcher = Arc::new(ScriptedToolDispatcher::new(vec![Ok(
            ToolExecOutcome::recoverable("done".to_string(), "tool output".to_string()),
        )]));
        let completer = Arc::new(ScriptedCompleter::new(vec![Ok(openai_tool_call(
            "run_command",
            r#"{"command":"cargo test"}"#,
        ))]));

        let mut history = vec![user_message("use a tool")];
        let (response, _runtime) = harness
            .replay_with_sink(
                completer,
                dispatcher,
                events.clone(),
                default_policy(),
                "session-events",
                &mut history,
            )
            .await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert!(response.is_ok());
        let activity = events
            .events()
            .into_iter()
            .filter_map(|event| match event {
                RecordedRuntimeEvent::Activity(status) => status,
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(activity, vec!["responding", "responding"]);
    }

    #[tokio::test]
    async fn real_dispatcher_executes_against_service() {
        let harness = ReplayHarness::new();
        let completer = Arc::new(ScriptedCompleter::new(vec![
            Ok(openai_tool_call(
                "run_command",
                r#"{"command":"echo harper"}"#,
            )),
            Ok("Done.".to_string()),
        ]));
        let dispatcher = Arc::new(RealToolDispatcher);

        let mut history = vec![user_message("run echo harper")];
        let (response, _runtime) = harness
            .replay(
                completer,
                dispatcher,
                allow_all_policy(),
                "session-real",
                &mut history,
            )
            .await;

        assert_eq!(response.as_deref().unwrap_or(""), "Done.");
        assert!(
            history
                .iter()
                .any(|message| message.content.contains("harper")),
            "real dispatcher should append command output to history"
        );
        assert!(history.len() >= 2);
    }

    #[tokio::test]
    async fn scripted_completer_sees_only_history() {
        let harness = ReplayHarness::new();

        #[derive(Default)]
        struct StateTouchCheck {
            touched: AtomicUsize,
        }

        impl StateTouchCheck {
            fn record(&self) {
                self.touched.fetch_add(1, Ordering::SeqCst);
            }
            fn touched(&self) -> usize {
                self.touched.load(Ordering::SeqCst)
            }
        }

        let probe = Arc::new(StateTouchCheck::default());
        let probe_clone = Arc::clone(&probe);

        struct ProbingCompleter {
            probe: Arc<StateTouchCheck>,
        }

        #[async_trait]
        impl LlmCompleter for ProbingCompleter {
            async fn complete(
                &self,
                _client: &Client,
                _config: &ApiConfig,
                _history: &[Message],
            ) -> Result<String, HarperError> {
                self.probe.record();
                Ok("hello".to_string())
            }
        }

        let (response, _runtime) = harness
            .replay(
                Arc::new(ProbingCompleter { probe: probe_clone }),
                Arc::new(ScriptedToolDispatcher::new(vec![])),
                default_policy(),
                "session-state",
                &mut vec![user_message("hello")],
            )
            .await;

        assert!(matches!(response.as_deref(), Ok("hello")));
        assert_eq!(
            probe.touched(),
            1,
            "completer should see history without engine state"
        );
    }
}
