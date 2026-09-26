// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::core::ApiProvider;
use crate::core::error::{HarperError, HarperResult};
use crate::core::models::ProviderModels;
use config::{ConfigBuilder, File};
use serde::Deserialize;
use std::env;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct HarperConfig {
    pub api: ApiConfig,
    pub auth: AuthConfig,
    pub database: DatabaseConfig,
    pub mcp: McpConfig,
    pub prompts: PromptConfig,
    pub ui: UiConfig,
    pub tools: ToolsConfig,
    pub exec_policy: ExecPolicyConfig,
    pub custom_commands: CustomCommandsConfig,
    pub firmware: FirmwareConfig,
    pub server: ServerConfig,
}

#[derive(Debug, Deserialize)]
pub struct ApiConfig {
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthConfig {
    pub enabled: Option<bool>,
    pub supabase: Option<SupabaseAuthConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SupabaseAuthConfig {
    pub project_url: Option<String>,
    pub anon_key: Option<String>,
    pub jwt_secret: Option<String>,
    pub redirect_url: Option<String>,
    pub allowed_providers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct McpConfig {
    pub enabled: bool,
    pub server_url: String,
}

#[derive(Debug, Deserialize)]
pub struct PromptConfig {
    pub system_prompt_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UiConfig {
    pub theme: Option<String>,
    pub keys: Option<KeyConfig>,
    pub header_widgets: Option<Vec<String>>,
    pub show_menu_logo: Option<bool>,
    pub mouse_capture: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyConfig {
    pub next: Option<String>,
    pub previous: Option<String>,
    pub enter: Option<String>,
    pub exit: Option<String>,
    pub tab: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ToolsConfig {
    #[allow(dead_code)]
    pub enabled_tools: Option<Vec<String>>,
    #[allow(dead_code)]
    pub disabled_tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecPolicyConfig {
    pub approval_profile: Option<ApprovalProfile>,
    pub execution_strategy: Option<ExecutionStrategy>,
    #[allow(dead_code)]
    pub allowed_commands: Option<Vec<String>>,
    #[allow(dead_code)]
    pub blocked_commands: Option<Vec<String>>,
    pub sandbox_profile: Option<SandboxProfile>,
    pub sandbox: Option<SandboxConfig>,
    pub retry_max_attempts: Option<u32>,
    pub retry_network_commands: Option<Vec<String>>,
    pub retry_write_commands: Option<Vec<String>>,
}

impl Default for ExecPolicyConfig {
    fn default() -> Self {
        Self {
            approval_profile: Some(ApprovalProfile::AllowListed),
            execution_strategy: Some(ExecutionStrategy::Auto),
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Disabled),
            sandbox: Some(SandboxConfig {
                enabled: Some(false),
                allowed_dirs: None,
                writable_dirs: None,
                network_access: Some(true),
                readonly_home: Some(false),
                max_execution_time_secs: None,
            }),
            retry_max_attempts: Some(1),
            retry_network_commands: Some(vec!["curl".to_string(), "wget".to_string()]),
            retry_write_commands: Some(vec!["mkdir".to_string(), "touch".to_string()]),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalProfile {
    Strict,
    AllowListed,
    AllowAll,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    Auto,
    Grounded,
    Deterministic,
    ModelOnly,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    Disabled,
    Workspace,
    NetworkedWorkspace,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SandboxConfig {
    pub enabled: Option<bool>,
    pub allowed_dirs: Option<Vec<String>>,
    pub writable_dirs: Option<Vec<String>>,
    pub network_access: Option<bool>,
    pub readonly_home: Option<bool>,
    pub max_execution_time_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct CustomCommandsConfig {
    pub commands: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FirmwareConfig {
    pub enabled: Option<bool>,
    pub devices: Option<Vec<FirmwareDeviceConfig>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FirmwareDeviceConfig {
    pub name: String,
    pub platform: String,
    pub port: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            host: Some("127.0.0.1".to_string()),
            port: Some(8081),
        }
    }
}

impl HarperConfig {
    /// Load and validate configuration
    pub fn new() -> HarperResult<Self> {
        let mut builder = ConfigBuilder::<config::builder::DefaultState>::default()
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name("config/local").required(false))
            .add_source(config::Environment::with_prefix("HARPER"));

        Self::apply_env_overrides(&mut builder)?;

        let config = builder.build()?;
        let harper_config: Self = config.try_deserialize()?;

        // Validate the configuration
        harper_config.validate()?;

        Ok(harper_config)
    }

    /// Apply environment variable overrides to the config builder
    fn apply_env_overrides(
        builder: &mut ConfigBuilder<config::builder::DefaultState>,
    ) -> HarperResult<()> {
        let mut temp_builder = std::mem::take(builder);
        if let Ok(key) = env::var("OPENAI_API_KEY") {
            if !key.trim().is_empty() {
                temp_builder = temp_builder.set_override("api.api_key", key)?;
                temp_builder = temp_builder.set_override("api.provider", "OpenAI")?;
                temp_builder =
                    temp_builder.set_override("api.base_url", ProviderModels::OPENAI.base_url)?;
                temp_builder = temp_builder
                    .set_override("api.model_name", ProviderModels::OPENAI.default_model)?;
            }
        } else if let Ok(key) = env::var("SAMBASTUDIO_API_KEY") {
            if !key.trim().is_empty() {
                temp_builder = temp_builder.set_override("api.api_key", key)?;
                temp_builder = temp_builder.set_override("api.provider", "Sambanova")?;
                temp_builder = temp_builder
                    .set_override("api.base_url", ProviderModels::SAMBANOVA.base_url)?;
                temp_builder = temp_builder
                    .set_override("api.model_name", ProviderModels::SAMBANOVA.default_model)?;
            }
        } else if let Ok(key) = env::var("GEMINI_API_KEY") {
            if !key.trim().is_empty() {
                temp_builder = temp_builder.set_override("api.api_key", key)?;
                temp_builder = temp_builder.set_override("api.provider", "Gemini")?;
                temp_builder =
                    temp_builder.set_override("api.base_url", ProviderModels::GEMINI.base_url)?;
                temp_builder = temp_builder
                    .set_override("api.model_name", ProviderModels::GEMINI.default_model)?;
            }
        } else if let Ok(host) = env::var("OLLAMA_HOST").or_else(|_| env::var("OLLAMA_BASE_URL")) {
            if !host.trim().is_empty() {
                let mut normalized = host.trim().trim_end_matches('/').to_string();
                if !normalized.starts_with("http://") && !normalized.starts_with("https://") {
                    normalized = format!("http://{}", normalized);
                }
                let base_url = if normalized.ends_with("/api/chat") {
                    normalized
                } else {
                    format!("{}/api/chat", normalized)
                };
                let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3".to_string());
                temp_builder = temp_builder.set_override("api.provider", "Ollama")?;
                temp_builder = temp_builder.set_override("api.base_url", base_url)?;
                temp_builder = temp_builder.set_override("api.model_name", model)?;
                temp_builder = temp_builder.set_override("api.api_key", "")?;
            }
        } else if let Ok(key) = env::var("OPENROUTER_API_KEY") {
            if !key.trim().is_empty() {
                temp_builder = temp_builder.set_override("api.api_key", key)?;
                temp_builder = temp_builder.set_override("api.provider", "OpenRouter")?;
                temp_builder = temp_builder
                    .set_override("api.base_url", ProviderModels::OPENROUTER.base_url)?;
                temp_builder = temp_builder
                    .set_override("api.model_name", ProviderModels::OPENROUTER.default_model)?;
            }
        } else if let Ok(key) = env::var("OPENCODE_API_KEY") {
            if !key.trim().is_empty() {
                temp_builder = temp_builder.set_override("api.api_key", key)?;
                temp_builder = temp_builder.set_override("api.provider", "Zen")?;
                temp_builder =
                    temp_builder.set_override("api.base_url", ProviderModels::ZEN.base_url)?;
                temp_builder = temp_builder
                    .set_override("api.model_name", ProviderModels::ZEN.default_model)?;
            }
        }

        // Map DATABASE_PATH
        if let Ok(path) = env::var("DATABASE_PATH") {
            if !path.trim().is_empty() {
                temp_builder = temp_builder.set_override("database.path", path)?;
            }
        }

        // Map Supabase auth env names into Harper auth config
        if let Ok(url) = env::var("SUPABASE_URL") {
            if !url.trim().is_empty() {
                temp_builder = temp_builder.set_override("auth.enabled", true)?;
                temp_builder = temp_builder.set_override("auth.supabase.project_url", url)?;
            }
        }

        if let Ok(key) = env::var("SUPABASE_ANON_KEY") {
            if !key.trim().is_empty() {
                temp_builder = temp_builder.set_override("auth.enabled", true)?;
                temp_builder = temp_builder.set_override("auth.supabase.anon_key", key)?;
            }
        }

        if let Ok(secret) = env::var("SUPABASE_JWT_SECRET") {
            if !secret.trim().is_empty() {
                temp_builder = temp_builder.set_override("auth.enabled", true)?;
                temp_builder = temp_builder.set_override("auth.supabase.jwt_secret", secret)?;
            }
        }

        if let Ok(redirect_url) = env::var("SUPABASE_REDIRECT_URL") {
            if !redirect_url.trim().is_empty() {
                temp_builder = temp_builder.set_override("auth.enabled", true)?;
                temp_builder =
                    temp_builder.set_override("auth.supabase.redirect_url", redirect_url)?;
            }
        }

        if let Ok(providers) = env::var("SUPABASE_ALLOWED_PROVIDERS") {
            let providers = providers
                .split(',')
                .map(str::trim)
                .filter(|provider| !provider.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if !providers.is_empty() {
                temp_builder = temp_builder.set_override("auth.enabled", true)?;
                temp_builder =
                    temp_builder.set_override("auth.supabase.allowed_providers", providers)?;
            }
        }

        *builder = temp_builder;
        Ok(())
    }

    /// Validate configuration values
    fn validate(&self) -> HarperResult<()> {
        self.api.validate()?;
        self.database.validate()?;
        self.mcp.validate()?;
        self.ui.validate()?;
        self.tools.validate()?;
        self.exec_policy.validate()?;
        self.custom_commands.validate()?;
        Ok(())
    }
}

pub fn should_enable_server(config_enabled: bool, args: &[String]) -> bool {
    config_enabled && !args.iter().any(|arg| arg == "--no-server")
}

impl ApiConfig {
    /// Validate API configuration
    fn validate(&self) -> HarperResult<()> {
        // Validate provider
        let requires_api_key = match self.provider.as_str() {
            "OpenAI" | "Sambanova" | "Gemini" | "OpenRouter" | "Zen" => true,
            "Ollama" => false,
            _ => {
                return Err(HarperError::Config(format!(
                    "Invalid API provider: {}. Supported providers: OpenAI, Sambanova, Gemini, Ollama, OpenRouter, Zen",
                    self.provider
                )));
            }
        };

        // Validate API key
        if requires_api_key && self.api_key.trim().is_empty() {
            return Err(HarperError::Config("API key cannot be empty".to_string()));
        }

        // Validate base URL
        if self.base_url.trim().is_empty() {
            return Err(HarperError::Config("Base URL cannot be empty".to_string()));
        }

        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(HarperError::Config(
                "Base URL must start with http:// or https://".to_string(),
            ));
        }

        // Validate model name
        if self.model_name.trim().is_empty() {
            return Err(HarperError::Config(
                "Model name cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Convert string provider to ApiProvider enum
    pub fn get_provider(&self) -> HarperResult<ApiProvider> {
        match self.provider.as_str() {
            "OpenAI" => Ok(ApiProvider::OpenAI),
            "Sambanova" => Ok(ApiProvider::Sambanova),
            "Gemini" => Ok(ApiProvider::Gemini),
            "Ollama" => Ok(ApiProvider::Ollama),
            "OpenRouter" => Ok(ApiProvider::OpenRouter),
            "Zen" => Ok(ApiProvider::Zen),
            _ => Err(HarperError::Config(format!(
                "Unsupported provider: {}",
                self.provider
            ))),
        }
    }
}

impl DatabaseConfig {
    /// Validate database configuration
    fn validate(&self) -> HarperResult<()> {
        if self.path.trim().is_empty() {
            return Err(HarperError::Config(
                "Database path cannot be empty".to_string(),
            ));
        }

        // Check if the parent directory exists or can be created
        if let Some(parent) = Path::new(&self.path).parent() {
            if !parent.exists() {
                // Try to create the directory if it doesn't exist
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(HarperError::Config(format!(
                        "Failed to create database directory {}: {}",
                        parent.display(),
                        e
                    )));
                }
            }
        }

        Ok(())
    }
}

impl McpConfig {
    /// Validate MCP configuration
    fn validate(&self) -> HarperResult<()> {
        if self.enabled {
            if self.server_url.trim().is_empty() {
                return Err(HarperError::Config(
                    "MCP server URL cannot be empty when MCP is enabled".to_string(),
                ));
            }

            if !self.server_url.starts_with("http://") && !self.server_url.starts_with("https://") {
                return Err(HarperError::Config(
                    "MCP server URL must start with http:// or https://".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl UiConfig {
    /// Validate UI configuration
    fn validate(&self) -> HarperResult<()> {
        if let Some(ref theme) = self.theme {
            match theme.as_str() {
                "minimal" => {}
                _ => {
                    return Err(HarperError::Config(format!(
                        "Invalid theme: {}. Supported theme: minimal",
                        theme
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn effective_header_widgets(&self) -> Vec<String> {
        self.header_widgets.clone().unwrap_or_default()
    }
}

impl ToolsConfig {
    /// Validate tools configuration
    fn validate(&self) -> HarperResult<()> {
        // Basic validation - could add more specific tool name checks
        Ok(())
    }
}

impl ExecPolicyConfig {
    /// Validate exec policy configuration
    fn validate(&self) -> HarperResult<()> {
        Ok(())
    }

    pub fn effective_execution_strategy(&self) -> ExecutionStrategy {
        self.execution_strategy.unwrap_or(ExecutionStrategy::Auto)
    }

    pub fn effective_approval_profile(&self) -> ApprovalProfile {
        self.approval_profile
            .unwrap_or(ApprovalProfile::AllowListed)
    }

    pub fn effective_sandbox_profile(&self) -> SandboxProfile {
        self.sandbox_profile.unwrap_or(SandboxProfile::Disabled)
    }

    pub fn effective_sandbox_config(&self) -> SandboxConfig {
        let mut effective = match self.effective_sandbox_profile() {
            SandboxProfile::Disabled => SandboxConfig {
                enabled: Some(false),
                allowed_dirs: Some(vec![]),
                writable_dirs: Some(vec![]),
                network_access: Some(true),
                readonly_home: Some(false),
                max_execution_time_secs: None,
            },
            SandboxProfile::Workspace => SandboxConfig {
                enabled: Some(true),
                allowed_dirs: Some(vec![".".to_string()]),
                writable_dirs: Some(vec![".".to_string()]),
                network_access: Some(false),
                readonly_home: Some(true),
                max_execution_time_secs: Some(30),
            },
            SandboxProfile::NetworkedWorkspace => SandboxConfig {
                enabled: Some(true),
                allowed_dirs: Some(vec![".".to_string()]),
                writable_dirs: Some(vec![".".to_string()]),
                network_access: Some(true),
                readonly_home: Some(true),
                max_execution_time_secs: Some(30),
            },
        };

        if let Some(raw) = &self.sandbox {
            if raw.enabled.is_some() {
                effective.enabled = raw.enabled;
            }
            if raw.allowed_dirs.is_some() {
                effective.allowed_dirs = raw.allowed_dirs.clone();
            }
            if raw.writable_dirs.is_some() {
                effective.writable_dirs = raw.writable_dirs.clone();
            }
            if raw.network_access.is_some() {
                effective.network_access = raw.network_access;
            }
            if raw.readonly_home.is_some() {
                effective.readonly_home = raw.readonly_home;
            }
            if raw.max_execution_time_secs.is_some() {
                effective.max_execution_time_secs = raw.max_execution_time_secs;
            }
        }

        effective
    }

    pub fn effective_retry_max_attempts(&self) -> u32 {
        self.retry_max_attempts.unwrap_or(1)
    }

    pub fn retries_network_command(&self, command: &str) -> bool {
        self.retry_network_commands
            .as_ref()
            .map(|commands| commands.iter().any(|configured| configured == command))
            .unwrap_or_else(|| matches!(command, "curl" | "wget"))
    }

    pub fn retries_write_command(&self, command: &str) -> bool {
        self.retry_write_commands
            .as_ref()
            .map(|commands| commands.iter().any(|configured| configured == command))
            .unwrap_or_else(|| matches!(command, "mkdir" | "touch"))
    }
}

impl CustomCommandsConfig {
    /// Validate custom commands configuration
    fn validate(&self) -> HarperResult<()> {
        // Basic validation
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalProfile, ExecPolicyConfig, HarperConfig, SandboxConfig, SandboxProfile,
        ServerConfig, should_enable_server,
    };
    use config::{ConfigBuilder, File};
    use std::env;
    use std::path::Path;

    #[test]
    fn loads_supabase_auth_from_harper_env_names() {
        let original_url = env::var("SUPABASE_URL").ok();
        let original_anon = env::var("SUPABASE_ANON_KEY").ok();
        let original_jwt = env::var("SUPABASE_JWT_SECRET").ok();
        let original_redirect = env::var("SUPABASE_REDIRECT_URL").ok();
        let original_providers = env::var("SUPABASE_ALLOWED_PROVIDERS").ok();

        unsafe {
            env::set_var("SUPABASE_URL", "https://example.supabase.co");
            env::set_var("SUPABASE_ANON_KEY", "anon-key");
            env::set_var("SUPABASE_JWT_SECRET", "jwt-secret");
            env::set_var(
                "SUPABASE_REDIRECT_URL",
                "http://127.0.0.1:8081/auth/callback",
            );
            env::set_var("SUPABASE_ALLOWED_PROVIDERS", "github, google");
        }

        let mut builder = ConfigBuilder::<config::builder::DefaultState>::default()
            .set_override("api.provider", "OpenAI")
            .expect("api.provider")
            .set_override("api.api_key", "test-key")
            .expect("api.api_key")
            .set_override("api.base_url", "https://api.openai.com/v1/chat/completions")
            .expect("api.base_url")
            .set_override("api.model_name", "gpt-5")
            .expect("api.model_name")
            .set_override("database.path", ".harper/test.db")
            .expect("database.path")
            .set_override("mcp.enabled", false)
            .expect("mcp.enabled")
            .set_override("mcp.server_url", "http://127.0.0.1:3001")
            .expect("mcp.server_url")
            .set_override("prompts.system_prompt_id", "default")
            .expect("prompts.system_prompt_id")
            .set_override("firmware.enabled", false)
            .expect("firmware.enabled")
            .set_override("server.enabled", false)
            .expect("server.enabled");
        HarperConfig::apply_env_overrides(&mut builder).expect("apply env overrides");
        let config = builder.build().expect("builder builds");

        assert_eq!(config.get_bool("auth.enabled").ok(), Some(true));
        assert_eq!(
            config
                .get_string("auth.supabase.project_url")
                .ok()
                .as_deref(),
            Some("https://example.supabase.co")
        );
        assert_eq!(
            config.get_string("auth.supabase.anon_key").ok().as_deref(),
            Some("anon-key")
        );
        assert_eq!(
            config
                .get_string("auth.supabase.jwt_secret")
                .ok()
                .as_deref(),
            Some("jwt-secret")
        );
        assert_eq!(
            config
                .get_string("auth.supabase.redirect_url")
                .ok()
                .as_deref(),
            Some("http://127.0.0.1:8081/auth/callback")
        );
        assert_eq!(
            config
                .get_array("auth.supabase.allowed_providers")
                .expect("providers array")
                .iter()
                .filter_map(|value| value.clone().into_string().ok())
                .collect::<Vec<_>>(),
            vec!["github".to_string(), "google".to_string()]
        );

        restore_env_var("SUPABASE_URL", original_url);
        restore_env_var("SUPABASE_ANON_KEY", original_anon);
        restore_env_var("SUPABASE_JWT_SECRET", original_jwt);
        restore_env_var("SUPABASE_REDIRECT_URL", original_redirect);
        restore_env_var("SUPABASE_ALLOWED_PROVIDERS", original_providers);
    }

    fn restore_env_var(key: &str, value: Option<String>) {
        unsafe {
            if let Some(value) = value {
                env::set_var(key, value);
            } else {
                env::remove_var(key);
            }
        }
    }

    #[test]
    fn local_dotenv_exposes_supabase_settings() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        if !repo_root.join(".env").exists() {
            return;
        }
        std::env::set_current_dir(repo_root).expect("set current dir to workspace root");
        let _ = dotenvy::dotenv();
        let config = HarperConfig::new().expect("config loads from local files and env");
        let supabase = config
            .auth
            .supabase
            .as_ref()
            .expect("supabase config should exist");

        let project_url = supabase.project_url.as_deref().unwrap_or_default().trim();
        let anon_key = supabase.anon_key.as_deref().unwrap_or_default().trim();
        let jwt_secret = supabase.jwt_secret.as_deref().unwrap_or_default().trim();

        let looks_unconfigured = project_url.is_empty()
            || project_url.contains("your-project-id")
            || anon_key.is_empty()
            || anon_key.contains("your-supabase-anon-key")
            || jwt_secret.is_empty()
            || jwt_secret.contains("your-supabase-jwt-secret");
        if looks_unconfigured {
            return;
        }

        assert!(
            supabase.project_url.as_deref().is_some_and(
                |value| !value.trim().is_empty() && !value.contains("your-project-id")
            ),
            "project_url should be loaded from .env"
        );
        assert!(
            supabase.anon_key.as_deref().is_some_and(
                |value| !value.trim().is_empty() && !value.contains("your-supabase-anon-key")
            ),
            "anon_key should be loaded from .env"
        );
        assert!(
            supabase.jwt_secret.as_deref().is_some_and(
                |value| !value.trim().is_empty() && !value.contains("your-supabase-jwt-secret")
            ),
            "jwt_secret should be loaded from .env"
        );
    }

    #[test]
    fn default_config_enables_server() {
        let config = ServerConfig::default();

        assert_eq!(config.enabled, Some(true));
        assert_eq!(config.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(config.port, Some(8081));
    }

    #[test]
    fn no_server_flag_overrides_enabled_server_config() {
        assert!(should_enable_server(true, &[]));
        assert!(!should_enable_server(true, &["--no-server".to_string()]));
        assert!(!should_enable_server(
            true,
            &["harper".to_string(), "--no-server".to_string()]
        ));
        assert!(!should_enable_server(false, &[]));
    }

    #[test]
    fn exec_policy_profiles_default_to_current_behavior() {
        let config = ExecPolicyConfig::default();
        assert_eq!(
            config.effective_approval_profile(),
            ApprovalProfile::AllowListed
        );
        assert_eq!(config.effective_sandbox_profile(), SandboxProfile::Disabled);

        let sandbox = config.effective_sandbox_config();
        assert_eq!(sandbox.enabled, Some(false));
        assert_eq!(sandbox.writable_dirs, Some(vec![]));
        assert_eq!(sandbox.network_access, Some(true));
        assert_eq!(sandbox.readonly_home, Some(false));
    }

    #[test]
    fn workspace_sandbox_profile_sets_safe_defaults() {
        let config = ExecPolicyConfig {
            approval_profile: None,
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Workspace),
            sandbox: None,
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };

        let sandbox = config.effective_sandbox_config();
        assert_eq!(sandbox.enabled, Some(true));
        assert_eq!(sandbox.allowed_dirs, Some(vec![".".to_string()]));
        assert_eq!(sandbox.writable_dirs, Some(vec![".".to_string()]));
        assert_eq!(sandbox.network_access, Some(false));
        assert_eq!(sandbox.readonly_home, Some(true));
        assert_eq!(sandbox.max_execution_time_secs, Some(30));
    }

    #[test]
    fn default_file_does_not_override_sandbox_profile_defaults() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let default_config_path = repo_root
            .join("config/default.toml")
            .exists()
            .then(|| repo_root.join("config/default.toml"))
            .or_else(|| {
                env::var("RUNFILES_DIR").ok().and_then(|runfiles_dir| {
                    let path = Path::new(&runfiles_dir).join("_main/config/default.toml");
                    path.exists().then_some(path)
                })
            })
            .expect("config/default.toml is available");
        let config: HarperConfig = ConfigBuilder::<config::builder::DefaultState>::default()
            .add_source(File::from(default_config_path))
            .set_override("exec_policy.sandbox_profile", "workspace")
            .expect("sandbox profile override")
            .build()
            .expect("default config builds")
            .try_deserialize()
            .expect("default config deserializes");

        let sandbox = config.exec_policy.effective_sandbox_config();
        assert_eq!(sandbox.enabled, Some(true));
        assert_eq!(sandbox.allowed_dirs, Some(vec![".".to_string()]));
        assert_eq!(sandbox.writable_dirs, Some(vec![".".to_string()]));
        assert_eq!(sandbox.network_access, Some(false));
        assert_eq!(sandbox.readonly_home, Some(true));
    }

    #[test]
    fn explicit_sandbox_fields_override_profile_defaults() {
        let config = ExecPolicyConfig {
            approval_profile: None,
            execution_strategy: None,
            allowed_commands: None,
            blocked_commands: None,
            sandbox_profile: Some(SandboxProfile::Workspace),
            sandbox: Some(SandboxConfig {
                enabled: None,
                allowed_dirs: Some(vec!["/tmp".to_string()]),
                writable_dirs: Some(vec!["/tmp/work".to_string()]),
                network_access: Some(true),
                readonly_home: None,
                max_execution_time_secs: Some(5),
            }),
            retry_max_attempts: None,
            retry_network_commands: None,
            retry_write_commands: None,
        };

        let sandbox = config.effective_sandbox_config();
        assert_eq!(sandbox.enabled, Some(true));
        assert_eq!(sandbox.allowed_dirs, Some(vec!["/tmp".to_string()]));
        assert_eq!(sandbox.writable_dirs, Some(vec!["/tmp/work".to_string()]));
        assert_eq!(sandbox.network_access, Some(true));
        assert_eq!(sandbox.readonly_home, Some(true));
        assert_eq!(sandbox.max_execution_time_secs, Some(5));
    }
}
