// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

//! Azure Data Explorer query tool.

use crate::core::error::{HarperError, HarperResult};
use crate::core::io_traits::UserApproval;
use crate::tools::parsing;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const AZURE_CLI_CLIENT_ID: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";
const DEFAULT_ROW_LIMIT: usize = 100;
const TOKEN_EXPIRY_SKEW_SECONDS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdxQueryRequest {
    pub cluster_url: String,
    pub database: String,
    pub query: String,
    pub tenant_id: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<Value>,
    expires_on: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    message: String,
    interval: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CachedToken {
    access_token: String,
    expires_at: i64,
}

pub async fn query_from_json(
    client: &Client,
    args: &Value,
    approver: Option<Arc<dyn UserApproval>>,
) -> HarperResult<String> {
    let request = AdxQueryRequest::from_json(args)?;
    let Some(request) = request else {
        return Ok(
            "The ADX query tool needs a `query` field. Provide a read-only KQL query, for example: print hello=\"world\""
                .to_string(),
        );
    };
    request.validate()?;

    let is_approved = if let Some(approver) = approver {
        approver
            .approve(
                &format!(
                    "Run Azure Data Explorer query on {}/{}?",
                    request.cluster_url, request.database
                ),
                &request.query,
            )
            .await?
    } else {
        let cluster_url = request.cluster_url.clone();
        let database = request.database.clone();
        let query = request.query.clone();
        tokio::task::spawn_blocking(move || {
            println!(
                "Run Azure Data Explorer query on {}/{}? (y/n): {}",
                cluster_url, database, query
            );
            io::stdout()
                .flush()
                .map_err(|err| HarperError::Io(err.to_string()))?;
            let mut approval = String::new();
            io::stdin()
                .read_line(&mut approval)
                .map_err(|err| HarperError::Io(err.to_string()))?;
            Ok::<bool, HarperError>(approval.trim().eq_ignore_ascii_case("y"))
        })
        .await
        .map_err(|err| HarperError::Command(format!("Task failed: {}", err)))??
    };

    if !is_approved {
        return Ok("Azure Data Explorer query cancelled by user".to_string());
    }

    query(client, &request).await
}

pub fn args_from_bracket_call(response: &str) -> HarperResult<Value> {
    let args = parsing::extract_tool_args(response, "[ADX_QUERY", 3)?;
    Ok(json!({
        "cluster_url": args[0],
        "database": args[1],
        "query": args[2],
    }))
}

async fn query(client: &Client, request: &AdxQueryRequest) -> HarperResult<String> {
    let token = fetch_access_token(client, request).await?;
    let query_url = format!(
        "{}/v1/rest/query",
        request.cluster_url.trim_end_matches('/')
    );
    let response = client
        .post(&query_url)
        .bearer_auth(token)
        .json(&json!({
            "db": request.database,
            "csl": request.query,
        }))
        .send()
        .await
        .map_err(|err| HarperError::Command(format!("ADX query request failed: {}", err)))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| HarperError::Command(format!("Failed to read ADX response: {}", err)))?;

    if !status.is_success() {
        return Err(HarperError::Command(format!(
            "ADX query failed with status {}: {}",
            status,
            body.trim()
        )));
    }

    format_query_response(&body)
}

async fn fetch_access_token(client: &Client, request: &AdxQueryRequest) -> HarperResult<String> {
    if let Some(client_secret) = request.client_secret.as_deref() {
        return fetch_client_credentials_token(client, request, client_secret).await;
    }

    if uses_device_token_cache(request) {
        if let Some(token) = read_cached_token(request)? {
            return Ok(token);
        }
    }

    fetch_device_code_token(client, request).await
}

fn uses_device_token_cache(request: &AdxQueryRequest) -> bool {
    request.client_secret.is_none()
}

