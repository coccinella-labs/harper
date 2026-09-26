// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Parsed tool-call boundary, provided by the `coccinella-agent-sdk`.
//!
//! Spike: Harper consumes the extracted boundary through a re-export. The
//! parsing logic now lives in the SDK; this module keeps the
//! `harper_core::core::tool_call` path stable and adapts `from_provider` to
//! the local `ApiProvider`.

pub use coccinella_agent_sdk::tool_call::{ToolCall, ToolCallSource, parse_tool_calls};

use crate::core::ApiProvider;

impl From<&ApiProvider> for ToolCallSource {
    fn from(provider: &ApiProvider) -> Self {
        match provider {
            ApiProvider::OpenAI => ToolCallSource::OpenAi,
            ApiProvider::Sambanova => ToolCallSource::Sambanova,
            ApiProvider::OpenRouter => ToolCallSource::OpenRouter,
            ApiProvider::Zen => ToolCallSource::Zen,
            ApiProvider::Gemini => ToolCallSource::Gemini,
            ApiProvider::Ollama => ToolCallSource::Ollama,
        }
    }
}
