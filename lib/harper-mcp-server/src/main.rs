// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use axum::{Router, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tracing::info;

const PROTOCOL_VERSION: &str = "2024-11-05";
const RATE_LIMIT_REQUESTS: u64 = 100;
const RATE_LIMIT_WINDOW_SECS: u64 = 60;

struct RateLimiter {
    requests: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, client_ip: &str) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(RATE_LIMIT_WINDOW_SECS);

        // Recover from lock poisoning rather than crashing the server:
        // a poisoned lock still guards valid data for a rate limiter.
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Clean up expired entries
        for v in requests.values_mut() {
            v.retain(|t| now.duration_since(*t) < window);
        }

        // Check rate limit
        let entry = requests.entry(client_ip.to_string()).or_default();

        if entry.len() >= usize::try_from(RATE_LIMIT_REQUESTS).unwrap_or(usize::MAX) {
            return false;
        }

        entry.push(now);
        true
    }
}

// Global rate limiter instance
static RATE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

fn error_response(
    id: Option<Value>,
    code: i32,
    message: &str,
    data: Option<Value>,
) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data,
        }),
    }
}

#[derive(Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    data: Option<Value>,
}

fn handle_initialize(request_id: Value, params: Option<&Value>) -> JsonRpcResponse {
    // Check protocol version
    if let Some(params) = params {
        if let Some(version) = params.get("protocolVersion") {
            if version != PROTOCOL_VERSION {
                return error_response(
                    Some(request_id),
                    -32602,
                    "Unsupported protocol version",
                    None,
                );
            }
        } else {
            return error_response(Some(request_id), -32600, "Invalid Request", None);
        }
    } else {
        return error_response(Some(request_id), -32601, "Method not found", None);
    }
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(request_id),
        result: Some(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {
                    "listChanged": true
                }
            },
            "serverInfo": {
                "name": "harper-mcp-server",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        error: None,
    }
}

fn handle_tools_list(request_id: Value) -> JsonRpcResponse {
    let tools = json!({
        "tools": [
            {
                "name": "echo",
                "description": "Echo the input message",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "The message to echo"
                        }
                    },
                    "required": ["message"]
                }
            },
            {
                "name": "get_time",
                "description": "Get the current UTC time",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        ]
    });
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(request_id),
        result: Some(tools),
        error: None,
    }
}

fn handle_tools_call(request_id: Value, params: Option<&Value>) -> JsonRpcResponse {
    if let Some(params) = params {
        if let Some(name) = params.get("name") {
            if name == "echo" {
                if let Some(args) = params.get("arguments") {
                    if let Some(Value::String(msg)) = args.get("message") {
                        let result = json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": msg
                                }
                            ]
                        });
                        JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            id: Some(request_id),
                            result: Some(result),
                            error: None,
                        }
                    } else {
                        error_response(Some(request_id), -32602, "Invalid params", None)
                    }
                } else {
                    error_response(Some(request_id), -32602, "Invalid params", None)
                }
            } else if name == "get_time" {
                let now = chrono::Utc::now().to_rfc3339();
                let result = json!({
                    "content": [
                        {
                            "type": "text",
                            "text": now
                        }
                    ]
                });
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: Some(request_id),
                    result: Some(result),
                    error: None,
                }
            } else {
                error_response(Some(request_id), -32601, "Method not found", None)
            }
        } else {
            error_response(Some(request_id), -32602, "Invalid params", None)
        }
    } else {
        error_response(Some(request_id), -32600, "Invalid Request", None)
    }
}

async fn handle_request(
    axum::Json(rpc_req): axum::Json<JsonRpcRequest>,
) -> axum::Json<JsonRpcResponse> {
    let client_ip = "default";
    if !RATE_LIMITER.check(client_ip) {
        return axum::Json(error_response(
            Some(rpc_req.id.clone()),
            -32029,
            "Rate limit exceeded. Try again later.",
            None,
        ));
    }

    info!(
        "MCP request: method={}, id={:?}",
        rpc_req.method, rpc_req.id
    );
    let response = match rpc_req.method.as_str() {
        "initialize" => handle_initialize(rpc_req.id.clone(), rpc_req.params.as_ref()),
        "tools/list" => handle_tools_list(rpc_req.id.clone()),
        "tools/call" => handle_tools_call(rpc_req.id.clone(), rpc_req.params.as_ref()),
        _ => error_response(Some(rpc_req.id), -32601, "Method not found", None),
    };

    axum::Json(response)
}

