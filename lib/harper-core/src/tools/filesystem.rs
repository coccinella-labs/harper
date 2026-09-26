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

//! Filesystem operations tool
//!
//! This module provides functionality for reading, writing, and
//! searching files with user approval.

use crate::core::error::{HarperError, HarperResult};
use crate::memory::cache::CacheAlignedBuffer;
use crate::tools::parsing;
use colored::*;
use std::io::{self, Write};
use std::path::Path;

use crate::core::io_traits::{RuntimeEventSink, UserApproval};
use crate::tools::shell::emit_activity_update;
use std::sync::Arc;

fn looks_like_absolute_path(raw_path: &str) -> bool {
    let normalized = raw_path.replace('\\', "/");
    normalized.starts_with('/')
        || normalized.starts_with("~/")
        || normalized.starts_with("//")
        || normalized
            .as_bytes()
            .get(1)
            .is_some_and(|byte| *byte == b':')
}

fn ground_workspace_path_for_cwd(raw_path: &str, cwd: &Path) -> HarperResult<String> {
    let path = Path::new(raw_path);
    if !path.is_absolute() && !looks_like_absolute_path(raw_path) {
        return Ok(raw_path.to_string());
    }

    if path.starts_with(cwd) {
        return Ok(raw_path.to_string());
    }

    if let Some(file_name) = path.file_name() {
        let candidate = cwd.join(file_name);
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }

    Err(HarperError::Command(format!(
        "Refusing file path outside the current workspace: {}. Use a repository-relative path.",
        raw_path
    )))
}

fn ground_workspace_path(raw_path: &str) -> HarperResult<String> {
    let cwd = std::env::current_dir()
        .map_err(|e| HarperError::Command(format!("Failed to get current dir: {}", e)))?;
    ground_workspace_path_for_cwd(raw_path, &cwd)
}

fn should_skip_workspace_dir(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules")
}

fn looks_like_placeholder_path(raw_path: &str) -> bool {
    let normalized = raw_path.replace('\\', "/").to_ascii_lowercase();
    normalized.starts_with("/users/username/")
        || normalized.starts_with("/home/user/")
        || normalized.starts_with("user_request/")
        || normalized.contains("/user_request/")
        || normalized.contains("actual_tool_call")
        || normalized.contains("placeholder")
}

fn find_workspace_file_match_for_cwd(raw_path: &str, cwd: &Path) -> HarperResult<Option<String>> {
    let query = raw_path.trim().trim_start_matches("./");
    if query.is_empty() {
        return Ok(None);
    }

    let query_path = Path::new(query);
    let basename = query_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(query);
    let stem = Path::new(basename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(basename);

    let mut exact_relative = Vec::new();
    let mut exact_basename = Vec::new();
    let mut exact_stem = Vec::new();

    for path in super::search::walk_files(cwd).filter(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !should_skip_workspace_dir(name))
    }) {
        let relative = path.strip_prefix(cwd).unwrap_or(&path);
        let relative_str = relative.to_string_lossy().replace('\\', "/");
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let file_stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");

        if relative_str == query {
            exact_relative.push(path.to_string_lossy().into_owned());
        } else if file_name == basename {
            exact_basename.push(path.to_string_lossy().into_owned());
        } else if file_stem == stem {
            exact_stem.push(path.to_string_lossy().into_owned());
        }
    }

    for matches in [exact_relative, exact_basename, exact_stem] {
        if matches.len() == 1 {
            return Ok(matches.into_iter().next());
        }
        if matches.len() > 1 {
            return Err(HarperError::Command(format!(
                "Multiple workspace files match '{}'. Use a more specific path.",
                raw_path
            )));
        }
    }

    Ok(None)
}