async fn fetch_client_credentials_token(
    client: &Client,
    request: &AdxQueryRequest,
    client_secret: &str,
) -> HarperResult<String> {
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        request.tenant_id
    );
    let scope = format!("{}/.default", request.cluster_url.trim_end_matches('/'));
    let client_id = request.client_id.as_deref().ok_or_else(|| {
        HarperError::Config(
            "Missing Azure Data Explorer setting: client_id or HARPER_ADX_CLIENT_ID".to_string(),
        )
    })?;
    let response = client
        .post(token_url)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "client_credentials"),
            ("scope", scope.as_str()),
        ])
        .send()
        .await
        .map_err(|err| HarperError::Command(format!("ADX token request failed: {}", err)))?;

    let status = response.status();
    let body = response.text().await.map_err(|err| {
        HarperError::Command(format!("Failed to read ADX token response: {}", err))
    })?;

    if !status.is_success() {
        return Err(HarperError::Command(format!(
            "ADX token request failed with status {}: {}",
            status,
            body.trim()
        )));
    }

    serde_json::from_str::<TokenResponse>(&body)
        .map(|token| token.access_token)
        .map_err(|err| HarperError::Command(format!("Failed to parse ADX token response: {}", err)))
}

async fn fetch_device_code_token(
    client: &Client,
    request: &AdxQueryRequest,
) -> HarperResult<String> {
    let client_id = request
        .client_id
        .as_deref()
        .unwrap_or(AZURE_CLI_CLIENT_ID)
        .to_string();
    let resource = request.cluster_url.trim_end_matches('/').to_string();
    let device_code_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/devicecode",
        request.tenant_id
    );
    let device_response = client
        .post(device_code_url)
        .form(&[
            ("client_id", client_id.as_str()),
            ("resource", resource.as_str()),
        ])
        .send()
        .await
        .map_err(|err| HarperError::Command(format!("ADX device login request failed: {}", err)))?;

    let status = device_response.status();
    let body = device_response.text().await.map_err(|err| {
        HarperError::Command(format!("Failed to read ADX device login response: {}", err))
    })?;

    if !status.is_success() {
        return Err(HarperError::Command(format!(
            "ADX device login failed with status {}: {}",
            status,
            body.trim()
        )));
    }

    let device_code = serde_json::from_str::<DeviceCodeResponse>(&body).map_err(|err| {
        HarperError::Command(format!(
            "Failed to parse ADX device login response: {}",
            err
        ))
    })?;

    eprintln!("{}", device_code.message);
    poll_device_code_token(client, request, &client_id, &device_code).await
}

async fn poll_device_code_token(
    client: &Client,
    request: &AdxQueryRequest,
    client_id: &str,
    device_code: &DeviceCodeResponse,
) -> HarperResult<String> {
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/token",
        request.tenant_id
    );
    let mut interval = Duration::from_secs(
        device_code
            .interval
            .as_ref()
            .and_then(value_as_u64)
            .unwrap_or(5)
            .max(1),
    );

    loop {
        tokio::time::sleep(interval).await;
        let response = client
            .post(&token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("code", device_code.device_code.as_str()),
            ])
            .send()
            .await
            .map_err(|err| HarperError::Command(format!("ADX token poll failed: {}", err)))?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            HarperError::Command(format!("Failed to read ADX token poll: {}", err))
        })?;

        if status.is_success() {
            let token = serde_json::from_str::<TokenResponse>(&body).map_err(|err| {
                HarperError::Command(format!("Failed to parse ADX token response: {}", err))
            })?;
            let expires_at = token_expiry(&token);
            write_cached_token(
                request,
                &CachedToken {
                    access_token: token.access_token.clone(),
                    expires_at,
                },
            )?;
            return Ok(token.access_token);
        }

        let oauth_error = serde_json::from_str::<OAuthErrorResponse>(&body).ok();
        match oauth_error.as_ref().map(|err| err.error.as_str()) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += Duration::from_secs(5);
                continue;
            }
            Some("authorization_declined") => {
                return Err(HarperError::Command(
                    "ADX device login was cancelled".to_string(),
                ));
            }
            Some("expired_token") => {
                return Err(HarperError::Command("ADX device login expired".to_string()));
            }
            _ => {
                let detail = oauth_error
                    .and_then(|err| err.error_description)
                    .unwrap_or_else(|| body.trim().to_string());
                return Err(HarperError::Command(format!(
                    "ADX token poll failed with status {}: {}",
                    status, detail
                )));
            }
        }
    }
}

