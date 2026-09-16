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

use crate::core::ApiProvider;
use serde::Serialize;
use serde_json::Value;

/// The source provider that produced this tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ToolCallSource {
    OpenAi,
    Sambanova,
    OpenRouter,
    Zen,
    Gemini,
    Ollama,
    Mcp,
    BracketLegacy,
}

impl ToolCallSource {
    /// Map an `ApiProvider` to its parse source.
    pub fn from_provider(provider: &ApiProvider) -> Self {
        match provider {
            ApiProvider::OpenAI => Self::OpenAi,
            ApiProvider::Sambanova => Self::Sambanova,
            ApiProvider::OpenRouter => Self::OpenRouter,
            ApiProvider::Zen => Self::Zen,
            ApiProvider::Gemini => Self::Gemini,
            ApiProvider::Ollama => Self::Ollama,
        }
    }
}

/// A parsed tool call from the LLM response.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    /// Provider-assigned tool call id (OpenAI, Ollama).
    pub id: Option<String>,
    /// The function/tool name.
    pub name: String,
    /// Parsed arguments (object or null).
    pub arguments: Value,
    /// Which provider format produced this call.
    pub source: ToolCallSource,
}

impl ToolCall {
    /// Canonical dedup key: `name` + JSON-canonicalized arguments.
    pub fn dedupe_key(&self) -> String {
        format!("{}:{}", self.name, canonicalize_value(&self.arguments))
    }
}

/// Canonicalize a JSON value for dedup (sorted object keys, stable formatting).
fn canonicalize_value(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            let inner: Vec<String> = pairs
                .into_iter()
                .map(|(k, val)| format!("\"{}\":{}", k, canonicalize_value(val)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonicalize_value).collect();
            format!("[{}]", items.join(","))
        }
        Value::String(s) => format!("\"{}\"", s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
    }
}

/// Parse tool calls from the raw string returned by `call_llm` / `extract_assistant_reply`.
pub fn parse_tool_calls(raw: &str, source: ToolCallSource) -> Vec<ToolCall> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return vec![];
    }

    let json: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            if let Some(tc) = try_parse_bracket(trimmed, source) {
                return vec![tc];
            }
            return vec![];
        }
    };

    match source {
        ToolCallSource::OpenAi
        | ToolCallSource::Sambanova
        | ToolCallSource::OpenRouter
        | ToolCallSource::Zen
        | ToolCallSource::Ollama => parse_openai_tool_calls(&json, source),
        ToolCallSource::Gemini => parse_gemini_tool_calls(&json, source),
        ToolCallSource::Mcp => parse_mcp_tool_call(&json, source),
        ToolCallSource::BracketLegacy => {
            if let Some(tc) = try_parse_bracket(trimmed, source) {
                vec![tc]
            } else {
                vec![]
            }
        }
    }
}

/// Parse OpenAI-style tool_calls array.
fn parse_openai_tool_calls(json: &Value, source: ToolCallSource) -> Vec<ToolCall> {
    let arr = match json.as_array() {
        Some(a) => a,
        None => return vec![],
    };

    arr.iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(|v| v.as_str()).map(String::from);
            let function = item.get("function")?;
            let name = function.get("name")?.as_str()?.to_string();
            let arguments = match function.get("arguments") {
                Some(Value::String(s)) => {
                    serde_json::from_str(s).unwrap_or(Value::Object(serde_json::Map::new()))
                }
                Some(v) => v.clone(),
                None => Value::Object(serde_json::Map::new()),
            };
            Some(ToolCall {
                id,
                name,
                arguments,
                source,
            })
        })
        .collect()
}

/// Parse Gemini functionCall (single object).
fn parse_gemini_tool_calls(json: &Value, source: ToolCallSource) -> Vec<ToolCall> {
    if let Some(fc) = json.get("functionCall") {
        if let Some(name) = fc.get("name").and_then(|v| v.as_str()) {
            let args = fc
                .get("args")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));
            return vec![ToolCall {
                id: None,
                name: name.to_string(),
                arguments: args,
                source,
            }];
        }
    }
    vec![]
}

/// Parse MCP tool call object.
fn parse_mcp_tool_call(json: &Value, source: ToolCallSource) -> Vec<ToolCall> {
    let Some(name) = json
        .get("mcp_tool")
        .or_else(|| json.get("tool"))
        .or_else(|| json.get("name"))
        .and_then(|v| v.as_str())
    else {
        return vec![];
    };
    let arguments = json
        .get("arguments")
        .or_else(|| json.get("args"))
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    vec![ToolCall {
        id: None,
        name: name.to_string(),
        arguments,
        source,
    }]
}

