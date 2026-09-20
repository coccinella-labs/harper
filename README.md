<p align="center">
  <img src="https://raw.githubusercontent.com/Coccinella-Labs/harper/main/.github/assets/thumbnail.png" alt="harper" width="100%">
</p>

# Harper

Harper is a Rust based agent runtime that coordinates large language model calls, tool execution, sandboxing, and multi turn reasoning. It runs as a terminal user interface, an HTTP API, or an embedded service. The core loop and process isolation are active.

## Quick Start

You need Rust 1.85 or higher, an API key for a hosted provider (OpenAI, Sambanova, Gemini, OpenRouter, or Zen), and macOS or Linux. Ollama runs locally and needs no key. On Windows, the runtime builds but process isolation is unavailable because the sandbox relies on `bwrap` or `sandbox-exec`, so WSL2 is recommended.

Build the terminal user interface and set your model key before running:

```bash
cargo build --release -p harper-ui --bin harper
export OPENAI_API_KEY=sk-...
./target/release/harper
```

The interface opens in your current directory. Type a request such as fixing failing tests and Harper inspects the codebase, reads files, runs commands, and iterates.

The HTTP server is enabled by default through `config/default.toml` and listens on `127.0.0.1:8081`. Start it the same way, or pass `--no-server` to run only the terminal interface. Change the bind address in the `[server]` section of your config:

```toml
[server]
enabled = true
host = "127.0.0.1"
port = 8081
```

Run the workspace checks the same way CI does:

```bash
cargo test --workspace
cargo clippy --all-targets --all-features --workspace -- -A clippy::pedantic -D warnings
```

## Architecture Overview

The project is organized into five workspace members:

```text
harper/
├── lib/harper-core/         # ChatService, ToolService, LLM providers, persistence
├── lib/harper-ui/           # TUI (ratatui), CLI, authentication (bins: harper, harper-batch)
├── lib/harper-mcp-server/   # Model Context Protocol (MCP) server (demo server with echo and get_time)
├── lib/harper-sandbox/      # Process isolation (bwrap on Linux, sandbox-exec on macOS)
└── lib/harper-firmware/     # Hardware abstractions (GPIO, I2C, SPI, UART), feature gated behind esp32, stm32, or raspberry_pi
```

The core loop begins when a user message reaches `ChatService::send_message`. Harper first selects a task mode, and deterministic requests such as reading files or checking git status can be answered without a model call. Otherwise it calls the configured provider, sanitizes and parses the reply into tool calls, and handles each call in order: a dedupe key skips calls already executed in the session, policy decides whether the tool is allowed, an approval gate can pause execution, the sandbox runs the command with a 30 second default timeout, and the result is recorded in SQLite and fed back to the model for the next round. The loop runs at most four rounds per message (`MAX_TOOL_ROUNDS`) and stops early when the model returns prose with no tool call.

Sessions, messages, todos, command logs, pending tools, plans, plan runtime, plan events, and session agents persist to a local SQLite database in WAL mode, nine tables in total.

## Configuration and Variables

Configuration follows a precedence chain: `config/default.toml`, then an optional `config/local.toml`, then environment variables. The local file is untracked, so keep machine specific values there.

Provider selection is driven by the key you export. Setting one of these switches the provider, base URL, and default model:

```bash
export OPENAI_API_KEY=sk-...          # OpenAI
export SAMBASTUDIO_API_KEY=...        # Sambanova
export GEMINI_API_KEY=...             # Gemini
export OPENROUTER_API_KEY=...         # OpenRouter
export OPENCODE_API_KEY=...           # Zen
export OLLAMA_HOST=http://localhost:11434   # Ollama, also OLLAMA_BASE_URL
export OLLAMA_MODEL=llama3
```

Other supported environment variables:

```bash
export DATABASE_PATH=.harper/sessions.db   # session database location

# Sandbox overrides
export HARPER_SANDBOX_ENABLED=true
export HARPER_SANDBOX_ALLOWED_DIRS=/path/a:/path/b
export HARPER_SANDBOX_WRITABLE_DIRS=/path/c
export HARPER_SANDBOX_NETWORK=false
export HARPER_SANDBOX_READONLY_HOME=true
```

Provider and behavior settings live in `config/default.toml`:

- `api.provider` accepts `OpenAI`, `Sambanova`, `Gemini`, `Ollama`, `OpenRouter`, or `Zen`.
- `api.model_name` defaults to `gpt-5.5` for OpenAI.
- `database.path` defaults to `.harper/sessions.db`.
- `exec_policy.approval_profile` accepts `strict`, `allow_listed`, or `allow_all`.
- `exec_policy.sandbox_profile` accepts `disabled`, `workspace`, or `networked_workspace`.
- `exec_policy.sandbox.max_execution_time_secs` bounds command runtime.
- `tools.enabled_tools` and `tools.disabled_tools` restrict the tool set.
- `[server] enabled`, `host`, and `port` control the HTTP server.

Supabase backed auth reads `SUPABASE_URL`, `SUPABASE_ANON_KEY`, `SUPABASE_JWT_SECRET`, `SUPABASE_REDIRECT_URL`, and `SUPABASE_ALLOWED_PROVIDERS`.

## HTTP API

With the server enabled, these routes are available:

- `GET /health`: status and version.
- `POST /api/chat`: submit `{"message": "...", "session_id": "uuid"}` and receive the assistant reply and plan.
- `GET /api/sessions`: list recorded sessions.
- `GET /api/sessions/{id}`: load a session with recent messages.
- `GET /api/sessions/{id}/plan`: current plan and loop stage.
- `GET /api/sessions/{id}/plan/stream`: server sent event stream of plan updates.
- `DELETE /api/sessions/{id}`: delete a session.
- `GET /api/approvals/{session_id}`: list pending tool approvals.
- `POST /api/approvals/{session_id}`: approve a command.
- `POST /api/chat/approve/{pending_id}`: approve a pending tool call.
- `POST /api/review`: review a file. The body takes `file_path`, `content`, optional `language`, `workspace_root`, `instructions`, `selection`, and `max_findings`. Reviews call the configured model, and the response reports the model used.

Authentication routes (`/auth/...`) support Supabase backed login when configured.

## Contributing

Fork the repository, create a feature branch, add tests, and open a pull request against `main`. Avoid `unwrap` and panics outside tests and hardened contexts, and prefer explicit error types for fallible operations. Every change must pass `cargo fmt --all -- --check`, clippy, and the workspace test suite.

## Troubleshooting

If the model returns malformed tool output, inspect the session database at `database.path` to see the raw exchange. A stuck approval gate clears when you restart the terminal interface. Long running builds can be given more time with `exec_policy.sandbox.max_execution_time_secs`. A port conflict is resolved by changing `server.port`. An invalid provider error means `api.provider` is not one of the supported values.

## License

Harper is available under the MIT License or the Apache License, Version 2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`. Commercial use terms are in `COMMERCIAL_LICENSE`.
