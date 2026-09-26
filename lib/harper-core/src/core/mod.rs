// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Core functionality for Harper AI Agent
//!
//! This module contains the fundamental types and services used throughout the application.

pub mod agents;
pub mod auth;
pub mod cache;
pub mod constants;
pub mod error;
pub mod io_traits;
pub mod llm_client;
pub mod models;
pub mod native_shell;
pub mod plan;
pub mod plan_events;
pub mod tool_call;

/// Supported AI API providers
#[derive(Debug, Clone, Copy)]
pub enum ApiProvider {
    OpenAI,
    Sambanova,
    Gemini,
    Ollama,
    OpenRouter,
    Zen,
}

impl std::fmt::Display for ApiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiProvider::OpenAI => write!(f, "OpenAI"),
            ApiProvider::Sambanova => write!(f, "Sambanova"),
            ApiProvider::Gemini => write!(f, "Gemini"),
            ApiProvider::Ollama => write!(f, "Ollama"),
            ApiProvider::OpenRouter => write!(f, "OpenRouter"),
            ApiProvider::Zen => write!(f, "Zen"),
        }
    }
}

/// Configuration for AI API connections
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// The AI provider to use
    pub provider: ApiProvider,
    /// API key for authentication
    pub api_key: String,
    /// Base URL for the API endpoint
    pub base_url: String,
    /// Name of the model to use
    pub model_name: String,
}

use serde::{Deserialize, Serialize};

/// A message in a conversation with an AI model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// The role of the message sender (user, assistant, system)
    pub role: String,
    /// The content of the message
    pub content: String,
}
