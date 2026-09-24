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

pub mod agent;
pub mod core;
pub mod harness;
pub mod memory;
pub mod runtime;
pub mod server;
pub mod tools;

// Re-export error module for external access
pub use crate::core::error;

// Re-export core types
pub use crate::core::agents::{AgentsSource, ResolvedAgents};
pub use crate::core::auth::{AuthSession, AuthenticatedUser, UserAuthClaims, UserAuthProvider};
pub use crate::core::constants::VERSION;
pub use crate::core::error::{HarperError, HarperResult};
pub use crate::core::llm_client::call_llm;
pub use crate::core::models::ProviderModels;
pub use crate::core::native_shell::{
    AuthShellContext, ConfigShellContext, NativeShellCommand, NativeShellContext,
    NativeShellOutcome, PlanShellCommand, execute_native_shell_command,
    execute_native_shell_command_with_context, parse_native_shell_command, resolve_session_target,
};
pub use crate::core::plan::{PlanItem, PlanRuntime, PlanState, PlanStepStatus};
pub use crate::core::{ApiConfig, ApiProvider, Message};

// Re-export agent types
pub use crate::agent::chat::ChatService;

// Re-export tools
pub use crate::tools::{
    ToolService, api, code_analysis, db, filesystem, firmware, git, github, image, parsing, plan,
    screenpipe, shell, todo, web,
};

// Re-export memory utilities
pub use crate::memory::cache::{CACHE_LINE_BYTES, CacheAligned, CacheAlignedBuffer};
pub use crate::memory::session_service::SessionStateView;
pub use crate::memory::storage::{
    CommandLogEntry, clear_todos, create_connection, delete_messages, delete_session, delete_todo,
    init_db, insert_command_log, list_sessions, load_active_agents, load_command_logs_for_session,
    load_history, load_latest_command_log, load_plan_state, load_todos, save_active_agents,
    save_message, save_plan_state, save_session, save_todo,
};
pub use crate::runtime::utils;