async fn health() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/", post(handle_request))
        .route("/health", axum::routing::get(health));
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 5001));
    println!("MCP server listening on http://127.0.0.1:5001");
    println!("Rate limit: {RATE_LIMIT_REQUESTS} requests per {RATE_LIMIT_WINDOW_SECS} seconds");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_rate_limiter_new() {
        let limiter = RateLimiter::new();
        // Should allow requests initially
        assert!(limiter.check("127.0.0.1"));
    }

    #[test]
    fn test_rate_limiter_under_limit() {
        let limiter = RateLimiter::new();
        let ip = "127.0.0.1";

        // Should allow up to RATE_LIMIT_REQUESTS
        for _ in 0..RATE_LIMIT_REQUESTS {
            assert!(limiter.check(ip), "Should allow request under limit");
        }
    }

    #[test]
    fn test_rate_limiter_over_limit() {
        let limiter = RateLimiter::new();
        let ip = "127.0.0.1";

        // Fill up the limit
        for _ in 0..RATE_LIMIT_REQUESTS {
            assert!(limiter.check(ip));
        }

        // This should be blocked
        assert!(!limiter.check(ip), "Should block request over limit");
    }

    #[test]
    fn test_rate_limiter_cleanup() {
        let limiter = RateLimiter::new();
        let ip = "127.0.0.1";

        // Fill up the limit
        for _ in 0..RATE_LIMIT_REQUESTS {
            assert!(limiter.check(ip));
        }
        assert!(!limiter.check(ip));

        // Wait for the window to expire (simulate time passing)
        thread::sleep(Duration::from_secs(RATE_LIMIT_WINDOW_SECS + 1));

        // Should allow again after cleanup
        assert!(limiter.check(ip), "Should allow after window expires");
    }

    #[test]
    fn test_rate_limiter_different_ips() {
        let limiter = RateLimiter::new();

        // Fill up for one IP
        for _ in 0..RATE_LIMIT_REQUESTS {
            assert!(limiter.check("127.0.0.1"));
        }
        assert!(!limiter.check("127.0.0.1"));

        // Different IP should still work
        assert!(limiter.check("127.0.0.2"));
    }

    #[test]
    fn test_error_response() {
        let response = error_response(Some(json!(1)), -32600, "Test error", None);
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, Some(json!(1)));
        assert!(response.result.is_none());
        assert!(response.error.is_some());

        let error = response.error.unwrap();
        assert_eq!(error.code, -32600);
        assert_eq!(error.message, "Test error");
        assert!(error.data.is_none());
    }

    #[test]
    fn test_handle_initialize_valid() {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION
        });
        let response = handle_initialize(json!(1), Some(&params));

        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, Some(json!(1)));
        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert!(result.get("capabilities").is_some());
        assert!(result.get("serverInfo").is_some());
    }

    #[test]
    fn test_handle_initialize_invalid_protocol() {
        let params = json!({
            "protocolVersion": "invalid"
        });
        let response = handle_initialize(json!(1), Some(&params));

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_initialize_no_params() {
        let response = handle_initialize(json!(1), None);

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn test_handle_tools_list() {
        let response = handle_tools_list(json!(1));

        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, Some(json!(1)));
        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "echo");
        assert_eq!(tools[1]["name"], "get_time");
    }

    #[test]
    fn test_handle_tools_call_echo() {
        let params = json!({
            "name": "echo",
            "arguments": {
                "message": "Hello, World!"
            }
        });
        let response = handle_tools_call(json!(1), Some(&params));

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        let content = &result["content"][0];
        assert_eq!(content["type"], "text");
        assert_eq!(content["text"], "Hello, World!");
    }

    #[test]
    fn test_handle_tools_call_get_time() {
        let params = json!({
            "name": "get_time",
            "arguments": {}
        });
        let response = handle_tools_call(json!(1), Some(&params));

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        let content = &result["content"][0];
        assert_eq!(content["type"], "text");
        // Should be a valid RFC3339 timestamp
        assert!(content["text"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn test_handle_tools_call_invalid_tool() {
        let params = json!({
            "name": "invalid_tool",
            "arguments": {}
        });
        let response = handle_tools_call(json!(1), Some(&params));

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn test_handle_tools_call_missing_args() {
        let params = json!({
            "name": "echo"
        });
        let response = handle_tools_call(json!(1), Some(&params));

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }
}
