// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

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

    pub const OPENROUTER: ProviderModels = ProviderModels {
        base_url: "https://openrouter.ai/api/v1/chat/completions",
        default_model: "openai/gpt-4o",
    };

    pub const ZEN: ProviderModels = ProviderModels {
        base_url: "https://opencode.ai/zen/v1/chat/completions",
        default_model: "deepseek-v4-pro",
    };
}
