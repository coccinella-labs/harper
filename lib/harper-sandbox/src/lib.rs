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

mod backend;
mod errors;
mod policy;
mod request;

pub use backend::SandboxBackend;
pub use errors::{Result, SandboxError};
pub use policy::SandboxConfig;
pub use request::{SandboxExecutionResult, SandboxRequest};

/// Sandbox execution environment for secure command running
///
/// Provides isolated execution of system commands with configurable
/// restrictions on directories, commands, and network access.
pub struct Sandbox {
    config: SandboxConfig,
    backend: SandboxBackend,
}

impl Sandbox {
    /// Create a new sandbox with the given configuration
    #[must_use]
    pub fn new(config: SandboxConfig) -> Self {
        let backend = backend::detect_backend();
        Self { config, backend }
    }

    #[cfg(test)]
    fn with_backend(config: SandboxConfig, backend: SandboxBackend) -> Self {
        Self { config, backend }
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.backend != SandboxBackend::None
    }

    #[must_use]
    pub fn backend_name(&self) -> &str {
        backend::backend_name(self.backend)
    }

    pub async fn execute(&self, command: &str, args: &[&str]) -> Result<std::process::Output> {
        let request = SandboxRequest::new(command, args)?;
        let result = self.execute_request(request).await?;
        Ok(result.output)
    }

    pub async fn execute_request(&self, request: SandboxRequest) -> Result<SandboxExecutionResult> {
        self.execute_request_streaming(request, |_chunk, _is_error| {})
            .await
    }

    pub async fn execute_request_streaming<C>(
        &self,
        request: SandboxRequest,
        on_output: C,
    ) -> Result<SandboxExecutionResult>
    where
        C: FnMut(String, bool) + Send + 'static,
    {
        self.validate_request(&request)?;

        let backend = if !self.config.enabled {
            log::warn!("Sandbox disabled - executing directly");
            SandboxBackend::None
        } else if self.backend == SandboxBackend::None {
            return Err(SandboxError::BackendUnavailable(
                "sandbox is enabled but no supported backend is available".to_string(),
            ));
        } else {
            self.backend
        };

        let output =
            backend::execute_with_backend_streaming(backend, &self.config, &request, on_output)
                .await?;

        Ok(SandboxExecutionResult {
            request,
            backend,
            output,
        })
    }

    #[must_use]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        policy::is_command_allowed(&self.config, command)
    }

    pub fn validate_command(&self, command: &str) -> Result<()> {
        policy::validate_command(&self.config, command)
    }

    pub fn validate_working_dir(&self, path: &std::path::Path) -> Result<()> {
        policy::validate_working_dir(&self.config, path)
    }

    pub fn validate_requested_paths(
        &self,
        args: &[&str],
        base_dir: &std::path::Path,
    ) -> Result<()> {
        policy::validate_requested_paths(&self.config, args, base_dir)
    }

    pub fn validate_request(&self, request: &SandboxRequest) -> Result<()> {
        self.validate_command(&request.command)?;
        self.validate_working_dir(&request.working_dir)?;
        policy::validate_network_request(&self.config, request.requires_network)?;

        for path in &request.declared_read_paths {
            policy::validate_read_path_access(&self.config, path, &request.working_dir)?;
        }
        for path in &request.declared_write_paths {
            policy::validate_write_path_access(&self.config, path, &request.working_dir)?;
        }

        if request.declared_read_paths.is_empty() && request.declared_write_paths.is_empty() {
            let arg_refs: Vec<&str> = request.args.iter().map(String::as_str).collect();
            self.validate_requested_paths(&arg_refs, &request.working_dir)?;
        }
        Ok(())
    }
}