fn resolve_read_target_for_cwd(raw_path: &str, cwd: &Path) -> HarperResult<String> {
    let grounded = match ground_workspace_path_for_cwd(raw_path, cwd) {
        Ok(grounded) => grounded,
        Err(err) => {
            if Path::new(raw_path).is_absolute() || looks_like_absolute_path(raw_path) {
                if let Some(candidate) = find_workspace_file_match_for_cwd(raw_path, cwd)? {
                    return Ok(candidate);
                }
                if looks_like_placeholder_path(raw_path) {
                    let display_name = Path::new(raw_path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(raw_path);
                    return Err(HarperError::Command(format!(
                        "Read target uses a placeholder path not present in this workspace: {}. Inspect the repo and choose a real repository-relative file.",
                        display_name
                    )));
                }
            }
            return Err(err);
        }
    };
    let path = Path::new(&grounded);
    let candidate_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };

    if candidate_path.is_file() {
        return Ok(candidate_path.to_string_lossy().into_owned());
    }

    if !candidate_path.exists() {
        if let Some(candidate) = find_workspace_file_match_for_cwd(raw_path, cwd)? {
            return Ok(candidate);
        }
    }

    Ok(grounded)
}

fn resolve_read_target(raw_path: &str) -> HarperResult<String> {
    let cwd = std::env::current_dir()
        .map_err(|e| HarperError::Command(format!("Failed to get current dir: {}", e)))?;
    resolve_read_target_for_cwd(raw_path, &cwd)
}

fn validate_read_target(path: &str) -> HarperResult<()> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        HarperError::Command(format!("Read target does not exist: {} ({})", path, e))
    })?;

    if !metadata.is_file() {
        return Err(HarperError::Command(format!(
            "Read target is not a file: {}. Choose a file path, not a directory.",
            path
        )));
    }

    Ok(())
}

/// Read a file
pub async fn read_file(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    session_id: Option<&str>,
) -> HarperResult<String> {
    let path = resolve_read_target(&parsing::extract_tool_arg(response, "[READ_FILE")?)?;
    validate_read_target(&path)?;

    if let Some(appr) = approver {
        if !appr.approve("Read file?", &path).await? {
            return Ok("File read cancelled by user".to_string());
        }
    } else if runtime_events.is_none() {
        println!(
            "{} Reading file: {}",
            "System:".bold().magenta(),
            path.magenta()
        );
    } else {
        emit_activity_update(
            runtime_events,
            session_id,
            Some(format!("Reading file: {}", path)),
        )
        .await;
    }

    std::fs::read_to_string(&path)
        .map_err(|e| HarperError::Command(format!("Failed to read file {}: {}", path, e)))
}

/// Write to a file
pub async fn write_file(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    session_id: Option<&str>,
) -> HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[WRITE_FILE", 2)?;
    write_file_direct(&args[0], &args[1], approver, runtime_events, session_id).await
}

pub async fn write_file_direct(
    raw_path: &str,
    content: &str,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    session_id: Option<&str>,
) -> HarperResult<String> {
    let path = ground_workspace_path(raw_path)?;

    let is_approved = if let Some(appr) = approver {
        appr.approve("Write to file?", &path).await?
    } else {
        emit_activity_update(
            runtime_events,
            session_id,
            Some(format!("waiting approval: Write to file {}", path)),
        )
        .await;
        let show_prompt = runtime_events.is_none();
        let p = path.clone();
        tokio::task::spawn_blocking(move || {
            if show_prompt {
                println!(
                    "{} Write to file {}? (y/n): ",
                    "System:".bold().magenta(),
                    p.magenta()
                );
                io::stdout()
                    .flush()
                    .map_err(|e| HarperError::Io(e.to_string()))?;
            }
            let mut approval = String::new();
            io::stdin()
                .read_line(&mut approval)
                .map_err(|e| HarperError::Io(e.to_string()))?;
            Ok::<bool, HarperError>(approval.trim().eq_ignore_ascii_case("y"))
        })
        .await
        .map_err(|e| HarperError::Command(format!("Task failed: {}", e)))??
    };

    if !is_approved {
        return Ok("File write cancelled by user".to_string());
    }

    if runtime_events.is_none() {
        println!(
            "{} Writing to file: {}",
            "System:".bold().magenta(),
            path.magenta()
        );
    } else {
        emit_activity_update(
            runtime_events,
            session_id,
            Some(format!("Writing to file: {}", path)),
        )
        .await;
    }

    write_cache_aligned(&path, content.as_bytes())
        .map_err(|e| HarperError::Command(format!("Failed to write file {}: {}", path, e)))?;

    Ok(format!("Wrote file: {}\nCONTENT: {}", path, content))
}

