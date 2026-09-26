<!--
SPDX-License-Identifier: MIT OR Apache-2.0
Copyright 2026 coccinella-labs
-->

# Workspace Reset Guide

[![Git: Version Control](https://img.shields.io/badge/git-workspace%20reset-blue)](https://github.com/coccinella-labs/harper)
[![Cargo: Build System](https://img.shields.io/badge/cargo-clean-orange)](https://doc.rust-lang.org/cargo/)

This guide provides commands to completely reset your Harper workspace to a clean state. Use these commands when you need to start fresh or resolve development issues.

## Table of Contents

- [Quick Reset](#quick-reset)
- [Detailed Reset Commands](#detailed-reset-commands)
- [Common Scenarios](#common-scenarios)
- [Safety Precautions](#safety-precautions)
- [Troubleshooting](#troubleshooting)

## Quick Reset

For a complete workspace reset, run these commands in order:

```bash
# Reset repository to clean state
git reset --hard HEAD
git clean -fdx

# Clean build artifacts
cargo clean

# Rebuild from scratch
cargo build
```

## Detailed Reset Commands

### Git Repository Reset

<details>
<summary>Revert All Changes</summary>

**Command:**
```bash
git reset --hard HEAD
```

**What it does:**
- Resets all tracked files to the last commit state
- Discards all uncommitted changes in tracked files
- Moves HEAD pointer back to the current commit

**Use when:** You want to undo all local changes and return to the last commit.
</details>

<details>
<summary>Remove Untracked Files</summary>

**Command:**
```bash
git clean -fd
```

**What it does:**
- Removes all untracked files and directories
- Preserves ignored files (those in .gitignore)
- Safe for most cleanup operations

**Use when:** You have build artifacts or temporary files to remove.
</details>

<details>
<summary>Remove All Files (Including Ignored)</summary>

**Command:**
```bash
git clean -fdx
```

**What it does:**
- Removes untracked files AND ignored files
- More aggressive cleanup
-  **Caution**: May remove important files like IDE settings

**Use when:** You need a completely clean workspace, including ignored files.
</details>

### Build System Reset

<details>
<summary>Clean Build Artifacts</summary>

**Command:**
```bash
cargo clean
```

**What it does:**
- Removes all build artifacts from `target/` directory
- Clears compiled binaries and intermediate files
- Frees up disk space

**Use when:** Build issues, switching Rust versions, or cleaning up space.
</details>

<details>
<summary>Rebuild Project</summary>

**Command:**
```bash
cargo build
```

**What it does:**
- Downloads dependencies (if needed)
- Compiles the entire project from scratch
- Creates fresh binaries in `target/debug/`

**Use when:** After cleaning or when dependencies have changed.
</details>

### Advanced Reset

<details>
<summary>Update Submodules</summary>

**Command:**
```bash
git submodule update --init --recursive
```

**What it does:**
- Initializes and updates all git submodules
- Ensures submodules are at correct commits
- Required if project uses submodules

**Use when:** Working with projects that have git submodules.
</details>

<details>
<summary>Reset to Specific Commit</summary>

**Command:**
```bash
git reset --hard <commit-hash>
```

**What it does:**
- Resets repository to a specific commit
- Discards all changes after that commit

**Use when:** Need to revert to a known good state.
</details>

## Common Scenarios

### After Failed Build
```bash
cargo clean
cargo build
```

### Starting Fresh Development Session
```bash
git status  # Check current state
git stash   # Save changes if needed
git reset --hard HEAD
git clean -fd
cargo clean
cargo build
```

### Resolving Merge Conflicts
```bash
git merge --abort  # If in merge
git reset --hard HEAD
git clean -fd
```

### Switching Branches with Conflicts
```bash
git stash  # Save work
git checkout <branch>
git stash pop  # Restore work
```

## Safety Precautions

###  Important Warnings

- **Data Loss**: These commands permanently delete uncommitted changes
- **Backup First**: Always commit or stash important work before resetting
- **Review Changes**: Use `git status` and `git diff` to see what will be lost
- **Test Commands**: Run `git clean -fd --dry-run` to preview what will be removed

### Safe Practices

```bash
# Always check status first
git status

# Preview what will be cleaned
git clean -fd --dry-run

# Stash changes instead of losing them
git stash push -m "Work in progress"

# After reset, restore work
git stash pop
```

### Recovery Options

If you accidentally delete important files:

```bash
# Restore from git (if committed)
git checkout HEAD -- <file>

# Restore from stash
git stash pop

# Restore from reflog
git reflog
git reset --hard <commit-from-reflog>
```

## Troubleshooting

### Common Issues

**"Permission denied" errors:**
```bash
# On Unix systems
chmod +x scripts/*
sudo cargo clean  # If needed
```

**Build still failing after reset:**
```bash
# Clear Cargo cache
rm -rf ~/.cargo/registry/cache
rm -rf ~/.cargo/git/checkouts

# Rebuild
cargo clean
cargo build
```

**Git repository corrupted:**
```bash
# Remove and re-clone (last resort)
cd ..
rm -rf harper
git clone https://github.com/coccinella-labs/harper.git
cd harper
```

### Getting Help

- **Documentation**: See [Contributing Guide](https://github.com/coccinella-labs/harper/blob/main/CONTRIBUTING.md)
- **Issues**: [Report problems](https://github.com/coccinella-labs/harper/issues)
- **Discussions**: [Ask questions](https://github.com/coccinella-labs/harper/discussions)

---

**Remember**: When in doubt, commit your work first! It's always safer to have changes in git history than to lose them permanently.

For more information about Harper development, see the main [README](https://github.com/coccinella-labs/harper/blob/main/README.md) and [Contributing Guide](https://github.com/coccinella-labs/harper/blob/main/CONTRIBUTING.md).