// Re-export runtime
pub use crate::runtime::config::{
    ApprovalProfile, ExecPolicyConfig, ExecutionStrategy, SandboxProfile, SupabaseAuthConfig,
};
pub use crate::runtime::scheduler::TaskScheduler;
pub use crate::runtime::update::{
    InstallSource, ReleaseArtifact, ReleaseManifest, UpdateCheckResult, compare_versions,
    current_target_key, download_release_artifact, evaluate_update, extract_release_executable,
    fetch_release_manifest, install_downloaded_executable, load_persisted_install_source,
    resolve_install_source, resolve_update_public_key, save_persisted_install_source,
    verify_artifact_checksum, verify_artifact_signature,
};

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn setup_test_db() -> Result<(NamedTempFile, Connection), Box<dyn std::error::Error>> {
        let temp_file = NamedTempFile::new()?;
        let conn = Connection::open(temp_file.path())?;
        init_db(&conn)?;
        Ok((temp_file, conn))
    }

    fn default_api_config() -> ApiConfig {
        ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        }
    }

    #[test]
    fn test_api_provider_variants() {
        let providers = [
            ApiProvider::OpenAI,
            ApiProvider::Sambanova,
            ApiProvider::Gemini,
            ApiProvider::Ollama,
            ApiProvider::OpenRouter,
            ApiProvider::Zen,
        ];

        for provider in providers {
            assert!(!format!("{:?}", provider).is_empty());
        }
    }

    #[test]
    fn test_api_config_creation() {
        let config = ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        };

        assert!(matches!(config.provider, ApiProvider::OpenAI));
        assert_eq!(config.api_key, "test-key");
        assert!(config.base_url.contains("openai.com"));
        assert_eq!(config.model_name, "gpt-5.5");
    }

    #[test]
    fn test_message_creation() {
        let message = Message {
            role: "user".to_string(),
            content: "Hello, world!".to_string(),
        };

        assert_eq!(message.role, "user");
        assert_eq!(message.content, "Hello, world!");
        assert!(!message.role.is_empty());
        assert!(!message.content.is_empty());
    }

    #[test]
    fn test_message_roles() {
        let roles = ["user", "assistant", "system"];

        for role in roles {
            let message = Message {
                role: role.to_string(),
                content: "Test content".to_string(),
            };
            assert_eq!(message.role, role);
        }
    }

    #[test]
    fn test_harper_error_display() {
        let config_error = HarperError::Config("test config error".to_string());
        assert_eq!(
            format!("{}", config_error),
            "Configuration error: test config error"
        );

        let validation_error = HarperError::Validation("invalid input".to_string());
        assert_eq!(
            format!("{}", validation_error),
            "Validation error: invalid input"
        );

        let api_error = HarperError::Api("test api error".to_string());
        assert_eq!(format!("{}", api_error), "API error: test api error");

        let db_error = HarperError::Database("test db error".to_string());
        assert_eq!(format!("{}", db_error), "Database error: test db error");
    }

    #[tokio::test]
    async fn test_chat_service_build_system_prompt() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;
        let config = default_api_config();
        let chat_service = ChatService::new_test(&conn, &config);

        let prompt = chat_service.build_system_prompt(false).await;
        assert!(prompt.contains("gpt-5.5"));
        assert!(prompt.contains("run_command"));
        assert!(!prompt.contains("SEARCH:"));

        let prompt = chat_service.build_system_prompt(true).await;
        assert!(prompt.contains("gpt-5.5"));
        assert!(prompt.contains("run_command"));
        assert!(prompt.contains("SEARCH:"));
        Ok(())
    }

    #[test]
    fn test_preprocess_file_references() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;
        let config = default_api_config();
        let chat_service = ChatService::new_test(&conn, &config);

        assert_eq!(
            chat_service.preprocess_file_references("Check this file @src/main.rs"),
            "Check this file [READ_FILE src/main.rs]"
        );
        assert_eq!(
            chat_service.preprocess_file_references("@Cargo.toml please"),
            "[READ_FILE Cargo.toml] please"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Look at @file1.txt and @file2.txt"),
            "Look at [READ_FILE file1.txt] and [READ_FILE file2.txt]"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Just a normal message"),
            "Just a normal message"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Message with @"),
            "Message with @"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Use @/help command"),
            "Use @/help command"
        );
        assert_eq!(
            chat_service.preprocess_file_references("@file1.txt @/invalid @file2.txt"),
            "[READ_FILE file1.txt] @/invalid [READ_FILE file2.txt]"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Check @src/main.rs and @README.md"),
            "Check [READ_FILE src/main.rs] and [READ_FILE README.md]"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Check @ and continue"),
            "Check @ and continue"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Check @ file.txt"),
            "Check @ file.txt"
        );
        assert_eq!(
            chat_service.preprocess_file_references("@file1.txt@file2.txt"),
            "[READ_FILE file1.txt@file2.txt]"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Check this @"),
            "Check this @"
        );
        assert_eq!(
            chat_service.preprocess_file_references("@valid.txt @ @invalid/ @another.txt"),
            "[READ_FILE valid.txt] @ [READ_FILE invalid/] [READ_FILE another.txt]"
        );
        assert_eq!(
            chat_service.preprocess_file_references("Check this @ /tmp/ignored.png"),
            "Check this @ /tmp/ignored.png"
        );
        assert_eq!(
            chat_service
                .preprocess_file_references(r#"Check this @"/tmp/Screenshot 2026-05-02.png""#),
            "Check this [IMAGE_INFO /tmp/Screenshot 2026-05-02.png]"
        );
        let image_path = std::env::temp_dir().join("Screenshot 2026-05-02 test.png");
        std::fs::write(&image_path, b"not a decoded image").expect("write temp image");
        assert_eq!(
            chat_service.preprocess_file_references(&format!("'{}'", image_path.display())),
            format!("[IMAGE_INFO {}]", image_path.display())
        );
        let _ = std::fs::remove_file(image_path);
        Ok(())
    }

    #[test]
    fn test_remove_todo_by_index() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;

        crate::memory::storage::save_todo(&conn, "First todo")?;
        crate::memory::storage::save_todo(&conn, "Second todo")?;
        crate::memory::storage::save_todo(&conn, "Third todo")?;

        let result = crate::tools::todo::manage_todo(&conn, "[TODO remove 2]")?;
        assert_eq!(result, "Removed todo: Second todo");

        let todos = crate::memory::storage::load_todos(&conn)?;
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].1, "First todo");
        assert_eq!(todos[1].1, "Third todo");

        let result = crate::tools::todo::manage_todo(&conn, "[TODO remove 1]")?;
        assert_eq!(result, "Removed todo: First todo");

        let result = crate::tools::todo::manage_todo(&conn, "[TODO remove 1]")?;
        assert_eq!(result, "Removed todo: Third todo");

        let todos = crate::memory::storage::load_todos(&conn)?;
        assert_eq!(todos.len(), 0);

        let result = crate::tools::todo::manage_todo(&conn, "[TODO remove 1]");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invalid todo index: 1"));

        let result = crate::tools::todo::manage_todo(&conn, "[TODO remove 0]");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("1-based index expected"));
        Ok(())
    }

    #[test]
    fn test_clear_todos_returns_count() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;

        crate::memory::storage::save_todo(&conn, "Test todo 1")?;
        crate::memory::storage::save_todo(&conn, "Test todo 2")?;
        crate::memory::storage::save_todo(&conn, "Test todo 3")?;

        let todos = crate::memory::storage::load_todos(&conn)?;
        assert_eq!(todos.len(), 3);

        let cleared_count = crate::memory::storage::clear_todos(&conn)?;
        assert_eq!(cleared_count, 3);

        let todos_after = crate::memory::storage::load_todos(&conn)?;
        assert_eq!(todos_after.len(), 0);
        Ok(())
    }

    #[test]
    fn test_plan_state_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;
        let plan = PlanState {
            explanation: Some("Implement plan persistence".to_string()),
            items: vec![
                PlanItem {
                    step: "Add storage".to_string(),
                    status: PlanStepStatus::Completed,
                    job_id: None,
                },
                PlanItem {
                    step: "Render UI".to_string(),
                    status: PlanStepStatus::InProgress,
                    job_id: None,
                },
            ],
            runtime: None,
            updated_at: None,
        };

        save_plan_state(&conn, "session-plan-test", &plan)?;
        let loaded =
            load_plan_state(&conn, "session-plan-test")?.expect("plan state should be present");

        assert_eq!(loaded.explanation, plan.explanation);
        assert_eq!(loaded.items, plan.items);
        assert!(loaded.updated_at.is_some());
        Ok(())
    }

    #[test]
    fn test_update_plan_tool_persists_items() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;
        let args = serde_json::json!({
            "explanation": "Tracking implementation progress",
            "items": [
                { "step": "Inspect dispatch", "status": "completed" },
                { "step": "Persist plan", "status": "in_progress" }
            ]
        });

        let result = crate::tools::plan::update_plan(&conn, "tool-plan-test", &args)?;
        assert!(result.contains("Plan updated: 2 steps"));

        let loaded =
            load_plan_state(&conn, "tool-plan-test")?.expect("plan state should be present");
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(
            loaded.explanation.as_deref(),
            Some("Tracking implementation progress")
        );
        assert_eq!(loaded.items[0].status, PlanStepStatus::Completed);
        assert_eq!(loaded.items[1].status, PlanStepStatus::InProgress);
        Ok(())
    }

    #[test]
    fn test_chat_service_should_exit() -> Result<(), Box<dyn std::error::Error>> {
        let (_temp, conn) = setup_test_db()?;
        let config = default_api_config();
        let chat_service = ChatService::new_test(&conn, &config);

        assert!(chat_service.should_exit("exit"));
        assert!(chat_service.should_exit("quit"));
        assert!(chat_service.should_exit("EXIT"));
        assert!(chat_service.should_exit(""));
        assert!(!chat_service.should_exit("hello"));
        assert!(!chat_service.should_exit("how are you?"));
        Ok(())
    }
}