#[allow(dead_code)]
pub mod config {
    pub use crate::SandboxConfig;
    pub use crate::policy::from_env;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SandboxConfig::default();
        assert!(!config.enabled);
        assert!(config.readonly_home);
        assert!(!config.network_access);
        assert!(config.writable_dirs.is_empty());
        assert_eq!(config.max_execution_time_secs, Some(30));
    }

    #[test]
    fn test_from_env() {
        unsafe {
            std::env::set_var("HARPER_SANDBOX_ENABLED", "true");
        }
        let config = config::from_env();
        assert!(config.enabled);
        unsafe {
            std::env::remove_var("HARPER_SANDBOX_ENABLED");
        }
    }

    #[test]
    fn test_sandbox_new() {
        let config = SandboxConfig::default();
        let sandbox = Sandbox::new(config);
        assert!(matches!(
            sandbox.backend,
            SandboxBackend::Bubblewrap | SandboxBackend::SandboxExec | SandboxBackend::None
        ));
    }

    #[test]
    fn test_backend_name() {
        let config = SandboxConfig::default();
        let sandbox = Sandbox::new(config);

        let name = sandbox.backend_name();
        match sandbox.backend {
            SandboxBackend::Bubblewrap => assert_eq!(name, "bubblewrap (bwrap)"),
            SandboxBackend::SandboxExec => assert_eq!(name, "sandbox-exec (macOS)"),
            SandboxBackend::None => assert_eq!(name, "none"),
        }
    }

    #[test]
    fn test_is_available() {
        let config = SandboxConfig::default();
        let sandbox = Sandbox::new(config);
        let available = sandbox.is_available();
        assert_eq!(available, sandbox.backend != SandboxBackend::None);
    }

    #[test]
    fn test_is_command_allowed_no_restrictions() {
        let config = SandboxConfig::default();
        let sandbox = Sandbox::new(config);

        assert!(sandbox.is_command_allowed("ls"));
        assert!(sandbox.is_command_allowed("echo hello"));
        assert!(sandbox.is_command_allowed("rm -rf /"));
    }

    #[test]
    fn test_is_command_allowed_with_blocked() {
        let config = SandboxConfig {
            blocked_commands: Some(vec!["rm".to_string(), "sudo".to_string()]),
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);

        assert!(sandbox.is_command_allowed("ls"));
        assert!(!sandbox.is_command_allowed("rm -rf /"));
        assert!(!sandbox.is_command_allowed("sudo apt update"));
    }

    #[test]
    fn test_is_command_allowed_with_allowed() {
        let config = SandboxConfig {
            allowed_commands: Some(vec!["ls".to_string(), "cat".to_string()]),
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);

        assert!(sandbox.is_command_allowed("ls"));
        assert!(sandbox.is_command_allowed("cat file.txt"));
        assert!(!sandbox.is_command_allowed("rm -rf /"));
    }

    #[test]
    fn test_command_matching_is_exact() {
        let config = SandboxConfig {
            blocked_commands: Some(vec!["rm".to_string()]),
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);

        assert!(!sandbox.is_command_allowed("rm -rf /"));
        assert!(!sandbox.is_command_allowed("/bin/rm -rf /"));
        assert!(sandbox.is_command_allowed("rmdir tmp"));
        assert!(sandbox.is_command_allowed("arm-none-eabi-gcc --version"));
    }

    #[test]
    fn test_blocked_commands_override_allowed_commands() {
        let config = SandboxConfig {
            allowed_commands: Some(vec!["rm".to_string(), "ls".to_string()]),
            blocked_commands: Some(vec!["rm".to_string()]),
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);

        assert!(sandbox.is_command_allowed("ls"));
        assert!(!sandbox.is_command_allowed("rm -rf /"));
    }

    #[test]
    fn test_validate_requested_paths_blocks_traversal_outside_allowed_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("allowed");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let config = SandboxConfig {
            allowed_dirs: vec![allowed.clone()],
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);

        let nested = allowed.join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        let result = sandbox.validate_requested_paths(&["../../outside/file.txt"], &nested);
        assert!(matches!(result, Err(SandboxError::PathBlocked { .. })));
    }

    #[test]
    fn test_validate_working_dir_blocks_disallowed_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("allowed");
        let disallowed = temp_dir.path().join("disallowed");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&disallowed).unwrap();

        let config = SandboxConfig {
            allowed_dirs: vec![allowed],
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);

        let result = sandbox.validate_working_dir(&disallowed);
        assert!(matches!(result, Err(SandboxError::PathBlocked { .. })));
    }

    #[test]
    fn test_validate_request_blocks_disallowed_command() {
        let config = SandboxConfig {
            blocked_commands: Some(vec!["rm".to_string()]),
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);
        let request = SandboxRequest {
            command: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/demo".to_string()],
            working_dir: std::env::current_dir().unwrap(),
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: false,
        };

        let result = sandbox.validate_request(&request);
        assert!(matches!(result, Err(SandboxError::CommandBlocked { .. })));
    }

    #[test]
    fn test_validate_request_blocks_declared_path_outside_allowed_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("allowed");
        let disallowed = temp_dir.path().join("disallowed");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&disallowed).unwrap();

        let config = SandboxConfig {
            allowed_dirs: vec![allowed.clone()],
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);
        let request = SandboxRequest {
            command: "cat".to_string(),
            args: vec!["visible.txt".to_string()],
            working_dir: allowed,
            env: vec![],
            declared_read_paths: vec![disallowed.join("secret.txt")],
            declared_write_paths: vec![],
            requires_network: false,
        };

        let result = sandbox.validate_request(&request);
        assert!(matches!(result, Err(SandboxError::PathBlocked { .. })));
    }

    #[test]
    fn test_validate_request_blocks_network_when_disabled() {
        let sandbox = Sandbox::new(SandboxConfig::default());
        let request = SandboxRequest {
            command: "curl".to_string(),
            args: vec!["https://example.com".to_string()],
            working_dir: std::env::current_dir().unwrap(),
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: true,
        };

        let result = sandbox.validate_request(&request);
        assert!(matches!(result, Err(SandboxError::NetworkBlocked)));
    }

    #[test]
    fn test_validate_request_blocks_write_path_in_readonly_home() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let home_path = std::path::PathBuf::from(&home);
        let sandbox = Sandbox::new(SandboxConfig {
            readonly_home: true,
            allowed_dirs: vec![home_path.clone()],
            ..Default::default()
        });
        let request = SandboxRequest {
            command: "touch".to_string(),
            args: vec!["notes.txt".to_string()],
            working_dir: std::env::current_dir().unwrap(),
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![home_path.join("notes.txt")],
            requires_network: false,
        };

        let result = sandbox.validate_request(&request);
        assert!(matches!(result, Err(SandboxError::PathBlocked { .. })));
    }

    #[test]
    fn test_validate_request_blocks_write_outside_writable_dirs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("allowed");
        let writable = allowed.join("writable");
        let readonly = allowed.join("readonly");
        std::fs::create_dir_all(&writable).unwrap();
        std::fs::create_dir_all(&readonly).unwrap();

        let sandbox = Sandbox::new(SandboxConfig {
            allowed_dirs: vec![allowed.clone()],
            writable_dirs: vec![writable],
            readonly_home: false,
            ..Default::default()
        });
        let request = SandboxRequest {
            command: "touch".to_string(),
            args: vec!["blocked.txt".to_string()],
            working_dir: allowed,
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![readonly.join("blocked.txt")],
            requires_network: false,
        };

        let result = sandbox.validate_request(&request);
        assert!(matches!(result, Err(SandboxError::PathBlocked { .. })));
    }

    #[test]
    fn test_validate_request_falls_back_to_heuristic_path_detection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("allowed");
        let disallowed = temp_dir.path().join("disallowed");
        let nested = allowed.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&disallowed).unwrap();

        let config = SandboxConfig {
            allowed_dirs: vec![allowed],
            ..Default::default()
        };
        let sandbox = Sandbox::new(config);
        let request = SandboxRequest {
            command: "cat".to_string(),
            args: vec!["../../disallowed/file.txt".to_string()],
            working_dir: nested,
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: false,
        };

        let result = sandbox.validate_request(&request);
        assert!(matches!(result, Err(SandboxError::PathBlocked { .. })));
    }

    #[test]
    fn test_execute_request_returns_output_and_backend() {
        let sandbox = Sandbox::new(SandboxConfig::default());
        let request = SandboxRequest {
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            working_dir: std::env::current_dir().unwrap(),
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: false,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(sandbox.execute_request(request)).unwrap();
        assert_eq!(result.backend, SandboxBackend::None);
        assert_eq!(
            String::from_utf8_lossy(&result.output.stdout).trim(),
            "hello"
        );
    }

    #[test]
    fn test_enabled_sandbox_without_backend_fails_closed() {
        let sandbox = Sandbox::with_backend(
            SandboxConfig {
                enabled: true,
                ..Default::default()
            },
            SandboxBackend::None,
        );
        let request = SandboxRequest {
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            working_dir: std::env::current_dir().unwrap(),
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: false,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(sandbox.execute_request(request));
        assert!(matches!(result, Err(SandboxError::BackendUnavailable(_))));
    }

    #[test]
    fn test_execute_request_streaming_emits_chunks() {
        let sandbox = Sandbox::new(SandboxConfig::default());
        let request = SandboxRequest {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "printf 'alpha\\nbeta\\n'".to_string()],
            working_dir: std::env::current_dir().unwrap(),
            env: vec![],
            declared_read_paths: vec![],
            declared_write_paths: vec![],
            requires_network: false,
        };
        let chunks = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, bool)>::new()));
        let collected = chunks.clone();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime
            .block_on(
                sandbox.execute_request_streaming(request, move |chunk, is_error| {
                    collected
                        .lock()
                        .expect("chunks lock")
                        .push((chunk, is_error));
                }),
            )
            .unwrap();

        let chunks = chunks.lock().expect("chunks lock");
        assert!(
            chunks
                .iter()
                .any(|(chunk, is_error)| !is_error && chunk == "alpha\n")
        );
        assert!(
            chunks
                .iter()
                .any(|(chunk, is_error)| !is_error && chunk == "beta\n")
        );
        assert_eq!(
            String::from_utf8_lossy(&result.output.stdout),
            "alpha\nbeta\n"
        );
    }
}
