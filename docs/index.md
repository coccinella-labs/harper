# Harper

Harper is a terminal-first AI agent for code and shell work. It keeps command execution visible, supports approval-gated workflows, tracks multi-step work explicitly, and provides a native command shell for Harper concepts instead of hiding what it did.

## Features

- **Visible command execution**: Harper shows the concrete commands it plans to run
- **Approval gates**: Sensitive or higher-risk operations can require confirmation
- **Session and plan tracking**: Conversations, titles, audit logs, and plans persist with the session
- **Native shell commands**: Use commands such as `plan show`, `session list`, `config show`, `update check`, and explicit `run ...`
- **Provider support**: Works with OpenAI, Gemini, SambaNova, and Ollama
- **TUI and batch flows**: Use the interactive terminal UI or verify behavior headlessly with `harper-batch`

## Getting Started

New to Harper? Start here:

1. [Installation Guide](user-guide/installation.md) - Install Harper from source, Homebrew, or direct release artifacts
2. [Quick Start](user-guide/quick-start.md) - Get the TUI running and learn the basic interaction model
3. [Configuration](user-guide/configuration.md) - Configure providers, execution policy, and UI behavior

## User Guide

Learn about Harper's features in detail:

- [About the Binary](user-guide/about.md) - Runtime model, install source behavior, and binary overview
- [Chat Interface](user-guide/chat.md) - Native commands, slash commands, routing, follow-ups, and update checks
- [Clipboard Features](user-guide/clipboard.md) - Working with images and text from the terminal
- [Configuration](user-guide/configuration.md) - Complete configuration options
- [Troubleshooting](user-guide/troubleshooting.md) - Common issues and validation paths

## Development

For developers wanting to contribute:

- [Contributing Guide](development/contributing.md)
- [API Reference](development/api.md)
- [CI/CD](development/cicd.md)

## Support

Having issues? Check the [Troubleshooting Guide](user-guide/troubleshooting.md) or visit our [GitHub Issues](https://github.com/coccinella-labs/harper/issues).
