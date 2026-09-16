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

//! AI provider integrations and cryptographic utilities
//!
//! This module handles communication with different AI providers (OpenAI, Sambanova, Gemini, Ollama)
//! and provides cryptographic functions for secure data handling.

use crate::core::constants::crypto::*;
use crate::core::error::{HarperError, HarperResult};
use crate::core::{ApiConfig, ApiProvider, Message};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::StatusCode;
use ring::{
    aead::{self},
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{json, Value};

fn built_in_tool_functions() -> Vec<Value> {
    vec![
        json!({
            "name": "read_file",
            "description": "Read the contents of a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file to read"
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "write_file",
            "description": "Write content to a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "search_replace",
            "description": "Search and replace text in a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the file"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The text to replace"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text"
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }
        }),
        json!({
            "name": "run_command",
            "description": "Run a shell command",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to run"
                    },
                    "declared_read_paths": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional explicit paths the command will read"
                    },
                    "declared_write_paths": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional explicit paths the command will write"
                    },
                    "requires_network": {
                        "type": "boolean",
                        "description": "Set true if the command needs network access"
                    },
                    "retry_policy": {
                        "type": "string",
                        "enum": ["never", "safe"],
                        "description": "Optional explicit retry policy for transient-safe commands"
                    }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "search",
            "description": "Search the web for current external information",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The web search query"
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "adx_query",
            "description": "Run a read-only KQL query against Azure Data Explorer. Credentials come from HARPER_ADX_* environment variables unless explicitly provided.",
            "parameters": {
                "type": "object",
                "properties": {
                    "cluster_url": {
                        "type": "string",
                        "description": "Azure Data Explorer cluster URL, for example https://help.kusto.windows.net"
                    },
                    "database": {
                        "type": "string",
                        "description": "ADX database name"
                    },
                    "query": {
                        "type": "string",
                        "description": "Read-only KQL query. Management commands that start with a dot are not allowed."
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "todo",
            "description": "Manage todo list. Supported actions: add, list, remove, clear",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["add", "list", "remove", "clear"],
                        "description": "The action to perform"
                    },
                    "description": {
                        "type": "string",
                        "description": "Description for 'add' action"
                    },
                    "index": {
                        "type": "integer",
                        "description": "1-based index for 'remove' action"
                    }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "update_plan",
            "description": "Update the current execution plan for the active session",
            "parameters": {
                "type": "object",
                "properties": {
                    "explanation": {
                        "type": "string",
                        "description": "Optional note explaining the current plan or why it changed"
                    },
                    "authoring_plan": {
                        "type": "object",
                        "description": "Optional structured authoring plan for open-ended code changes",
                        "properties": {
                            "primary_files": {"type": "array", "items": {"type": "string"}},
                            "supporting_files": {"type": "array", "items": {"type": "string"}},
                            "validation_files": {"type": "array", "items": {"type": "string"}},
                            "planned_edits": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "path": {"type": "string"},
                                        "change": {"type": "string"},
                                        "why": {"type": "string"}
                                    },
                                    "required": ["path", "change"]
                                }
                            },
                            "validation_plan": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "command": {"type": "string"},
                                        "scope": {"type": "string"}
                                    },
                                    "required": ["command"]
                                }
                            }
                        }
                    },
                    "items": {
                        "type": "array",
                        "description": "Ordered plan steps",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": {
                                    "type": "string",
                                    "description": "Short step description"
                                },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed", "blocked"],
                                    "description": "Current status of the step"
                                }
                            },
                            "required": ["step", "status"]
                        }
                    }
                },
                "required": ["items"]
            }
        }),
        json!({
            "name": "git_status",
            "description": "Get the current git status",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "git_diff",
            "description": "Get the git diff",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "git_commit",
            "description": "Commit changes with a message",
            "parameters": {
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The commit message"
                    }
                },
                "required": ["message"]
            }
        }),
        json!({
            "name": "git_add",
            "description": "Add files to git staging",
            "parameters": {
                "type": "object",
                "properties": {
                    "files": {
                        "type": "string",
                        "description": "Files to add (space-separated)"
                    }
                },
                "required": ["files"]
            }
        }),
        json!({
            "name": "list_changed_files",
            "description": "List changed files with optional filters",
            "parameters": {
                "type": "object",
                "properties": {
                    "ext": {
                        "type": "string",
                        "description": "Optional file extension filter, for example 'rs'"
                    },
                    "tracked_only": {
                        "type": "boolean",
                        "description": "When true, exclude untracked files"
                    },
                    "since": {
                        "type": "string",
                        "description": "Optional git --since expression, for example 'today'"
                    }
                },
                "required": []
            }
        }),
    ]
}

