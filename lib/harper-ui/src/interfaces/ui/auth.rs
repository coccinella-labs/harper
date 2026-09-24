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

use harper_core::{AuthSession, SessionStateView};
use keyring::{Entry, Error as KeyringError};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "harper";
const TUI_AUTH_SESSION_ACCOUNT: &str = "tui-auth-session";

#[derive(Debug, Clone)]
pub enum TuiAuthCommand {
    Login { provider: String },
    Logout,
    Status,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuiAuthStartResponse {
    pub flow_id: String,
    pub login_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuiAuthPollResponse {
    pub status: String,
    pub session: Option<AuthSession>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteSessionListItem {
    pub id: String,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TuiRefreshRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAuthSession {
    session: AuthSession,
}

pub fn parse_tui_auth_command(input: &str) -> Option<TuiAuthCommand> {
    let trimmed = input.trim();
    let parts: Vec<_> = trimmed.split_whitespace().collect();
    match parts.as_slice() {
        ["/auth", "login", provider] => Some(TuiAuthCommand::Login {
            provider: provider.to_string(),
        }),
        ["/auth", "logout"] => Some(TuiAuthCommand::Logout),
        ["/auth", "status"] => Some(TuiAuthCommand::Status),
        _ => None,
    }
}

pub async fn start_tui_auth_flow(
    client: &reqwest::Client,
    server_base_url: &str,
    provider: &str,
) -> Result<TuiAuthStartResponse, String> {
    let url = format!(
        "{}/auth/tui/start/{}",
        server_base_url.trim_end_matches('/'),
        provider
    );
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(extract_http_error(response).await);
    }
    response.json().await.map_err(|err| err.to_string())
}

pub async fn poll_tui_auth_flow(
    client: &reqwest::Client,
    server_base_url: &str,
    flow_id: &str,
) -> Result<TuiAuthPollResponse, String> {
    let url = format!(
        "{}/auth/tui/flow/{}",
        server_base_url.trim_end_matches('/'),
        flow_id
    );
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(extract_http_error(response).await);
    }
    response.json().await.map_err(|err| err.to_string())
}

pub async fn fetch_remote_sessions(
    client: &reqwest::Client,
    server_base_url: &str,
    session: &mut AuthSession,
) -> Result<Vec<RemoteSessionListItem>, String> {
    let url = format!("{}/api/sessions", server_base_url.trim_end_matches('/'));
    fetch_remote_json_with_refresh(client, &url, session).await
}

pub async fn fetch_remote_session_state(
    client: &reqwest::Client,
    server_base_url: &str,
    session: &mut AuthSession,
    session_id: &str,
) -> Result<SessionStateView, String> {
    let url = format!(
        "{}/api/sessions/{}",
        server_base_url.trim_end_matches('/'),
        session_id
    );
    fetch_remote_json_with_refresh(client, &url, session).await
}

pub async fn delete_remote_session(
    client: &reqwest::Client,
    server_base_url: &str,
    session: &mut AuthSession,
    session_id: &str,
) -> Result<(), String> {
    let url = format!(
        "{}/api/sessions/{}",
        server_base_url.trim_end_matches('/'),
        session_id
    );
    send_remote_delete_with_refresh(client, &url, session).await
}

pub async fn refresh_auth_session(
    client: &reqwest::Client,
    server_base_url: &str,
    session: &mut AuthSession,
) -> Result<(), String> {
    let refresh_token = session
        .refresh_token
        .clone()
        .ok_or_else(|| "No refresh token available".to_string())?;
    let refreshed = refresh_tui_auth_session(client, server_base_url, &refresh_token).await?;
    *session = refreshed;
    save_auth_session(session)
}

pub fn load_auth_session() -> Option<AuthSession> {
    if let Some(session) = load_auth_session_from_keyring() {
        return Some(session);
    }

    let session = load_auth_session_from_file()?;
    if save_auth_session_to_keyring(&session).is_ok() {
        let _ = delete_legacy_auth_session_file();
    }
    Some(session)
}

pub fn save_auth_session(session: &AuthSession) -> Result<(), String> {
    match save_auth_session_to_keyring(session) {
        Ok(()) => {
            let _ = delete_legacy_auth_session_file();
            Ok(())
        }
        Err(keyring_error) => {
            save_auth_session_to_file(session)?;
            eprintln!(
                "Warning: falling back to file-backed TUI auth session storage: {}",
                keyring_error
            );
            Ok(())
        }
    }
}

pub fn clear_auth_session() -> Result<(), String> {
    if let Ok(entry) = auth_session_entry() {
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(err) => return Err(err.to_string()),
        }
    }
    delete_legacy_auth_session_file()
}

pub fn launch_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(url);
        cmd
    };

    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        cmd
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err("Browser launch is not supported on this platform".to_string());

    cmd.spawn().map_err(|err| err.to_string())?;
    Ok(())
}

fn auth_session_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".harper").join("auth").join("tui-session.json"))
}

