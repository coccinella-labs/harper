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

/// Model configurations for different AI providers
#[derive(Debug, Clone)]
pub struct ProviderModels {
    pub base_url: &'static str,
    pub default_model: &'static str,
}

impl ProviderModels {
    pub const OPENAI: ProviderModels = ProviderModels {
        base_url: "https://api.openai.com/v1/chat/completions",
        default_model: "gpt-5.5",
    };

    pub const SAMBANOVA: ProviderModels = ProviderModels {
        base_url: "https://api.sambanova.ai/v1/chat/completions",
        default_model: "Llama-4-Maverick-17B-128E-Instruct",
    };

    pub const GEMINI: ProviderModels = ProviderModels {
        base_url: "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
        default_model: "gemini-2.5-flash",
    };

    pub const OLLAMA: ProviderModels = ProviderModels {
        base_url: "http://localhost:11434/api/chat",
        default_model: "llama3",
    };

    pub const CEREBRAS: ProviderModels = ProviderModels {
        base_url: "https://api.cerebras.ai/v1/chat/completions",
        default_model: "qwen-3-32b-a4b",
    };

    pub const OPENROUTER: ProviderModels = ProviderModels {
        base_url: "https://openrouter.ai/api/v1/chat/completions",
        default_model: "openai/gpt-4o",
    };

    pub const ZEN: ProviderModels = ProviderModels {
        base_url: "https://opencode.ai/zen/v1/chat/completions",
        default_model: "deepseek-v4-pro",
    };
}
