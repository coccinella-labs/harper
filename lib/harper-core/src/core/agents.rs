// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use crate::core::error::{HarperError, HarperResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
enum AgentsRuleDirective {
    Set { key: String, text: String },
    Remove { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsSection {
    pub heading: Option<String>,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveAgentsRule {
    pub text: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveAgentsSection {
    pub heading: Option<String>,
    pub rules: Vec<EffectiveAgentsRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsSource {
    pub path: PathBuf,
    pub content: String,
    #[serde(default)]
    pub sections: Vec<AgentsSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResolvedAgents {
    pub sources: Vec<AgentsSource>,
    #[serde(default)]
    pub effective_sections: Vec<AgentsSection>,
    #[serde(default)]
    pub effective_rule_sections: Vec<EffectiveAgentsSection>,
}

impl ResolvedAgents {
    pub fn render_for_prompt(&self) -> Option<String> {
        if self.sources.is_empty()
            && self.effective_sections.is_empty()
            && self.effective_rule_sections.is_empty()
        {
            return None;
        }

        if !self.effective_rule_sections.is_empty() {
            return Some(render_effective_sections(&self.effective_rule_sections));
        }
        if !self.effective_sections.is_empty() {
            return Some(render_sections(&self.effective_sections));
        }

        let mut rendered = String::new();
        for source in &self.sources {
            rendered.push_str(&format!("Source: {}\n", source.path.display()));
            rendered.push_str(&render_source_sections(source));
            rendered.push_str("\n\n");
        }
        Some(rendered.trim().to_string())
    }

    pub fn render_effective_for_display(&self) -> Option<String> {
        if !self.effective_rule_sections.is_empty() {
            return Some(render_effective_sections_with_sources(
                &self.effective_rule_sections,
            ));
        }
        if self.effective_sections.is_empty() {
            return self.render_for_prompt();
        }
        Some(render_sections(&self.effective_sections))
    }
}

pub fn resolve_agents_for_dir(dir: &Path) -> HarperResult<ResolvedAgents> {
    resolve_agents_for_targets(dir, std::iter::once(dir))
}

pub fn resolve_agents_for_targets<'a, I>(
    base_dir: &Path,
    targets: I,
) -> HarperResult<ResolvedAgents>
where
    I: IntoIterator<Item = &'a Path>,
{
    let repo_root = find_repo_root(base_dir)?;
    let mut sources = BTreeMap::<PathBuf, AgentsSource>::new();

    for target in targets {
        let normalized_target = normalize_target(base_dir, target)?;
        if !normalized_target.starts_with(&repo_root) {
            return Err(HarperError::Validation(format!(
                "Target '{}' is outside repository root '{}'",
                normalized_target.display(),
                repo_root.display()
            )));
        }

        for ancestor in normalized_target.ancestors() {
            if !ancestor.starts_with(&repo_root) {
                continue;
            }
            let agents_path = ancestor.join("AGENTS.md");
            if agents_path.is_file() {
                let content = std::fs::read_to_string(&agents_path).map_err(|err| {
                    HarperError::Io(format!("Failed to read {}: {}", agents_path.display(), err))
                })?;
                let sections = parse_agents_sections(&content);
                sources.entry(agents_path.clone()).or_insert(AgentsSource {
                    path: agents_path,
                    content,
                    sections,
                });
            }
            if ancestor == repo_root {
                break;
            }
        }
    }

    let sources: Vec<AgentsSource> = sources.into_values().collect();
    let effective_rule_sections = merge_agents_sections(&sources);
    let effective_sections = effective_rule_sections
        .iter()
        .map(|section| AgentsSection {
            heading: section.heading.clone(),
            rules: section.rules.iter().map(|rule| rule.text.clone()).collect(),
        })
        .collect();
    Ok(ResolvedAgents {
        sources,
        effective_sections,
        effective_rule_sections,
    })
}

fn normalize_target(base_dir: &Path, target: &Path) -> HarperResult<PathBuf> {
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        base_dir.join(target)
    };

    if joined.exists() {
        return joined.canonicalize().map_err(|err| {
            HarperError::Io(format!(
                "Failed to resolve target path '{}': {}",
                joined.display(),
                err
            ))
        });
    }

    let mut existing_ancestor = joined.as_path();
    let mut missing_suffix = Vec::new();
    while !existing_ancestor.exists() {
        let Some(name) = existing_ancestor.file_name().map(|name| name.to_owned()) else {
            return Err(HarperError::Io(format!(
                "Failed to resolve target path '{}': no existing ancestor found",
                joined.display()
            )));
        };
        missing_suffix.push(name);
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            HarperError::Io(format!(
                "Failed to resolve target path '{}': no existing ancestor found",
                joined.display()
            ))
        })?;
    }

    let mut resolved = existing_ancestor.canonicalize().map_err(|err| {
        HarperError::Io(format!(
            "Failed to resolve target path '{}': {}",
            existing_ancestor.display(),
            err
        ))
    })?;
    for component in missing_suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn find_repo_root(start_dir: &Path) -> HarperResult<PathBuf> {
    let start_dir = start_dir.canonicalize().map_err(|err| {
        HarperError::Io(format!(
            "Failed to resolve directory '{}': {}",
            start_dir.display(),
            err
        ))
    })?;

    let mut git_root = None;
    let mut agents_root = None;
    for ancestor in start_dir.ancestors() {
        if ancestor.join(".git").exists() {
            git_root = Some(ancestor.to_path_buf());
        }
        if ancestor.join("AGENTS.md").exists() {
            agents_root = Some(ancestor.to_path_buf());
        }
    }

    Ok(git_root.or(agents_root).unwrap_or(start_dir))
}

fn render_source_sections(source: &AgentsSource) -> String {
    if source.sections.is_empty() {
        return source.content.trim().to_string();
    }

    let mut rendered = String::new();
    for section in &source.sections {
        if let Some(heading) = &section.heading {
            rendered.push_str(heading);
            rendered.push('\n');
        }
        for rule in &section.rules {
            rendered.push_str("- ");
            rendered.push_str(rule);
            rendered.push('\n');
        }
        rendered.push('\n');
    }
    rendered.trim().to_string()
}

fn render_sections(sections: &[AgentsSection]) -> String {
    let mut rendered = String::new();
    for section in sections {
        if let Some(heading) = &section.heading {
            rendered.push_str(heading);
            rendered.push('\n');
        }
        for rule in &section.rules {
            rendered.push_str("- ");
            rendered.push_str(rule);
            rendered.push('\n');
        }
        rendered.push('\n');
    }
    rendered.trim().to_string()
}

fn render_effective_sections(sections: &[EffectiveAgentsSection]) -> String {
    let simplified = sections
        .iter()
        .map(|section| AgentsSection {
            heading: section.heading.clone(),
            rules: section.rules.iter().map(|rule| rule.text.clone()).collect(),
        })
        .collect::<Vec<_>>();
    render_sections(&simplified)
}

fn render_effective_sections_with_sources(sections: &[EffectiveAgentsSection]) -> String {
    let mut rendered = String::new();
    for section in sections {
        if let Some(heading) = &section.heading {
            rendered.push_str(heading);
            rendered.push('\n');
        }
        for rule in &section.rules {
            rendered.push_str("- ");
            rendered.push_str(&rule.text);
            rendered.push_str(" [");
            rendered.push_str(&rule.source_path.display().to_string());
            rendered.push_str("]\n");
        }
        rendered.push('\n');
    }
    rendered.trim().to_string()
}

fn parse_agents_sections(content: &str) -> Vec<AgentsSection> {
    let mut sections = Vec::new();
    let mut current_heading = None;
    let mut current_rules = Vec::new();
    let mut in_comment = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with("<!--") {
            in_comment = true;
        }
        if in_comment {
            if line.ends_with("-->") {
                in_comment = false;
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }

        if line.starts_with('#') {
            if !current_rules.is_empty() || current_heading.is_some() {
                sections.push(AgentsSection {
                    heading: current_heading.take(),
                    rules: std::mem::take(&mut current_rules),
                });
            }
            current_heading = Some(line.trim_start_matches('#').trim().to_string());
            continue;
        }

        let normalized_rule = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line)
            .trim()
            .to_string();

        if !normalized_rule.is_empty() {
            current_rules.push(normalized_rule);
        }
    }

    if !current_rules.is_empty() || current_heading.is_some() {
        sections.push(AgentsSection {
            heading: current_heading,
            rules: current_rules,
        });
    }

    sections
}

fn merge_agents_sections(sources: &[AgentsSource]) -> Vec<EffectiveAgentsSection> {
    let mut section_order = Vec::<String>::new();
    let mut merged = HashMap::<String, Vec<(String, EffectiveAgentsRule)>>::new();
    let mut heading_map = HashMap::<String, Option<String>>::new();

    for source in sources {
        for section in &source.sections {
            let section_key = normalize_heading_key(section.heading.as_deref());
            if !heading_map.contains_key(&section_key) {
                section_order.push(section_key.clone());
                heading_map.insert(section_key.clone(), section.heading.clone());
            } else if let Some(heading) = &section.heading {
                heading_map.insert(section_key.clone(), Some(heading.clone()));
            }

            let entry = merged.entry(section_key.clone()).or_default();
            for rule in &section.rules {
                match parse_rule_directive(rule) {
                    AgentsRuleDirective::Set { key, text } => {
                        let effective_rule = EffectiveAgentsRule {
                            text,
                            source_path: source.path.clone(),
                        };
                        if let Some(existing_index) = entry
                            .iter()
                            .position(|(existing_key, _)| *existing_key == key)
                        {
                            entry[existing_index] = (key, effective_rule);
                        } else {
                            entry.push((key, effective_rule));
                        }
                    }
                    AgentsRuleDirective::Remove { key } => {
                        if let Some(existing_index) = entry
                            .iter()
                            .position(|(existing_key, _)| *existing_key == key)
                        {
                            entry.remove(existing_index);
                        }
                    }
                }
            }
        }
    }

    section_order
        .into_iter()
        .filter_map(|section_key| {
            let rules = merged
                .remove(&section_key)?
                .into_iter()
                .map(|(_, rule)| rule)
                .collect::<Vec<_>>();
            if rules.is_empty() {
                return None;
            }
            Some(EffectiveAgentsSection {
                heading: heading_map.remove(&section_key).flatten(),
                rules,
            })
        })
        .collect()
}

fn normalize_heading_key(heading: Option<&str>) -> String {
    heading.unwrap_or("__root__").trim().to_ascii_lowercase()
}

fn normalize_rule_key(rule: &str) -> String {
    let trimmed = rule.trim();
    if let Some((prefix, _)) = trimmed.split_once(':') {
        return prefix.trim().to_ascii_lowercase();
    }
    if let Some((prefix, _)) = trimmed.split_once("->") {
        return prefix.trim().to_ascii_lowercase();
    }
    trimmed.to_ascii_lowercase()
}

fn parse_rule_directive(rule: &str) -> AgentsRuleDirective {
    let trimmed = rule.trim();

    if let Some(rest) = trimmed.strip_prefix("remove:") {
        return AgentsRuleDirective::Remove {
            key: normalize_rule_key(rest),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("! ") {
        return AgentsRuleDirective::Remove {
            key: normalize_rule_key(rest),
        };
    }
    if let Some(rest) = trimmed.strip_prefix("replace:") {
        let replacement = rest.trim();
        return AgentsRuleDirective::Set {
            key: normalize_rule_key(replacement),
            text: replacement.to_string(),
        };
    }

    AgentsRuleDirective::Set {
        key: normalize_rule_key(trimmed),
        text: trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write file");
    }

    #[test]
    fn resolves_agents_from_repo_root_to_target_directory() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path().join("repo");
        let nested = repo.join("src/feature");
        fs::create_dir_all(&nested).expect("mkdirs");
        fs::create_dir(repo.join(".git")).expect("git dir");
        write_file(&repo.join("AGENTS.md"), "root rules");
        write_file(&repo.join("src/AGENTS.md"), "src rules");
        write_file(&nested.join("AGENTS.md"), "feature rules");
        let repo = repo.canonicalize().expect("canonical repo");

        let resolved = resolve_agents_for_dir(&nested).expect("resolve");
        let paths: Vec<String> = resolved
            .sources
            .iter()
            .map(|source| {
                source
                    .path
                    .strip_prefix(&repo)
                    .expect("strip")
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();

        assert_eq!(
            paths,
            vec!["AGENTS.md", "src/AGENTS.md", "src/feature/AGENTS.md"]
        );
    }

    #[test]
    fn resolves_only_agents_on_target_ancestor_paths() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path().join("repo");
        let feature_a = repo.join("src/a");
        let feature_b = repo.join("src/b");
        fs::create_dir_all(&feature_a).expect("mkdir a");
        fs::create_dir_all(&feature_b).expect("mkdir b");
        fs::create_dir(repo.join(".git")).expect("git dir");
        write_file(&repo.join("AGENTS.md"), "root rules");
        write_file(&feature_a.join("AGENTS.md"), "a rules");
        write_file(&feature_b.join("AGENTS.md"), "b rules");

        let resolved = resolve_agents_for_targets(&repo, [feature_a.as_path()]).expect("resolve");
        let rendered = resolved.render_for_prompt().expect("render");

        assert!(rendered.contains("root rules"));
        assert!(rendered.contains("a rules"));
        assert!(!rendered.contains("b rules"));
    }

    #[test]
    fn resolves_agents_for_nonexistent_target_using_existing_ancestors() {
        let temp = TempDir::new().expect("temp dir");
        let repo = temp.path().join("repo");
        let nested = repo.join("src/feature");
        fs::create_dir_all(&nested).expect("mkdirs");
        fs::create_dir(repo.join(".git")).expect("git dir");
        write_file(&repo.join("AGENTS.md"), "root rules");
        write_file(&repo.join("src/AGENTS.md"), "src rules");
        write_file(&nested.join("AGENTS.md"), "feature rules");
        let repo = repo.canonicalize().expect("canonical repo");

        let nonexistent = nested.join("new_file.rs");
        let resolved = resolve_agents_for_targets(&repo, [nonexistent.as_path()]).expect("resolve");

        let paths: Vec<String> = resolved
            .sources
            .iter()
            .map(|source| {
                source
                    .path
                    .strip_prefix(&repo)
                    .expect("strip")
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();

        assert_eq!(
            paths,
            vec!["AGENTS.md", "src/AGENTS.md", "src/feature/AGENTS.md"]
        );
    }

    #[test]
    fn parses_agents_sections_from_headings_and_rules() {
        let parsed = parse_agents_sections(
            r#"
            <!-- ignored -->
            # Root Rules
            - Keep changes small
            Explain commands first

            ## Safety
            - Ask before delete
            "#,
        );

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].heading.as_deref(), Some("Root Rules"));
        assert_eq!(
            parsed[0].rules,
            vec![
                "Keep changes small".to_string(),
                "Explain commands first".to_string()
            ]
        );
        assert_eq!(parsed[1].heading.as_deref(), Some("Safety"));
        assert_eq!(parsed[1].rules, vec!["Ask before delete".to_string()]);
    }

    #[test]
    fn merges_sections_with_deeper_rule_override() {
        let resolved = ResolvedAgents {
            sources: vec![
                AgentsSource {
                    path: PathBuf::from("AGENTS.md"),
                    content: String::new(),
                    sections: vec![AgentsSection {
                        heading: Some("Safety".to_string()),
                        rules: vec![
                            "Delete: Ask first".to_string(),
                            "Explain commands first".to_string(),
                        ],
                    }],
                },
                AgentsSource {
                    path: PathBuf::from("src/AGENTS.md"),
                    content: String::new(),
                    sections: vec![AgentsSection {
                        heading: Some("Safety".to_string()),
                        rules: vec!["Delete: Use git rm instead".to_string()],
                    }],
                },
            ],
            effective_sections: vec![AgentsSection {
                heading: Some("Safety".to_string()),
                rules: vec![
                    "Delete: Use git rm instead".to_string(),
                    "Explain commands first".to_string(),
                ],
            }],
            effective_rule_sections: merge_agents_sections(&[
                AgentsSource {
                    path: PathBuf::from("AGENTS.md"),
                    content: String::new(),
                    sections: vec![AgentsSection {
                        heading: Some("Safety".to_string()),
                        rules: vec![
                            "Delete: Ask first".to_string(),
                            "Explain commands first".to_string(),
                        ],
                    }],
                },
                AgentsSource {
                    path: PathBuf::from("src/AGENTS.md"),
                    content: String::new(),
                    sections: vec![AgentsSection {
                        heading: Some("Safety".to_string()),
                        rules: vec!["Delete: Use git rm instead".to_string()],
                    }],
                },
            ]),
        };

        assert_eq!(resolved.effective_rule_sections.len(), 1);
        assert_eq!(
            resolved.effective_rule_sections[0]
                .rules
                .iter()
                .map(|rule| rule.text.clone())
                .collect::<Vec<_>>(),
            vec![
                "Delete: Use git rm instead".to_string(),
                "Explain commands first".to_string()
            ]
        );
        assert_eq!(
            resolved.effective_rule_sections[0].rules[0].source_path,
            PathBuf::from("src/AGENTS.md")
        );
    }

    #[test]
    fn removes_broader_rule_with_explicit_remove_directive() {
        let effective_sections = merge_agents_sections(&[
            AgentsSource {
                path: PathBuf::from("AGENTS.md"),
                content: String::new(),
                sections: vec![AgentsSection {
                    heading: Some("Safety".to_string()),
                    rules: vec![
                        "Delete: Ask first".to_string(),
                        "Explain commands first".to_string(),
                    ],
                }],
            },
            AgentsSource {
                path: PathBuf::from("src/AGENTS.md"),
                content: String::new(),
                sections: vec![AgentsSection {
                    heading: Some("Safety".to_string()),
                    rules: vec!["remove: Delete".to_string()],
                }],
            },
        ]);

        assert_eq!(effective_sections.len(), 1);
        assert_eq!(
            effective_sections[0]
                .rules
                .iter()
                .map(|rule| rule.text.clone())
                .collect::<Vec<_>>(),
            vec!["Explain commands first".to_string()]
        );
    }

    #[test]
    fn replaces_broader_rule_with_explicit_replace_directive() {
        let effective_sections = merge_agents_sections(&[
            AgentsSource {
                path: PathBuf::from("AGENTS.md"),
                content: String::new(),
                sections: vec![AgentsSection {
                    heading: Some("Safety".to_string()),
                    rules: vec!["Delete: Ask first".to_string()],
                }],
            },
            AgentsSource {
                path: PathBuf::from("src/AGENTS.md"),
                content: String::new(),
                sections: vec![AgentsSection {
                    heading: Some("Safety".to_string()),
                    rules: vec!["replace: Delete: Use git rm instead".to_string()],
                }],
            },
        ]);

        assert_eq!(effective_sections.len(), 1);
        assert_eq!(effective_sections[0].rules.len(), 1);
        assert_eq!(
            effective_sections[0].rules[0].text,
            "Delete: Use git rm instead".to_string()
        );
        assert_eq!(
            effective_sections[0].rules[0].source_path,
            PathBuf::from("src/AGENTS.md")
        );
    }
}