fn build_ollama_tools() -> Vec<Value> {
    built_in_tool_functions()
        .into_iter()
        .map(|function| json!({"type": "function", "function": function}))
        .collect()
}

fn build_ollama_request_body(config: &ApiConfig, history: &[Message]) -> Value {
    let messages_json: Vec<_> = history
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();

    json!({
        "model": config.model_name,
        "messages": messages_json,
        "tools": build_ollama_tools(),
        "stream": false,
    })
}

fn extract_assistant_reply(provider: &ApiProvider, resp_json: &Value) -> String {
    match provider {
        ApiProvider::OpenAI
        | ApiProvider::Sambanova
        | ApiProvider::OpenRouter
        | ApiProvider::Zen => {
            let message = &resp_json["choices"][0]["message"];
            if let Some(tool_calls) = message.get("tool_calls") {
                serde_json::to_string(tool_calls).unwrap_or_else(|_| "[No response]".to_string())
            } else {
                message["content"]
                    .as_str()
                    .unwrap_or("[No response]")
                    .to_string()
            }
        }
        ApiProvider::Gemini => {
            let part = &resp_json["candidates"][0]["content"]["parts"][0];
            if let Some(function_call) = part.get("functionCall") {
                serde_json::to_string(function_call).unwrap_or_else(|_| "[No response]".to_string())
            } else {
                part.get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("[No response]")
                    .to_string()
            }
        }
        ApiProvider::Ollama => {
            if let Some(tool_calls) = resp_json
                .get("message")
                .and_then(|msg| msg.get("tool_calls"))
                .or_else(|| resp_json.get("tool_calls"))
            {
                serde_json::to_string(tool_calls).unwrap_or_else(|_| "[No response]".to_string())
            } else {
                resp_json
                    .get("message")
                    .and_then(|msg| msg.get("content"))
                    .and_then(|content| content.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        resp_json
                            .get("response")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| "[No response]".to_string())
            }
        }
    }
}

