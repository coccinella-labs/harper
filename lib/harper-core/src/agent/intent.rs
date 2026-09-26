// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Lightweight intent routing for deterministic actions.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFilesIntent {
    pub ext: Option<String>,
    pub tracked_only: bool,
    pub since: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFileIntent {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodebaseSearchIntent {
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCommandIntent {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFileIntent {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterministicIntent {
    ListChangedFiles(ChangedFilesIntent),
    GitStatus,
    GitDiff,
    GitBranch,
    CurrentDirectory,
    RepoIdentity,
    ReadFile(ReadFileIntent),
    WriteFile(WriteFileIntent),
    CodebaseOverview,
    CodebaseSearch(CodebaseSearchIntent),
    RunCommand(RunCommandIntent),
}

pub fn route_intent(query: &str) -> Option<DeterministicIntent> {
    let normalized = normalize(query);
    let tokens: Vec<&str> = normalized.split_whitespace().collect();

    if is_git_status_intent(&normalized) {
        return Some(DeterministicIntent::GitStatus);
    }

    if is_git_diff_intent(&normalized) {
        return Some(DeterministicIntent::GitDiff);
    }

    if is_git_branch_intent(&normalized) {
        return Some(DeterministicIntent::GitBranch);
    }

    if is_current_directory_intent(&normalized) {
        return Some(DeterministicIntent::CurrentDirectory);
    }

    if is_repo_identity_intent(&normalized) {
        return Some(DeterministicIntent::RepoIdentity);
    }

    if is_codebase_overview_intent(&normalized) {
        return Some(DeterministicIntent::CodebaseOverview);
    }

    if let Some((path, content)) = infer_write_file_intent(query, &normalized) {
        return Some(DeterministicIntent::WriteFile(WriteFileIntent {
            path,
            content,
        }));
    }

    if let Some(path) = infer_read_file_path(query, &tokens) {
        return Some(DeterministicIntent::ReadFile(ReadFileIntent { path }));
    }

    if let Some(pattern) = infer_codebase_search_pattern(query, &normalized) {
        return Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
            pattern,
        }));
    }

    if is_changed_files_intent(&normalized, &tokens) {
        return Some(DeterministicIntent::ListChangedFiles(ChangedFilesIntent {
            ext: infer_extension_filter(&tokens),
            tracked_only: tokens.contains(&"tracked"),
            since: infer_since_filter(&tokens),
        }));
    }

    if let Some(command) = infer_simple_run_command(query, &normalized) {
        return Some(DeterministicIntent::RunCommand(RunCommandIntent {
            command,
        }));
    }

    None
}

fn normalize(input: &str) -> String {
    input
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c.is_ascii_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
}

fn is_changed_files_intent(normalized: &str, tokens: &[&str]) -> bool {
    let has_file_noun = tokens.iter().any(|t| {
        matches!(
            *t,
            "file" | "files" | "codebase" | "repo" | "repository" | "worktree"
        )
    });
    let has_change_signal = tokens.iter().any(|t| {
        matches!(
            *t,
            "change"
                | "changed"
                | "changing"
                | "modified"
                | "edit"
                | "edited"
                | "editing"
                | "updated"
                | "update"
                | "touch"
                | "touched"
        )
    });

    if has_file_noun && has_change_signal {
        return true;
    }

    normalized.contains("what am i changing")
        || normalized.contains("what i am changing")
        || normalized.contains("files i am changing")
        || normalized.contains("files am changing")
        || normalized.contains("files im changing")
}

fn infer_since_filter(tokens: &[&str]) -> Option<String> {
    if tokens.contains(&"today") {
        return Some("today".to_string());
    }
    if tokens.contains(&"yesterday") {
        return Some("yesterday".to_string());
    }
    if tokens.contains(&"week") {
        return Some("1 week ago".to_string());
    }
    None
}

fn infer_extension_filter(tokens: &[&str]) -> Option<String> {
    if tokens.iter().any(|t| matches!(*t, "rust" | "rs")) {
        return Some("rs".to_string());
    }
    if tokens.iter().any(|t| matches!(*t, "python" | "py")) {
        return Some("py".to_string());
    }
    if tokens.iter().any(|t| matches!(*t, "javascript" | "js")) {
        return Some("js".to_string());
    }
    if tokens.iter().any(|t| matches!(*t, "typescript" | "ts")) {
        return Some("ts".to_string());
    }
    if tokens.iter().any(|t| matches!(*t, "markdown" | "md")) {
        return Some("md".to_string());
    }
    None
}

fn is_git_status_intent(normalized: &str) -> bool {
    normalized.contains("git status")
        || normalized.contains("status of repo")
        || normalized.contains("status of repository")
}

fn is_git_diff_intent(normalized: &str) -> bool {
    normalized.contains("git diff")
        || normalized.contains("show diff")
        || normalized.contains("show the diff")
        || normalized.contains("check code changes")
        || normalized.contains("check the code changes")
        || normalized.contains("show code changes")
        || normalized.contains("what changed in code")
}

fn is_git_branch_intent(normalized: &str) -> bool {
    normalized.contains("which branch")
        || normalized.contains("what branch")
        || normalized.contains("current branch")
        || normalized.contains("branch am i")
        || normalized.contains("branch are we")
        || normalized.contains("branch i am")
        || normalized.contains("branch we are on")
}

fn is_current_directory_intent(normalized: &str) -> bool {
    normalized.contains("current directory")
        || normalized.contains("working directory")
        || normalized.contains("present working directory")
        || normalized.contains("which directory")
        || normalized.contains("what directory")
        || normalized.contains("where am i")
        || normalized.contains("where are we")
}

fn is_repo_identity_intent(normalized: &str) -> bool {
    (normalized.contains("which repo") || normalized.contains("what repo"))
        && (normalized.contains("working on")
            || normalized.contains("working in")
            || normalized.contains("are we in")
            || normalized.contains("am i in"))
}

fn is_codebase_overview_intent(normalized: &str) -> bool {
    let has_scope = normalized.contains("codebase")
        || normalized.contains("repo")
        || normalized.contains("repository")
        || normalized.contains("project")
        || normalized.contains("workspace");
    let has_overview_signal = normalized.contains("tell me")
        || normalized.contains("check")
        || normalized.contains("summarize")
        || normalized.contains("describe")
        || normalized.contains("overview")
        || normalized.contains("inspect")
        || normalized.contains("read me through")
        || normalized.contains("walk me through")
        || normalized.contains("quick overview")
        || normalized.contains("high level overview");

    has_scope
        && has_overview_signal
        && !is_changed_files_intent(
            normalized,
            &normalized.split_whitespace().collect::<Vec<_>>(),
        )
        && !normalized.contains("find where")
        && !normalized.contains("where is")
        && !normalized.contains("where does")
        && !normalized.contains("what renders")
        && !normalized.contains("who calls")
}

fn infer_read_file_path(query: &str, tokens: &[&str]) -> Option<String> {
    let lower = query.to_ascii_lowercase();
    let has_read_verb = [
        "read ", "open ", "show ", "view ", "look at ", "inspect ", "check ", "see ",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    if !has_read_verb {
        return None;
    }

    let raw_tokens: Vec<&str> = query.split_whitespace().collect();
    for token in raw_tokens {
        let cleaned = token
            .trim_matches(|c: char| {
                matches!(c, '.' | ',' | ';' | ':' | ')' | '(' | '"' | '\'' | '`')
            })
            .trim();
        if cleaned.is_empty() {
            continue;
        }
        if cleaned.contains('/') || cleaned.contains('\\') {
            return Some(cleaned.to_string());
        }
        if cleaned.contains('.') && cleaned.len() > 2 {
            return Some(cleaned.to_string());
        }
    }

    let _ = tokens;
    None
}

fn infer_write_file_intent(query: &str, normalized: &str) -> Option<(String, String)> {
    if let Some(explicit) = infer_explicit_write_file_intent(query, normalized) {
        return Some(explicit);
    }

    let wants_create = normalized.contains("create ")
        || normalized.contains("make ")
        || normalized.contains("write ");
    if !wants_create {
        return None;
    }

    if normalized.contains(" file ")
        || normalized.starts_with("file ")
        || normalized.ends_with(" file")
    {
        if let Some(content) = extract_content_after_markers(
            query,
            normalized,
            &[" saying ", " that says ", " containing ", " with "],
        ) {
            return Some(("note.txt".to_string(), content));
        }
    }

    if let Some(generic) = infer_generic_named_file_intent(query, normalized) {
        return Some(generic);
    }

    let (file_kind, language_label) = if normalized.contains(" python ")
        || normalized.contains(" py file")
        || normalized.contains(" python file")
        || normalized.contains(" in python")
        || normalized.contains(" as python")
    {
        ("py", Some("python"))
    } else if normalized.contains(" rust ")
        || normalized.contains(" rs file")
        || normalized.contains(" rust file")
        || normalized.contains(" in rust")
        || normalized.contains(" as rust")
    {
        ("rs", Some("rust"))
    } else if normalized.contains(" javascript ")
        || normalized.contains(" js file")
        || normalized.contains(" javascript file")
        || normalized.contains(" in javascript")
        || normalized.contains(" as javascript")
    {
        ("js", Some("javascript"))
    } else if normalized.contains(" typescript ")
        || normalized.contains(" ts file")
        || normalized.contains(" typescript file")
        || normalized.contains(" in typescript")
        || normalized.contains(" as typescript")
    {
        ("ts", Some("typescript"))
    } else if normalized.contains(" json file")
        || normalized.contains(" in json")
        || normalized.contains(" as json")
    {
        ("json", Some("json"))
    } else if normalized.contains(" markdown file")
        || normalized.contains(" md file")
        || normalized.contains(" in markdown")
        || normalized.contains(" as markdown")
        || normalized.contains(" in md")
        || normalized.contains(" as md")
    {
        ("md", Some("markdown"))
    } else if normalized.contains(" shell file")
        || normalized.contains(" sh file")
        || normalized.contains(" bash file")
        || normalized.contains(" in shell")
        || normalized.contains(" as shell")
        || normalized.contains(" in bash")
        || normalized.contains(" as bash")
    {
        ("sh", Some("shell"))
    } else if normalized.contains(" txt file")
        || normalized.ends_with(" txt")
        || normalized.contains(" text file")
        || normalized.contains(" in text")
        || normalized.contains(" as text")
        || normalized.contains(" in txt")
        || normalized.contains(" as txt")
    {
        ("txt", None)
    } else {
        return None;
    };

    let before_kind = if let Some(idx) = normalized.find(" txt file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" text file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" python file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" py file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" rust file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" rs file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" javascript file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" js file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" typescript file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" ts file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" json file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" markdown file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" md file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in markdown") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as markdown") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in md") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as md") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" shell file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" sh file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" bash file") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in python") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as python") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in rust") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as rust") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in javascript") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as javascript") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in typescript") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as typescript") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in json") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as json") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in shell") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as shell") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in bash") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as bash") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in text") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as text") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" in txt") {
        &query[..idx]
    } else if let Some(idx) = normalized.find(" as txt") {
        &query[..idx]
    } else if let Some(idx) = normalized.rfind(" txt") {
        &query[..idx]
    } else {
        query
    };

    let descriptor = before_kind
        .trim()
        .trim_start_matches(|c: char| c.is_whitespace())
        .to_string();
    let descriptor = descriptor
        .strip_prefix("create ")
        .or_else(|| descriptor.strip_prefix("Create "))
        .or_else(|| descriptor.strip_prefix("make "))
        .or_else(|| descriptor.strip_prefix("Make "))
        .or_else(|| descriptor.strip_prefix("write "))
        .or_else(|| descriptor.strip_prefix("Write "))
        .unwrap_or(descriptor.as_str())
        .trim_start_matches("a ")
        .trim_start_matches("an ")
        .trim_start_matches("the ")
        .trim();

    let descriptor = if let Some(label) = language_label {
        descriptor
            .strip_prefix(label)
            .or_else(|| descriptor.strip_prefix(&capitalize_first(label)))
            .unwrap_or(descriptor)
            .trim()
    } else {
        descriptor
    };
    let descriptor = descriptor
        .strip_suffix(" file")
        .unwrap_or(descriptor)
        .trim();

    if descriptor.is_empty() {
        return None;
    }

    let authoring_descriptor = authoring_content_descriptor(descriptor);
    let content = starter_content_for(file_kind, &authoring_descriptor);
    let slug = slugify_filename(&authoring_slug_descriptor(&authoring_descriptor));
    Some((format!("{}.{}", slug, file_kind), content))
}