fn env_or_arg(args: &Value, key: &str, env_key: &str) -> HarperResult<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            env::var(env_key)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| {
            HarperError::Config(format!(
                "Missing Azure Data Explorer setting: {} or {}",
                key, env_key
            ))
        })
}

fn optional_env_or_arg(args: &Value, key: &str, env_key: &str) -> Option<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            env::var(env_key)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

impl AdxQueryRequest {
    fn from_json(args: &Value) -> HarperResult<Option<Self>> {
        let Some(query) = args
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        else {
            return Ok(None);
        };

        Ok(Self {
            cluster_url: normalize_cluster_url(&env_or_arg(
                args,
                "cluster_url",
                "HARPER_ADX_CLUSTER_URL",
            )?),
            database: env_or_arg(args, "database", "HARPER_ADX_DATABASE")?,
            query,
            tenant_id: env_or_arg(args, "tenant_id", "HARPER_ADX_TENANT_ID")?,
            client_id: optional_env_or_arg(args, "client_id", "HARPER_ADX_CLIENT_ID"),
            client_secret: optional_env_or_arg(args, "client_secret", "HARPER_ADX_CLIENT_SECRET"),
        }
        .into())
    }

    fn validate(&self) -> HarperResult<()> {
        if self.query.trim_start().starts_with('.') {
            return Err(HarperError::Command(
                "ADX management commands are not allowed. Use a read-only KQL query.".to_string(),
            ));
        }
        Ok(())
    }
}

fn token_expiry(token: &TokenResponse) -> i64 {
    if let Some(expires_on) = token.expires_on.as_ref().and_then(value_as_i64) {
        return expires_on;
    }

    let expires_in = token
        .expires_in
        .as_ref()
        .and_then(value_as_i64)
        .unwrap_or(3600);
    Utc::now().timestamp() + expires_in
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
}

fn read_cached_token(request: &AdxQueryRequest) -> HarperResult<Option<String>> {
    let Some(path) = token_cache_path(request) else {
        return Ok(None);
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Ok(None);
    };
    let token = serde_json::from_str::<CachedToken>(&raw)
        .map_err(|err| HarperError::Config(format!("Failed to parse ADX token cache: {}", err)))?;
    if token.expires_at <= Utc::now().timestamp() + TOKEN_EXPIRY_SKEW_SECONDS {
        return Ok(None);
    }
    Ok(Some(token.access_token))
}

fn write_cached_token(request: &AdxQueryRequest, token: &CachedToken) -> HarperResult<()> {
    let Some(path) = token_cache_path(request) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            HarperError::Io(format!(
                "Failed to create ADX token cache directory: {}",
                err
            ))
        })?;
    }
    let raw = serde_json::to_string(token)
        .map_err(|err| HarperError::Config(format!("Failed to encode ADX token cache: {}", err)))?;
    fs::write(path, raw)
        .map_err(|err| HarperError::Io(format!("Failed to write ADX token cache: {}", err)))
}

fn token_cache_path(request: &AdxQueryRequest) -> Option<PathBuf> {
    let mut path = dirs::cache_dir()?;
    path.push("harper");
    path.push("adx");
    path.push(format!(
        "{}-{}-{}.json",
        sanitize_cache_key(&request.tenant_id),
        sanitize_cache_key(&request.cluster_url),
        sanitize_cache_key(&request.database)
    ));
    Some(path)
}

fn sanitize_cache_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn normalize_cluster_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    }
}