/// Call the configured LLM API with conversation history
///
/// Sends a request to the configured AI provider with the conversation history
/// and returns the AI's response.
///
/// # Arguments
/// * `client` - HTTP client for making API requests
/// * `config` - API configuration including provider, key, and model
/// * `history` - Conversation history as a slice of messages
///
/// # Returns
/// The AI's response as a string
///
/// # Errors
/// Returns `HarperError` if the API call fails or response parsing fails
pub async fn call_llm(
    client: &reqwest::Client,
    config: &ApiConfig,
    history: &[Message],
) -> HarperResult<String> {
    let res = match config.provider {
        ApiProvider::OpenAI
        | ApiProvider::Sambanova
        | ApiProvider::OpenRouter
        | ApiProvider::Zen => {
            let messages_json: Vec<_> = history
                .iter()
                .map(|m| json!({"role": m.role, "content": m.content}))
                .collect();

            let extra_query = String::new();

            let body = json!({
                "model": config.model_name,
                "messages": messages_json,
                "temperature": 0.1,
                "top_p": 0.1,
                "extra_query": extra_query,
            });
            client
                .post(&config.base_url)
                .header(AUTHORIZATION, format!("Bearer {}", config.api_key))
                .header(CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await?
        }
        ApiProvider::Gemini => {
            let mut system_instructions = Vec::new();
            let mut gemini_contents = Vec::new();

            for msg in history {
                if msg.role == "system" {
                    system_instructions.push(msg.content.as_str());
                } else {
                    let role = if msg.role == "assistant" {
                        "model"
                    } else {
                        "user"
                    };
                    gemini_contents.push(json!({
                        "role": role,
                        "parts": [{"text": msg.content}]
                    }));
                }
            }

            let tools = json!({
                "function_declarations": built_in_tool_functions()
            });

            let mut body = json!({
                "contents": gemini_contents,
                "tools": [tools]
            });

            if !system_instructions.is_empty() {
                body["systemInstruction"] = json!({
                    "parts": [{"text": system_instructions.join("

")}]
                });
            }
            let url = config.base_url.clone();
            client
                .post(&url)
                .header(CONTENT_TYPE, "application/json")
                .header("x-goog-api-key", &config.api_key)
                .json(&body)
                .send()
                .await?
        }
        ApiProvider::Ollama => {
            client
                .post(&config.base_url)
                .header(CONTENT_TYPE, "application/json")
                .json(&build_ollama_request_body(config, history))
                .send()
                .await?
        }
    };

    if !res.status().is_success() {
        let status = res.status();
        let error_text = res
            .text()
            .await
            .unwrap_or_else(|_| "Could not read error body".to_string());
        return Err(HarperError::Api(format_api_error(status, &error_text)));
    }

    let resp_json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| HarperError::Api(e.to_string()))?;

    let assistant_reply = extract_assistant_reply(&config.provider, &resp_json);

    Ok(assistant_reply)
}

fn format_api_error(status: StatusCode, error_text: &str) -> String {
    if let Ok(error_json) = serde_json::from_str::<serde_json::Value>(error_text) {
        let message = error_json
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown API error");
        let reason = error_json
            .pointer("/error/details/0/reason")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let domain = error_json
            .pointer("/error/details/0/domain")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if status == StatusCode::FORBIDDEN
            && reason == "API_SERVICE_BLOCKED"
            && domain == "googleapis.com"
        {
            return "Permission denied (403): Google Generative Language API is blocked for this key/project. Enable the API + billing in Google Cloud, or switch provider.".to_string();
        }

        return format!("{} ({})", message, status);
    }

    let compact = error_text.replace('\n', " ");
    let truncated = if compact.chars().count() > 240 {
        format!("{}...", compact.chars().take(240).collect::<String>())
    } else {
        compact
    };
    format!("{} ({})", truncated, status)
}

/// Encrypts data using AES-GCM with a randomly generated key
///
/// # Arguments
/// * `data` - The data to encrypt
///
/// # Returns
/// A tuple containing (encrypted_data, key) on success
///
/// # Errors
/// Returns `HarperError` if any cryptographic operation fails
#[allow(dead_code)]
pub fn encrypt_data(data: &[u8]) -> HarperResult<(Vec<u8>, Vec<u8>)> {
    if data.is_empty() {
        return Err(HarperError::Crypto("empty data".to_string()));
    }

    let rng = SystemRandom::new();

    // Generate a random 256-bit key
    let mut key_bytes = [0u8; AES_256_KEY_LEN];
    rng.fill(&mut key_bytes)
        .map_err(|e| HarperError::Crypto(format!("Key generation failed: {}", e)))?;

    // Generate a random 96-bit nonce (12 bytes)
    let mut nonce_bytes = [0u8; AES_GCM_NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|e| HarperError::Crypto(format!("Nonce generation failed: {}", e)))?;

    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|e| HarperError::Crypto(format!("Invalid key: {}", e)))?;

    let sealing_key = aead::LessSafeKey::new(key);
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);

    // Create a mutable copy of the data to encrypt
    let mut in_out = data.to_vec();

    // Encrypt the data in-place
    let tag = sealing_key
        .seal_in_place_separate_tag(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|e| HarperError::Crypto(format!("Encryption failed: {}", e)))?;

    // Combine nonce (12) + ciphertext (same as input) + tag (16)
    let mut encrypted = Vec::with_capacity(AES_GCM_NONCE_LEN + in_out.len() + AES_GCM_TAG_LEN);
    encrypted.extend_from_slice(&nonce_bytes);
    encrypted.extend_from_slice(&in_out);
    encrypted.extend_from_slice(tag.as_ref());

    Ok((encrypted, key_bytes.to_vec()))
}

/// Decrypts data using AES-GCM
///
/// # Arguments
/// * `encrypted_data` - The encrypted data (nonce + ciphertext + tag)
/// * `key` - The 256-bit key used for decryption
///
/// # Returns
/// The decrypted data on success
///
/// # Errors
/// Returns `HarperError` if decryption fails or input is invalid
#[allow(dead_code)]
pub fn decrypt_data(encrypted_data: &[u8], key: &[u8]) -> HarperResult<Vec<u8>> {
    // Minimum size is nonce (12) + tag (16) = 28 bytes
    if encrypted_data.is_empty() {
        return Err(HarperError::Crypto("empty data".to_string()));
    }
    if encrypted_data.len() < MIN_ENCRYPTED_LEN {
        return Err(HarperError::Crypto("data too short".to_string()));
    }

    // Split the encrypted data into nonce, ciphertext, and tag
    let (nonce_bytes, rest) = encrypted_data.split_at(AES_GCM_NONCE_LEN);
    let (ciphertext, tag) = rest.split_at(rest.len() - AES_GCM_TAG_LEN);

    // Validate key length (256 bits = 32 bytes)
    if key.len() != AES_256_KEY_LEN {
        return Err(HarperError::Crypto("invalid key length".to_string()));
    }

    // Create the key and nonce
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|e| HarperError::Crypto(format!("Invalid key: {}", e)))?;

    let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| HarperError::Crypto("invalid nonce format".to_string()))?;

    // Combine ciphertext and tag for decryption
    let mut in_out = ciphertext.to_vec();
    in_out.extend_from_slice(tag);

    // Decrypt the data
    let opening_key = aead::LessSafeKey::new(key);
    let decrypted = opening_key
        .open_in_place(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|e| HarperError::Crypto(format!("Decryption failed: {}", e)))?;

    Ok(decrypted.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::constants::test_data;
    use crate::core::{ApiConfig, ApiProvider, Message};
    use hex_literal::hex;
    use serde_json::json;

    fn test_ollama_config() -> ApiConfig {
        ApiConfig {
            provider: ApiProvider::Ollama,
            api_key: String::new(),
            base_url: "http://127.0.0.1:11434/api/chat".to_string(),
            model_name: "qwen2.5:1.5b".to_string(),
        }
    }

    #[test]
    fn build_ollama_request_body_includes_tools() {
        let history = vec![Message {
            role: "user".to_string(),
            content: "read Cargo.toml".to_string(),
        }];

        let body = build_ollama_request_body(&test_ollama_config(), &history);
        let tools = body
            .get("tools")
            .and_then(|value| value.as_array())
            .expect("tools array");

        assert!(!tools.is_empty());
        assert_eq!(body["stream"], json!(false));
        assert_eq!(body["model"], json!("qwen2.5:1.5b"));
        assert_eq!(tools[0]["type"], json!("function"));
        assert_eq!(tools[0]["function"]["name"], json!("read_file"));
    }

    #[test]
    fn extract_assistant_reply_prefers_ollama_tool_calls() {
        let resp_json = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {
                        "function": {
                            "name": "read_file",
                            "arguments": {
                                "path": "Cargo.toml"
                            }
                        }
                    }
                ]
            }
        });

        let reply = extract_assistant_reply(&ApiProvider::Ollama, &resp_json);
        assert!(reply.contains("\"name\":\"read_file\""));
        assert!(reply.contains("Cargo.toml"));
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let message = b"Hello, World! This is a test message.";

        // Test with empty data (should fail)
        assert!(
            matches!(encrypt_data(&[]), Err(HarperError::Crypto(_))),
            "Encryption with empty data should fail with Crypto error"
        );

        // Test with valid data
        let (encrypted, key) = encrypt_data(message).expect("Encryption should succeed");

        // Verify the encrypted data has the correct structure
        assert!(encrypted.len() > AES_GCM_NONCE_LEN + AES_GCM_TAG_LEN);

        // Decrypt the message
        let decrypted = decrypt_data(&encrypted, &key).expect("Decryption should succeed");

        // Verify the decrypted message matches the original
        assert_eq!(message, decrypted.as_slice());

        // Test with maximum length data
        let large_message = vec![0xAA; test_data::LARGE_MESSAGE_SIZE];
        let (encrypted_large, key_large) =
            encrypt_data(&large_message).expect("Should encrypt large data");
        let decrypted_large =
            decrypt_data(&encrypted_large, &key_large).expect("Should decrypt large data");
        assert_eq!(large_message, decrypted_large);
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let message = b"Test message";

        // Encrypt with one key
        let (encrypted, _) = encrypt_data(message).expect("Encryption should succeed");

        // Try to decrypt with a different key
        let wrong_key = [0u8; AES_256_KEY_LEN]; // All zeros key
        let result = decrypt_data(&encrypted, &wrong_key);

        // Should fail with Crypto error
        assert!(
            matches!(result, Err(HarperError::Crypto(_))),
            "Decryption with wrong key should fail with Crypto error"
        );

        // Try with invalid key length
        let invalid_key = [0u8; test_data::INVALID_KEY_LEN]; // 128-bit key (too short)
        let result = decrypt_data(&encrypted, &invalid_key);

        // Should fail with Crypto error
        assert!(
            matches!(result, Err(HarperError::Crypto(_))),
            "Decryption with invalid key length should fail with Crypto error"
        );
    }

    #[test]
    fn test_decrypt_invalid_data_fails() {
        // Test with empty data
        assert!(
            matches!(decrypt_data(&[], &[0u8; 32]), Err(HarperError::Crypto(_))),
            "Decryption with empty data should fail with Crypto error"
        );

        // Test with data that's too short (less than nonce + tag)
        let short_data = [0u8; test_data::SHORT_DATA_LEN];
        assert!(
            matches!(
                decrypt_data(&short_data, &[0u8; 32]),
                Err(HarperError::Crypto(_))
            ),
            "Decryption with short data should fail with Crypto error"
        );

        // Test with valid length but invalid format
        let mut invalid_data = vec![0u8; AES_GCM_NONCE_LEN + AES_GCM_TAG_LEN + AES_GCM_TAG_LEN];
        invalid_data[..AES_GCM_NONCE_LEN].copy_from_slice(&hex!("000000000000000000000000")); // Valid nonce

        // This should fail during decryption
        let result = decrypt_data(&invalid_data, &[0u8; 32]);
        assert!(
            matches!(result, Err(HarperError::Crypto(_))),
            "Decryption of invalid data should fail with Crypto error"
        );
    }

    #[test]
    fn test_error_messages() {
        // Test error message formatting
        let key_error = HarperError::Crypto("Invalid key: test".to_string());
        assert_eq!(
            format!("{}", key_error),
            "Cryptography error: Invalid key: test"
        );

        let nonce_error = HarperError::Crypto("Invalid nonce: test".to_string());
        assert_eq!(
            format!("{}", nonce_error),
            "Cryptography error: Invalid nonce: test"
        );

        let decryption_error = HarperError::Crypto("Decryption failed: test".to_string());
        assert_eq!(
            format!("{}", decryption_error),
            "Cryptography error: Decryption failed: test"
        );

        let input_error = HarperError::Crypto("Invalid input: test".to_string());
        assert_eq!(
            format!("{}", input_error),
            "Cryptography error: Invalid input: test"
        );
    }
}
