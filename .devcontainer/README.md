# Dev Container

Built on Debian Bookworm with Rust. Installs:

- `libsqlite3-dev` - for rusqlite
- `pkg-config` - build helper
- `bubblewrap` - sandbox isolation

## What It Does

Provides a reproducible development environment with the exact dependencies Harper needs.

For sandbox work, this container is the preferred validation environment because it matches the
Linux `bubblewrap` backend that `harper-sandbox` exercises.

## Helpful

- Open in VS Code: `Cmd+Shift+P` → "Rebuild and Reopen in Container"
- Or use GitHub Codespaces
- Extensions auto-install (rust-analyzer, even-better-toml, vscode-json)
- Clippy runs on save via `rust-analyzer.check.command`