fn format_query_response(body: &str) -> HarperResult<String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| HarperError::Command(format!("Failed to parse ADX response: {}", err)))?;
    let tables = value
        .get("Tables")
        .and_then(|tables| tables.as_array())
        .ok_or_else(|| HarperError::Command("ADX response did not include Tables".to_string()))?;

    let Some(table) = tables.first() else {
        return Ok("ADX query returned no tables.".to_string());
    };

    let columns = table
        .get("Columns")
        .and_then(|columns| columns.as_array())
        .map(|columns| {
            columns
                .iter()
                .filter_map(|column| column.get("ColumnName").and_then(|name| name.as_str()))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let rows = table
        .get("Rows")
        .and_then(|rows| rows.as_array())
        .cloned()
        .unwrap_or_default();

    let mut output = format!("ADX query returned {} rows.\n", rows.len());
    if !columns.is_empty() {
        output.push_str(&format!("Columns: {}\n", columns.join(", ")));
    }

    for (index, row) in rows.iter().take(DEFAULT_ROW_LIMIT).enumerate() {
        output.push_str(&format!("Row {}: {}\n", index, format_row(row, &columns)));
    }
    if rows.len() > DEFAULT_ROW_LIMIT {
        output.push_str("... truncated\n");
    }

    Ok(output)
}

fn format_row(row: &Value, columns: &[String]) -> String {
    let Some(values) = row.as_array() else {
        return row.to_string();
    };

    if columns.is_empty() {
        return values
            .iter()
            .map(format_value)
            .collect::<Vec<_>>()
            .join(", ");
    }

    columns
        .iter()
        .zip(values.iter())
        .map(|(column, value)| format!("{}={}", column, format_value(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{AdxQueryRequest, format_query_response, normalize_cluster_url, query_from_json};
    use reqwest::Client;
    use serde_json::json;

    #[test]
    fn rejects_management_commands() {
        let request = AdxQueryRequest {
            cluster_url: "https://example.kusto.windows.net".to_string(),
            database: "Samples".to_string(),
            query: ".show tables".to_string(),
            tenant_id: "tenant".to_string(),
            client_id: Some("client".to_string()),
            client_secret: Some("secret".to_string()),
        };

        let err = request.validate().expect_err("management command rejected");
        assert!(
            err.to_string()
                .contains("management commands are not allowed")
        );
    }

    #[test]
    fn formats_query_response_table() {
        let body = r#"{
            "Tables": [{
                "TableName": "PrimaryResult",
                "Columns": [
                    {"ColumnName": "State", "DataType": "String"},
                    {"ColumnName": "Count", "DataType": "Int64"}
                ],
                "Rows": [["WA", 3], ["CA", 5]]
            }]
        }"#;

        let formatted = format_query_response(body).expect("formatted");
        assert!(formatted.contains("ADX query returned 2 rows."));
        assert!(formatted.contains("Columns: State, Count"));
        assert!(formatted.contains("Row 0: State=WA, Count=3"));
    }

    #[test]
    fn normalizes_cluster_url() {
        assert_eq!(
            normalize_cluster_url("cluster.kusto.windows.net/"),
            "https://cluster.kusto.windows.net"
        );
    }

    #[tokio::test]
    async fn missing_query_returns_tool_guidance() {
        let output = query_from_json(&Client::new(), &json!({}), None)
            .await
            .expect("tool guidance");

        assert!(output.contains("needs a `query` field"));
    }

    #[test]
    fn sanitizes_token_cache_key() {
        assert_eq!(
            super::sanitize_cache_key("https://cluster.kusto.windows.net/db"),
            "https---cluster-kusto-windows-net-db"
        );
    }

    #[test]
    fn service_principal_request_is_not_cache_eligible() {
        let request = AdxQueryRequest {
            cluster_url: "https://example.kusto.windows.net".to_string(),
            database: "Samples".to_string(),
            query: "print hello=\"world\"".to_string(),
            tenant_id: "tenant".to_string(),
            client_id: Some("client".to_string()),
            client_secret: Some("secret".to_string()),
        };

        assert!(!super::uses_device_token_cache(&request));
    }
}