/// Search and replace in a file
pub async fn search_replace(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
    runtime_events: Option<&Arc<dyn RuntimeEventSink>>,
    session_id: Option<&str>,
) -> HarperResult<String> {
    let args = parsing::extract_tool_args(response, "[SEARCH_REPLACE", 3)?;
    let path = resolve_read_target(&args[0])?;
    validate_read_target(&path)?;
    let old_string = &args[1];
    let new_string = &args[2];

    let is_approved = if let Some(appr) = approver {
        appr.approve("Search and replace in file?", &path).await?
    } else {
        emit_activity_update(
            runtime_events,
            session_id,
            Some(format!(
                "waiting approval: Search and replace in file {}",
                path
            )),
        )
        .await;
        let show_prompt = runtime_events.is_none();
        let p = path.clone();
        tokio::task::spawn_blocking(move || {
            if show_prompt {
                println!(
                    "{} Search and replace in file {}? (y/n): ",
                    "System:".bold().magenta(),
                    p.magenta()
                );
                io::stdout()
                    .flush()
                    .map_err(|e| HarperError::Io(e.to_string()))?;
            }
            let mut approval = String::new();
            io::stdin()
                .read_line(&mut approval)
                .map_err(|e| HarperError::Io(e.to_string()))?;
            Ok::<bool, HarperError>(approval.trim().eq_ignore_ascii_case("y"))
        })
        .await
        .map_err(|e| HarperError::Command(format!("Task failed: {}", e)))??
    };

    if !is_approved {
        return Ok("Search and replace cancelled by user".to_string());
    }

    if runtime_events.is_none() {
        println!(
            "{} Searching and replacing in file: {}",
            "System:".bold().magenta(),
            path.magenta()
        );
    } else {
        emit_activity_update(
            runtime_events,
            session_id,
            Some(format!("Searching and replacing in file: {}", path)),
        )
        .await;
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| HarperError::Command(format!("Failed to read file {}: {}", path, e)))?;

    let new_content = content.replace(old_string, new_string);
    let replacements = content.matches(old_string).count();

    write_cache_aligned(&path, new_content.as_bytes())
        .map_err(|e| HarperError::Command(format!("Failed to write file {}: {}", path, e)))?;

    Ok(format!("Replaced {} occurrences in {}", replacements, path))
}

fn write_cache_aligned(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut buffer = CacheAlignedBuffer::with_capacity(bytes.len());
    buffer.write_bytes(bytes);
    std::fs::write(path, buffer.as_slice())
}

#[cfg(test)]
mod tests {
    use crate::core::error::HarperResult;
    use crate::core::io_traits::UserApproval;
    use async_trait::async_trait;
    use std::path::Path;
    use std::sync::Arc;

    struct AllowApproval;

    #[async_trait]
    impl UserApproval for AllowApproval {
        async fn approve(&self, _prompt: &str, _command: &str) -> HarperResult<bool> {
            Ok(true)
        }
    }
    use super::{
        ground_workspace_path_for_cwd, read_file, resolve_read_target_for_cwd, search_replace,
        validate_read_target,
    };
    use crate::core::error::HarperError;
    use crate::core::io_traits::RuntimeEventSink;

    static CWD_GUARD: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[derive(Default)]
    struct CapturingSink {
        activities: std::sync::Mutex<Vec<String>>,
    }

    impl CapturingSink {
        fn activities(&self) -> Vec<String> {
            self.activities.lock().expect("activities lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl RuntimeEventSink for CapturingSink {
        async fn plan_updated(
            &self,
            _session_id: &str,
            _plan: Option<crate::core::plan::PlanState>,
        ) -> HarperResult<()> {
            Ok(())
        }

        async fn agents_updated(
            &self,
            _session_id: &str,
            _agents: Option<crate::core::agents::ResolvedAgents>,
        ) -> HarperResult<()> {
            Ok(())
        }

        async fn activity_updated(
            &self,
            _session_id: &str,
            status: Option<String>,
        ) -> HarperResult<()> {
            self.activities
                .lock()
                .expect("activities lock")
                .push(status.unwrap_or_default());
            Ok(())
        }

        async fn command_output_updated(
            &self,
            _session_id: &str,
            _command: String,
            _chunk: String,
            _is_error: bool,
            _done: bool,
        ) -> HarperResult<()> {
            Ok(())
        }
    }

    #[test]
    fn grounds_foreign_absolute_path_to_workspace_basename_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_file = temp.path().join("Cargo.toml");
        std::fs::write(&workspace_file, "[package]\nname = \"harper\"\n").expect("write");

        let grounded =
            ground_workspace_path_for_cwd("/home/user/projects/my_project/Cargo.toml", temp.path())
                .expect("grounded");

        assert_eq!(Path::new(&grounded), workspace_file.as_path());
    }

