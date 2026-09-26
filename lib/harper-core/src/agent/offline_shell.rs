// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Offline natural language to shell command routing.

use std::collections::HashSet;

pub fn plan_offline_shell_commands(query: &str) -> Vec<String> {
    let normalized = normalize(query);
    let tokens = token_set(&normalized);

    if let Some(cmd) = extract_explicit_command(query) {
        return vec![cmd];
    }

    if contains_any(
        &normalized,
        &[
            "current directory",
            "working directory",
            "present working directory",
            "where am i",
            "where are we",
            "where are we at",
            "which dir",
            "what dir",
            "what directory",
            "in which dir",
            "in what dir",
        ],
    ) || (has_any(&tokens, &["where", "which", "what"])
        && has_any(&tokens, &["dir", "directory"]))
    {
        return vec!["pwd".to_string()];
    }
    if contains_any(&normalized, &["who am i", "current user", "my username"]) {
        return vec!["whoami".to_string()];
    }
    if contains_any(
        &normalized,
        &[
            "list files",
            "show files",
            "files in this directory",
            "directory contents",
            "list dir",
            "what files are here",
            "show me what files are here",
        ],
    ) || (has_any(&tokens, &["list", "show"]) && has_any(&tokens, &["file", "files"]))
    {
        return vec!["ls -la".to_string()];
    }
    if contains_any(
        &normalized,
        &["list processes", "running processes", "process list"],
    ) {
        return vec!["ps aux".to_string()];
    }
    if contains_any(
        &normalized,
        &["disk usage", "disk space", "free space", "storage usage"],
    ) {
        return vec!["df -h".to_string()];
    }
    if contains_any(&normalized, &["memory usage", "ram usage", "memory stats"]) {
        return vec!["free -h".to_string()];
    }
    if contains_any(&normalized, &["system uptime", "uptime"]) {
        return vec!["uptime".to_string()];
    }
    if contains_any(&normalized, &["list rust files", "show rust files"]) {
        return vec!["find . -name '*.rs'".to_string()];
    }
    if contains_any(
        &normalized,
        &[
            "git status",
            "repo status",
            "repository status",
            "changes in git",
        ],
    ) {
        return vec!["git status --short".to_string()];
    }
    if contains_any(
        &normalized,
        &[
            "what branch",
            "which branch",
            "current branch",
            "git branch",
        ],
    ) {
        return vec!["git branch --show-current".to_string()];
    }
    if contains_any(
        &normalized,
        &[
            "last commit",
            "latest commit",
            "recent commit",
            "last git commit",
        ],
    ) {
        return vec!["git log -1 --oneline".to_string()];
    }
    if contains_any(&normalized, &["python version", "which python"]) {
        return vec!["python3 --version".to_string()];
    }
    if contains_any(&normalized, &["node version", "which node"]) {
        return vec!["node --version".to_string()];
    }
    if contains_any(&normalized, &["npm version"]) {
        return vec!["npm --version".to_string()];
    }
    if contains_any(&normalized, &["rust version", "rustc version"]) {
        return vec!["rustc --version".to_string()];
    }
    if contains_any(&normalized, &["go version"]) {
        return vec!["go version".to_string()];
    }
    if contains_any(&normalized, &["java version"]) {
        return vec!["java -version".to_string()];
    }
    if contains_any(
        &normalized,
        &["environment variables", "env vars", "show env"],
    ) {
        return vec!["printenv".to_string()];
    }

    let asks_network = contains_any(
        &normalized,
        &[
            "network info",
            "network status",
            "ip address",
            "network interfaces",
            "route table",
            "routing table",
            "open ports",
            "listening ports",
            "dns config",
            "hostname",
        ],
    ) || has_any(
        &tokens,
        &[
            "network",
            "ip",
            "interface",
            "interfaces",
            "route",
            "routing",
            "ports",
            "dns",
        ],
    );
    let asks_os = contains_any(&normalized, &["os info", "kernel version", "uname"])
        || has_any(&tokens, &["os", "kernel", "system"]);

    if asks_os || asks_network {
        let mut cmds = Vec::new();
        if asks_os {
            cmds.push("uname -a".to_string());
        }
        if asks_network {
            cmds.push("hostname".to_string());
            cmds.push("ip addr".to_string());
            cmds.push("ip route".to_string());
            cmds.push("ss -tuln".to_string());
        }
        return cmds;
    }

    Vec::new()
}

pub fn plan_offline_shell_command(query: &str) -> Option<String> {
    plan_offline_shell_commands(query).into_iter().next()
}

fn normalize(input: &str) -> String {
    let mut normalized = input
        .to_ascii_lowercase()
        .replace(" ini ", " in ")
        .replace(" dir ", " directory ")
        .replace(" pls ", " please ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if normalized.starts_with("ini ") {
        normalized = normalized.replacen("ini ", "in ", 1);
    }
    normalized
}

fn extract_explicit_command(query: &str) -> Option<String> {
    let trimmed = query.trim();
    let lower = trimmed.to_ascii_lowercase();
    for prefix in ["run ", "execute ", "cmd ", "command "] {
        if lower.starts_with(prefix) {
            let cmd = trimmed[prefix.len()..].trim();
            if !cmd.is_empty() {
                return Some(cmd.to_string());
            }
        }
    }
    None
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

fn token_set(input: &str) -> HashSet<String> {
    input
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>()
}

fn has_any(tokens: &HashSet<String>, words: &[&str]) -> bool {
    words.iter().any(|w| tokens.contains(*w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_current_directory_phrase() {
        assert_eq!(
            plan_offline_shell_command("can you show my current directory"),
            Some("pwd".to_string())
        );
    }

    #[test]
    fn maps_typo_directory_phrase() {
        assert_eq!(
            plan_offline_shell_command("tell me ini which dir we are at"),
            Some("pwd".to_string())
        );
    }

    #[test]
    fn maps_os_kernel_network_bundle() {
        let cmds = plan_offline_shell_commands("in which os and kernel info network and so on");
        assert!(cmds.contains(&"uname -a".to_string()));
        assert!(cmds.contains(&"ip addr".to_string()));
    }

    #[test]
    fn maps_list_files_phrase() {
        assert_eq!(
            plan_offline_shell_command("list files in this directory"),
            Some("ls -la".to_string())
        );
    }

    #[test]
    fn maps_explicit_run_command() {
        assert_eq!(
            plan_offline_shell_command("Run git status"),
            Some("git status".to_string())
        );
    }

    #[test]
    fn maps_branch_check_phrase() {
        assert_eq!(
            plan_offline_shell_command("which branch are we on"),
            Some("git branch --show-current".to_string())
        );
    }

    #[test]
    fn returns_none_for_non_shell_phrase() {
        assert_eq!(
            plan_offline_shell_commands("write me a poem"),
            Vec::<String>::new()
        );
    }
}