fn infer_generic_named_file_intent(query: &str, normalized: &str) -> Option<(String, String)> {
    let file_name = ["file named ", "file name ", "named file ", "name the file "]
        .iter()
        .find_map(|marker| {
            let idx = normalized.find(marker)?;
            let tail = query[idx + marker.len()..].trim_start();
            let token = tail
                .split_whitespace()
                .next()
                .map(|value| {
                    value.trim_matches(|c: char| {
                        matches!(c, '.' | ',' | ';' | ':' | ')' | '(' | '"' | '\'' | '`')
                    })
                })
                .filter(|value| !value.is_empty())?;
            Some(token.to_string())
        })?;

    let content = infer_generic_named_file_content(normalized)?;
    Some((file_name, content))
}

fn infer_generic_named_file_content(normalized: &str) -> Option<String> {
    if normalized.contains("1 to 10") || normalized.contains("1 through 10") {
        return Some(
            (1..=10)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    if normalized.contains("1 to 5") || normalized.contains("1 through 5") {
        return Some(
            (1..=5)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    None
}

fn authoring_content_descriptor(descriptor: &str) -> String {
    let normalized = descriptor
        .trim_start_matches("new file ")
        .trim_start_matches("new ")
        .trim_start_matches("file ")
        .trim();

    for marker in [" explaining ", " about ", " for ", " on "] {
        if let Some(idx) = normalized.find(marker) {
            let tail = normalized[idx + marker.len()..].trim();
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
    }

    normalized.to_string()
}

fn authoring_slug_descriptor(descriptor: &str) -> String {
    descriptor
        .trim_start_matches("about ")
        .trim_start_matches("explaining ")
        .trim_start_matches("introduction to ")
        .trim()
        .to_string()
}

fn infer_explicit_write_file_intent(query: &str, normalized: &str) -> Option<(String, String)> {
    let wants_create = normalized.contains("create ")
        || normalized.contains("make ")
        || normalized.contains("write ");
    let wants_modify = normalized.contains("modify ")
        || normalized.contains("update ")
        || normalized.contains("change ")
        || normalized.contains("replace ")
        || normalized.contains("edit ")
        || normalized.contains("save ");
    if !wants_create && !wants_modify {
        return None;
    }

    let path = extract_explicit_path(query)?;
    let lowered = query.to_ascii_lowercase();

    let content = if wants_modify {
        extract_content_after_markers(
            query,
            &lowered,
            &[" to ", " with ", " containing ", " so it says ", " saying "],
        )
    } else {
        extract_content_after_markers(
            query,
            &lowered,
            &[" with ", " containing ", " that says ", " saying "],
        )
    }
    .or_else(|| infer_content_from_explicit_path_and_query(&path, query));

    content.map(|value| (path, value))
}

fn extract_explicit_path(query: &str) -> Option<String> {
    query.split_whitespace().find_map(|token| {
        let cleaned = token
            .trim_matches(|c: char| {
                matches!(c, '.' | ',' | ';' | ':' | ')' | '(' | '"' | '\'' | '`')
            })
            .trim();
        if cleaned.is_empty() {
            return None;
        }

        let lowered = cleaned.to_ascii_lowercase();
        let has_path_separator = cleaned.contains('/') || cleaned.contains('\\');
        let has_known_extension = [
            ".rs", ".py", ".js", ".ts", ".json", ".md", ".sh", ".txt", ".toml", ".yaml", ".yml",
        ]
        .iter()
        .any(|ext| lowered.ends_with(ext));

        if has_path_separator || has_known_extension {
            Some(cleaned.to_string())
        } else {
            None
        }
    })
}

fn extract_content_after_markers(
    query: &str,
    lowered_query: &str,
    markers: &[&str],
) -> Option<String> {
    for marker in markers {
        if let Some(idx) = lowered_query.find(marker) {
            let content = query[idx + marker.len()..]
                .trim()
                .trim_matches(|c: char| matches!(c, '"' | '\''))
                .to_string();
            if !content.is_empty() {
                return Some(content);
            }
        }
    }
    None
}

fn infer_content_from_explicit_path_and_query(path: &str, query: &str) -> Option<String> {
    let lowered = path.to_ascii_lowercase();
    let stem = Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("hello");
    let humanized = humanize_descriptor(&stem.replace(['-', '_'], " "));

    if lowered.ends_with(".py") {
        if query.to_ascii_lowercase().contains("hello") {
            return Some(format!("print(\"{}\")", humanized));
        }
        return Some(format!("print(\"{}\")", humanized));
    }
    if lowered.ends_with(".rs") {
        return Some(format!(
            "fn main() {{\n    println!(\"{}\");\n}}",
            humanized
        ));
    }
    if lowered.ends_with(".js") || lowered.ends_with(".ts") {
        return Some(format!("console.log(\"{}\");", humanized));
    }
    if lowered.ends_with(".txt") {
        return Some(humanized);
    }
    if lowered.ends_with(".md") {
        return Some(format!("# {}\n", humanized));
    }

    None
}

fn humanize_descriptor(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    chars.as_str().to_ascii_lowercase()
                ),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn slugify_filename(input: &str) -> String {
    let slug = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let collapsed = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "new-file".to_string()
    } else {
        collapsed
    }
}

fn starter_content_for(file_kind: &str, descriptor: &str) -> String {
    let phrase = humanize_descriptor(descriptor);
    match file_kind {
        "py" => format!("print(\"{}\")", phrase),
        "rs" => format!("fn main() {{\n    println!(\"{}\");\n}}", phrase),
        "js" => format!("console.log(\"{}\");", phrase),
        "ts" => format!("console.log(\"{}\");", phrase),
        "json" => format!("{{\n  \"message\": \"{}\"\n}}", phrase),
        "md" => format!("# {}\n", phrase),
        "sh" => format!("#!/usr/bin/env bash\necho \"{}\"\n", phrase),
        _ => phrase,
    }
}

fn capitalize_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

fn infer_codebase_search_pattern(query: &str, normalized: &str) -> Option<String> {
    let codebase_scope = normalized.contains("repo")
        || normalized.contains("repository")
        || normalized.contains("codebase")
        || normalized.contains("project")
        || normalized.contains("this repo")
        || normalized.contains(" in this")
        || normalized.ends_with(" here")
        || normalized.contains(" in here");
    let search_intent = normalized.contains("find where")
        || normalized.contains("where is")
        || normalized.contains("where does")
        || normalized.contains("where is handled")
        || normalized.contains("where is implemented")
        || normalized.contains("where is defined")
        || normalized.contains("where is used")
        || normalized.contains("where does come from")
        || normalized.contains("get set")
        || normalized.contains("what file contains")
        || normalized.contains("what files contain")
        || normalized.contains("what calls")
        || normalized.contains("what updates")
        || normalized.contains("what renders")
        || (normalized.contains("how does") && normalized.contains(" flow"))
        || (normalized.contains("how do") && normalized.contains(" flow"))
        || normalized.contains("who calls")
        || normalized.contains("who handles")
        || normalized.contains("who implements")
        || normalized.contains("find ")
        || normalized.contains("locate ");

    if !codebase_scope || !search_intent {
        return None;
    }

    let mut phrase = query.to_string();
    for marker in [
        "Find where ",
        "find where ",
        "Where is ",
        "where is ",
        "Where does ",
        "where does ",
        "Where is handled ",
        "where is handled ",
        "Where is implemented ",
        "where is implemented ",
        "Where is defined ",
        "where is defined ",
        "Where is used ",
        "where is used ",
        "Where does ",
        "where does ",
        "What updates ",
        "what updates ",
        "What calls ",
        "what calls ",
        "What file contains ",
        "what file contains ",
        "What files contain ",
        "what files contain ",
        "How does ",
        "how does ",
        "How do ",
        "how do ",
        "What renders ",
        "what renders ",
        "Who calls ",
        "who calls ",
        "Who handles ",
        "who handles ",
        "Who implements ",
        "who implements ",
        "Find ",
        "find ",
        "Locate ",
        "locate ",
    ] {
        if let Some(rest) = phrase.strip_prefix(marker) {
            phrase = rest.to_string();
            break;
        }
    }

    let lower_phrase = phrase.to_ascii_lowercase();
    let cutoff_markers = [
        " in this repo",
        " in this repository",
        " in this codebase",
        " in this project",
        " in this",
        " in the repo",
        " in the repository",
        " in the codebase",
        " in project",
        " in here",
        " here",
    ];
    for marker in cutoff_markers {
        if let Some(idx) = lower_phrase.find(marker) {
            phrase.truncate(idx);
            break;
        }
    }

    let cleaned = phrase
        .replace(" come from", "")
        .replace(" get set", "")
        .replace(" gets set", "")
        .replace(" is handled", "")
        .replace(" handled", "")
        .replace(" is implemented", "")
        .replace(" implemented", "")
        .replace(" is rendered", "")
        .replace(" rendered", "")
        .replace(" render", "")
        .replace(" updates", "")
        .replace(" update", "")
        .replace(" called", "")
        .replace(" defined", "")
        .replace(" used", "")
        .replace(" flows", "")
        .replace(" flow", "")
        .trim()
        .to_string();

    if let Some(symbol) = extract_symbolic_search_target(&phrase) {
        Some(symbol)
    } else if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn extract_symbolic_search_target(phrase: &str) -> Option<String> {
    let primary_clause = primary_search_clause(phrase).unwrap_or(phrase);
    if primary_clause_contains_multiple_symbol_targets(primary_clause) {
        return None;
    }
    if let Some(symbol) = extract_backticked_symbol(primary_clause) {
        return Some(symbol);
    }
    if let Some(symbol) = first_symbol_like_token(primary_clause) {
        return Some(symbol);
    }

    if let Some(symbol) = extract_backticked_symbol(phrase) {
        return Some(symbol);
    }

    first_symbol_like_token(phrase)
}

fn primary_clause_contains_multiple_symbol_targets(clause: &str) -> bool {
    let lower = clause.to_ascii_lowercase();
    if !(lower.contains(" and ") || lower.contains(" or ") || lower.contains(',')) {
        return false;
    }

    symbol_like_tokens(clause).len() > 1
}

fn primary_search_clause(phrase: &str) -> Option<&str> {
    let lower = phrase.to_ascii_lowercase();
    let markers = [
        " in ",
        " from ",
        " through ",
        " within ",
        " inside ",
        " under ",
        " for ",
        " during ",
        " across ",
        " via ",
    ];

    let split_at = markers
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()?;
    Some(phrase[..split_at].trim())
}

fn first_symbol_like_token(phrase: &str) -> Option<String> {
    symbol_like_tokens(phrase).into_iter().next()
}

fn symbol_like_tokens(phrase: &str) -> Vec<String> {
    phrase
        .split_whitespace()
        .map(clean_symbol_token)
        .filter(|token| is_symbol_like_token(token))
        .map(ToString::to_string)
        .collect()
}

fn extract_backticked_symbol(phrase: &str) -> Option<String> {
    let start = phrase.find('`')?;
    let rest = &phrase[start + 1..];
    let end = rest.find('`')?;
    let symbol = rest[..end].trim();
    if symbol.is_empty() {
        None
    } else {
        Some(symbol.to_string())
    }
}

fn clean_symbol_token(token: &str) -> &str {
    token
        .trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | ')' | '(' | '"' | '\'' | '`'))
        .trim()
}

fn is_symbol_like_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }

    token.contains("::")
        || token.contains('_')
        || token.chars().skip(1).any(|ch| ch.is_ascii_uppercase())
}

fn infer_simple_run_command(query: &str, normalized: &str) -> Option<String> {
    let prefixes = ["run ", "execute "];
    let mut rest = None;
    for prefix in prefixes {
        if let Some(value) = query.strip_prefix(prefix) {
            rest = Some(value.trim());
            break;
        }
        if let Some(value) = query.strip_prefix(&prefix.to_ascii_uppercase()) {
            rest = Some(value.trim());
            break;
        }
    }

    if let Some(mut candidate) = rest.map(str::trim) {
        if let Some(stripped) = candidate.strip_prefix("the ") {
            candidate = stripped.trim();
        }
        if let Some(stripped) = candidate.strip_suffix(" command") {
            candidate = stripped.trim();
        }
        candidate =
            candidate.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '"' | '\''));
        candidate = sanitize_natural_command_candidate(candidate);
        if is_followup_command_placeholder(candidate) {
            return None;
        }

        let lowered = candidate.to_ascii_lowercase();
        match lowered.as_str() {
            "clear" | "pwd" | "ls" | "date" | "whoami" => return Some(lowered),
            "git status" => return Some("git status".to_string()),
            "git diff" => return Some("git diff".to_string()),
            "harper" | "run harper" => {
                return Some("cargo run -p harper-ui --bin harper".to_string());
            }
            "fmt" | "format" | "run fmt" => return Some("cargo fmt --all".to_string()),
            "fmt check" | "format check" => return Some("cargo fmt --all -- --check".to_string()),
            "tests" | "test" | "run tests" | "run test" => {
                return Some("cargo test --all-features --workspace".to_string());
            }
            "check" | "run check" => return Some("cargo check --workspace".to_string()),
            _ => {}
        }
    }

    match () {
        _ if normalized.contains("run clear") || normalized.contains("clear command") => {
            Some("clear".to_string())
        }
        _ if normalized.contains("run harper") || normalized.contains("start harper") => {
            Some("cargo run -p harper-ui --bin harper".to_string())
        }
        _ if normalized.contains("run fmt")
            || normalized.contains("run format")
            || normalized.contains("format the workspace")
            || normalized.contains("format the repo")
            || normalized.contains("format this repo")
            || normalized.contains("please format the repo")
            || normalized.contains("run the formatter")
            || normalized.contains("run formatter") =>
        {
            Some("cargo fmt --all".to_string())
        }
        _ if normalized.contains("run tests")
            || normalized.contains("run the tests")
            || normalized.contains("test this repo")
            || normalized.contains("test the repo")
            || normalized.contains("run the test suite")
            || normalized.contains("run test suite") =>
        {
            Some("cargo test --all-features --workspace".to_string())
        }
        _ if normalized.contains("run check")
            || normalized.contains("check this repo")
            || normalized.contains("cargo check")
            || normalized.contains("check the workspace")
            || normalized.contains("run the checks") =>
        {
            Some("cargo check --workspace".to_string())
        }
        _ if normalized.contains("which directory")
            || normalized.contains("what directory")
            || normalized.contains("current directory")
            || normalized.contains("where am i")
            || normalized.contains("where are we") =>
        {
            Some("pwd".to_string())
        }
        _ if normalized.contains("list files here")
            || normalized.contains("show files here")
            || normalized.contains("list the files")
            || normalized.contains("show the files") =>
        {
            Some("ls".to_string())
        }
        _ => None,
    }
}