fn auth_session_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, TUI_AUTH_SESSION_ACCOUNT).map_err(|err| err.to_string())
}

fn load_auth_session_from_keyring() -> Option<AuthSession> {
    let entry = auth_session_entry().ok()?;
    let content = entry.get_password().ok()?;
    deserialize_auth_session(&content).ok()
}

fn load_auth_session_from_file() -> Option<AuthSession> {
    let path = auth_session_path()?;
    let content = fs::read_to_string(path).ok()?;
    deserialize_auth_session(&content).ok()
}

fn save_auth_session_to_keyring(session: &AuthSession) -> Result<(), String> {
    let entry = auth_session_entry()?;
    let content = serialize_auth_session(session)?;
    entry.set_password(&content).map_err(|err| err.to_string())
}

fn save_auth_session_to_file(session: &AuthSession) -> Result<(), String> {
    let path = auth_session_path().ok_or_else(|| "Home directory not found".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content = serialize_auth_session(session)?;
    fs::write(path, content).map_err(|err| err.to_string())
}

fn delete_legacy_auth_session_file() -> Result<(), String> {
    let Some(path) = auth_session_path() else {
        return Ok(());
    };
    if path.exists() {
        fs::remove_file(path).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn serialize_auth_session(session: &AuthSession) -> Result<String, String> {
    serde_json::to_string_pretty(&StoredAuthSession {
        session: session.clone(),
    })
    .map_err(|err| err.to_string())
}

fn deserialize_auth_session(content: &str) -> Result<AuthSession, String> {
    serde_json::from_str::<StoredAuthSession>(content)
        .map(|stored| stored.session)
        .map_err(|err| err.to_string())
}

async fn extract_http_error(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    format_http_error(status, &body)
}

async fn refresh_tui_auth_session(
    client: &reqwest::Client,
    server_base_url: &str,
    refresh_token: &str,
) -> Result<AuthSession, String> {
    let url = format!("{}/auth/tui/refresh", server_base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .json(&TuiRefreshRequest { refresh_token })
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(extract_http_error(response).await);
    }
    response.json().await.map_err(|err| err.to_string())
}

async fn fetch_remote_json_with_refresh<T>(
    client: &reqwest::Client,
    url: &str,
    session: &mut AuthSession,
) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let response = client
        .get(url)
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if response.status().is_success() {
        return response.json().await.map_err(|err| err.to_string());
    }

    if response.status() == StatusCode::UNAUTHORIZED {
        let refresh_token = session
            .refresh_token
            .clone()
            .ok_or_else(|| extract_http_error_blocking(StatusCode::UNAUTHORIZED, ""))?;
        let refreshed =
            refresh_tui_auth_session(client, infer_base_url(url), &refresh_token).await?;
        *session = refreshed.clone();
        save_auth_session(session)?;

        let retry = client
            .get(url)
            .bearer_auth(&session.access_token)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        if !retry.status().is_success() {
            return Err(extract_http_error(retry).await);
        }
        return retry.json().await.map_err(|err| err.to_string());
    }

    Err(extract_http_error(response).await)
}

async fn send_remote_delete_with_refresh(
    client: &reqwest::Client,
    url: &str,
    session: &mut AuthSession,
) -> Result<(), String> {
    let response = client
        .delete(url)
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if response.status().is_success() {
        return Ok(());
    }

    if response.status() == StatusCode::UNAUTHORIZED {
        let refresh_token = session
            .refresh_token
            .clone()
            .ok_or_else(|| extract_http_error_blocking(StatusCode::UNAUTHORIZED, ""))?;
        let refreshed =
            refresh_tui_auth_session(client, infer_base_url(url), &refresh_token).await?;
        *session = refreshed;
        save_auth_session(session)?;

        let retry = client
            .delete(url)
            .bearer_auth(&session.access_token)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        if retry.status().is_success() {
            return Ok(());
        }
        return Err(extract_http_error(retry).await);
    }

    Err(extract_http_error(response).await)
}

fn infer_base_url(url: &str) -> &str {
    url.rsplit_once("/api/")
        .map(|(base, _)| base)
        .unwrap_or(url)
}

fn extract_http_error_blocking(status: StatusCode, body: &str) -> String {
    format_http_error(status, body)
}

fn format_http_error(status: StatusCode, body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return format!(
            "HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        );
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(message) = extract_json_error_message(&value) {
            return format!(
                "HTTP {} {}: {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown"),
                message
            );
        }
    }

    format!(
        "HTTP {} {}: {}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("Unknown"),
        trimmed
    )
}

fn extract_json_error_message(value: &Value) -> Option<String> {
    match value {
        Value::String(message) => Some(message.clone()),
        Value::Object(map) => {
            for key in ["error", "message", "detail"] {
                if let Some(raw) = map.get(key) {
                    match raw {
                        Value::String(message) if !message.trim().is_empty() => {
                            return Some(message.clone());
                        }
                        Value::Array(items) => {
                            let joined = items
                                .iter()
                                .filter_map(extract_json_error_message)
                                .collect::<Vec<_>>()
                                .join(": ");
                            if !joined.is_empty() {
                                return Some(joined);
                            }
                        }
                        Value::Object(_) => {
                            if let Some(message) = extract_json_error_message(raw) {
                                return Some(message);
                            }
                        }
                        _ => {}
                    }
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StoredAuthSession, TuiAuthCommand, auth_session_path, clear_auth_session,
        format_http_error, infer_base_url, load_auth_session, parse_tui_auth_command,
    };
    use harper_core::{AuthSession, AuthenticatedUser, UserAuthProvider};
    use reqwest::StatusCode;
    use std::sync::Mutex;

    static KEYRING_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Redirect keyring operations to the in-memory sample store so tests
    /// never touch the real OS keychain. Must be called with
    /// `KEYRING_TEST_LOCK` held.
    ///
    /// `keyring::Entry` performs a one-time platform store initialization
    /// that unconditionally overwrites the default store, so the sample
    /// store has to be installed after that init has run.
    fn use_sample_keyring_store() {
        let _ = keyring::Entry::store_status();
        use keyring_core::sample::Store;
        use std::collections::HashMap;
        let store = Store::new_with_configuration(&HashMap::new()).expect("sample keyring store");
        keyring_core::set_default_store(store);
    }

    #[test]
    fn parses_tui_auth_login_command() {
        let command = parse_tui_auth_command("/auth login github").expect("parse command");
        match command {
            TuiAuthCommand::Login { provider } => assert_eq!(provider, "github"),
            _ => panic!("expected login command"),
        }
    }

    #[test]
    fn loads_auth_session_from_legacy_file() {
        let _guard = keyring_test_guard();
        use_sample_keyring_store();

        let original_home = std::env::var_os("HOME");
        let temp_dir =
            std::env::temp_dir().join(format!("harper-keyring-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        unsafe {
            std::env::set_var("HOME", &temp_dir);
        }

        let session = AuthSession {
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            expires_at: Some(12345),
            user: AuthenticatedUser {
                user_id: "user-1".to_string(),
                email: Some("user@example.com".to_string()),
                display_name: Some("Example User".to_string()),
                provider: Some(UserAuthProvider::Github),
            },
        };

        let legacy_path = auth_session_path().expect("legacy path");
        let parent = legacy_path.parent().expect("legacy parent");
        std::fs::create_dir_all(parent).expect("create auth dir");
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&StoredAuthSession {
                session: session.clone(),
            })
            .expect("serialize session"),
        )
        .expect("write legacy session");

        assert!(legacy_path.exists());

        clear_auth_session().expect("clear auth session");
        assert!(!legacy_path.exists());
        assert!(load_auth_session().is_none());

        restore_home(original_home);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn clear_auth_session_removes_legacy_file_fallback() {
        let _guard = keyring_test_guard();
        use_sample_keyring_store();

        let original_home = std::env::var_os("HOME");
        let temp_dir =
            std::env::temp_dir().join(format!("harper-keyring-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        unsafe {
            std::env::set_var("HOME", &temp_dir);
        }

        let session = AuthSession {
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            expires_at: Some(12345),
            user: AuthenticatedUser {
                user_id: "user-1".to_string(),
                email: Some("user@example.com".to_string()),
                display_name: Some("Example User".to_string()),
                provider: Some(UserAuthProvider::Github),
            },
        };

        let legacy_path = auth_session_path().expect("legacy path");
        let parent = legacy_path.parent().expect("legacy parent");
        std::fs::create_dir_all(parent).expect("create auth dir");
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&StoredAuthSession {
                session: session.clone(),
            })
            .expect("serialize session"),
        )
        .expect("write legacy session");

        assert!(legacy_path.exists());

        clear_auth_session().expect("clear auth session");
        assert!(!legacy_path.exists());
        assert!(load_auth_session().is_none());

        restore_home(original_home);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn format_http_error_uses_status_when_body_is_empty() {
        let message = format_http_error(StatusCode::UNAUTHORIZED, "");
        assert_eq!(message, "HTTP 401 Unauthorized");
    }

    #[test]
    fn format_http_error_extracts_json_message() {
        let message = format_http_error(StatusCode::BAD_REQUEST, r#"{"error":"session expired"}"#);
        assert_eq!(message, "HTTP 400 Bad Request: session expired");
    }

    #[test]
    fn infer_base_url_strips_api_path() {
        assert_eq!(
            infer_base_url("http://127.0.0.1:8081/api/sessions/session-1"),
            "http://127.0.0.1:8081"
        );
    }

    fn keyring_test_guard() -> std::sync::MutexGuard<'static, ()> {
        KEYRING_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn restore_home(original_home: Option<std::ffi::OsString>) {
        unsafe {
            if let Some(value) = original_home {
                std::env::set_var("HOME", value);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }
}
