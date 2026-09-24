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