fn sanitize_natural_command_candidate(candidate: &str) -> &str {
    let lowered = candidate.to_ascii_lowercase();
    for suffix in [
        " and summarize it",
        " and summarize",
        " then summarize it",
        " then summarize",
        " and explain it",
        " and explain",
        " and show me",
    ] {
        if lowered.ends_with(suffix) {
            let idx = candidate.len() - suffix.len();
            return candidate[..idx].trim();
        }
    }
    candidate
}

fn is_followup_command_placeholder(candidate: &str) -> bool {
    matches!(
        candidate.trim().to_ascii_lowercase().as_str(),
        "that"
            | "this"
            | "it"
            | "that one"
            | "this one"
            | "that command"
            | "this command"
            | "the command"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_am_changing_phrase() {
        let intent = route_intent("tell me the files am changing here in the codebase");
        assert!(matches!(
            intent,
            Some(DeterministicIntent::ListChangedFiles(_))
        ));
    }

    #[test]
    fn routes_new_files_changed_phrase() {
        let intent = route_intent("show me the new files changed");
        assert!(matches!(
            intent,
            Some(DeterministicIntent::ListChangedFiles(_))
        ));
    }

    #[test]
    fn routes_plain_file_read_request() {
        let intent = route_intent("show me the new file gemini.md");
        assert_eq!(
            intent,
            Some(DeterministicIntent::ReadFile(ReadFileIntent {
                path: "gemini.md".to_string()
            }))
        );
    }

    #[test]
    fn routes_open_file_request() {
        let intent = route_intent("open gemini.md");
        assert_eq!(
            intent,
            Some(DeterministicIntent::ReadFile(ReadFileIntent {
                path: "gemini.md".to_string()
            }))
        );
    }

    #[test]
    fn infers_rust_today_filters() {
        let intent = route_intent("tell me the rust files changed today");
        match intent {
            Some(DeterministicIntent::ListChangedFiles(filters)) => {
                assert_eq!(filters.ext.as_deref(), Some("rs"));
                assert_eq!(filters.since.as_deref(), Some("today"));
            }
            _ => panic!("intent not routed"),
        }
    }

    #[test]
    fn routes_git_status_intent() {
        let intent = route_intent("Run git status and summarize it.");
        assert!(matches!(intent, Some(DeterministicIntent::GitStatus)));
    }

    #[test]
    fn routes_git_diff_intent() {
        let intent = route_intent("show git diff to check code changes");
        assert!(matches!(intent, Some(DeterministicIntent::GitDiff)));
    }

    #[test]
    fn routes_git_branch_intent() {
        let intent = route_intent("which branch i am on");
        assert!(matches!(intent, Some(DeterministicIntent::GitBranch)));
    }

    #[test]
    fn routes_repo_identity_intent() {
        let intent = route_intent("can you check which repo we are working on");
        assert!(matches!(intent, Some(DeterministicIntent::RepoIdentity)));
    }

    #[test]
    fn routes_codebase_overview_intent() {
        let intent = route_intent("tell me the codebase");
        assert!(matches!(
            intent,
            Some(DeterministicIntent::CodebaseOverview)
        ));
    }

    #[test]
    fn routes_quick_repo_overview_request() {
        let intent = route_intent("give me a quick overview of this repo");
        assert!(matches!(
            intent,
            Some(DeterministicIntent::CodebaseOverview)
        ));
    }

    #[test]
    fn routes_workspace_overview_request() {
        let intent = route_intent("walk me through this workspace");
        assert!(matches!(
            intent,
            Some(DeterministicIntent::CodebaseOverview)
        ));
    }

    #[test]
    fn routes_direct_read_file_intent() {
        let intent = route_intent("Read Cargo.toml and tell me the package name.");
        assert_eq!(
            intent,
            Some(DeterministicIntent::ReadFile(ReadFileIntent {
                path: "Cargo.toml".to_string()
            }))
        );
    }

    #[test]
    fn routes_can_you_read_named_file_request() {
        let intent = route_intent("can you read the chat.rs file");
        assert_eq!(
            intent,
            Some(DeterministicIntent::ReadFile(ReadFileIntent {
                path: "chat.rs".to_string()
            }))
        );
    }

    #[test]
    fn routes_inspect_path_request() {
        let intent = route_intent("inspect lib/harper-core/src/agent/chat.rs");
        assert_eq!(
            intent,
            Some(DeterministicIntent::ReadFile(ReadFileIntent {
                path: "lib/harper-core/src/agent/chat.rs".to_string()
            }))
        );
    }

    #[test]
    fn routes_codebase_search_intent() {
        let intent = route_intent("Find where retry metadata is rendered in this repo.");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "retry metadata".to_string()
            }))
        );
    }

    #[test]
    fn routes_where_is_handled_search_intent() {
        let intent = route_intent("where is retry metadata handled in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "retry metadata".to_string()
            }))
        );
    }

    #[test]
    fn routes_where_is_implemented_search_intent() {
        let intent = route_intent("where is agents context implemented in this codebase");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "agents context".to_string()
            }))
        );
    }

    #[test]
    fn routes_what_file_contains_search_intent() {
        let intent = route_intent("what file contains execution strategy in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "execution strategy".to_string()
            }))
        );
    }

    #[test]
    fn routes_who_handles_search_intent() {
        let intent = route_intent("who handles retry metadata in this repository");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "retry metadata".to_string()
            }))
        );
    }

    #[test]
    fn routes_where_is_used_search_intent() {
        let intent = route_intent("where is execution strategy used in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "execution strategy".to_string()
            }))
        );
    }

    #[test]
    fn routes_where_does_come_from_search_intent() {
        let intent = route_intent("where does model label come from in this codebase");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "model label".to_string()
            }))
        );
    }

    #[test]
    fn routes_what_calls_search_intent() {
        let intent = route_intent("what calls update_plan in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "update_plan".to_string()
            }))
        );
    }

    #[test]
    fn routes_where_does_get_set_search_intent() {
        let intent = route_intent("where does model label get set in this codebase");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "model label".to_string()
            }))
        );
    }

    #[test]
    fn routes_what_updates_search_intent() {
        let intent = route_intent("what updates execution strategy in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "execution strategy".to_string()
            }))
        );
    }

    #[test]
    fn routes_how_does_flow_search_intent() {
        let intent = route_intent("how does approval flow in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "approval".to_string()
            }))
        );
    }

    #[test]
    fn routes_backticked_symbol_search_intent() {
        let intent = route_intent("what calls `update_plan` in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "update_plan".to_string()
            }))
        );
    }

    #[test]
    fn routes_camel_case_symbol_search_intent() {
        let intent = route_intent("where is PlanRuntime used in this codebase");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "PlanRuntime".to_string()
            }))
        );
    }

    #[test]
    fn routes_namespaced_symbol_search_intent() {
        let intent = route_intent("where is ExecutionStrategy::Grounded defined in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "ExecutionStrategy::Grounded".to_string()
            }))
        );
    }

    #[test]
    fn prefers_primary_symbol_before_context_symbol() {
        let intent = route_intent("what calls update_plan in ChatService in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "update_plan".to_string()
            }))
        );
    }

    #[test]
    fn prefers_primary_snake_case_symbol_before_type_context() {
        let intent = route_intent("where does model_label get set in TuiApp in this codebase");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "model_label".to_string()
            }))
        );
    }

    #[test]
    fn keeps_full_phrase_for_multi_symbol_primary_clause() {
        let intent =
            route_intent("what calls update_plan and refresh_chat_render_cache in this repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "update_plan and refresh_chat_render_cache".to_string()
            }))
        );
    }

    #[test]
    fn routes_partial_codebase_search_intent_without_repo_noun() {
        let intent = route_intent("Find where retry metadata is rendered in this");
        assert_eq!(
            intent,
            Some(DeterministicIntent::CodebaseSearch(CodebaseSearchIntent {
                pattern: "retry metadata".to_string()
            }))
        );
    }

    #[test]
    fn routes_simple_clear_command_intent() {
        let intent = route_intent("run the clear command");
        assert_eq!(
            intent,
            Some(DeterministicIntent::RunCommand(RunCommandIntent {
                command: "clear".to_string()
            }))
        );
    }

    #[test]
    fn routes_run_harper_command_intent() {
        let intent = route_intent("run harper");
        assert_eq!(
            intent,
            Some(DeterministicIntent::RunCommand(RunCommandIntent {
                command: "cargo run -p harper-ui --bin harper".to_string()
            }))
        );
    }

    #[test]
    fn routes_run_fmt_command_intent() {
        let intent = route_intent("run fmt");
        assert_eq!(
            intent,
            Some(DeterministicIntent::RunCommand(RunCommandIntent {
                command: "cargo fmt --all".to_string()
            }))
        );
    }

    #[test]
    fn routes_natural_language_format_command_intent() {
        let intent = route_intent("please format the repo");
        assert_eq!(
            intent,
            Some(DeterministicIntent::RunCommand(RunCommandIntent {
                command: "cargo fmt --all".to_string()
            }))
        );
    }

    #[test]
    fn routes_run_tests_command_intent() {
        let intent = route_intent("run tests");
        assert_eq!(
            intent,
            Some(DeterministicIntent::RunCommand(RunCommandIntent {
                command: "cargo test --all-features --workspace".to_string()
            }))
        );
    }

    #[test]
    fn routes_run_check_command_intent() {
        let intent = route_intent("run check");
        assert_eq!(
            intent,
            Some(DeterministicIntent::RunCommand(RunCommandIntent {
                command: "cargo check --workspace".to_string()
            }))
        );
    }

    #[test]
    fn does_not_route_placeholder_followup_as_literal_command() {
        let intent = route_intent("run that command?");
        assert_eq!(intent, None);
    }

    #[test]
    fn routes_natural_language_directory_command_intent() {
        let intent = route_intent("which directory are we in");
        assert_eq!(intent, Some(DeterministicIntent::CurrentDirectory));
    }

    #[test]
    fn routes_simple_create_text_file_intent() {
        let intent = route_intent("create a hello world txt file");
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "hello-world.txt".to_string(),
                content: "Hello World".to_string(),
            }))
        );
    }

    #[test]
    fn routes_generic_create_file_saying_prompt_to_text_file() {
        let intent = route_intent("can you create a file saying i am niladri");
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "note.txt".to_string(),
                content: "i am niladri".to_string(),
            }))
        );
    }

    #[test]
    fn routes_generic_named_file_with_numeric_content() {
        let intent =
            route_intent("can you create a sample file name hello and put 1 to 10 in numbers");
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "hello".to_string(),
                content: "1\n2\n3\n4\n5\n6\n7\n8\n9\n10".to_string(),
            }))
        );
    }

    #[test]
    fn sanitizes_git_status_summary_command_prompt() {
        let intent = route_intent("Run git status and summarize it.");
        assert!(matches!(intent, Some(DeterministicIntent::GitStatus)));
    }

    #[test]
    fn routes_simple_create_python_file_intent() {
        let intent = route_intent("create a python hello joy file");
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "hello-joy.py".to_string(),
                content: "print(\"Hello Joy\")".to_string(),
            }))
        );
    }

    #[test]
    fn routes_markdown_authoring_prompt_with_in_markdown() {
        let intent = route_intent("create a new file explaining ai in markdown");
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "ai.md".to_string(),
                content: "# Explaining Ai\n".to_string(),
            }))
        );
    }

    #[test]
    fn routes_markdown_authoring_prompt_with_as_markdown() {
        let intent = route_intent("write a file about planners as markdown");
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "planners.md".to_string(),
                content: "# About Planners\n".to_string(),
            }))
        );
    }

    #[test]
    fn routes_explicit_create_file_with_content() {
        let intent = route_intent(r#"create hello.rs with fn main() { println!("Hi"); }"#);
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "hello.rs".to_string(),
                content: r#"fn main() { println!("Hi"); }"#.to_string(),
            }))
        );
    }

    #[test]
    fn routes_explicit_modify_file_with_content() {
        let intent = route_intent(r#"modify notes.txt to hello from harper"#);
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "notes.txt".to_string(),
                content: "hello from harper".to_string(),
            }))
        );
    }

    #[test]
    fn routes_explicit_edit_file_with_content() {
        let intent = route_intent(r#"edit notes.txt with hello from harper"#);
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "notes.txt".to_string(),
                content: "hello from harper".to_string(),
            }))
        );
    }

    #[test]
    fn routes_explicit_edit_file_with_to_marker() {
        let intent = route_intent(r#"edit README.md to add a badge"#);
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "README.md".to_string(),
                content: "add a badge".to_string(),
            }))
        );
    }

    #[test]
    fn routes_explicit_edit_nested_path_with_content() {
        let intent = route_intent(r#"edit lib/harper-core/src/lib.rs with a new export"#);
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "lib/harper-core/src/lib.rs".to_string(),
                content: "a new export".to_string(),
            }))
        );
    }

    #[test]
    fn routes_explicit_save_file_with_content() {
        let intent = route_intent(r#"save notes.txt with hello"#);
        assert_eq!(
            intent,
            Some(DeterministicIntent::WriteFile(WriteFileIntent {
                path: "notes.txt".to_string(),
                content: "hello".to_string(),
            }))
        );
    }
}
