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

//! Prompt building and management

use crate::core::ApiConfig;
use crate::core::error::HarperError;
use chrono::Datelike;
use turul_mcp_client::McpClient;

/// Prompt building functionality
pub struct PromptBuilder<'a> {
    pub config: &'a ApiConfig,
    pub prompt_id: Option<String>,
    pub mcp_client: Option<&'a McpClient>,
}

impl<'a> PromptBuilder<'a> {
    const IGNORED_CONTEXT_FILES: [&'static str; 2] = ["README.md", "README"];
    const IGNORED_CONTEXT_DIRS: [&'static str; 3] = ["target", "node_modules", "website"];

    pub fn new(
        config: &'a ApiConfig,
        prompt_id: Option<String>,
        mcp_client: Option<&'a McpClient>,
    ) -> Self {
        Self {
            config,
            prompt_id,
            mcp_client,
        }
    }

    /// Build system prompt
    pub async fn build_system_prompt(&self, web_search_enabled: bool) -> String {
        // Load custom prompt if specified
        if let Some(ref id) = self.prompt_id {
            if id != "default" {
                if let Ok(custom_prompt) = self.load_custom_prompt(id) {
                    return custom_prompt;
                }
            }
        }

        let mut prompt = format!(
            "You are Harper, a system-integrated assistant.
Operating on model: {}.
Capabilities: File I/O, shell execution, and persistent memory{}.",
            self.config.model_name,
            if web_search_enabled {
                ", plus web search"
            } else {
                ""
            }
        );

        if let Ok(context) = self.get_project_context().await {
            prompt.push_str(&format!("\n\nContext:\n{}", context));
        }

        prompt.push_str(
            "\n\nInterface via JSON tool commands. Analysis should be concise and direct.
For multi-step work, call update_plan early, keep exactly one step in_progress when active work remains, and update the plan as progress changes.

User Intent Recognition:
- read a specific file -> use read_file
- update or fix a file -> use search_replace or write_file
- run a command -> use run_command
- manage a multi-step task -> use update_plan
- list or show files -> use run_command or list tool
- search or find something -> use codebase_investigator or run_command with grep
- create a new file -> use write_file
- delete or remove a file -> ask first, then run_command
- commit or push -> use git tools
- understand how something works -> use codebase_investigator
- what changed -> use git_diff or list_changed_files
- tell me about a specific file -> use read_file
- query Azure Data Explorer or Kusto -> use adx_query

Use this JSON shape for built-in tools:
{\"tool\":\"tool_name\",\"args\":{...}}

If the user asks for a file read, file edit, search, diff, git inspection, or command execution, do not answer with an apology or a capability disclaimer. Emit the correct tool JSON immediately.

File path rules:
- For file tools, use repository-relative paths whenever possible.
- Prefer the exact filename mentioned by the user.
- If the user asks for a common root file like Cargo.toml or README.md, target that workspace file directly.
- Do not invent absolute paths like /home/user/... or placeholder project roots.
- After a tool succeeds, do not call the same file tool again unless the previous result clearly did not answer the request.

Codebase discovery rules:
- If the user asks about the codebase, a symbol location, where something is rendered, where logic lives, or uses an ambiguous file reference, do not guess a file path first.
- For ambiguous repository questions, use codebase_investigator or a search command first to locate the relevant file(s).
- Use read_file directly only when the target file is explicit and unambiguous.

Core Tools:
- read_file(args: {\"path\": \"src/main.rs\"})
- write_file(args: {\"path\": \"src/main.rs\", \"content\": \"...\"})
- search_replace(args: {\"path\": \"src/main.rs\", \"old_string\": \"old\", \"new_string\": \"new\"})
- run_command(args: {\"command\": \"git status\"})
- run_command(args: {\"command\": \"cp ./src.txt ./build/out.txt\", \"declared_read_paths\": [\"./src.txt\"], \"declared_write_paths\": [\"./build/out.txt\"]})
- run_command(args: {\"command\": \"curl -fsSL http://127.0.0.1:8081/health\", \"requires_network\": true, \"retry_policy\": \"safe\"})
- adx_query(args: {\"query\": \"StormEvents | take 10\", \"database\": \"Samples\", \"cluster_url\": \"https://help.kusto.windows.net\"})
- todo(args: {\"action\": \"add|list|remove|clear\", \"description\": \"...\", \"index\": 1})
- update_plan(args: {\"explanation\": \"optional context\", \"items\": [{\"step\": \"Inspect files\", \"status\": \"in_progress\"}]})
- list_changed_files(args: {\"ext\": \"rs\", \"tracked_only\": true, \"since\": \"HEAD~1\"})
- git_status(args: {})
- git_diff(args: {})
- git_add(args: {\"files\": \"src/main.rs\"})
- git_commit(args: {\"message\": \"feat: update prompt contract\"})
- codebase_investigator(args: {\"action\": \"find_calls\", \"symbol\": \"ToolService\"})
- codebase_investigator(args: {\"action\": \"trace_relationship\", \"x\": \"PromptBuilder\", \"y\": \"ToolService\"})
- firmware_list(args: {})
- firmware_info(args: {\"device\": \"esp32\"})
- firmware_gpio(args: {\"pin\": 2, \"state\": true})

Example: {\"tool\":\"read_file\",\"args\":{\"path\":\"src/main.rs\"}}",
        );

        if let Some(mcp_tools) = self.get_mcp_tools_text().await {
            prompt.push_str(&mcp_tools);
        }

        if let Ok(guidelines) = self.resolve_agents_guidelines() {
            if let Some(rendered) = guidelines.render_for_prompt() {
                prompt.push_str(&format!("\n\nGuidelines:\n{}\n", rendered));
            }
        }

        if web_search_enabled {
            let year = chrono::Local::now().year();
            prompt.push_str(&format!("\nWeb: `[SEARCH: query]`. Current: {}\n", year));
        }

        prompt
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
    async fn get_project_context(&self) -> Result<String, HarperError> {
        let mut context = String::new();

        let current_dir = std::env::current_dir()
            .map_err(|e| HarperError::Command(format!("Failed to get current dir: {}", e)))?;

        let mut entries = tokio::fs::read_dir(&current_dir)
            .await
            .map_err(|e| HarperError::Command(format!("Failed to read dir: {}", e)))?;

        let mut files = Vec::new();
        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => {
                    if let Ok(file_name) = entry.file_name().into_string() {
                        if !file_name.starts_with('.')
                            && !Self::IGNORED_CONTEXT_FILES.contains(&file_name.as_str())
                            && !Self::IGNORED_CONTEXT_DIRS.contains(&file_name.as_str())
                        {
                            let metadata = entry.metadata().await.map_err(|e| {
                                HarperError::Command(format!("Failed to get metadata: {}", e))
                            })?;
                            if metadata.is_dir() {
                                files.push(format!("{}/", file_name));
                            } else {
                                files.push(file_name);
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => return Err(HarperError::Command(format!("Failed to read entry: {}", e))),
            }
        }

        context.push_str(&format!("Dir: {}\n", current_dir.display()));
        context.push_str(&format!("Files: {}\n", files.join(", ")));

        // Git status
        if let Ok(git_status) = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .output()
        {
            if git_status.status.success() {
                let status = String::from_utf8_lossy(&git_status.stdout);
                if !status.trim().is_empty() {
                    context.push_str(&format!("Git:\n{}", status));
                }
            }
        }

        Ok(context)
    }

    fn resolve_agents_guidelines(
        &self,
    ) -> Result<crate::core::agents::ResolvedAgents, HarperError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| HarperError::Command(format!("Failed to get current dir: {}", e)))?;
        crate::core::agents::resolve_agents_for_dir(&current_dir)
    }

    /// Get MCP tools text
    async fn get_mcp_tools_text(&self) -> Option<String> {
        if let Some(client) = self.mcp_client {
            match client.list_tools().await {
                Ok(tools) => {
                    if tools.is_empty() {
                        return None;
                    }

                    let mut tools_text = String::from("\n\nMCP Tools:\n");
                    for tool in &tools {
                        tools_text.push_str(&format!(
                            "- {}: {}\n",
                            tool.name,
                            tool.description.as_deref().unwrap_or("...")
                        ));
                    }
                    tools_text
                        .push_str("\nRespond: {\"mcp_tool\": \"name\", \"arguments\": {...}}");
                    Some(tools_text)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to list MCP tools: {}", e);
                    None
                }
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PromptBuilder;
    use crate::core::{ApiConfig, ApiProvider};

    fn test_config() -> ApiConfig {
        ApiConfig {
            provider: ApiProvider::OpenAI,
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            model_name: "gpt-5.5".to_string(),
        }
    }

    #[tokio::test]
    async fn build_system_prompt_includes_workspace_file_rules() {
        let config = test_config();
        let builder = PromptBuilder::new(&config, None, None);
        let prompt = builder.build_system_prompt(false).await;

        assert!(prompt.contains("File path rules:"));
        assert!(prompt.contains("use repository-relative paths whenever possible"));
        assert!(prompt.contains("Do not invent absolute paths like /home/user/..."));
        assert!(prompt.contains("do not call the same file tool again"));
        assert!(prompt.contains("Codebase discovery rules:"));
        assert!(prompt.contains("do not guess a file path first"));
        assert!(prompt.contains("use codebase_investigator or a search command first"));
    }
}
