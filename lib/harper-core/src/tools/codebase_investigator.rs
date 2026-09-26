// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Codebase investigation tool for deep structural analysis

use crate::core::error::{HarperError, HarperResult};
use crate::core::io_traits::UserApproval;
use crate::tools::parsing;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use syn::visit::Visit;
use tempfile::tempdir;

#[derive(Debug, Clone)]
struct WorkspaceGraph {
    root: String,
    package_name: Option<String>,
    members: Vec<WorkspaceMemberOverview>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    workspace_root: String,
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: String,
    targets: Vec<CargoTarget>,
}

#[derive(Debug, Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
    src_path: String,
}

#[derive(Debug, Clone)]
struct WorkspaceMemberOverview {
    name: String,
    role: String,
    root: String,
    entrypoints: Vec<String>,
    notable_files: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct CompilerContext {
    target_ownership: BTreeMap<String, Vec<String>>,
    diagnostics: Vec<CompilerDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CargoCheckTarget {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
    #[serde(default)]
    src_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CargoCheckMessage {
    reason: String,
    #[serde(default)]
    target: Option<CargoCheckTarget>,
    #[serde(default)]
    message: Option<RustcMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RustcMessage {
    level: String,
    #[serde(default)]
    rendered: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    spans: Vec<RustcSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RustcSpan {
    file_name: String,
    line_start: usize,
    #[serde(default)]
    is_primary: bool,
}

#[derive(Debug, Clone)]
struct CompilerDiagnostic {
    file: String,
    line: usize,
    level: String,
    message: String,
}

#[derive(Debug, Clone)]
struct SearchMatch {
    score: i32,
    path: String,
    role: String,
    reasons: Vec<String>,
    snippets: Vec<String>,
}

#[derive(Debug, Clone)]
struct RustSemanticFile {
    path: String,
    module_path: String,
    defines: Vec<String>,
    type_aliases: BTreeMap<String, String>,
    imports: Vec<String>,
    import_map: BTreeMap<String, String>,
    modules: Vec<String>,
    calls: Vec<String>,
    method_owners: BTreeMap<String, Vec<String>>,
    method_traits: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryFocus {
    UiRendering,
    StateFlow,
    Tooling,
    Runtime,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchIntentKind {
    General,
    Used,
    Calls,
    Defined,
}

/// Investigate codebase structural graph and relationships
pub async fn investigate_codebase(
    response: &str,
    approver: Option<Arc<dyn UserApproval>>,
) -> HarperResult<String> {
    let action_prefix = "[CODEBASE_INVESTIGATE";
    let action = parsing::extract_tool_args(response, action_prefix, 2)?[0].clone();

    match action.as_str() {
        "find_calls" => {
            let symbol = parsing::extract_tool_args(response, action_prefix, 2)?[1].clone();
            find_symbol_calls(&symbol).await
        }
        "trace_relationship" => {
            let args = parsing::extract_tool_args(response, action_prefix, 3)?;
            let x = &args[1];
            let y = &args[2];
            trace_relationship(x, y).await
        }
        "clone_context" => {
            let repo_url = parsing::extract_tool_args(response, action_prefix, 2)?[1].clone();
            clone_temp_context(&repo_url, approver).await
        }
        "search_text" => {
            let query = parsing::extract_tool_args(response, action_prefix, 2)?[1].clone();
            search_text(&query).await
        }
        _ => Err(HarperError::Command(format!(
            "Unknown investigation action: {}",
            action
        ))),
    }
}

pub async fn search_text(query: &str) -> HarperResult<String> {
    search_text_with_intent(query, SearchIntentKind::General).await
}

pub async fn search_text_with_intent(
    query: &str,
    intent: SearchIntentKind,
) -> HarperResult<String> {
    let matches = collect_search_matches_with_intent(query, intent)?;
    if matches.is_empty() {
        return Ok(format!("No source-focused matches found for '{}'.", query));
    }

    let focus = infer_query_focus(
        query,
        &query
            .split_whitespace()
            .map(|term| term.trim().to_ascii_lowercase())
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>(),
    );

    Ok(format!(
        "CODEBASE_SEARCH\nQUERY: {}\nFOCUS: {}\nINTENT: {}\nTOP_MATCHES:\n{}",
        query,
        focus.as_str(),
        intent.as_str(),
        matches
            .into_iter()
            .take(8)
            .map(|entry| format_search_match(&entry))
            .collect::<Vec<_>>()
            .join("\n\n")
    ))
}

fn collect_search_matches(query: &str) -> HarperResult<Vec<SearchMatch>> {
    collect_search_matches_with_intent(query, SearchIntentKind::General)
}

fn collect_search_matches_with_intent(
    query: &str,
    intent: SearchIntentKind,
) -> HarperResult<Vec<SearchMatch>> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect();

    if terms.is_empty() {
        return Err(HarperError::Command(
            "Search query cannot be empty.".to_string(),
        ));
    }

    let focus = infer_query_focus(query, &terms);
    let symbol_variants = search_symbol_variants(&terms);
    let semantic_target = infer_semantic_symbol_target(query, &terms);

    let mut matches: Vec<SearchMatch> = Vec::new();
    for path in super::search::walk_files(Path::new(".")).filter(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !should_skip_entry(name))
    }) {
        if !is_searchable_file(&path) {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = content.to_ascii_lowercase();
        if !terms.iter().all(|term| lower.contains(term)) {
            continue;
        }

        let mut scored_matches = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if let Some(score) = search_line_score(line, &terms, &symbol_variants, intent) {
                scored_matches.push((score, idx, format!("{}: {}", idx + 1, line.trim())));
            }
        }

        scored_matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let line_score: i32 = scored_matches
            .iter()
            .take(3)
            .map(|(score, _, _)| *score)
            .sum();
        let file_matches: Vec<String> = scored_matches
            .into_iter()
            .take(3)
            .map(|(_, _, rendered)| rendered)
            .collect();

        if file_matches.is_empty() {
            continue;
        }

        let semantic_bonus = extract_rust_semantic_file(&path)
            .map(|semantic| search_semantic_bonus(&semantic, semantic_target.as_deref(), intent))
            .unwrap_or(0);
        let score = search_match_score(&path, &lower, &file_matches, &terms, focus, intent)
            + line_score
            + semantic_bonus;
        matches.push(SearchMatch {
            score,
            path: path.display().to_string(),
            role: classify_file_role(&path),
            reasons: search_match_reasons(&path, &lower, &terms, focus),
            snippets: file_matches,
        });
    }

    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    Ok(matches)
}

fn infer_semantic_symbol_target(query: &str, terms: &[String]) -> Option<String> {
    if let Some(start) = query.find('`') {
        let rest = &query[start + 1..];
        if let Some(end) = rest.find('`') {
            let symbol = rest[..end].trim();
            if !symbol.is_empty() {
                return Some(symbol.to_string());
            }
        }
    }

    let symbol_like_tokens = query
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':')))
        .filter(|token| {
            !token.is_empty()
                && (token.contains('_')
                    || token.contains("::")
                    || token.chars().any(|ch| ch.is_ascii_uppercase()))
        })
        .collect::<Vec<_>>();
    if let Some(symbol) = symbol_like_tokens.first() {
        return Some((*symbol).to_string());
    }

    if terms.len() == 1 {
        Some(terms[0].clone())
    } else {
        Some(terms.join("_"))
    }
}

fn search_line_score(
    line: &str,
    terms: &[String],
    symbol_variants: &[String],
    intent: SearchIntentKind,
) -> Option<i32> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    let compact = compact_search_text(trimmed);
    let compact_terms = compact_search_text(&terms.join(""));
    let snake_case_variant = terms.join("_");
    let camel_case_variant = terms
        .iter()
        .map(|term| {
            let mut chars = term.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<String>()
        .to_ascii_lowercase();
    let enum_decl = format!("enum {camel_case_variant}");
    let struct_decl = format!("struct {camel_case_variant}");
    let trait_decl = format!("trait {camel_case_variant}");
    let type_decl = format!("type {camel_case_variant}");
    let fn_decl = format!("fn {snake_case_variant}(");
    let has_any_term = terms.iter().any(|term| lower.contains(term));
    if !has_any_term {
        return None;
    }

    let is_license_noise = lower.starts_with("//")
        || lower.starts_with("/*")
        || lower.starts_with('*')
        || lower.contains("licensed under")
        || lower.contains("apache license")
        || lower.contains("http://www.apache.org/licenses");
    let mut score = 5;

    if terms.iter().all(|term| lower.contains(term)) {
        score += 50;
    }
    if compact.contains(&compact_terms) {
        score += 100;
    }
    if lower.contains(&snake_case_variant) {
        score += 80;
    }
    if lower.contains(&camel_case_variant) {
        score += 80;
    }
    if symbol_variants
        .iter()
        .any(|variant| lower.contains(variant))
    {
        score += 40;
    }
    if lower.contains(&format!(".{snake_case_variant}"))
        || lower.contains(&format!("{snake_case_variant}("))
        || lower.contains(&format!("({snake_case_variant}"))
        || lower.contains(&format!("{snake_case_variant} ="))
        || lower.contains(&format!("{snake_case_variant}:"))
    {
        score += 45;
    }
    if lower.starts_with("use ") {
        score -= 35;
    }
    if lower.starts_with("fn ")
        || lower.starts_with("pub fn ")
        || lower.starts_with("struct ")
        || lower.starts_with("enum ")
        || lower.starts_with("impl ")
    {
        score -= 10;
    }
    if lower.contains("assert!(")
        || lower.contains("assert_eq!(")
        || lower.contains("assert_ne!(")
        || lower.contains("contains(\"")
        || lower.contains("expect(\"")
        || lower.starts_with("#[test]")
    {
        score -= 140;
    }
    if is_license_noise {
        score -= 120;
    }

    match intent {
        SearchIntentKind::Used => {
            if lower.starts_with("use ") {
                score -= 35;
            }
            if lower.starts_with("fn ")
                || lower.starts_with("pub fn ")
                || lower.starts_with("struct ")
                || lower.starts_with("enum ")
                || lower.starts_with("type ")
                || lower.starts_with("pub type ")
            {
                score -= 20;
            }
            if lower.contains(&format!(".{snake_case_variant}"))
                || lower.contains(&format!("{snake_case_variant} ="))
                || lower.contains(&format!("{snake_case_variant}:"))
                || lower.contains(&format!("{snake_case_variant}("))
                || lower.contains(&format!("app.{snake_case_variant}"))
            {
                score += 55;
            }
        }
        SearchIntentKind::Calls => {
            if lower.starts_with("fn ") || lower.starts_with("pub fn ") {
                score -= 55;
            }
            if lower.contains(&format!("{snake_case_variant}("))
                || lower.contains(&format!(".{snake_case_variant}("))
            {
                score += 70;
            }
        }
        SearchIntentKind::Defined => {
            if lower.contains(&enum_decl)
                || lower.contains(&struct_decl)
                || lower.contains(&trait_decl)
                || lower.contains(&type_decl)
                || lower.contains(&fn_decl)
            {
                score += 140;
            } else if lower.starts_with("fn ")
                || lower.starts_with("pub fn ")
                || lower.starts_with("struct ")
                || lower.starts_with("enum ")
                || lower.starts_with("type ")
                || lower.starts_with("pub type ")
                || lower.starts_with("trait ")
                || lower.starts_with("pub trait ")
                || lower.starts_with("const ")
                || lower.starts_with("pub const ")
            {
                score += 25;
            }
            if lower.starts_with("use ") {
                score -= 45;
            }
            if lower.contains(&format!(": {camel_case_variant}"))
                || lower.contains(&format!("-> {camel_case_variant}"))
                || lower.contains(&format!("({camel_case_variant}"))
            {
                score -= 35;
            }
            if lower.contains(&format!(".{snake_case_variant}("))
                || lower.contains(&format!(".{snake_case_variant}"))
            {
                score -= 20;
            }
        }
        SearchIntentKind::General => {}
    }

    (score > 20).then_some(score)
}

fn search_symbol_variants(terms: &[String]) -> Vec<String> {
    if terms.is_empty() {
        return Vec::new();
    }

    let snake_case = terms.join("_");
    let camel_case = terms
        .iter()
        .map(|term| {
            let mut chars = term.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<String>()
        .to_ascii_lowercase();

    let mut variants = vec![snake_case, camel_case, compact_search_text(&terms.join(""))];
    variants.sort();
    variants.dedup();
    variants
}

fn compact_search_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub async fn authoring_context(query: &str) -> HarperResult<String> {
    let workspace_graph = load_workspace_graph().ok();
    let compiler_context = load_compiler_context(workspace_graph.as_ref()).ok();
    let overview = if let Some(graph) = &workspace_graph {
        format_workspace_graph(graph)
    } else {
        overview_snapshot().await?
    };
    let search_query = authoring_search_query(query);
    let candidates = if search_query.is_empty() {
        Vec::new()
    } else {
        collect_search_matches(&search_query)?
    };
    let primary_files = candidates
        .iter()
        .take(4)
        .map(|entry| {
            format_authoring_primary_file(
                entry,
                workspace_graph.as_ref(),
                compiler_context.as_ref(),
            )
        })
        .collect::<Vec<_>>();
    let related_files = infer_related_files(
        &candidates,
        workspace_graph.as_ref(),
        compiler_context.as_ref(),
    );
    let edit_plan = infer_edit_plan_candidates(
        &candidates,
        workspace_graph.as_ref(),
        compiler_context.as_ref(),
    );
    let semantic_graph = build_semantic_graph(&candidates);
    let compiler_summary = compiler_context
        .as_ref()
        .map(|ctx| format_compiler_context(ctx, &candidates))
        .unwrap_or_else(|| "Compiler context unavailable.".to_string());

    Ok(format!(
        "AUTHORING_CONTEXT
REQUEST: {}
SEARCH_QUERY: {}
WORKSPACE:
{}

PRIMARY_FILES:
{}

RELATED_FILES:
{}

EDIT_PLAN_CANDIDATES:
{}

SEMANTIC_GRAPH:
{}

COMPILER_CONTEXT:
{}",
        query,
        if search_query.is_empty() {
            "<none>"
        } else {
            search_query.as_str()
        },
        overview,
        if primary_files.is_empty() {
            "No primary files identified.".to_string()
        } else {
            primary_files.join(
                "

",
            )
        },
        if related_files.is_empty() {
            "No related files identified.".to_string()
        } else {
            related_files.join(
                "
",
            )
        },
        if edit_plan.is_empty() {
            "No edit-plan candidates identified.".to_string()
        } else {
            edit_plan.join(
                "
",
            )
        },
        semantic_graph,
        compiler_summary
    ))
}

pub async fn overview_snapshot() -> HarperResult<String> {
    if let Ok(graph) = load_workspace_graph() {
        return Ok(format_workspace_graph(&graph));
    }

    let cwd = std::env::current_dir()
        .map_err(|e| HarperError::Command(format!("Failed to inspect workspace: {}", e)))?;
    Ok(format!("Workspace root: {}", cwd.display()))
}

fn load_workspace_graph() -> HarperResult<WorkspaceGraph> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| HarperError::Command(format!("cargo metadata failed: {}", e)))?;

    if !output.status.success() {
        return Err(HarperError::Command(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let metadata: CargoMetadata = serde_json::from_slice(&output.stdout)?;
    let root = metadata.workspace_root.clone();
    let members = metadata
        .packages
        .iter()
        .filter(|package| {
            metadata
                .workspace_members
                .iter()
                .any(|member| member == &package.id)
        })
        .map(summarize_workspace_package)
        .collect::<Vec<_>>();
    let package_name = members.first().map(|member| member.name.clone());

    Ok(WorkspaceGraph {
        root,
        package_name,
        members,
    })
}

fn summarize_workspace_package(package: &CargoPackage) -> WorkspaceMemberOverview {
    let manifest_path = Path::new(&package.manifest_path);
    let root = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let root_str = root.display().to_string();
    let role = classify_member_role(&package.name, &root_str);
    let mut entrypoints = package
        .targets
        .iter()
        .filter(|target| {
            target
                .kind
                .iter()
                .any(|kind| matches!(kind.as_str(), "bin" | "lib"))
        })
        .map(|target| path_relative_to_root(&root, Path::new(&target.src_path)))
        .collect::<Vec<_>>();
    entrypoints.sort();
    entrypoints.dedup();

    let notable_candidates = [
        "src/agent/chat.rs",
        "src/tools/mod.rs",
        "src/tools/codebase_investigator.rs",
        "src/runtime/config.rs",
        "src/interfaces/ui/widgets.rs",
        "src/interfaces/ui/events.rs",
        "src/server/mod.rs",
    ];
    let notable_files = notable_candidates
        .iter()
        .filter_map(|candidate| {
            let candidate_path = root.join(candidate);
            if candidate_path.exists() {
                Some(candidate.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    WorkspaceMemberOverview {
        name: package.name.clone(),
        role,
        root: root_str,
        entrypoints,
        notable_files,
    }
}

fn format_workspace_graph(graph: &WorkspaceGraph) -> String {
    let mut top_level = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&graph.root) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if matches!(name, ".git" | "target" | "node_modules" | ".harper") {
                    continue;
                }
                top_level.push(name.to_string());
            }
        }
    }
    top_level.sort();

    let mut summary = vec![format!("Workspace root: {}", graph.root)];
    if let Some(package_name) = &graph.package_name {
        summary.push(format!("Workspace package: {}", package_name));
    }
    if !graph.members.is_empty() {
        summary.push(format!(
            "Workspace members: {}",
            graph
                .members
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !top_level.is_empty() {
        summary.push(format!(
            "Top-level entries: {}",
            top_level
                .into_iter()
                .take(12)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !graph.members.is_empty() {
        summary.push("Workspace map:".to_string());
        for overview in &graph.members {
            summary.push(format!(
                "- crate={} role={} root={} entrypoints=[{}] notable=[{}]",
                overview.name,
                overview.role,
                overview.root,
                overview.entrypoints.join(", "),
                overview.notable_files.join(", ")
            ));
        }
    }

    summary.join("\n")
}

fn load_compiler_context(graph: Option<&WorkspaceGraph>) -> HarperResult<CompilerContext> {
    let output = Command::new("cargo")
        .args([
            "check",
            "--workspace",
            "--all-targets",
            "--message-format=json",
        ])
        .output()
        .map_err(|e| HarperError::Command(format!("cargo check failed: {}", e)))?;

    let mut context = CompilerContext::default();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(message) = serde_json::from_str::<CargoCheckMessage>(line) else {
            continue;
        };
        if let Some(target) = message.target {
            if !target.src_path.is_empty() {
                let path = relativize_workspace_path(graph, &target.src_path);
                let kinds = if target.kind.is_empty() {
                    vec!["unknown".to_string()]
                } else {
                    target.kind
                };
                context
                    .target_ownership
                    .entry(path)
                    .or_default()
                    .extend(kinds);
            }
        }
        if message.reason != "compiler-message" {
            continue;
        }
        let Some(msg) = message.message else {
            continue;
        };
        for span in msg.spans.iter().filter(|span| span.is_primary) {
            if should_ignore_compiler_path(&span.file_name) {
                continue;
            }
            context.diagnostics.push(CompilerDiagnostic {
                file: relativize_workspace_path(graph, &span.file_name),
                line: span.line_start,
                level: msg.level.clone(),
                message: msg.message.clone(),
            });
        }
    }
    for kinds in context.target_ownership.values_mut() {
        kinds.sort();
        kinds.dedup();
    }
    Ok(context)
}

fn relativize_workspace_path(graph: Option<&WorkspaceGraph>, raw: &str) -> String {
    let path = Path::new(raw);
    if let Some(graph) = graph {
        let root = Path::new(&graph.root);
        if let Ok(rel) = path.strip_prefix(root) {
            return rel.display().to_string();
        }
    }
    raw.trim_start_matches("./").to_string()
}

fn should_ignore_compiler_path(path: &str) -> bool {
    path.contains("/target/") || path.ends_with("build.rs")
}

fn format_compiler_context(context: &CompilerContext, candidates: &[SearchMatch]) -> String {
    let candidate_paths = candidates
        .iter()
        .map(|entry| entry.path.trim_start_matches("./").to_string())
        .collect::<BTreeSet<_>>();
    let mut target_lines = Vec::new();
    for (path, kinds) in &context.target_ownership {
        if candidate_paths.iter().any(|candidate| {
            candidate == path || candidate.ends_with(path) || path.ends_with(candidate)
        }) {
            target_lines.push(format!(
                "- target_owner: {} kinds=[{}]",
                path,
                kinds.join(", ")
            ));
        }
    }
    let mut diag_lines = Vec::new();
    for diagnostic in &context.diagnostics {
        if candidate_paths.iter().any(|candidate| {
            candidate == &diagnostic.file
                || candidate.ends_with(&diagnostic.file)
                || diagnostic.file.ends_with(candidate)
        }) {
            diag_lines.push(format!(
                "- diagnostic: {}:{} level={} message={}",
                diagnostic.file, diagnostic.line, diagnostic.level, diagnostic.message
            ));
        }
        if diag_lines.len() >= 8 {
            break;
        }
    }
    let ownership = if target_lines.is_empty() {
        "No candidate target ownership found.".to_string()
    } else {
        target_lines.join(
            "
",
        )
    };
    let diagnostics = if diag_lines.is_empty() {
        "No compiler diagnostics for current candidates.".to_string()
    } else {
        diag_lines.join(
            "
",
        )
    };
    format!(
        "TARGET_OWNERSHIP:
{}
DIAGNOSTICS:
{}",
        ownership, diagnostics
    )
}

fn classify_member_role(name: &str, root: &str) -> String {
    if name.contains("core") || root.contains("harper-core") {
        "runtime/tools/storage/agent".to_string()
    } else if name.contains("ui") || root.contains("harper-ui") {
        "tui/widgets/events".to_string()
    } else if name.contains("mcp") {
        "mcp_integration".to_string()
    } else if name.contains("sandbox") || root.contains("harper-sandbox") {
        "sandbox_execution".to_string()
    } else if name.contains("firmware") || root.contains("harper-firmware") {
        "firmware_support".to_string()
    } else {
        "workspace_member".to_string()
    }
}

fn path_relative_to_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn workspace_member_for_path<'a>(
    path: &Path,
    graph: Option<&'a WorkspaceGraph>,
) -> Option<&'a WorkspaceMemberOverview> {
    let graph = graph?;
    let path_str = path.display().to_string();
    graph
        .members
        .iter()
        .find(|member| path_str.starts_with(&member.root))
}

fn authoring_search_query(query: &str) -> String {
    let lowered = query.to_ascii_lowercase();
    let cleaned = lowered.replace(
        |c: char| !c.is_ascii_alphanumeric() && !c.is_ascii_whitespace(),
        " ",
    );
    let stop_words = [
        "create",
        "make",
        "modify",
        "update",
        "change",
        "edit",
        "refactor",
        "implement",
        "add",
        "build",
        "wire",
        "fix",
        "the",
        "a",
        "an",
        "this",
        "that",
        "these",
        "those",
        "repo",
        "repository",
        "codebase",
        "project",
        "file",
        "files",
        "screen",
        "feature",
        "subsystem",
        "behavior",
        "in",
        "on",
        "for",
        "to",
        "of",
        "with",
        "and",
        "or",
        "please",
        "can",
        "you",
        "we",
        "are",
        "is",
        "be",
        "it",
        "flow",
    ];

    cleaned
        .split_whitespace()
        .filter(|term| term.len() > 2 && !stop_words.contains(term))
        .take(6)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

async fn find_symbol_calls(symbol: &str) -> HarperResult<String> {
    println!(
        "{} Searching for all callers of: {}",
        "System:".bold().magenta(),
        symbol.magenta()
    );

    let hits = super::search::search_files(Path::new("."), symbol);
    if hits.is_empty() {
        Ok(format!("No callers found for symbol: {}", symbol))
    } else {
        let mut result = String::new();
        for (path, lines) in &hits {
            for (line_no, line) in lines {
                result.push_str(&format!("{}:{}:{}\n", path.display(), line_no, line));
            }
        }
        Ok(format!("Callers found for {}:\n{}", symbol, result))
    }
}

async fn trace_relationship(x: &str, y: &str) -> HarperResult<String> {
    println!(
        "{} Tracing relationship between {} and {}",
        "System:".bold().magenta(),
        x.magenta(),
        y.magenta()
    );

    let x_files = super::search::files_with_matches(Path::new("."), x);
    let mut relationships = Vec::new();

    for file in &x_files {
        let has_y = file_contains_symbol(&file.to_string_lossy(), y)?;

        if has_y {
            relationships.push(format!("Found both in: {}", file.display()));
        }
    }

    if relationships.is_empty() {
        Ok(format!(
            "No direct file-level relationship found between {} and {}",
            x, y
        ))
    } else {
        Ok(format!(
            "Relationships found:\n{}",
            relationships.join("\n")
        ))
    }
}

async fn clone_temp_context(
    repo_url: &str,
    approver: Option<Arc<dyn UserApproval>>,
) -> HarperResult<String> {
    println!(
        "{} Cloning temporary context from: {}",
        "System:".bold().magenta(),
        repo_url.magenta()
    );

    if let Some(appr) = approver {
        if !appr
            .approve("Clone repository for investigation?", repo_url)
            .await?
        {
            return Ok("Repository clone cancelled by user".to_string());
        }
    }

    let dir =
        tempdir().map_err(|e| HarperError::Command(format!("Failed to create temp dir: {}", e)))?;
    let path = dir.path();
    let path_str = path
        .to_str()
        .ok_or_else(|| HarperError::Command("Temporary path is not valid UTF-8".to_string()))?;

    let output = Command::new("git")
        .args(["clone", "--depth", "1", repo_url, path_str])
        .output()
        .map_err(|e| HarperError::Command(format!("Git clone failed: {}", e)))?;

    if !output.status.success() {
        return Err(HarperError::Command(format!(
            "Failed to clone repository {}: {}",
            repo_url,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    // Analyze the cloned repo briefly
    let file_count = super::search::walk_files(Path::new(path)).count();

    Ok(format!(
        "Cloned {} to temporary directory. Found {} items for context.",
        repo_url, file_count
    ))
}

fn file_contains_symbol(file: &str, symbol: &str) -> HarperResult<bool> {
    Ok(super::search::file_contains(Path::new(file), symbol))
}

fn should_skip_entry(name: &str) -> bool {
    matches!(name, "search" | "site" | "website" | "docs")
}

fn is_searchable_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    if path_str.contains("/tests/")
        || path_str.ends_with("/lib/harper-core/src/agent/intent.rs")
        || path_str.ends_with("/lib/harper-core/src/agent/prompt.rs")
        || path_str.ends_with("/lib/harper-core/src/agent/chat.rs")
        || path_str.starts_with("tests/")
        || path_str == "lib/harper-core/src/agent/intent.rs"
        || path_str == "lib/harper-core/src/agent/prompt.rs"
        || path_str == "lib/harper-core/src/agent/chat.rs"
    {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if file_name.ends_with(".lock")
        || file_name.ends_with(".db")
        || file_name.ends_with(".min.js")
        || file_name.ends_with(".map")
        || file_name.contains("lock")
    {
        return false;
    }

    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    matches!(ext, "rs" | "toml" | "js" | "ts" | "py" | "sh")
}

fn classify_file_role(path: &Path) -> String {
    let path_str = path.to_string_lossy();
    if path_str.contains("lib/harper-ui/src/interfaces/ui/widgets.rs") {
        return "ui_widget_rendering".to_string();
    }
    if path_str.contains("lib/harper-ui/src/interfaces/ui/") {
        return "ui".to_string();
    }
    if path_str.contains("lib/harper-core/src/runtime/") {
        return "runtime".to_string();
    }
    if path_str.contains("lib/harper-core/src/tools/") {
        return "tooling".to_string();
    }
    if path_str.contains("lib/harper-core/src/agent/") {
        return "agent".to_string();
    }
    if path_str.contains("lib/harper-sandbox/") {
        return "sandbox".to_string();
    }
    "source".to_string()
}

fn search_match_reasons(
    path: &Path,
    content_lower: &str,
    terms: &[String],
    focus: QueryFocus,
) -> Vec<String> {
    let mut reasons = Vec::new();
    let role = classify_file_role(path);
    reasons.push(format!("role={}", role));
    if terms.iter().all(|term| content_lower.contains(term)) {
        reasons.push("contains_all_terms".to_string());
    }
    let path_str = path.to_string_lossy();
    if path_str.contains("widgets.rs") {
        reasons.push("widget_render_path".to_string());
    }
    if content_lower.contains("planfollowup") || content_lower.contains("retry_count") {
        reasons.push("followup_state_reference".to_string());
    }
    match focus {
        QueryFocus::UiRendering if role == "ui_widget_rendering" || role == "ui" => {
            reasons.push("matches_ui_render_focus".to_string())
        }
        QueryFocus::StateFlow if content_lower.contains("planfollowup") => {
            reasons.push("matches_state_flow_focus".to_string())
        }
        QueryFocus::Tooling if role == "tooling" => {
            reasons.push("matches_tooling_focus".to_string())
        }
        QueryFocus::Runtime if role == "runtime" => {
            reasons.push("matches_runtime_focus".to_string())
        }
        _ => {}
    }
    reasons
}

fn format_search_match(entry: &SearchMatch) -> String {
    format!(
        "FILE: {}\nROLE: {}\nREASONS: {}\nSNIPPETS:\n{}",
        entry.path,
        entry.role,
        entry.reasons.join(", "),
        entry
            .snippets
            .iter()
            .map(|snippet| format!("- {}", snippet))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn format_authoring_primary_file(
    entry: &SearchMatch,
    graph: Option<&WorkspaceGraph>,
    compiler: Option<&CompilerContext>,
) -> String {
    let path = Path::new(&entry.path);
    let symbols = extract_file_symbols(path);
    let crate_name = workspace_member_for_path(path, graph)
        .map(|member| member.name.as_str())
        .unwrap_or("<unknown>");
    let compiler_notes = compiler
        .map(|ctx| {
            let target = ctx
                .target_ownership
                .iter()
                .find(|(path, _)| {
                    entry.path.trim_start_matches("./") == path.as_str()
                        || entry.path.ends_with(path.as_str())
                        || path.ends_with(entry.path.trim_start_matches("./"))
                })
                .map(|(_, kinds)| kinds.join(", "))
                .unwrap_or_else(|| "<none>".to_string());
            let diagnostics = ctx
                .diagnostics
                .iter()
                .filter(|diag| {
                    entry.path.trim_start_matches("./") == diag.file
                        || entry.path.ends_with(&diag.file)
                        || diag.file.ends_with(entry.path.trim_start_matches("./"))
                })
                .take(2)
                .map(|diag| format!("{}:{} {}", diag.file, diag.line, diag.message))
                .collect::<Vec<_>>();
            format!(
                "TARGET_KINDS: {}
DIAGNOSTICS: {}",
                target,
                if diagnostics.is_empty() {
                    "<none>".to_string()
                } else {
                    diagnostics.join(" | ")
                }
            )
        })
        .unwrap_or_else(|| {
            "TARGET_KINDS: <none>
DIAGNOSTICS: <none>"
                .to_string()
        });
    format!(
        "FILE: {}
ROLE: {}
CRATE: {}
WHY: {}
SYMBOLS: {}
{}
SNIPPETS:
{}",
        entry.path,
        entry.role,
        crate_name,
        entry.reasons.join(", "),
        if symbols.is_empty() {
            "<none>".to_string()
        } else {
            symbols.join(", ")
        },
        compiler_notes,
        entry
            .snippets
            .iter()
            .map(|snippet| format!("- {}", snippet))
            .collect::<Vec<_>>()
            .join(
                "
"
            )
    )
}
fn extract_file_symbols(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut symbols = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let candidate = if let Some(rest) = trimmed
            .strip_prefix("pub fn ")
            .or_else(|| trimmed.strip_prefix("fn "))
        {
            rest.split('(').next().map(str::trim)
        } else if let Some(rest) = trimmed
            .strip_prefix("pub struct ")
            .or_else(|| trimmed.strip_prefix("struct "))
            .or_else(|| trimmed.strip_prefix("pub enum "))
            .or_else(|| trimmed.strip_prefix("enum "))
            .or_else(|| trimmed.strip_prefix("pub trait "))
            .or_else(|| trimmed.strip_prefix("trait "))
            .or_else(|| trimmed.strip_prefix("impl "))
        {
            rest.split_whitespace().next().map(str::trim)
        } else if let Some(rest) = trimmed
            .strip_prefix("class ")
            .or_else(|| trimmed.strip_prefix("def "))
        {
            rest.split(['(', ':']).next().map(str::trim)
        } else {
            None
        };
        if let Some(symbol) = candidate.filter(|value| !value.is_empty()) {
            symbols.push(symbol.to_string());
        }
        if symbols.len() >= 6 {
            break;
        }
    }
    symbols
}

fn infer_related_files(
    matches: &[SearchMatch],
    graph: Option<&WorkspaceGraph>,
    compiler: Option<&CompilerContext>,
) -> Vec<String> {
    let mut related = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in matches.iter().take(6) {
        let path = Path::new(&entry.path);
        if let Some(member) = workspace_member_for_path(path, graph) {
            let crate_line = format!(
                "- crate: {} role={} root={}",
                member.name, member.role, member.root
            );
            if seen.insert(crate_line.clone()) {
                related.push(crate_line);
            }
        }
        if let Some(parent) = path.parent() {
            let parent_line = format!("- dir: {}", parent.display());
            if seen.insert(parent_line.clone()) {
                related.push(parent_line);
            }
        }
        if let Some(ctx) = compiler {
            let rel_path = entry.path.trim_start_matches("./");
            if let Some(kinds) = ctx
                .target_ownership
                .iter()
                .find(|(owned, _)| {
                    rel_path == owned.as_str()
                        || rel_path.ends_with(owned.as_str())
                        || owned.ends_with(rel_path)
                })
                .map(|(_, kinds)| kinds)
            {
                let target_line = format!("- target: {} kinds=[{}]", rel_path, kinds.join(", "));
                if seen.insert(target_line.clone()) {
                    related.push(target_line);
                }
            }
        }
        if related.len() >= 8 {
            break;
        }
    }
    related
}

fn infer_edit_plan_candidates(
    matches: &[SearchMatch],
    graph: Option<&WorkspaceGraph>,
    compiler: Option<&CompilerContext>,
) -> Vec<String> {
    let Some(primary) = matches.first() else {
        return Vec::new();
    };

    let primary_crate = workspace_member_for_path(Path::new(&primary.path), graph)
        .map(|member| member.name.clone())
        .unwrap_or_else(|| "<unknown>".to_string());
    let primary_validation = validation_hint_for_path(&primary.path, compiler);
    let mut plan = vec![format!(
        "- PRIMARY: {} [crate={}] role={} ({}, {})",
        primary.path,
        primary_crate,
        authoring_edit_role(&primary.role, true),
        primary.reasons.join(", "),
        primary_validation
    )];

    for entry in matches.iter().skip(1).take(4) {
        let crate_name = workspace_member_for_path(Path::new(&entry.path), graph)
            .map(|member| member.name.clone())
            .unwrap_or_else(|| "<unknown>".to_string());
        let support_reason = support_relation_reason(primary, entry, graph, compiler);
        let validation_hint = validation_hint_for_path(&entry.path, compiler);
        plan.push(format!(
            "- SUPPORTING: {} [crate={}] role={} ({}, {}, {})",
            entry.path,
            crate_name,
            authoring_edit_role(&entry.role, false),
            support_reason,
            entry.reasons.join(", "),
            validation_hint
        ));
    }

    plan
}

fn authoring_edit_role(role: &str, primary: bool) -> &'static str {
    match (role, primary) {
        ("ui_widget_rendering" | "ui", true) => "primary_edit_or_render_validation",
        ("runtime", true) => "primary_runtime_edit",
        ("tooling", true) => "primary_tooling_edit",
        ("agent", true) => "primary_agent_or_prompt_edit",
        ("ui_widget_rendering" | "ui", false) => "ui_or_render_support",
        ("runtime", false) => "runtime_state_or_config_support",
        ("tooling", false) => "tool_or_execution_support",
        ("agent", false) => "agent_or_prompt_support",
        _ if primary => "primary_edit_file",
        _ => "supporting_file",
    }
}

fn validation_hint_for_path(path: &str, compiler: Option<&CompilerContext>) -> String {
    compiler
        .and_then(|ctx| {
            ctx.target_ownership
                .iter()
                .find(|(owned, _)| {
                    path.trim_start_matches("./") == owned.as_str()
                        || path.ends_with(owned.as_str())
                        || owned.ends_with(path.trim_start_matches("./"))
                })
                .map(|(_, kinds)| format!("validate_with={}", kinds.join("/")))
        })
        .unwrap_or_else(|| "validate_with=source-check".to_string())
}

fn support_relation_reason(
    primary: &SearchMatch,
    candidate: &SearchMatch,
    graph: Option<&WorkspaceGraph>,
    compiler: Option<&CompilerContext>,
) -> String {
    let primary_path = Path::new(&primary.path);
    let candidate_path = Path::new(&candidate.path);

    if workspace_member_for_path(primary_path, graph).map(|member| member.name.as_str())
        == workspace_member_for_path(candidate_path, graph).map(|member| member.name.as_str())
    {
        return "same_crate".to_string();
    }

    if primary_path.parent() == candidate_path.parent() {
        return "same_directory".to_string();
    }

    let primary_target = compiler.and_then(|ctx| target_kinds_for_path(&primary.path, ctx));
    let candidate_target = compiler.and_then(|ctx| target_kinds_for_path(&candidate.path, ctx));
    if primary_target.is_some() && primary_target == candidate_target {
        return "same_target_kind".to_string();
    }

    if primary.role == candidate.role {
        return "same_role_family".to_string();
    }

    "ranked_candidate_context".to_string()
}

fn target_kinds_for_path<'a>(path: &str, compiler: &'a CompilerContext) -> Option<&'a [String]> {
    compiler
        .target_ownership
        .iter()
        .find(|(owned, _)| {
            path.trim_start_matches("./") == owned.as_str()
                || path.ends_with(owned.as_str())
                || owned.ends_with(path.trim_start_matches("./"))
        })
        .map(|(_, kinds)| kinds.as_slice())
}

fn build_semantic_graph(matches: &[SearchMatch]) -> String {
    let rust_files = matches
        .iter()
        .take(6)
        .filter(|entry| entry.path.ends_with(".rs"))
        .filter_map(|entry| extract_rust_semantic_file(Path::new(&entry.path)))
        .collect::<Vec<_>>();

    if rust_files.is_empty() {
        return "No Rust semantic graph available for the current candidate set.".to_string();
    }

    let workspace_index = collect_workspace_rust_semantics();
    let definition_to_file = build_workspace_definition_index(&workspace_index);
    let module_to_file = build_workspace_module_index(&workspace_index);

    let file_blocks = rust_files
        .iter()
        .map(|file| {
            format!(
                "FILE: {}\nMODULE: {}\nDEFINES: {}\nTYPE_LINKS: {}\nTRAIT_IMPLS: {}\nIMPORTS: {}\nCALLS: {}",
                file.path,
                file.module_path,
                join_or_none(&file.defines),
                join_or_none(
                    &file
                        .type_aliases
                        .iter()
                        .map(|(alias, target)| format!("{alias} -> {target}"))
                        .collect::<Vec<_>>(),
                ),
                join_or_none(
                    &file
                        .method_traits
                        .iter()
                        .flat_map(|(method, traits)| {
                            traits.iter().map(move |trait_name| format!("{method} via {trait_name}"))
                        })
                        .collect::<Vec<_>>(),
                ),
                join_or_none(&file.imports),
                join_or_none(&file.calls),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut relationships = Vec::new();
    for file in &rust_files {
        let mut linked = BTreeSet::new();
        for module_name in &file.modules {
            let full_module = if file.module_path.is_empty() {
                module_name.clone()
            } else {
                format!("{}::{}", file.module_path, module_name)
            };
            if let Some(target_files) = module_to_file.get(&full_module) {
                for target_file in target_files {
                    if target_file == &file.path {
                        continue;
                    }
                    linked.insert(format!(
                        "- {} -> {} via module {}",
                        file.path, target_file, full_module
                    ));
                }
            }
        }
        for symbol in file.calls.iter().chain(file.imports.iter()) {
            for resolved_symbol in resolve_symbol_candidates(file, symbol) {
                if let Some(target_files) = definition_to_file.get(&resolved_symbol) {
                    for target_file in target_files {
                        if target_file == &file.path {
                            continue;
                        }
                        linked.insert(format!(
                            "- {} -> {} via {}",
                            file.path, target_file, resolved_symbol
                        ));
                    }
                }
                if let Some(target_files) = module_to_file.get(&resolved_symbol) {
                    for target_file in target_files {
                        if target_file == &file.path {
                            continue;
                        }
                        linked.insert(format!(
                            "- {} -> {} via module {}",
                            file.path, target_file, resolved_symbol
                        ));
                    }
                }
            }
        }
        for target in file.type_aliases.values() {
            for resolved_symbol in resolve_symbol_candidates(file, target) {
                if let Some(target_files) = definition_to_file.get(&resolved_symbol) {
                    for target_file in target_files {
                        if target_file == &file.path {
                            continue;
                        }
                        linked.insert(format!(
                            "- {} -> {} via type {}",
                            file.path, target_file, resolved_symbol
                        ));
                    }
                }
            }
        }
        relationships.extend(linked);
    }

    format!(
        "{}\n\nRELATIONSHIPS:\n{}",
        file_blocks,
        if relationships.is_empty() {
            "No cross-file symbol relationships inferred.".to_string()
        } else {
            relationships.join("\n")
        }
    )
}

fn extract_rust_semantic_file(path: &Path) -> Option<RustSemanticFile> {
    let content = std::fs::read_to_string(path).ok()?;
    let parsed = syn::parse_file(&content).ok()?;
    let mut visitor = RustSemanticVisitor::default();
    visitor.visit_file(&parsed);
    Some(RustSemanticFile {
        path: path.display().to_string(),
        module_path: rust_module_path(path),
        defines: visitor.defines.into_iter().collect(),
        type_aliases: visitor.type_aliases,
        imports: visitor.imports.into_iter().collect(),
        import_map: visitor.import_map,
        modules: visitor.modules.into_iter().collect(),
        calls: visitor.calls.into_iter().collect(),
        method_owners: visitor
            .method_owners
            .into_iter()
            .map(|(method, owners)| (method, owners.into_iter().collect()))
            .collect(),
        method_traits: visitor
            .method_traits
            .into_iter()
            .map(|(method, traits)| (method, traits.into_iter().collect()))
            .collect(),
    })
}

fn collect_workspace_rust_semantics() -> Vec<RustSemanticFile> {
    super::search::walk_files(Path::new("."))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !should_skip_entry(name))
        })
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter(|path| is_searchable_file(path))
        .filter_map(|path| extract_rust_semantic_file(&path))
        .collect()
}

fn build_workspace_definition_index(files: &[RustSemanticFile]) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    for file in files {
        for symbol in &file.defines {
            push_unique_index_entry(&mut index, symbol.clone(), file.path.clone());
            push_unique_index_entry(
                &mut index,
                format!("{}::{}", file.module_path, symbol),
                file.path.clone(),
            );
        }
        for (method, owners) in &file.method_owners {
            for owner in owners {
                push_unique_index_entry(
                    &mut index,
                    format!("{}::{}", owner, method),
                    file.path.clone(),
                );
                push_unique_index_entry(
                    &mut index,
                    format!("{}::{}::{}", file.module_path, owner, method),
                    file.path.clone(),
                );
            }
        }
        for (method, traits) in &file.method_traits {
            for trait_name in traits {
                push_unique_index_entry(
                    &mut index,
                    format!("{}::{}", trait_name, method),
                    file.path.clone(),
                );
            }
            if let Some(owners) = file.method_owners.get(method) {
                for owner in owners {
                    for trait_name in traits {
                        push_unique_index_entry(
                            &mut index,
                            format!("{}::{}::{}", owner, trait_name, method),
                            file.path.clone(),
                        );
                        push_unique_index_entry(
                            &mut index,
                            format!(
                                "{}::{}::{}::{}",
                                file.module_path, owner, trait_name, method
                            ),
                            file.path.clone(),
                        );
                    }
                }
            }
        }
    }
    index
}

fn push_unique_index_entry(index: &mut HashMap<String, Vec<String>>, key: String, value: String) {
    let entry = index.entry(key).or_default();
    if !entry.iter().any(|existing| existing == &value) {
        entry.push(value);
    }
}

fn build_workspace_module_index(files: &[RustSemanticFile]) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    for file in files {
        index
            .entry(file.module_path.clone())
            .or_insert_with(Vec::new)
            .push(file.path.clone());
    }
    index
}

fn resolve_symbol_candidates(file: &RustSemanticFile, symbol: &str) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    candidates.insert(symbol.to_string());
    if let Some(mapped) = file.import_map.get(symbol) {
        candidates.insert(mapped.clone());
        if let Some(last) = mapped.split("::").last() {
            candidates.insert(last.to_string());
        }
    }
    if let Some((head, tail)) = symbol.split_once("::") {
        if let Some(mapped) = file.import_map.get(head) {
            candidates.insert(format!("{}::{}", mapped, tail));
        }
    } else if let Some(owners) = file.method_owners.get(symbol) {
        for owner in owners {
            candidates.insert(format!("{}::{}", owner, symbol));
            if let Some(mapped) = file.import_map.get(owner) {
                candidates.insert(format!("{}::{}", mapped, symbol));
            }
        }
        if let Some(traits) = file.method_traits.get(symbol) {
            for owner in owners {
                for trait_name in traits {
                    candidates.insert(format!("{}::{}::{}", owner, trait_name, symbol));
                }
            }
        }
    }
    if let Some(traits) = file.method_traits.get(symbol) {
        for trait_name in traits {
            candidates.insert(format!("{}::{}", trait_name, symbol));
            if let Some(mapped) = file.import_map.get(trait_name) {
                candidates.insert(format!("{}::{}", mapped, symbol));
            }
        }
    }
    for imported in file.import_map.values() {
        if let Some(last) = imported.split("::").last() {
            candidates.insert(format!("{}::{}", last, symbol));
            candidates.insert(format!("{}::{}", imported, symbol));
        }
    }
    candidates.into_iter().collect()
}

fn rust_module_path(path: &Path) -> String {
    let mut components = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();

    if let Some(src_idx) = components.iter().position(|part| *part == "src") {
        components.drain(..=src_idx);
    }

    if let Some(last) = components.last_mut() {
        if *last == "mod.rs" {
            components.pop();
        } else if last.ends_with(".rs") {
            *last = last.trim_end_matches(".rs");
        }
    }

    components
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("::")
}

#[derive(Default)]
struct RustSemanticVisitor {
    defines: BTreeSet<String>,
    type_aliases: BTreeMap<String, String>,
    imports: BTreeSet<String>,
    import_map: BTreeMap<String, String>,
    modules: BTreeSet<String>,
    calls: BTreeSet<String>,
    method_owners: BTreeMap<String, BTreeSet<String>>,
    method_traits: BTreeMap<String, BTreeSet<String>>,
    current_impl_owner: Option<String>,
    current_impl_trait: Option<String>,
    variable_types: BTreeMap<String, String>,
}

impl<'ast> Visit<'ast> for RustSemanticVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.defines.insert(node.sig.ident.to_string());
        for input in &node.sig.inputs {
            if let syn::FnArg::Typed(arg) = input {
                if let syn::Pat::Ident(pat_ident) = &*arg.pat {
                    if let Some(type_name) = rust_type_name(&arg.ty) {
                        self.variable_types
                            .insert(pat_ident.ident.to_string(), type_name);
                    }
                }
            }
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.defines.insert(node.ident.to_string());
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.defines.insert(node.ident.to_string());
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.defines.insert(node.ident.to_string());
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        self.defines.insert(node.ident.to_string());
        if let Some(target) = rust_type_symbol(&node.ty) {
            self.type_aliases.insert(node.ident.to_string(), target);
        }
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if node.content.is_none() {
            self.modules.insert(node.ident.to_string());
        }
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let previous_trait = self.current_impl_trait.take();
        if let Some((path, _)) = &node.trait_ {
            let trait_path = rust_path_name(path);
            if let Some(segment) = path.segments.last() {
                self.imports.insert(segment.ident.to_string());
                self.current_impl_trait = Some(trait_path);
            }
        }
        let previous_owner = self.current_impl_owner.take();
        self.current_impl_owner = rust_type_name(&node.self_ty);
        syn::visit::visit_item_impl(self, node);
        self.current_impl_owner = previous_owner;
        self.current_impl_trait = previous_trait;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.defines.insert(node.sig.ident.to_string());
        if let Some(owner) = &self.current_impl_owner {
            self.method_owners
                .entry(node.sig.ident.to_string())
                .or_default()
                .insert(owner.clone());
            self.defines
                .insert(format!("{}::{}", owner, node.sig.ident));
            if let Some(trait_name) = &self.current_impl_trait {
                self.method_traits
                    .entry(node.sig.ident.to_string())
                    .or_default()
                    .insert(trait_name.clone());
                self.defines
                    .insert(format!("{}::{}::{}", owner, trait_name, node.sig.ident));
                self.defines
                    .insert(format!("{}::{}", trait_name, node.sig.ident));
            }
        }
        for input in &node.sig.inputs {
            if let syn::FnArg::Typed(arg) = input {
                if let syn::Pat::Ident(pat_ident) = &*arg.pat {
                    if let Some(type_name) = rust_type_name(&arg.ty) {
                        self.variable_types
                            .insert(pat_ident.ident.to_string(), type_name);
                    }
                }
            }
        }
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let syn::Pat::Type(pat_type) = &node.pat {
            if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                if let Some(type_name) = rust_type_name(&pat_type.ty) {
                    self.variable_types
                        .insert(pat_ident.ident.to_string(), type_name);
                }
            }
        } else if let syn::Pat::Ident(pat_ident) = &node.pat {
            if let Some(init) = &node.init {
                if let Some(type_name) =
                    rust_expr_inferred_type_name(&init.expr, &self.variable_types)
                {
                    self.variable_types
                        .insert(pat_ident.ident.to_string(), type_name);
                }
            }
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        collect_use_tree(
            &node.tree,
            &mut Vec::new(),
            &mut self.imports,
            &mut self.import_map,
        );
        syn::visit::visit_item_use(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(expr_path) = &*node.func {
            let segments = expr_path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if let Some(segment) = segments.last() {
                self.calls.insert(segment.to_string());
            }
            if segments.len() > 1 {
                self.calls.insert(segments.join("::"));
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.calls.insert(node.method.to_string());
        if let Some(owner) = rust_expr_owner_name(&node.receiver, &self.variable_types) {
            self.calls.insert(format!("{}::{}", owner, node.method));
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn rust_type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        syn::Type::Reference(reference) => rust_type_name(&reference.elem),
        syn::Type::Group(group) => rust_type_name(&group.elem),
        syn::Type::Paren(paren) => rust_type_name(&paren.elem),
        _ => None,
    }
}

fn rust_type_symbol(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(type_path) => Some(rust_path_name(&type_path.path)),
        syn::Type::Reference(reference) => rust_type_symbol(&reference.elem),
        syn::Type::Group(group) => rust_type_symbol(&group.elem),
        syn::Type::Paren(paren) => rust_type_symbol(&paren.elem),
        _ => None,
    }
}

fn rust_path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn rust_expr_owner_name(
    expr: &syn::Expr,
    variable_types: &BTreeMap<String, String>,
) -> Option<String> {
    match expr {
        syn::Expr::Path(expr_path) => {
            if expr_path.path.segments.len() == 1 {
                if let Some(segment) = expr_path.path.segments.last() {
                    if let Some(type_name) = variable_types.get(&segment.ident.to_string()) {
                        return Some(type_name.clone());
                    }
                }
            }
            expr_path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
        }
        syn::Expr::Reference(reference) => rust_expr_owner_name(&reference.expr, variable_types),
        syn::Expr::Paren(paren) => rust_expr_owner_name(&paren.expr, variable_types),
        syn::Expr::Call(call) => rust_expr_owner_name(&call.func, variable_types),
        syn::Expr::MethodCall(method_call) => Some(method_call.method.to_string()),
        _ => None,
    }
}

fn rust_expr_inferred_type_name(
    expr: &syn::Expr,
    variable_types: &BTreeMap<String, String>,
) -> Option<String> {
    match expr {
        syn::Expr::Struct(expr_struct) => expr_struct
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        syn::Expr::Call(call) => match &*call.func {
            syn::Expr::Path(expr_path) => expr_path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            other => rust_expr_owner_name(other, variable_types),
        },
        syn::Expr::Path(_) => rust_expr_owner_name(expr, variable_types),
        syn::Expr::Reference(reference) => {
            rust_expr_inferred_type_name(&reference.expr, variable_types)
        }
        syn::Expr::Paren(paren) => rust_expr_inferred_type_name(&paren.expr, variable_types),
        _ => None,
    }
}

fn collect_use_tree(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    imports: &mut BTreeSet<String>,
    import_map: &mut BTreeMap<String, String>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree(&path.tree, prefix, imports, import_map);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let alias = name.ident.to_string();
            imports.insert(alias.clone());
            let mut full = prefix.clone();
            full.push(alias.clone());
            import_map.insert(alias, full.join("::"));
        }
        syn::UseTree::Rename(rename) => {
            let alias = rename.rename.to_string();
            imports.insert(alias.clone());
            let mut full = prefix.clone();
            full.push(rename.ident.to_string());
            import_map.insert(alias, full.join("::"));
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix, imports, import_map);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(", ")
    }
}

fn search_match_score(
    path: &Path,
    content_lower: &str,
    snippets: &[String],
    terms: &[String],
    focus: QueryFocus,
    intent: SearchIntentKind,
) -> i32 {
    let path_str = path.to_string_lossy();
    let mut score = 0;
    let compact_terms = compact_search_text(&terms.join(""));

    if path_str.starts_with("./lib/harper-ui/src") || path_str.starts_with("lib/harper-ui/src") {
        score += if intent == SearchIntentKind::Defined {
            20
        } else {
            60
        };
    } else if path_str.starts_with("./lib/harper-core/src")
        || path_str.starts_with("lib/harper-core/src")
    {
        score += if intent == SearchIntentKind::Defined {
            70
        } else {
            40
        };
    }

    if path_str.contains("widgets.rs") {
        score += 40;
    }
    if path_str.contains("events.rs") || path_str.contains("tui.rs") || path_str.contains("app.rs")
    {
        score += 20;
    }
    if path_str.contains("/tests/")
        || path_str.contains("agent/intent.rs")
        || path_str.contains("agent/prompt.rs")
        || path_str.contains("agent/chat.rs")
    {
        score -= 40;
    }

    if terms.iter().all(|term| {
        snippets
            .iter()
            .any(|snippet| snippet.to_ascii_lowercase().contains(term))
    }) {
        score += 20;
    }

    if snippets.iter().any(|snippet| {
        let snippet_lower = snippet.to_ascii_lowercase();
        terms.iter().all(|term| snippet_lower.contains(term))
    }) {
        score += 45;
    }

    if snippets
        .iter()
        .any(|snippet| compact_search_text(snippet).contains(&compact_terms))
    {
        score += 60;
    }

    if snippets.iter().all(|snippet| {
        let lower = snippet.to_ascii_lowercase();
        lower.contains("licensed under")
            || lower.contains("apache license")
            || lower.contains("http://www.apache.org/licenses")
    }) {
        score -= 120;
    }

    if content_lower.contains("planfollowup") || content_lower.contains("followup") {
        score += 10;
    }

    let compact_terms = compact_search_text(&terms.join(""));
    match intent {
        SearchIntentKind::Defined => {
            if snippets.iter().any(|snippet| {
                let lower = snippet.to_ascii_lowercase();
                lower.contains(&format!("enum {}", compact_terms))
                    || lower.contains(&format!("struct {}", compact_terms))
                    || lower.contains(&format!("trait {}", compact_terms))
                    || lower.contains(&format!("type {}", compact_terms))
                    || lower.contains(&format!("fn {}(", terms.join("_")))
            }) {
                score += 180;
            }
            if snippets.iter().any(|snippet| snippet.contains("::")) {
                score -= 50;
            }
        }
        SearchIntentKind::Calls => {
            if snippets.iter().any(|snippet| {
                snippet
                    .to_ascii_lowercase()
                    .contains(&format!("fn {}(", terms.join("_")))
            }) {
                score -= 120;
            }
        }
        SearchIntentKind::Used | SearchIntentKind::General => {}
    }

    match focus {
        QueryFocus::UiRendering => {
            if path_str.contains("widgets.rs") {
                score += 50;
            }
            if path_str.contains("interfaces/ui/") {
                score += 30;
            }
        }
        QueryFocus::StateFlow => {
            if content_lower.contains("planfollowup") || content_lower.contains("retry_count") {
                score += 30;
            }
        }
        QueryFocus::Tooling => {
            if path_str.contains("/tools/") {
                score += 25;
            }
        }
        QueryFocus::Runtime => {
            if path_str.contains("/runtime/") {
                score += 25;
            }
        }
        QueryFocus::General => {}
    }

    score
}

fn search_semantic_bonus(
    semantic: &RustSemanticFile,
    semantic_target: Option<&str>,
    intent: SearchIntentKind,
) -> i32 {
    let Some(target) = semantic_target else {
        return 0;
    };
    let lowered = target.to_ascii_lowercase();
    let snake_case = lowered.replace("::", "_");
    let target_variants = [lowered.as_str(), snake_case.as_str(), target];

    match intent {
        SearchIntentKind::Defined => {
            if semantic.defines.iter().any(|symbol| {
                target_variants
                    .iter()
                    .any(|variant| symbol.eq_ignore_ascii_case(variant))
            }) {
                260
            } else {
                0
            }
        }
        SearchIntentKind::Calls => {
            if semantic.calls.iter().any(|symbol| {
                target_variants
                    .iter()
                    .any(|variant| symbol.eq_ignore_ascii_case(variant))
            }) {
                220
            } else {
                0
            }
        }
        SearchIntentKind::Used => {
            let import_hit = semantic.imports.iter().any(|symbol| {
                target_variants
                    .iter()
                    .any(|variant| symbol.eq_ignore_ascii_case(variant))
            });
            let define_hit = semantic.defines.iter().any(|symbol| {
                target_variants
                    .iter()
                    .any(|variant| symbol.eq_ignore_ascii_case(variant))
            });
            let call_hit = semantic.calls.iter().any(|symbol| {
                target_variants
                    .iter()
                    .any(|variant| symbol.eq_ignore_ascii_case(variant))
            });
            (if define_hit { 60 } else { 0 })
                + (if call_hit { 90 } else { 0 })
                + (if import_hit { 25 } else { 0 })
        }
        SearchIntentKind::General => 0,
    }
}

impl QueryFocus {
    fn as_str(self) -> &'static str {
        match self {
            QueryFocus::UiRendering => "ui_rendering",
            QueryFocus::StateFlow => "state_flow",
            QueryFocus::Tooling => "tooling",
            QueryFocus::Runtime => "runtime",
            QueryFocus::General => "general",
        }
    }
}

impl SearchIntentKind {
    fn as_str(self) -> &'static str {
        match self {
            SearchIntentKind::General => "general",
            SearchIntentKind::Used => "used",
            SearchIntentKind::Calls => "calls",
            SearchIntentKind::Defined => "defined",
        }
    }
}

fn infer_query_focus(query: &str, terms: &[String]) -> QueryFocus {
    let lower = query.to_ascii_lowercase();
    if lower.contains("render") || lower.contains("ui") || lower.contains("widget") {
        return QueryFocus::UiRendering;
    }
    if lower.contains("retry")
        || lower.contains("followup")
        || lower.contains("state")
        || terms
            .iter()
            .any(|term| matches!(term.as_str(), "retry" | "metadata" | "followup"))
    {
        return QueryFocus::StateFlow;
    }
    if lower.contains("tool") || lower.contains("command") {
        return QueryFocus::Tooling;
    }
    if lower.contains("runtime") || lower.contains("config") || lower.contains("policy") {
        return QueryFocus::Runtime;
    }
    QueryFocus::General
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        QueryFocus, RustSemanticFile, SearchIntentKind, SearchMatch, WorkspaceGraph,
        WorkspaceMemberOverview, authoring_search_query, build_semantic_graph,
        build_workspace_definition_index, classify_member_role, extract_rust_semantic_file,
        format_workspace_graph, infer_edit_plan_candidates, infer_query_focus, is_searchable_file,
        path_relative_to_root, resolve_symbol_candidates, rust_module_path, search_line_score,
        search_symbol_variants, workspace_member_for_path,
    };
    use std::path::Path;

    #[test]
    fn excludes_self_referential_agent_files_from_search() {
        assert!(!is_searchable_file(Path::new(
            "lib/harper-core/src/agent/intent.rs"
        )));
        assert!(!is_searchable_file(Path::new(
            "lib/harper-core/src/agent/prompt.rs"
        )));
        assert!(!is_searchable_file(Path::new(
            "lib/harper-core/src/agent/chat.rs"
        )));
        assert!(!is_searchable_file(Path::new("tests/integration.rs")));
        assert!(is_searchable_file(Path::new(
            "lib/harper-ui/src/interfaces/ui/widgets.rs"
        )));
    }

    #[test]
    fn infers_ui_render_focus_for_render_queries() {
        assert_eq!(
            infer_query_focus(
                "Find where retry metadata is rendered in this repo.",
                &["retry".to_string(), "metadata".to_string()]
            ),
            QueryFocus::UiRendering
        );
    }

    #[test]
    fn authoring_search_query_strips_authoring_boilerplate() {
        assert_eq!(
            authoring_search_query("refactor the planner flow in this repo"),
            "planner"
        );
        assert_eq!(
            authoring_search_query("change the retry rendering behavior in the tui"),
            "retry rendering tui"
        );
    }

    #[test]
    fn extracts_rust_semantic_file_symbols_and_calls() {
        let src = r#"
use std::path::{Path, PathBuf};

pub fn search_text() {}

pub fn authoring_context() {
    search_text();
    let _ = PathBuf::new();
}
"#;
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), src).unwrap();
        let semantic = extract_rust_semantic_file(temp.path()).expect("semantic file");
        assert!(
            semantic
                .defines
                .iter()
                .any(|symbol| symbol == "search_text")
        );
        assert!(
            semantic
                .defines
                .iter()
                .any(|symbol| symbol == "authoring_context")
        );
        assert!(
            semantic
                .imports
                .iter()
                .any(|symbol| symbol == "Path" || symbol == "PathBuf")
        );
    }

    #[test]
    fn infers_rust_module_path_from_source_file() {
        assert_eq!(
            rust_module_path(Path::new(
                "lib/harper-core/src/tools/codebase_investigator.rs"
            )),
            "tools::codebase_investigator"
        );
        assert_eq!(
            rust_module_path(Path::new("lib/harper-ui/src/interfaces/ui/mod.rs")),
            "interfaces::ui"
        );
    }

    #[test]
    fn builds_workspace_definition_index_for_qualified_and_unqualified_symbols() {
        let files = vec![RustSemanticFile {
            path: "src/tools/example.rs".to_string(),
            module_path: "tools::example".to_string(),
            defines: vec!["search_text".to_string()],
            type_aliases: BTreeMap::new(),
            imports: Vec::new(),
            import_map: BTreeMap::new(),
            modules: Vec::new(),
            calls: Vec::new(),
            method_owners: BTreeMap::new(),
            method_traits: BTreeMap::new(),
        }];
        let index = build_workspace_definition_index(&files);
        assert_eq!(
            index.get("search_text"),
            Some(&vec!["src/tools/example.rs".to_string()])
        );
        assert_eq!(
            index.get("tools::example::search_text"),
            Some(&vec!["src/tools/example.rs".to_string()])
        );
    }

    #[test]
    fn extracts_use_aliases_and_module_declarations() {
        let src = r#"
mod helper;
use crate::tools::codebase_investigator::search_text as searcher;
use crate::agent::{chat::ChatService, intent::route_intent};

fn run() {
    searcher("retry metadata");
    route_intent("check repo");
}
"#;
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), src).unwrap();
        let semantic = extract_rust_semantic_file(temp.path()).unwrap();
        assert!(semantic.modules.iter().any(|m| m == "helper"));
        assert!(semantic.imports.iter().any(|m| m == "searcher"));
        assert_eq!(
            semantic.import_map.get("searcher").map(String::as_str),
            Some("crate::tools::codebase_investigator::search_text")
        );
        assert!(semantic.calls.iter().any(|c| c == "searcher"));
    }

    #[test]
    fn resolves_alias_candidates_to_full_paths() {
        let mut import_map = BTreeMap::new();
        import_map.insert(
            "searcher".to_string(),
            "crate::tools::codebase_investigator::search_text".to_string(),
        );
        let file = RustSemanticFile {
            path: "lib/harper-core/src/tools/example.rs".to_string(),
            module_path: "tools::example".to_string(),
            defines: vec!["run".to_string()],
            type_aliases: BTreeMap::new(),
            imports: vec!["searcher".to_string()],
            import_map,
            modules: Vec::new(),
            calls: vec!["searcher".to_string()],
            method_owners: BTreeMap::new(),
            method_traits: BTreeMap::new(),
        };
        let candidates = resolve_symbol_candidates(&file, "searcher");
        assert!(candidates.iter().any(|c| c == "searcher"));
        assert!(
            candidates
                .iter()
                .any(|c| c == "crate::tools::codebase_investigator::search_text")
        );
        assert!(candidates.iter().any(|c| c == "search_text"));
    }

    #[test]
    fn extracts_impl_method_owners_and_receiver_calls() {
        let src = r#"
struct Planner;

impl Planner {
    fn replan(&self) {}
}

fn run(planner: Planner) {
    planner.replan();
}
"#;
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), src).unwrap();
        let semantic = extract_rust_semantic_file(temp.path()).unwrap();
        assert!(semantic.defines.iter().any(|d| d == "Planner::replan"));
        assert_eq!(
            semantic
                .method_owners
                .get("replan")
                .map(|owners| owners.as_slice()),
            Some(&["Planner".to_string()][..])
        );
        assert!(semantic.calls.iter().any(|c| c == "Planner::replan"));
    }

    #[test]
    fn indexes_qualified_method_definitions() {
        let mut method_owners = BTreeMap::new();
        method_owners.insert("replan".to_string(), vec!["Planner".to_string()]);
        let mut method_traits = BTreeMap::new();
        method_traits.insert("replan".to_string(), vec!["Refactorable".to_string()]);
        let files = vec![RustSemanticFile {
            path: "src/tools/example.rs".to_string(),
            module_path: "tools::example".to_string(),
            defines: vec![
                "Planner".to_string(),
                "Planner::replan".to_string(),
                "Planner::Refactorable::replan".to_string(),
            ],
            type_aliases: BTreeMap::new(),
            imports: Vec::new(),
            import_map: BTreeMap::new(),
            modules: Vec::new(),
            calls: Vec::new(),
            method_owners,
            method_traits,
        }];
        let index = build_workspace_definition_index(&files);
        assert_eq!(
            index.get("Planner::replan"),
            Some(&vec!["src/tools/example.rs".to_string()])
        );
        assert_eq!(
            index.get("tools::example::Planner::replan"),
            Some(&vec!["src/tools/example.rs".to_string()])
        );
        assert_eq!(
            index.get("Refactorable::replan"),
            Some(&vec!["src/tools/example.rs".to_string()])
        );
        assert_eq!(
            index.get("Planner::Refactorable::replan"),
            Some(&vec!["src/tools/example.rs".to_string()])
        );
    }

    #[test]
    fn extracts_trait_impl_methods_and_initializer_based_receiver_calls() {
        let src = r#"
trait Runner {
    fn run(&self);
}

struct Planner;

impl Runner for Planner {
    fn run(&self) {}
}

fn execute() {
    let planner = Planner {};
    planner.run();
}
"#;
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), src).unwrap();
        let semantic = extract_rust_semantic_file(temp.path()).unwrap();
        assert!(semantic.defines.iter().any(|d| d == "Planner::Runner::run"));
        assert_eq!(
            semantic
                .method_traits
                .get("run")
                .map(|traits| traits.as_slice()),
            Some(&["Runner".to_string()][..])
        );
        assert!(semantic.calls.iter().any(|c| c == "Planner::run"));
    }

    #[test]
    fn extracts_type_alias_links_and_full_trait_identity() {
        let src = r#"
use crate::runtime::Runner as RuntimeRunner;

type SharedRunner = RuntimeRunner;

struct Planner;

impl RuntimeRunner for Planner {
    fn run(&self) {}
}
"#;
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), src).unwrap();
        let semantic = extract_rust_semantic_file(temp.path()).unwrap();
        assert_eq!(
            semantic
                .type_aliases
                .get("SharedRunner")
                .map(String::as_str),
            Some("RuntimeRunner")
        );
        assert!(
            semantic
                .defines
                .iter()
                .any(|d| d == "Planner::RuntimeRunner::run")
        );
        assert_eq!(
            semantic
                .method_traits
                .get("run")
                .map(|traits| traits.as_slice()),
            Some(&["RuntimeRunner".to_string()][..])
        );
    }

    #[test]
    fn semantic_graph_surfaces_type_links() {
        let temp = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
        std::fs::write(
            temp.path(),
            r#"
type SharedPlanner = Planner;

struct Planner;
"#,
        )
        .unwrap();
        let matches = vec![SearchMatch {
            score: 10,
            path: temp.path().display().to_string(),
            role: "runtime".to_string(),
            reasons: vec!["candidate".to_string()],
            snippets: Vec::new(),
        }];

        let graph = build_semantic_graph(&matches);

        assert!(graph.contains("TYPE_LINKS: SharedPlanner -> Planner"));
    }

    #[test]
    fn edit_plan_candidates_distinguish_primary_and_supporting_files() {
        let matches = vec![
            SearchMatch {
                score: 100,
                path: "lib/harper-ui/src/interfaces/ui/widgets.rs".to_string(),
                role: "ui_widget_rendering".to_string(),
                reasons: vec!["matches_ui_render_focus".to_string()],
                snippets: Vec::new(),
            },
            SearchMatch {
                score: 90,
                path: "lib/harper-ui/src/interfaces/ui/app.rs".to_string(),
                role: "ui".to_string(),
                reasons: vec!["contains_all_terms".to_string()],
                snippets: Vec::new(),
            },
            SearchMatch {
                score: 80,
                path: "lib/harper-core/src/tools/plan.rs".to_string(),
                role: "tooling".to_string(),
                reasons: vec!["matches_tooling_focus".to_string()],
                snippets: Vec::new(),
            },
        ];
        let graph = WorkspaceGraph {
            root: "/repo".to_string(),
            package_name: Some("harper-workspace".to_string()),
            members: vec![
                WorkspaceMemberOverview {
                    name: "harper-ui".to_string(),
                    role: "lib".to_string(),
                    root: "lib/harper-ui".to_string(),
                    entrypoints: Vec::new(),
                    notable_files: Vec::new(),
                },
                WorkspaceMemberOverview {
                    name: "harper-core".to_string(),
                    role: "lib".to_string(),
                    root: "lib/harper-core".to_string(),
                    entrypoints: Vec::new(),
                    notable_files: Vec::new(),
                },
            ],
        };

        let plan = infer_edit_plan_candidates(&matches, Some(&graph), None);

        assert_eq!(plan.len(), 3);
        assert!(plan[0].contains("PRIMARY: lib/harper-ui/src/interfaces/ui/widgets.rs"));
        assert!(plan[0].contains("primary_edit_or_render_validation"));
        assert!(plan[1].contains("SUPPORTING: lib/harper-ui/src/interfaces/ui/app.rs"));
        assert!(plan[1].contains("same_crate"));
        assert!(plan[2].contains("SUPPORTING: lib/harper-core/src/tools/plan.rs"));
    }

    #[test]
    fn classifies_workspace_member_for_path_from_graph() {
        let graph = WorkspaceGraph {
            root: "/repo".to_string(),
            package_name: Some("harper-workspace".to_string()),
            members: vec![
                WorkspaceMemberOverview {
                    name: "harper-core".to_string(),
                    role: "runtime/tools/storage/agent".to_string(),
                    root: "lib/harper-core".to_string(),
                    entrypoints: vec!["src/lib.rs".to_string()],
                    notable_files: Vec::new(),
                },
                WorkspaceMemberOverview {
                    name: "harper-ui".to_string(),
                    role: "tui/widgets/events".to_string(),
                    root: "lib/harper-ui".to_string(),
                    entrypoints: vec!["src/lib.rs".to_string()],
                    notable_files: Vec::new(),
                },
            ],
        };
        let member = workspace_member_for_path(
            Path::new("lib/harper-ui/src/interfaces/ui/widgets.rs"),
            Some(&graph),
        )
        .unwrap();
        assert_eq!(member.name, "harper-ui");
    }

    #[test]
    fn formats_workspace_graph_with_member_summary() {
        let graph = WorkspaceGraph {
            root: "/repo".to_string(),
            package_name: Some("harper-workspace".to_string()),
            members: vec![WorkspaceMemberOverview {
                name: "harper-core".to_string(),
                role: classify_member_role("harper-core", "lib/harper-core"),
                root: "lib/harper-core".to_string(),
                entrypoints: vec!["src/lib.rs".to_string()],
                notable_files: vec!["src/tools/codebase_investigator.rs".to_string()],
            }],
        };
        let rendered = format_workspace_graph(&graph);
        assert!(rendered.contains("Workspace root: /repo"));
        assert!(rendered.contains("Workspace members: harper-core"));
        assert!(rendered.contains("crate=harper-core"));
    }

    #[test]
    fn path_relative_to_root_prefers_member_relative_paths() {
        let root = Path::new("/repo/lib/harper-core");
        let rel = path_relative_to_root(root, Path::new("/repo/lib/harper-core/src/lib.rs"));
        assert_eq!(rel, "src/lib.rs");
    }

    #[test]
    fn search_matches_prefer_symbol_usage_over_license_comment_noise() {
        let terms = vec!["execution".to_string(), "strategy".to_string()];
        let symbol_variants = search_symbol_variants(&terms);
        let symbol_score = search_line_score(
            "settings::execution_strategy_name(app.execution_strategy)",
            &terms,
            &symbol_variants,
            SearchIntentKind::General,
        )
        .expect("symbol usage score");
        let license_score = search_line_score(
            "// execution strategy overview and notes",
            &terms,
            &symbol_variants,
            SearchIntentKind::General,
        )
        .expect("comment line score");

        assert!(symbol_score > license_score);
    }

    #[test]
    fn search_matches_prefer_real_usage_sites_over_imports_for_execution_strategy() {
        let terms = vec!["execution".to_string(), "strategy".to_string()];
        let symbol_variants = search_symbol_variants(&terms);
        let usage_score = search_line_score(
            "settings::execution_strategy_name(app.execution_strategy)",
            &terms,
            &symbol_variants,
            SearchIntentKind::Used,
        )
        .expect("usage score");
        let import_score = search_line_score(
            "use harper_core::ExecutionStrategy;",
            &terms,
            &symbol_variants,
            SearchIntentKind::Used,
        )
        .expect("import score");

        assert!(usage_score > import_score);
    }

    #[test]
    fn search_matches_prefer_call_sites_over_definitions_for_update_plan() {
        let terms = vec!["update_plan".to_string()];
        let symbol_variants = search_symbol_variants(&terms);
        let call_score = search_line_score(
            "let plan_result = plan::update_plan(self.conn, session_id, args)?;",
            &terms,
            &symbol_variants,
            SearchIntentKind::Calls,
        )
        .expect("call site score");
        let definition_score = search_line_score(
            "pub fn update_plan(",
            &terms,
            &symbol_variants,
            SearchIntentKind::Calls,
        )
        .expect("definition score");

        assert!(call_score > definition_score);
    }

    #[test]
    fn search_matches_prefer_exact_definition_for_execution_strategy() {
        let terms = vec!["executionstrategy".to_string()];
        let symbol_variants = search_symbol_variants(&terms);
        let definition_score = search_line_score(
            "pub enum ExecutionStrategy {",
            &terms,
            &symbol_variants,
            SearchIntentKind::Defined,
        )
        .expect("definition score");
        let import_score = search_line_score(
            "use crate::runtime::config::{ExecPolicyConfig, ExecutionStrategy};",
            &terms,
            &symbol_variants,
            SearchIntentKind::Defined,
        )
        .expect("import score");

        assert!(definition_score > import_score);
    }
}
