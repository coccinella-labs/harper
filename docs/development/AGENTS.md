<!--
SPDX-License-Identifier: MIT OR Apache-2.0
Copyright 2026 coccinella-labs
-->

# AI Agent Guidelines for File Operations

This document outlines the policies and guidelines for AI agents (including Harper) when performing file operations, ensuring security, reliability, and user safety.

## Table of Contents

- [Core Principles](#core-principles)
- [File Operation Policies](#file-operation-policies)
- [Security Guidelines](#security-guidelines)
- [Validation Rules](#validation-rules)
- [Error Handling](#error-handling)
- [Audit Trail](#audit-trail)
- [User Consent](#user-consent)
- [Pull Request Guidelines](#pull-request-guidelines)

## Core Principles

### 1. **Safety First**
- Never perform destructive operations without explicit user consent
- Validate all file paths and operations before execution
- Implement fail-safe mechanisms for rollback scenarios

### 2. **Transparency**
- Clearly communicate all intended file operations to users
- Provide detailed feedback on operation results
- Maintain comprehensive logging of all file activities

### 3. **Minimal Impact**
- Operate only on files within the project workspace
- Avoid system-wide or global file modifications
- Respect file permissions and ownership

### 4. **Reliability**
- Implement atomic operations where possible
- Provide backup mechanisms for critical files
- Validate operation success before reporting completion

## File Operation Policies

### Read Operations
```rust
//  ALLOWED: Reading project files
read_file("src/main.rs") // Within project scope
read_file("README.md")   // Documentation files

//  FORBIDDEN: Reading sensitive files
read_file("/etc/passwd")        // System files
read_file("~/.ssh/id_rsa")      // Private keys
read_file("../other_project/*") // Outside project scope
```

**Policy Rules:**
- Only read files within the current project directory
- Respect `.gitignore` patterns
- Never read sensitive configuration files
- Limit file size to prevent memory exhaustion

### Write Operations
```rust
//  ALLOWED: With user consent
write_file("src/new_feature.rs", content) // New project files
write_file("tests/test_file.rs", content) // Test files

//  FORBIDDEN: Without validation
write_file("/etc/hosts", content)        // System files
write_file("~/.bashrc", content)         // User config
write_file("../other/file.rs", content)  // Outside scope
```

**Policy Rules:**
- Require explicit user approval for all write operations
- Create backups of modified files
- Validate file paths are within project scope
- Check file permissions before writing

### Delete Operations
```rust
//  STRICTLY FORBIDDEN: Direct file deletion
// Use git operations instead for version control
```

**Policy Rules:**
- Never perform direct file deletion
- Use version control operations for file removal
- Suggest `git rm` for tracked files

### Search and Replace Operations
```rust
//  ALLOWED: With validation
search_replace("src/file.rs", "old_code", "new_code")

// Requirements:
// - User approval required
// - Backup original file
// - Validate regex patterns
// - Limit replacement scope
```

**Policy Rules:**
- Require user approval for all modifications
- Create backup before modification
- Validate search patterns are safe
- Limit the number of replacements

## Security Guidelines

### Path Validation
```rust
fn validate_path(path: &str) -> Result<(), AgentError> {
    // Prevent directory traversal
    if path.contains("..") {
        return Err(AgentError::Security("Path traversal detected"));
    }

    // Ensure path is within project
    let canonical_path = std::fs::canonicalize(path)?;
    let project_root = std::env::current_dir()?;
    if !canonical_path.starts_with(project_root) {
        return Err(AgentError::Security("Path outside project scope"));
    }

    Ok(())
}
```

### Content Validation
```rust
fn validate_content(content: &str) -> Result<(), AgentError> {
    // Check for malicious patterns
    let dangerous_patterns = [
        r"<script[^>]*>.*?</script>",  // XSS attempts
        r"rm\s+-rf\s+/",               // Dangerous commands
        r"eval\s*\(",                  // Code injection
    ];

    for pattern in &dangerous_patterns {
        if regex::Regex::new(pattern)?.is_match(content) {
            return Err(AgentError::Security("Potentially dangerous content detected"));
        }
    }

    Ok(())
}
```

### Permission Checks
```rust
fn check_permissions(path: &Path) -> Result<(), AgentError> {
    let metadata = std::fs::metadata(path)?;

    // Check if file is writable
    if metadata.permissions().readonly() {
        return Err(AgentError::Security("File is read-only"));
    }

    // Check if we're the owner or have write access
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current_uid = unsafe { libc::getuid() };
        if metadata.uid() != current_uid {
            return Err(AgentError::Security("Permission denied"));
        }
    }

    Ok(())
}
```

## Validation Rules

For routing, repo-aware tool use, transcript rendering, TUI command/file behavior, or authoring-manager changes, run the smoke checklist in `docs/testing/tui-routing-smoke.md`.

### File Type Restrictions
```rust
const ALLOWED_EXTENSIONS: &[&str] = &[
    "rs", "toml", "md", "txt", "json", "yml", "yaml",
    "js", "ts", "py", "sh", "sql"
];

const FORBIDDEN_FILES: &[&str] = &[
    "Cargo.lock", ".env", "id_rsa", "secrets.json"
];
```

### Size Limits
```rust
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB
const MAX_READ_SIZE: usize = 1024 * 1024;    // 1MB for display
```

### Operation Limits
```rust
const MAX_FILES_PER_OPERATION: usize = 10;
const MAX_SEARCH_REPLACEMENTS: usize = 100;
```

## Error Handling

### Structured Error Types
```rust
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Security violation: {0}")]
    Security(String),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("File operation failed: {0}")]
    FileOperation(String),

    #[error("Permission denied: {0}")]
    Permission(String),
}
```

### Error Recovery
```rust
fn perform_safe_operation<F, T>(operation: F) -> Result<T, AgentError>
where
    F: FnOnce() -> Result<T, AgentError>,
{
    // Pre-operation validation
    validate_preconditions()?;

    // Execute with rollback capability
    match operation() {
        Ok(result) => {
            log_operation_success();
            Ok(result)
        }
        Err(e) => {
            attempt_rollback()?;
            log_operation_failure(&e);
            Err(e)
        }
    }
}
```

## Audit Trail

### Operation Logging
```rust
#[derive(Debug, serde::Serialize)]
struct OperationLog {
    timestamp: chrono::DateTime<chrono::Utc>,
    operation: String,
    path: String,
    user_consent: bool,
    success: bool,
    error_message: Option<String>,
}

fn log_operation(op: &OperationLog) {
    // Log to file and/or database
    // Include in session history
}
```

Harper now persists shell executions to a `command_logs` table (session ID, command text, approval state, exit code, runtime, and stdout/stderr previews), ensuring every approved or rejected command is audit-able alongside the chat transcript. Operators can review the last `n` entries interactively via `/audit` (defaults to 10, e.g., `/audit 25 failed approved`) and see a summary in the session viewer/export flow.

### Session Tracking
```rust
struct AgentSession {
    session_id: String,
    operations: Vec<OperationLog>,
    start_time: chrono::DateTime<chrono::Utc>,
    user_approvals: usize,
}
```

## User Consent

### Consent Workflow
```rust
fn request_user_consent(operation: &str, details: &str) -> Result<bool, AgentError> {
    println!(" AI Agent Request:");
    println!("Operation: {}", operation);
    println!("Details: {}", details);
    println!("");
    println!("Do you approve? (y/n): ");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    Ok(input.trim().eq_ignore_ascii_case("y"))
}
```

### Consent Types
- **Read Operations**: Automatic (low risk)
- **Write Operations**: Required (medium risk)
- **Delete Operations**: Forbidden (high risk)
- **System Operations**: Required (high risk)

## Implementation Checklist

- [ ] Path validation functions
- [ ] Content security scanning
- [ ] User consent workflow
- [ ] Operation logging
- [ ] Backup mechanisms
- [ ] Size and count limits
- [ ] Permission checking
- [ ] Error recovery
- [ ] Audit trail
- [ ] Session tracking

## Testing Requirements

### Security Tests
```rust
#[test]
fn test_path_traversal_prevention() {
    assert!(validate_path("../../../etc/passwd").is_err());
}

#[test]
fn test_malicious_content_detection() {
    let malicious = "<script>alert('xss')</script>";
    assert!(validate_content(malicious).is_err());
}
```

### Integration Tests
```rust
#[test]
fn test_file_operation_workflow() {
    // Test complete read-write cycle with consent
    // Verify backups are created
    // Check audit logs
}
```

---

## Rust Coding Guidelines

When contributing to Harper's Rust codebase, follow these guidelines to ensure efficient, safe, and maintainable code.

### Preferring Structs and Enums over Complex Inheritance

Use structs and enums for data modeling. Leverage traits for polymorphism rather than inheritance hierarchies.

- **Seamless Ownership**: Structs and enums work naturally with Rust's ownership and borrowing system.
- **Reduced Boilerplate**: Derive macros (`#[derive(Debug, Clone)]`) provide common functionality.
- **Enhanced Readability**: Explicit fields and pattern matching make data structures clear and safe.
- **Immutability by Default**: Rust's immutability encourages functional patterns.

### Embracing Iterator Methods

Leverage Rust's iterator methods like `.map()`, `.filter()`, `.fold()`, `.collect()` for transforming data immutably and declaratively.

- **Promotes Immutability**: Most methods return new collections.
- **Improves Readability**: Chaining leads to concise, expressive code.
- **Facilitates Functional Programming**: Pure functions that transform data.
- **Performance**: Lazy evaluation and efficient composition.

### Avoiding `unwrap()` and `expect()`; Preferring Proper Error Handling

Avoid `unwrap()` and `expect()` in production code. Use `?` operator, `match`, or `if let` for explicit error handling.

- **Prevents Panics**: Graceful error propagation instead of crashes.
- **Robustness**: Forces handling of potential failures.
- **Maintainability**: Clear error paths make debugging easier.

```rust
// Preferred
fn process_data(data: Option<Data>) -> Result<Processed, Error> {
    let data = data.ok_or(Error::MissingData)?;
    Ok(process(data))
}

// Avoid
fn process_data(data: Option<Data>) -> Processed {
    data.unwrap() // Panics on None
}
```

### Result and Option Patterns

Use `Result` and `Option` extensively. Prefer early returns with `?` and pattern matching.

- **Early Returns**: `?` for propagating errors.
- **Pattern Matching**: `match` or `if let` over `unwrap()`.
- **Builder Pattern**: For complex construction with error handling.

### Avoiding Global State; Preferring Dependency Injection

Avoid global variables and static mutables. Pass dependencies explicitly.

- **Testability**: Easier unit testing with explicit deps.
- **Concurrency**: Safer in multi-threaded code.
- **Modularity**: Clear component interfaces.

### Embracing Cargo Features for Conditional Compilation

Use Cargo features for optional functionality and platform-specific code.

- **Optional Dependencies**: Enable only when needed.
- **Platform-Specific**: `#[cfg()]` for targeted implementations.
- **Modular Builds**: Customize for different use cases.

### Testing Guidelines

Follow these patterns for comprehensive testing:

- **Framework**: Use `#[test]` and `#[tokio::test]` for async.
- **Mocking**: Use libraries like `mockito` for HTTP, `tempfile` for FS.
- **Async Testing**: `#[tokio::test]`, fake timers with `tokio::time`.
- **Error Testing**: Assert `Result::Err` variants explicitly.
- **General**: Examine existing tests for conventions, prefer table-driven tests.

### Documentation and Comments

- **High-Value Comments**: Only add comments that explain why, not what.
- **API Docs**: Document public functions with examples.
- **Technical Accuracy**: Base all docs on actual code behavior.

**Remember**: AI agents should enhance human productivity while maintaining strict safety boundaries. When in doubt, require explicit user approval for any file operation.

## Pull Request Guidelines

When creating pull requests, follow these steps:

1. **Analyze Changes**: Review all changes to be included in the PR with `git diff` and `git log`
2. **Draft Summary**: Create a summary with 1-3 bullet points describing the changes
3. **Create PR**: Use `gh pr create` with the following format:

```bash
gh pr create --title "<title>" --body "$(cat <<'EOF'
## Summary
- <bullet point 1>
- <bullet point 2>
- <bullet point 3>
EOF
)"
```

4. **Push Branch**: If needed, push with `git push -u origin <branch-name>`

### PR Title Convention
- Use the format `[scope] description`
- Example: `[feat] add async git status helper`
- Scopes: `feat`, `fix`, `docs`, `chore`, `deps`, `refactor`, `test`

### Handling Pre-commit Hooks
If pre-commit hooks reject your commit message format, use `--no-verify` to skip:
```bash
git commit --no-verify -m "[chore] your commit message"
git push --no-verify origin main
```