    #[test]
    fn rejects_foreign_absolute_path_without_workspace_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let err = ground_workspace_path_for_cwd(
            "/home/user/projects/my_project/secrets.env",
            temp.path(),
        )
        .expect_err("should reject");

        assert!(matches!(err, HarperError::Command(_)));
        assert!(err.to_string().contains("outside the current workspace"));
    }

    #[test]
    fn validate_read_target_rejects_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let err = validate_read_target(temp.path().to_string_lossy().as_ref())
            .expect_err("should reject");

        assert!(matches!(err, HarperError::Command(_)));
        assert!(err.to_string().contains("not a file"));
    }

    #[test]
    fn resolve_read_target_finds_unique_workspace_basename_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let nested = temp.path().join("crates/harper-core");
        std::fs::create_dir_all(&nested).expect("mkdir");
        let target = nested.join("Cargo.toml");
        std::fs::write(&target, "[package]\nname = \"harper-core\"\n").expect("write");

        let resolved = resolve_read_target_for_cwd("Cargo.toml", temp.path()).expect("resolved");
        assert_eq!(Path::new(&resolved), target.as_path());
    }

    #[test]
    fn resolve_read_target_rejects_ambiguous_workspace_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        std::fs::create_dir_all(&a).expect("mkdir a");
        std::fs::create_dir_all(&b).expect("mkdir b");
        std::fs::write(a.join("README.md"), "a").expect("write a");
        std::fs::write(b.join("README.md"), "b").expect("write b");

        let err = resolve_read_target_for_cwd("README.md", temp.path()).expect_err("ambiguous");
        assert!(matches!(err, HarperError::Command(_)));
        assert!(err.to_string().contains("Multiple workspace files match"));
    }

    #[tokio::test]
    async fn search_replace_grounds_foreign_absolute_path_to_workspace_match() {
        let _cwd_guard = CWD_GUARD.lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let previous = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(temp.path()).expect("set cwd");

        let target = temp.path().join("widgets.rs");
        std::fs::write(
            &target,
            "retry_count
",
        )
        .expect("write target");

        let response = r#"[SEARCH_REPLACE /Users/username/Documents/project/source/widgets.rs retry_count retry_total]"#;
        let result = search_replace(response, Some(Arc::new(AllowApproval)), None, None)
            .await
            .expect("search replace");
        let content = std::fs::read_to_string(&target).expect("read back");

        std::env::set_current_dir(previous).expect("restore cwd");

        assert!(result.contains("Replaced 1 occurrences"));
        assert!(content.contains("retry_total"));
    }

    #[tokio::test]
    async fn read_file_routes_system_notice_through_sink_without_approver() {
        let _cwd_guard = CWD_GUARD.lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let previous = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(temp.path()).expect("set cwd");
        let cwd = std::env::current_dir().expect("current cwd");

        let target = cwd.join("sample.txt");
        std::fs::write(&target, "Filesystem operations tool sample\n").expect("write target");

        let sink = Arc::new(CapturingSink::default());
        let trait_sink: Arc<dyn RuntimeEventSink> = sink.clone();
        let result = read_file(
            "[READ_FILE sample.txt]",
            None,
            Some(&trait_sink),
            Some("session-1"),
        )
        .await;

        let _ = std::env::set_current_dir(previous);
        let result = result.expect("read file");

        assert!(result.contains("Filesystem operations tool sample"));
        let expected = format!("Reading file: {}", target.to_string_lossy());
        assert_eq!(sink.activities(), vec![expected]);
    }
}
