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

//! In-process file discovery and fixed-string content search.
//!
//! Replaces `walkdir` traversal plus `rg`/`grep` shell-outs with the
//! `ignore` crate (ripgrep's traversal library). Behavior is deterministic
//! across machines: no external binary is required, `.gitignore` is
//! respected, and hidden files are included to match the previous
//! `rg --hidden` invocations.

use std::path::{Path, PathBuf};

/// Directories never descended into, even if not gitignored.
const ALWAYS_SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".harper"];

/// Yield files under `root`, respecting `.gitignore` and always skipping
/// version-control, build-output, and Harper-internal directories.
pub fn walk_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            match entry.file_name().to_str() {
                Some(name) => !ALWAYS_SKIP_DIRS.contains(&name),
                None => false,
            }
        })
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path())
}

/// Read a file as text for searching. Returns `None` for missing,
/// unreadable, or binary (NUL byte) files.
fn read_searchable_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// One matching line: 1-based line number plus the line text.
pub type LineMatch = (usize, String);

/// Fixed-string search over walked files. Returns each file with its
/// matching `(line_number, line)` pairs, mirroring `rg -n -F`.
pub fn search_files(root: &Path, pattern: &str) -> Vec<(PathBuf, Vec<LineMatch>)> {
    let mut hits = Vec::new();
    for path in walk_files(root) {
        let Some(text) = read_searchable_text(&path) else {
            continue;
        };
        let lines: Vec<LineMatch> = text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(pattern))
            .map(|(idx, line)| (idx + 1, line.to_string()))
            .collect();
        if !lines.is_empty() {
            hits.push((path, lines));
        }
    }
    hits
}

/// Files containing `pattern`, mirroring `rg -l -F`.
pub fn files_with_matches(root: &Path, pattern: &str) -> Vec<PathBuf> {
    walk_files(root)
        .filter(|path| {
            read_searchable_text(path)
                .is_some_and(|text| text.lines().any(|line| line.contains(pattern)))
        })
        .collect()
}

/// Whether `path` contains `pattern`, mirroring `rg -q -F`.
pub fn file_contains(path: &Path, pattern: &str) -> bool {
    read_searchable_text(path).is_some_and(|text| text.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_files_respects_gitignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("keep.rs"), "hello\n").expect("write");
        std::fs::write(dir.path().join("skip.log"), "hello\n").expect("write");
        std::fs::write(dir.path().join(".gitignore"), "skip.log\n").expect("write");
        let mut found: Vec<String> = walk_files(dir.path())
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        assert!(found.contains(&"keep.rs".to_string()));
        assert!(!found.iter().any(|name| name == "skip.log"));
    }

    #[test]
    fn search_files_reports_line_numbers() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "one\nneedle here\nthree\n").expect("write");
        let hits = search_files(dir.path(), "needle");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, vec![(2, "needle here".to_string())]);
    }

    #[test]
    fn file_contains_skips_binary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bin.dat");
        std::fs::write(&path, [0x00, 0x01, 0x02]).expect("write");
        assert!(!file_contains(&path, "x"));
    }
}