/// Try to parse legacy bracket-format tool calls like `[RUN_COMMAND ls]`.
fn try_parse_bracket(trimmed: &str, source: ToolCallSource) -> Option<ToolCall> {
    const RUN_COMMAND: &str = "[RUN_COMMAND";
    const READ_FILE: &str = "[READ_FILE";
    const WRITE_FILE: &str = "[WRITE_FILE";
    const SEARCH_REPLACE: &str = "[SEARCH_REPLACE";
    const SEARCH: &str = "[SEARCH:";
    const GIT_STATUS: &str = "[GIT_STATUS";
    const GIT_DIFF: &str = "[GIT_DIFF";
    const GIT_COMMIT: &str = "[GIT_COMMIT";
    const GIT_ADD: &str = "[GIT_ADD";
    const TODO: &str = "[TODO";
    const UPDATE_PLAN: &str = "[UPDATE_PLAN";
    const CODEBASE_INVESTIGATOR: &str = "[CODEBASE_INVESTIGATOR";

    let upper = trimmed.to_ascii_uppercase();
    let (name, prefix) = if upper.starts_with(RUN_COMMAND) {
        ("run_command", RUN_COMMAND)
    } else if upper.starts_with(READ_FILE) {
        ("read_file", READ_FILE)
    } else if upper.starts_with(WRITE_FILE) {
        ("write_file", WRITE_FILE)
    } else if upper.starts_with(SEARCH_REPLACE) {
        ("search_replace", SEARCH_REPLACE)
    } else if upper.starts_with(SEARCH) {
        ("search", SEARCH)
    } else if upper.starts_with(GIT_STATUS) {
        ("git_status", GIT_STATUS)
    } else if upper.starts_with(GIT_DIFF) {
        ("git_diff", GIT_DIFF)
    } else if upper.starts_with(GIT_COMMIT) {
        ("git_commit", GIT_COMMIT)
    } else if upper.starts_with(GIT_ADD) {
        ("git_add", GIT_ADD)
    } else if upper.starts_with(TODO) {
        ("todo", TODO)
    } else if upper.starts_with(UPDATE_PLAN) {
        ("update_plan", UPDATE_PLAN)
    } else if upper.starts_with(CODEBASE_INVESTIGATOR) {
        ("codebase_investigator", CODEBASE_INVESTIGATOR)
    } else {
        return None;
    };

    let mut content = trimmed;
    if let Some(idx) = content.find('[') {
        content = &content[idx + 1..];
    }
    content = content.trim_start();
    content = content[prefix.len() - 1..].trim_end();
    content = content.trim_end_matches(']').trim();

    let mut args = serde_json::Map::new();
    match name {
        "run_command" => {
            args.insert("command".into(), Value::String(content.to_string()));
        }
        "read_file" | "write_file" => {
            args.insert("path".into(), Value::String(content.to_string()));
        }
        "search_replace" | "search" => {
            args.insert("query".into(), Value::String(content.to_string()));
        }
        _ => {}
    }

    Some(ToolCall {
        id: None,
        name: name.to_string(),
        arguments: Value::Object(args),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_tool_calls_array() {
        let raw = r#"[{"id":"call_1","function":{"name":"run_command","arguments":"{\"command\":\"ls\"}"}}]"#;
        let calls = parse_tool_calls(raw, ToolCallSource::OpenAi);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id.as_deref(), Some("call_1"));
        assert_eq!(calls[0].name, "run_command");
        assert_eq!(calls[0].arguments["command"], "ls");
        assert_eq!(calls[0].source, ToolCallSource::OpenAi);
    }

    #[test]
    fn openai_multiple_tool_calls() {
        let raw = r#"[{"id":"c1","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}},{"id":"c2","function":{"name":"run_command","arguments":"{\"command\":\"pwd\"}"}}]"#;
        let calls = parse_tool_calls(raw, ToolCallSource::OpenRouter);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[1].name, "run_command");
        assert_eq!(calls[1].source, ToolCallSource::OpenRouter);
    }

    #[test]
    fn openai_arguments_as_object() {
        let raw = r#"[{"function":{"name":"search","arguments":{"query":"rust async"}}}]"#;
        let calls = parse_tool_calls(raw, ToolCallSource::Zen);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["query"], "rust async");
    }

    #[test]
    fn gemini_function_call() {
        let raw = r#"{"functionCall":{"name":"read_file","args":{"path":"/tmp/a.txt"}}}"#;
        let calls = parse_tool_calls(raw, ToolCallSource::Gemini);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/tmp/a.txt");
        assert!(calls[0].id.is_none());
    }

    #[test]
    fn mcp_tool_call() {
        let raw = r#"{"mcp_tool":"mcp__read_file","arguments":{"path":"foo.rs"}}"#;
        let calls = parse_tool_calls(raw, ToolCallSource::Mcp);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "mcp__read_file");
        assert_eq!(calls[0].arguments["path"], "foo.rs");
    }

    #[test]
    fn bracket_run_command() {
        let raw = "[RUN_COMMAND git status]";
        let calls = parse_tool_calls(raw, ToolCallSource::BracketLegacy);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_command");
        assert_eq!(calls[0].arguments["command"], "git status");
    }

    #[test]
    fn bracket_read_file() {
        let raw = "[READ_FILE /tmp/hello.txt]";
        let calls = parse_tool_calls(raw, ToolCallSource::BracketLegacy);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/tmp/hello.txt");
    }

    #[test]
    fn bracket_search() {
        let raw = "[SEARCH: rust tool calling]";
        let calls = parse_tool_calls(raw, ToolCallSource::BracketLegacy);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments["query"], "rust tool calling");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse_tool_calls("", ToolCallSource::OpenAi).is_empty());
        assert!(parse_tool_calls("   ", ToolCallSource::OpenAi).is_empty());
    }

    #[test]
    fn plain_text_returns_empty() {
        assert!(
            parse_tool_calls("Hello, I am a helpful assistant.", ToolCallSource::OpenAi).is_empty()
        );
    }

    #[test]
    fn text_with_no_tool_calls() {
        let raw = r#"{"content":"I cannot do that"}"#;
        assert!(parse_tool_calls(raw, ToolCallSource::OpenAi).is_empty());
    }

    #[test]
    fn dedupe_key_is_canonical() {
        let tc1 = ToolCall {
            id: None,
            name: "run_command".into(),
            arguments: json!({"command": "ls", "flag": true}),
            source: ToolCallSource::OpenAi,
        };
        let tc2 = ToolCall {
            id: Some("call_99".into()),
            name: "run_command".into(),
            arguments: json!({"flag": true, "command": "ls"}),
            source: ToolCallSource::Gemini,
        };
        // Same name + same args (different order) should produce same dedupe key
        assert_eq!(tc1.dedupe_key(), tc2.dedupe_key());
    }

    #[test]
    fn dedupe_key_distinct_for_different_names() {
        let tc1 = ToolCall {
            id: None,
            name: "run_command".into(),
            arguments: json!({}),
            source: ToolCallSource::OpenAi,
        };
        let tc2 = ToolCall {
            id: None,
            name: "read_file".into(),
            arguments: json!({}),
            source: ToolCallSource::OpenAi,
        };
        assert_ne!(tc1.dedupe_key(), tc2.dedupe_key());
    }

    #[test]
    fn canonicalize_empty_object() {
        let tc = ToolCall {
            id: None,
            name: "test".into(),
            arguments: json!({}),
            source: ToolCallSource::OpenAi,
        };
        assert_eq!(tc.dedupe_key(), "test:{}");
    }

    #[test]
    fn gemini_single_object_not_array() {
        let raw = r#"{"functionCall":{"name":"search_replace","args":{"path":"a.rs","old":"foo","new":"bar"}}}"#;
        let calls = parse_tool_calls(raw, ToolCallSource::Gemini);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search_replace");
        assert_eq!(calls[0].arguments["old"], "foo");
    }

    #[test]
    fn openai_no_id_is_fine() {
        let raw = r#"[{"function":{"name":"search","arguments":{"query":"test"}}}]"#;
        let calls = parse_tool_calls(raw, ToolCallSource::OpenAi);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].id.is_none());
    }

    #[test]
    fn tool_call_source_from_provider() {
        assert_eq!(
            ToolCallSource::from_provider(&ApiProvider::OpenAI),
            ToolCallSource::OpenAi
        );
        assert_eq!(
            ToolCallSource::from_provider(&ApiProvider::Sambanova),
            ToolCallSource::Sambanova
        );
        assert_eq!(
            ToolCallSource::from_provider(&ApiProvider::OpenRouter),
            ToolCallSource::OpenRouter
        );
        assert_eq!(
            ToolCallSource::from_provider(&ApiProvider::Zen),
            ToolCallSource::Zen
        );
        assert_eq!(
            ToolCallSource::from_provider(&ApiProvider::Gemini),
            ToolCallSource::Gemini
        );
        assert_eq!(
            ToolCallSource::from_provider(&ApiProvider::Ollama),
            ToolCallSource::Ollama
        );
    }

    #[test]
    fn bracket_git_status() {
        let raw = "[GIT_STATUS]";
        let calls = parse_tool_calls(raw, ToolCallSource::BracketLegacy);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "git_status");
        assert!(calls[0].arguments.as_object().unwrap().is_empty());
    }

    #[test]
    fn bracket_write_file_with_path() {
        let raw = "[WRITE_FILE /tmp/out.rs]";
        let calls = parse_tool_calls(raw, ToolCallSource::BracketLegacy);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(calls[0].arguments["path"], "/tmp/out.rs");
    }

    #[test]
    fn canonicalize_nested_object() {
        let tc = ToolCall {
            id: None,
            name: "test".into(),
            arguments: json!({"b": {"z": 1, "a": 2}, "a": "hello"}),
            source: ToolCallSource::OpenAi,
        };
        let key = tc.dedupe_key();
        assert!(key.contains("\"a\":\"hello\""));
        assert!(key.contains("\"b\":"));
        // Keys within nested object should be sorted too
        assert!(key.contains("\"a\":2,\"z\":1"));
    }
}
