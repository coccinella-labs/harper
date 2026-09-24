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

mod auth;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        Html, IntoResponse, Json, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use base64::Engine;
use futures_util::{StreamExt, stream};
use reqwest::Client;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent::intent::route_intent;
use crate::core::auth::{AuthSession, AuthenticatedUser, UserAuthProvider};
use crate::core::error::{HarperError, HarperResult};
use crate::core::llm_client::call_llm;
use crate::core::plan_events;
use crate::core::{ApiConfig, Message};
use crate::memory::storage::{CommandLogRecord, save_message, save_session, save_session_for_user};
use crate::runtime::config::ExecPolicyConfig;
use crate::runtime::config::SupabaseAuthConfig;
use rusqlite::params;

#[derive(Clone)]
pub struct ServerState {
    pub conn: Arc<Mutex<Connection>>,
    pub api_config: ApiConfig,
    pub client: Client,
    pub exec_policy: ExecPolicyConfig,
    pub supabase_auth: Option<SupabaseAuthConfig>,
    pub oauth_states: Arc<Mutex<HashMap<String, PendingOauthState>>>,
    pub tui_auth_flows: Arc<Mutex<HashMap<String, TuiAuthFlowState>>>,
}

#[derive(Clone)]
pub struct PendingOauthState {
    pub provider: UserAuthProvider,
    pub code_verifier: String,
    pub created_at: Instant,
    pub tui_flow_id: Option<String>,
}

#[derive(Clone)]
pub struct TuiAuthFlowState {
    pub session: Option<AuthSession>,
    pub created_at: Instant,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Serialize)]
pub struct SessionListItem {
    pub id: String,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub title: Option<String>,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub session_id: Option<String>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub message: String,
    pub session_id: String,
    pub status: String,
    pub pending_id: Option<String>,
    pub pending_tools: Option<Vec<PendingTool>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PendingTool {
    pub id: String,
    pub tool: String,
    pub args: serde_json::Value,
}

#[derive(Serialize)]
pub struct PendingApproval {
    pub id: i64,
    pub command: String,
    pub source: String,
    pub session_id: String,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct ApprovalRequest {
    pub approved: bool,
}

#[derive(Deserialize)]
pub struct AuthCallbackQuery {
    pub code: Option<String>,
    pub harper_flow: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SupabaseTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    user: Option<SupabaseUser>,
}

#[derive(serde::Serialize)]
struct SupabasePkceTokenRequest<'a> {
    auth_code: &'a str,
    code_verifier: &'a str,
    redirect_to: &'a str,
}

#[derive(serde::Serialize)]
struct SupabaseRefreshTokenRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Debug, Deserialize)]
struct SupabaseUser {
    id: String,
    email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthMeResponse {
    pub authenticated: bool,
    pub user: Option<AuthenticatedUser>,
}

#[derive(Debug, Serialize)]
pub struct TuiAuthStartResponse {
    pub flow_id: String,
    pub login_url: String,
}

#[derive(Debug, Serialize)]
pub struct TuiAuthPollResponse {
    pub status: String,
    pub session: Option<AuthSession>,
}

#[derive(Debug, Deserialize)]
pub struct TuiRefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
pub struct ReviewRequest {
    pub file_path: String,
    pub content: String,
    pub language: Option<String>,
    pub workspace_root: Option<String>,
    pub instructions: Option<String>,
    pub selection: Option<ReviewRange>,
    pub max_findings: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ReviewRange {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewResponse {
    pub summary: String,
    pub findings: Vec<CodeReviewFinding>,
    pub model: String,
}

#[derive(Debug, Deserialize)]
struct ModelReviewResponse {
    pub summary: String,
    pub findings: Vec<CodeReviewFinding>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeReviewFinding {
    pub title: String,
    pub severity: String,
    pub message: String,
    pub range: ReviewRange,
    pub suggestion: Option<CodeSuggestion>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSuggestion {
    pub description: String,
    pub replacement: String,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: crate::core::constants::VERSION.to_string(),
    })
}

pub async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Vec<SessionListItem>>, StatusCode> {
    let auth_user = optional_authenticated_user_from_headers(&state, &headers)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let conn = state
        .conn
        .lock()
        .expect("Failed to lock database connection");
    let session_service = crate::memory::session_service::SessionService::new(&conn);
    let sessions = match auth_user {
        Some(user) => session_service
            .list_sessions_data_for_user(&user.user_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        None => session_service
            .list_sessions_data()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };

    Ok(Json(
        sessions
            .into_iter()
            .map(|session| SessionListItem {
                id: session.id,
                user_id: session.user_id,
                created_at: session.created_at,
                updated_at: session.updated_at,
                title: session.title,
            })
            .collect(),
    ))
}

pub async fn auth_login(
    State(state): State<Arc<ServerState>>,
    Path(provider): Path<String>,
) -> Result<Redirect, (StatusCode, String)> {
    let authorize_url = build_authorize_url(&state, &provider, None)?;
    Ok(Redirect::to(&authorize_url))
}

pub async fn auth_tui_start(
    State(state): State<Arc<ServerState>>,
    Path(provider): Path<String>,
) -> Result<Json<TuiAuthStartResponse>, (StatusCode, String)> {
    let flow_id = random_urlsafe(24)?;
    let login_url = build_authorize_url(&state, &provider, Some(flow_id.clone()))?;
    {
        let mut flows = state.tui_auth_flows.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "TUI auth flow lock failed".to_string(),
            )
        })?;
        flows.retain(|_, flow| flow.created_at.elapsed() < Duration::from_secs(600));
        flows.insert(
            flow_id.clone(),
            TuiAuthFlowState {
                session: None,
                created_at: Instant::now(),
            },
        );
    }

    Ok(Json(TuiAuthStartResponse { flow_id, login_url }))
}

pub async fn auth_tui_poll(
    State(state): State<Arc<ServerState>>,
    Path(flow_id): Path<String>,
) -> Result<Json<TuiAuthPollResponse>, (StatusCode, String)> {
    let mut flows = state.tui_auth_flows.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "TUI auth flow lock failed".to_string(),
        )
    })?;
    let Some(flow) = flows.get(&flow_id).cloned() else {
        return Err((StatusCode::NOT_FOUND, "Unknown TUI auth flow".to_string()));
    };
    if let Some(session) = flow.session {
        flows.remove(&flow_id);
        return Ok(Json(TuiAuthPollResponse {
            status: "complete".to_string(),
            session: Some(session),
        }));
    }

    Ok(Json(TuiAuthPollResponse {
        status: "pending".to_string(),
        session: None,
    }))
}

pub async fn auth_tui_refresh(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<TuiRefreshRequest>,
) -> Result<Json<AuthSession>, (StatusCode, String)> {
    let session = refresh_supabase_session(&state, request.refresh_token).await?;
    Ok(Json(session))
}

fn build_authorize_url(
    state: &ServerState,
    provider: &str,
    tui_flow_id: Option<String>,
) -> Result<String, (StatusCode, String)> {
    let supabase = state.supabase_auth.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            "Supabase auth is not configured".to_string(),
        )
    })?;

    let provider = parse_user_provider(provider)?;
    ensure_provider_allowed(supabase, provider)?;

    let project_url = supabase.project_url.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Supabase project URL is not configured".to_string(),
        )
    })?;
    let redirect_url = supabase.redirect_url.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Supabase redirect URL is not configured".to_string(),
        )
    })?;

    let flow_token = random_urlsafe(24)?;
    let code_verifier = random_urlsafe(48)?;
    let code_challenge = pkce_code_challenge(&code_verifier);
    let callback_url = append_callback_query_param(redirect_url, "harper_flow", &flow_token);

    {
        let mut oauth_states = state.oauth_states.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "OAuth state lock failed".to_string(),
            )
        })?;
        oauth_states.retain(|_, pending| pending.created_at.elapsed() < Duration::from_secs(600));
        oauth_states.insert(
            flow_token.clone(),
            PendingOauthState {
                provider,
                code_verifier,
                created_at: Instant::now(),
                tui_flow_id,
            },
        );
    }

    Ok(format!(
        "{}/auth/v1/authorize?provider={}&redirect_to={}&code_challenge={}&code_challenge_method=S256",
        project_url.trim_end_matches('/'),
        provider.as_str(),
        urlencoding::encode(&callback_url),
        urlencoding::encode(&code_challenge),
    ))
}

pub async fn auth_callback(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AuthCallbackQuery>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(error) = query.error {
        let message = query
            .error_description
            .unwrap_or_else(|| "Authentication failed".to_string());
        return Ok(Html(render_auth_message(
            "Sign-in failed",
            &format!("{}: {}", error, message),
        ))
        .into_response());
    }

    let code = query.code.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing auth code in callback".to_string(),
        )
    })?;
    let flow_token = query.harper_flow.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing Harper flow token in callback".to_string(),
        )
    })?;

    let supabase = state.supabase_auth.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            "Supabase auth is not configured".to_string(),
        )
    })?;
    let project_url = supabase.project_url.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Supabase project URL is not configured".to_string(),
        )
    })?;
    let redirect_url = supabase.redirect_url.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Supabase redirect URL is not configured".to_string(),
        )
    })?;

    let pending = {
        let mut oauth_states = state.oauth_states.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "OAuth state lock failed".to_string(),
            )
        })?;
        oauth_states.remove(&flow_token)
    }
    .ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Unknown or expired Harper flow token".to_string(),
        )
    })?;

    let token_url = format!(
        "{}/auth/v1/token?grant_type=pkce",
        project_url.trim_end_matches('/')
    );

    let token_response = state
        .client
        .post(&token_url)
        .header("apikey", supabase.anon_key.clone().unwrap_or_default())
        .json(&SupabasePkceTokenRequest {
            auth_code: code.as_str(),
            code_verifier: pending.code_verifier.as_str(),
            redirect_to: redirect_url,
        })
        .send()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Supabase token exchange failed: {}", err),
            )
        })?;

    if !token_response.status().is_success() {
        let body = token_response.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Supabase token exchange rejected callback: {}", body),
        ));
    }

    let token_payload: SupabaseTokenResponse = token_response.json().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Supabase token response was invalid: {}", err),
        )
    })?;

    let auth_session =
        auth_session_from_supabase_tokens(&state, token_payload, Some(pending.provider)).await;
    if let Some(flow_id) = pending.tui_flow_id.clone() {
        let mut flows = state.tui_auth_flows.lock().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "TUI auth flow lock failed".to_string(),
            )
        })?;
        if let Some(flow) = flows.get_mut(&flow_id) {
            flow.session = Some(auth_session.clone());
        }
    }

    let body = render_auth_success(&auth_session);
    let mut response = Html(body).into_response();
    append_auth_session_cookies(response.headers_mut(), &auth_session);

    Ok(response)
}

pub async fn auth_me(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(supabase) = state.supabase_auth.as_ref() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(AuthMeResponse {
                authenticated: false,
                user: None,
            }),
        )
            .into_response();
    };

    match auth::authenticate_request_with_client(&headers, Some(supabase), &state.client).await {
        Ok(user) => Json(AuthMeResponse {
            authenticated: true,
            user: Some(user),
        })
        .into_response(),
        Err(auth::AuthError::ExpiredToken) => {
            let Some(refresh_token) = auth::refresh_token(&headers) else {
                let mut response = (
                    StatusCode::UNAUTHORIZED,
                    Json(AuthMeResponse {
                        authenticated: false,
                        user: None,
                    }),
                )
                    .into_response();
                clear_auth_cookies(response.headers_mut());
                return response;
            };

            match refresh_supabase_session(&state, refresh_token).await {
                Ok(session) => {
                    let mut response = Json(AuthMeResponse {
                        authenticated: true,
                        user: Some(session.user.clone()),
                    })
                    .into_response();
                    append_auth_session_cookies(response.headers_mut(), &session);
                    response
                }
                Err(_) => {
                    let mut response = (
                        StatusCode::UNAUTHORIZED,
                        Json(AuthMeResponse {
                            authenticated: false,
                            user: None,
                        }),
                    )
                        .into_response();
                    clear_auth_cookies(response.headers_mut());
                    response
                }
            }
        }
        Err(auth::AuthError::MissingToken) => (
            StatusCode::UNAUTHORIZED,
            Json(AuthMeResponse {
                authenticated: false,
                user: None,
            }),
        )
            .into_response(),
        Err(error) => {
            let mut response = (
                StatusCode::UNAUTHORIZED,
                Json(AuthMeResponse {
                    authenticated: false,
                    user: None,
                }),
            )
                .into_response();
            if auth::cookie_value(&headers, auth::ACCESS_TOKEN_COOKIE).is_some()
                || auth::refresh_token(&headers).is_some()
            {
                clear_auth_cookies(response.headers_mut());
            }
            let _ = error;
            response
        }
    }
}

pub async fn auth_status_page() -> Html<String> {
    Html(render_auth_status_page())
}

pub async fn auth_logout() -> Response {
    let mut response = Html(render_auth_message(
        "Signed out",
        "Harper auth cookies were cleared.",
    ))
    .into_response();
    clear_auth_cookies(response.headers_mut());
    response
}

async fn authenticated_user_from_headers(
    state: &ServerState,
    headers: &axum::http::HeaderMap,
) -> Result<crate::core::auth::AuthenticatedUser, (StatusCode, String)> {
    auth::authenticate_request_with_client(headers, state.supabase_auth.as_ref(), &state.client)
        .await
        .map_err(auth::AuthError::into_http_error)
}

async fn optional_authenticated_user_from_headers(
    state: &ServerState,
    headers: &axum::http::HeaderMap,
) -> Result<Option<AuthenticatedUser>, (StatusCode, String)> {
    if state.supabase_auth.is_some() {
        authenticated_user_from_headers(state, headers)
            .await
            .map(Some)
    } else {
        Ok(None)
    }
}

fn claim_or_verify_session_access(
    conn: &Connection,
    session_id: &str,
    user: Option<&AuthenticatedUser>,
) -> Result<(), StatusCode> {
    match user {
        Some(user) => match save_session_for_user(conn, session_id, &user.user_id) {
            Ok(true) => Ok(()),
            Ok(false) => Err(StatusCode::FORBIDDEN),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        },
        None => save_session(conn, session_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn get_session(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let user = optional_authenticated_user_from_headers(&state, &headers)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let conn = state
        .conn
        .lock()
        .expect("Failed to lock database connection");
    let session_service = crate::memory::session_service::SessionService::new(&conn);
    let session_view = match user.as_ref() {
        Some(user) => session_service
            .load_session_state_view_for_user(&session_id, &user.user_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?,
        None => session_service
            .load_session_state_view(&session_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };
    Ok(Json(serde_json::json!(session_view)))
}

pub async fn get_session_plan(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let user = optional_authenticated_user_from_headers(&state, &headers)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let plan = load_session_plan_value(&state, &session_id, user.as_ref())?;
    Ok(Json(plan))
}

pub async fn get_session_plan_stream(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let user = optional_authenticated_user_from_headers(&state, &headers)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let initial_payload = load_session_plan_value(&state, &session_id, user.as_ref())?;
    let initial_json =
        serde_json::to_string(&initial_payload).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let last_event_id = load_latest_plan_event_id(&state, &session_id)?;
    let has_push_listener = database_key_from_state(&state)
        .map(|db_key| plan_events::ensure_cross_process_listener(&db_key))
        .unwrap_or(false);

    let receiver = plan_events::subscribe();
    let stream = stream::unfold(
        (
            state.clone(),
            receiver,
            session_id,
            user,
            last_event_id,
            has_push_listener,
        ),
        |(state, mut receiver, session_id, user, mut last_event_id, has_push_listener)| async move {
            loop {
                let recv_result = if has_push_listener {
                    receiver.recv().await
                } else {
                    let sleep = tokio::time::sleep(Duration::from_millis(500));
                    tokio::pin!(sleep);
                    tokio::select! {
                        recv = receiver.recv() => recv,
                        _ = &mut sleep => Err(tokio::sync::broadcast::error::RecvError::Lagged(0)),
                    }
                };

                match recv_result {
                    Ok(update) if update.session_id == session_id => {
                        last_event_id = Some(update.event_id);
                        let plan = match update.plan {
                            Some(plan) => serde_json::json!(Some(plan)),
                            None => {
                                match load_session_plan_value(&state, &session_id, user.as_ref()) {
                                    Ok(plan) => plan,
                                    Err(_) => return None,
                                }
                            }
                        };
                        let next_json = match serde_json::to_string(&plan) {
                            Ok(json) => json,
                            Err(_) => return None,
                        };
                        let event = Event::default().event("plan").data(next_json);
                        return Some((
                            Ok(event),
                            (
                                state,
                                receiver,
                                session_id,
                                user,
                                last_event_id,
                                has_push_listener,
                            ),
                        ));
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let latest_event_id = match load_latest_plan_event_id(&state, &session_id) {
                            Ok(event_id) => event_id,
                            Err(_) => return None,
                        };
                        if latest_event_id.is_some() && latest_event_id > last_event_id {
                            let plan =
                                match load_session_plan_value(&state, &session_id, user.as_ref()) {
                                    Ok(plan) => plan,
                                    Err(_) => return None,
                                };
                            let next_json = match serde_json::to_string(&plan) {
                                Ok(json) => json,
                                Err(_) => return None,
                            };
                            last_event_id = latest_event_id;
                            let event = Event::default().event("plan").data(next_json);
                            return Some((
                                Ok(event),
                                (
                                    state,
                                    receiver,
                                    session_id,
                                    user,
                                    last_event_id,
                                    has_push_listener,
                                ),
                            ));
                        }
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    let initial_event = stream::once(async move {
        Ok::<Event, Infallible>(Event::default().event("plan").data(initial_json))
    })
    .chain(stream);

    Ok(Sse::new(initial_event).keep_alive(KeepAlive::default()))
}

fn load_latest_plan_event_id(
    state: &ServerState,
    session_id: &str,
) -> Result<Option<i64>, StatusCode> {
    let conn = state
        .conn
        .lock()
        .expect("Failed to lock database connection");
    crate::memory::storage::load_latest_plan_event_id(&conn, session_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn database_key_from_state(state: &ServerState) -> Option<String> {
    let conn = state
        .conn
        .lock()
        .expect("Failed to lock database connection");
    let mut stmt = conn.prepare("PRAGMA database_list").ok()?;
    let mut rows = stmt.query([]).ok()?;
    while let Ok(Some(row)) = rows.next() {
        let name: String = row.get(1).ok()?;
        let path: String = row.get(2).ok()?;
        if name == "main" && !path.trim().is_empty() {
            return Some(path);
        }
    }
    None
}

pub async fn delete_session(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Path(session_id): Path<String>,
) -> StatusCode {
    let user = match optional_authenticated_user_from_headers(&state, &headers).await {
        Ok(user) => user,
        Err(_) => return StatusCode::UNAUTHORIZED,
    };
    let conn = state
        .conn
        .lock()
        .expect("Failed to lock database connection");
    let session_service = crate::memory::session_service::SessionService::new(&conn);

    match user.as_ref() {
        Some(user) => match session_service.delete_session_for_user(&session_id, &user.user_id) {
            Ok(true) => StatusCode::NO_CONTENT,
            Ok(false) => StatusCode::NOT_FOUND,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        },
        None => match session_service.delete_session(&session_id) {
            Ok(true) => StatusCode::NO_CONTENT,
            Ok(false) => StatusCode::NOT_FOUND,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        },
    }
}

fn load_session_plan_value(
    state: &ServerState,
    session_id: &str,
    user: Option<&AuthenticatedUser>,
) -> Result<serde_json::Value, StatusCode> {
    let conn = state
        .conn
        .lock()
        .expect("Failed to lock database connection");
    let session_service = crate::memory::session_service::SessionService::new(&conn);
    let session_view = match user {
        Some(user) => session_service
            .load_session_state_view_for_user(session_id, &user.user_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?,
        None => session_service
            .load_session_state_view(session_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };
    Ok(serde_json::json!(session_view.plan))
}

pub async fn chat_endpoint(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    let message = payload.message.clone();
    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(|| "api".to_string());
    let auth_user = optional_authenticated_user_from_headers(&state, &headers).await?;

    let system_prompt = r#"You are Harper, a CLI assistant. Use JSON for commands:
{"tool": "run_command", "args": {"command": "ls -la"}}
{"tool": "read_file", "args": {"filePath": "/path/to/file"}}
{"tool": "write_file", "args": {"filePath": "/path", "content": "text"}}
{"tool": "grep", "args": {"pattern": "search", "path": "."}}
{"tool": "update_plan", "args": {"explanation": "optional note", "items": [{"step": "Inspect files", "status": "in_progress"}]}}"#;

    let history = vec![
        Message {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        Message {
            role: "user".to_string(),
            content: message.clone(),
        },
    ];

    let response = call_llm(&state.client, &state.api_config, &history)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(crate::agent::intent::DeterministicIntent::ListChangedFiles(intent_args)) =
        route_intent(&message)
    {
        let mut git_cmd = std::process::Command::new("git");
        git_cmd.arg("diff");

        if let Some(since) = intent_args.since {
            git_cmd.arg(format!("--since={}", since));
        }
        git_cmd.arg("--name-only");

        if intent_args.ext.is_some() {
            git_cmd.arg("--");
            git_cmd.arg("*");
        }

        let output = git_cmd
            .output()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let files = String::from_utf8_lossy(&output.stdout);

        let result = if files.trim().is_empty() {
            "No changed files found".to_string()
        } else {
            format!("Changed files:\n{}", files)
        };

        let conn = state.conn.lock().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Lock error: {:?}", e),
            )
        })?;
        claim_or_verify_session_access(&conn, &session_id, auth_user.as_ref())
            .map_err(|status| (status, "Session access denied".to_string()))?;
        let _ = save_message(&conn, &session_id, "user", &message);
        let _ = save_message(&conn, &session_id, "assistant", &result);
        drop(conn);

        return Ok(Json(ChatResponse {
            message: result,
            session_id,
            status: "completed".to_string(),
            pending_id: None,
            pending_tools: None,
        }));
    }

    let conn = state.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {:?}", e),
        )
    })?;
    claim_or_verify_session_access(&conn, &session_id, auth_user.as_ref())
        .map_err(|status| (status, "Session access denied".to_string()))?;
    let _ = save_message(&conn, &session_id, "user", &message);
    drop(conn);

    let final_response = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&response) {
        if let Some(tool) = json.get("tool").and_then(|t| t.as_str()) {
            let args = json.get("args").and_then(|a| a.as_object());

            if tool == "run_command" {
                if let Some(cmd) = args.and_then(|a| a.get("command")).and_then(|c| c.as_str()) {
                    let cmd_str = cmd.to_string();
                    let session_clone = session_id.clone();

                    let output = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(&cmd_str)
                        .output()
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                    let stdout = output.stdout.clone();
                    let stderr = output.stderr.clone();
                    let exit_code = output.status.code();

                    let output_str = if exit_code == Some(0) {
                        format!("$ {}\n{}", cmd_str, String::from_utf8_lossy(&stdout))
                    } else {
                        format!(
                            "$ {}\nError (exit {:?}):\n{}",
                            cmd_str,
                            exit_code,
                            String::from_utf8_lossy(&stderr)
                        )
                    };

                    let conn = state.conn.lock().map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Lock error: {:?}", e),
                        )
                    })?;
                    let _ = save_message(&conn, &session_clone, "assistant", &output_str);
                    let record = CommandLogRecord {
                        session_id: Some(session_clone),
                        command: cmd_str,
                        source: "api_chat".to_string(),
                        requires_approval: false,
                        approved: true,
                        status: "completed".to_string(),
                        exit_code,
                        duration_ms: None,
                        stdout_preview: Some(String::from_utf8_lossy(&stdout).to_string()),
                        stderr_preview: Some(String::from_utf8_lossy(&stderr).to_string()),
                        error_message: None,
                    };
                    let _ = crate::memory::storage::insert_command_log(&conn, &record);
                    drop(conn);

                    return Ok(Json(ChatResponse {
                        message: output_str,
                        session_id,
                        status: "completed".to_string(),
                        pending_id: None,
                        pending_tools: None,
                    }));
                }
            }

            if tool == "read_file" {
                if let Some(path) = args
                    .and_then(|a| a.get("filePath"))
                    .and_then(|p| p.as_str())
                {
                    let path_str = path.to_string();
                    let content = std::fs::read_to_string(&path_str).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Read error: {}", e),
                        )
                    })?;

                    let truncated = if content.len() > 50000 {
                        format!(
                            "{}...\n(truncated {} bytes)",
                            &content[..50000],
                            content.len()
                        )
                    } else {
                        content
                    };

                    let conn = state.conn.lock().map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Lock error: {:?}", e),
                        )
                    })?;
                    let _ = save_message(&conn, &session_id, "assistant", &truncated);
                    let record = CommandLogRecord {
                        session_id: Some(session_id.clone()),
                        command: format!("read_file {}", path_str),
                        source: "api_chat".to_string(),
                        requires_approval: false,
                        approved: true,
                        status: "completed".to_string(),
                        exit_code: Some(0),
                        duration_ms: None,
                        stdout_preview: Some(truncated.clone()),
                        stderr_preview: None,
                        error_message: None,
                    };
                    let _ = crate::memory::storage::insert_command_log(&conn, &record);
                    drop(conn);

                    return Ok(Json(ChatResponse {
                        message: truncated,
                        session_id,
                        status: "completed".to_string(),
                        pending_id: None,
                        pending_tools: None,
                    }));
                }
            }

            if tool == "write_file" {
                if let Some(path) = args
                    .and_then(|a| a.get("filePath"))
                    .and_then(|p| p.as_str())
                {
                    if let Some(content) =
                        args.and_then(|a| a.get("content")).and_then(|c| c.as_str())
                    {
                        let path_str = path.to_string();
                        let content_str = content.to_string();

                        std::fs::write(&path_str, &content_str).map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Write error: {}", e),
                            )
                        })?;

                        let output_str =
                            format!("Written {} bytes to {}", content_str.len(), path_str);

                        let conn = state.conn.lock().map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Lock error: {:?}", e),
                            )
                        })?;
                        let _ = save_message(&conn, &session_id, "assistant", &output_str);
                        let record = CommandLogRecord {
                            session_id: Some(session_id.clone()),
                            command: format!("write_file {}", path_str),
                            source: "api_chat".to_string(),
                            requires_approval: false,
                            approved: true,
                            status: "completed".to_string(),
                            exit_code: Some(0),
                            duration_ms: None,
                            stdout_preview: Some(output_str.clone()),
                            stderr_preview: None,
                            error_message: None,
                        };
                        let _ = crate::memory::storage::insert_command_log(&conn, &record);
                        drop(conn);

                        return Ok(Json(ChatResponse {
                            message: output_str,
                            session_id,
                            status: "completed".to_string(),
                            pending_id: None,
                            pending_tools: None,
                        }));
                    }
                }
            }

            if tool == "grep" {
                if let Some(pattern) = args.and_then(|a| a.get("pattern")).and_then(|p| p.as_str())
                {
                    if let Some(path) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                        let pattern_str = pattern.to_string();
                        let path_str = path.to_string();

                        let hits = crate::tools::search::search_files(
                            std::path::Path::new(&path_str),
                            &pattern_str,
                        );
                        let mut stdout = String::new();
                        for (file, lines) in &hits {
                            for (line_no, line) in lines {
                                stdout.push_str(&format!(
                                    "{}:{}:{}\n",
                                    file.display(),
                                    line_no,
                                    line
                                ));
                            }
                        }
                        let exit_code = if hits.is_empty() { Some(1) } else { Some(0) };

                        let output_str = if stdout.is_empty() {
                            format!("No matches found for '{}' in {}", pattern_str, path_str)
                        } else {
                            format!("{}\n(matched {} lines)", stdout, stdout.lines().count())
                        };

                        let conn = state.conn.lock().map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Lock error: {:?}", e),
                            )
                        })?;
                        let _ = save_message(&conn, &session_id, "assistant", &output_str);
                        let record = CommandLogRecord {
                            session_id: Some(session_id.clone()),
                            command: format!("grep {} {}", pattern_str, path_str),
                            source: "api_chat".to_string(),
                            requires_approval: false,
                            approved: true,
                            status: "completed".to_string(),
                            exit_code,
                            duration_ms: None,
                            stdout_preview: Some(stdout.to_string()),
                            stderr_preview: None,
                            error_message: None,
                        };
                        let _ = crate::memory::storage::insert_command_log(&conn, &record);
                        drop(conn);

                        return Ok(Json(ChatResponse {
                            message: output_str,
                            session_id,
                            status: "completed".to_string(),
                            pending_id: None,
                            pending_tools: None,
                        }));
                    }
                }
            }

            if tool == "update_plan" {
                let args_json = json
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let conn = state.conn.lock().map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Lock error: {:?}", e),
                    )
                })?;
                let output_str = crate::tools::plan::update_plan(&conn, &session_id, &args_json)
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                let _ = save_message(&conn, &session_id, "assistant", &output_str);
                drop(conn);

                return Ok(Json(ChatResponse {
                    message: output_str,
                    session_id,
                    status: "completed".to_string(),
                    pending_id: None,
                    pending_tools: None,
                }));
            }
        }
        response.trim().to_string()
    } else {
        response.trim().to_string()
    };

    let conn = state.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {:?}", e),
        )
    })?;
    let _ = save_message(&conn, &session_id, "assistant", &final_response);
    drop(conn);

    Ok(Json(ChatResponse {
        message: final_response,
        session_id,
        status: "completed".to_string(),
        pending_id: None,
        pending_tools: None,
    }))
}

pub async fn review_code(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<ReviewRequest>,
) -> Result<Json<ReviewResponse>, (StatusCode, String)> {
    validate_review_request(&payload).map_err(into_http_error)?;

    let review = generate_review(&state.client, &state.api_config, &payload)
        .await
        .map_err(into_http_error)?;

    Ok(Json(review))
}

fn validate_review_request(request: &ReviewRequest) -> HarperResult<()> {
    if request.file_path.trim().is_empty() {
        return Err(HarperError::Validation(
            "file_path must not be empty".to_string(),
        ));
    }

    if request.content.trim().is_empty() {
        return Err(HarperError::Validation(
            "content must not be empty".to_string(),
        ));
    }

    if let Some(max_findings) = request.max_findings {
        if max_findings == 0 {
            return Err(HarperError::Validation(
                "max_findings must be greater than 0".to_string(),
            ));
        }
    }

    Ok(())
}

async fn generate_review(
    client: &Client,
    api_config: &ApiConfig,
    request: &ReviewRequest,
) -> HarperResult<ReviewResponse> {
    let max_findings = request.max_findings.unwrap_or(8).clamp(1, 20);
    let selection_text = request
        .selection
        .as_ref()
        .map(format_range)
        .unwrap_or_else(|| "entire file".to_string());
    let instructions = request.instructions.as_deref().unwrap_or(
        "Focus on correctness, regressions, missing validation, and concrete fix suggestions.",
    );

    let system_prompt = format!(
        "You are Harper's code review engine. Review code like a senior engineer.
Return JSON only with this exact schema:
{{
  \"summary\": \"short review summary\",
  \"findings\": [
    {{
      \"title\": \"brief title\",
      \"severity\": \"error|warning|info\",
      \"message\": \"clear explanation of the issue and why it matters\",
      \"range\": {{
        \"start_line\": 1,
        \"start_column\": 1,
        \"end_line\": 1,
        \"end_column\": 1
      }},
      \"suggestion\": {{
        \"description\": \"optional fix description\",
        \"replacement\": \"replacement text for the selected range\"
      }}
    }}
  ]
}}
Rules:
- Return only valid JSON, no markdown fences.
- Use 1-based line and column numbers.
- Report at most {} findings.
- Only include findings you can support from the provided code.
- Omit the suggestion field when you do not have a precise replacement.",
        max_findings
    );

    let user_prompt = format!(
        "Review this file for IDE inline diagnostics.
File: {}
Language: {}
Workspace root: {}
Selection: {}
Instructions: {}

Code with line numbers:
{}",
        request.file_path,
        request.language.as_deref().unwrap_or("unknown"),
        request.workspace_root.as_deref().unwrap_or("unknown"),
        selection_text,
        instructions,
        add_line_numbers(&request.content)
    );

    let messages = vec![
        Message {
            role: "system".to_string(),
            content: system_prompt,
        },
        Message {
            role: "user".to_string(),
            content: user_prompt,
        },
    ];

    let raw = call_llm(client, api_config, &messages).await?;
    let cleaned = extract_json_payload(&raw);
    let review_payload: ModelReviewResponse = serde_json::from_str(&cleaned)
        .map_err(|e| HarperError::Api(format!("Failed to parse review response: {}", e)))?;
    let mut review = ReviewResponse {
        summary: review_payload.summary,
        findings: review_payload.findings,
        model: api_config.model_name.clone(),
    };

    if review.findings.len() > max_findings {
        review.findings.truncate(max_findings);
    }

    for finding in &mut review.findings {
        normalize_finding_range(finding, &request.content);
    }
    Ok(review)
}

fn extract_json_payload(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix("```") {
        let without_lang = stripped
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(stripped);
        let without_fence = without_lang.strip_suffix("```").unwrap_or(without_lang);
        return without_fence.trim().to_string();
    }

    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    match (start, end) {
        (Some(start_idx), Some(end_idx)) if start_idx <= end_idx => {
            trimmed[start_idx..=end_idx].to_string()
        }
        _ => trimmed.to_string(),
    }
}

fn add_line_numbers(content: &str) -> String {
    content
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>4} | {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_range(range: &ReviewRange) -> String {
    format!(
        "{}:{}-{}:{}",
        range.start_line, range.start_column, range.end_line, range.end_column
    )
}

fn normalize_finding_range(finding: &mut CodeReviewFinding, content: &str) {
    let line_count = content.lines().count().max(1);
    let range = &mut finding.range;

    range.start_line = range.start_line.clamp(1, line_count);
    range.end_line = range.end_line.clamp(range.start_line, line_count);
    range.start_column = range.start_column.max(1);
    range.end_column = range.end_column.max(range.start_column);
}

fn into_http_error(error: HarperError) -> (StatusCode, String) {
    match error {
        HarperError::Validation(message) => (StatusCode::BAD_REQUEST, message),
        HarperError::Api(message) => (StatusCode::BAD_GATEWAY, message),
        HarperError::Config(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        HarperError::Database(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        HarperError::Command(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        HarperError::Io(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        HarperError::File(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        HarperError::Crypto(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        HarperError::Mcp(message) => (StatusCode::BAD_GATEWAY, message),
        HarperError::WebSearch(message) => (StatusCode::BAD_GATEWAY, message),
        HarperError::Firmware(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        HarperError::Sandbox(error) => (StatusCode::FORBIDDEN, error.to_string()),
    }
}

/// Creates the Axum router with all API routes configured
pub fn create_router(
    conn: Arc<Mutex<Connection>>,
    api_config: ApiConfig,
    exec_policy: ExecPolicyConfig,
    supabase_auth: Option<SupabaseAuthConfig>,
) -> Router {
    let state = Arc::new(ServerState {
        conn,
        api_config,
        client: Client::new(),
        exec_policy,
        supabase_auth,
        oauth_states: Arc::new(Mutex::new(HashMap::new())),
        tui_auth_flows: Arc::new(Mutex::new(HashMap::new())),
    });

    Router::new()
        .route("/health", get(health))
        .route("/auth/login/{provider}", get(auth_login))
        .route("/auth/tui/start/{provider}", get(auth_tui_start))
        .route("/auth/tui/flow/{flow_id}", get(auth_tui_poll))
        .route("/auth/tui/refresh", post(auth_tui_refresh))
        .route("/auth/callback", get(auth_callback))
        .route("/auth/me", get(auth_me))
        .route("/auth/status", get(auth_status_page))
        .route("/auth/logout", get(auth_logout))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/plan", get(get_session_plan))
        .route(
            "/api/sessions/{id}/plan/stream",
            get(get_session_plan_stream),
        )
        .route("/api/sessions/{id}", delete(delete_session))
        .route("/api/chat", post(chat_endpoint))
        .route("/api/approvals/{session_id}", get(list_pending_approvals))
        .route("/api/approvals/{session_id}", post(approve_command))
        .route("/api/chat/approve/{pending_id}", post(approve_pending_tool))
        .route("/api/review", post(review_code))
        .with_state(state)
}

fn parse_user_provider(provider: &str) -> Result<UserAuthProvider, (StatusCode, String)> {
    match provider.trim().to_lowercase().as_str() {
        "github" => Ok(UserAuthProvider::Github),
        "google" => Ok(UserAuthProvider::Google),
        "apple" => Ok(UserAuthProvider::Apple),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("Unsupported auth provider '{}'", other),
        )),
    }
}

fn ensure_provider_allowed(
    config: &SupabaseAuthConfig,
    provider: UserAuthProvider,
) -> Result<(), (StatusCode, String)> {
    let Some(allowed) = &config.allowed_providers else {
        return Ok(());
    };

    if allowed
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(provider.as_str()))
    {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("Provider '{}' is not enabled", provider.as_str()),
        ))
    }
}

fn random_urlsafe(num_bytes: usize) -> Result<String, (StatusCode, String)> {
    let mut bytes = vec![0_u8; num_bytes];
    getrandom::fill(&mut bytes).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to generate secure random bytes: {}", err),
        )
    })?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn pkce_code_challenge(code_verifier: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
}

fn append_callback_query_param(base: &str, key: &str, value: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!(
        "{}{}{}={}",
        base,
        separator,
        urlencoding::encode(key),
        urlencoding::encode(value)
    )
}

fn render_auth_success(session: &AuthSession) -> String {
    let email = session
        .user
        .email
        .clone()
        .unwrap_or_else(|| "signed-in user".to_string());
    render_auth_status_page_with_message("Signed in to Harper", &format!("Signed in as {}", email))
}

fn fallback_auth_session_from_supabase_tokens(
    token_payload: SupabaseTokenResponse,
    provider: Option<UserAuthProvider>,
) -> AuthSession {
    let authenticated_user = AuthenticatedUser {
        user_id: token_payload
            .user
            .as_ref()
            .map(|user| user.id.clone())
            .unwrap_or_default(),
        email: token_payload
            .user
            .as_ref()
            .and_then(|user| user.email.clone()),
        display_name: None,
        provider,
    };

    AuthSession {
        access_token: token_payload.access_token,
        refresh_token: token_payload.refresh_token,
        expires_at: token_payload
            .expires_in
            .map(|seconds| chrono::Utc::now().timestamp() + seconds),
        user: authenticated_user,
    }
}

async fn auth_session_from_supabase_tokens(
    state: &ServerState,
    token_payload: SupabaseTokenResponse,
    provider: Option<UserAuthProvider>,
) -> AuthSession {
    let mut session = fallback_auth_session_from_supabase_tokens(token_payload, provider);

    if let Some(supabase) = state.supabase_auth.as_ref() {
        if let Ok(user) =
            auth::decode_access_token_with_client(&session.access_token, supabase, &state.client)
                .await
        {
            session.user = user;
        }
    }

    session
}

async fn refresh_supabase_session(
    state: &ServerState,
    refresh_token: String,
) -> Result<AuthSession, (StatusCode, String)> {
    let supabase = state.supabase_auth.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_IMPLEMENTED,
            "Supabase auth is not configured".to_string(),
        )
    })?;
    let project_url = supabase.project_url.as_deref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Supabase project URL is not configured".to_string(),
        )
    })?;

    let token_url = format!(
        "{}/auth/v1/token?grant_type=refresh_token",
        project_url.trim_end_matches('/')
    );
    let token_response = state
        .client
        .post(&token_url)
        .header("apikey", supabase.anon_key.clone().unwrap_or_default())
        .json(&SupabaseRefreshTokenRequest {
            refresh_token: refresh_token.as_str(),
        })
        .send()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Supabase refresh failed: {}", err),
            )
        })?;

    if !token_response.status().is_success() {
        let body = token_response.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Supabase refresh rejected session: {}", body),
        ));
    }

    let token_payload: SupabaseTokenResponse = token_response.json().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Supabase refresh response was invalid: {}", err),
        )
    })?;

    Ok(auth_session_from_supabase_tokens(state, token_payload, None).await)
}

fn append_auth_session_cookies(headers: &mut axum::http::HeaderMap, session: &AuthSession) {
    headers.append(
        axum::http::header::SET_COOKIE,
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax",
            auth::ACCESS_TOKEN_COOKIE,
            session.access_token
        )
        .parse()
        .expect("valid access token cookie"),
    );
    if let Some(refresh_token) = &session.refresh_token {
        headers.append(
            axum::http::header::SET_COOKIE,
            format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax",
                auth::REFRESH_TOKEN_COOKIE,
                refresh_token
            )
            .parse()
            .expect("valid refresh token cookie"),
        );
    }
}

fn clear_auth_cookies(headers: &mut axum::http::HeaderMap) {
    headers.append(
        axum::http::header::SET_COOKIE,
        format!(
            "{}=; Path=/; HttpOnly; Max-Age=0; SameSite=Lax",
            auth::ACCESS_TOKEN_COOKIE
        )
        .parse()
        .expect("valid access token clear cookie"),
    );
    headers.append(
        axum::http::header::SET_COOKIE,
        format!(
            "{}=; Path=/; HttpOnly; Max-Age=0; SameSite=Lax",
            auth::REFRESH_TOKEN_COOKIE
        )
        .parse()
        .expect("valid refresh token clear cookie"),
    );
}

fn render_auth_message(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body><h1>{}</h1><p>{}</p></body></html>",
        title, title, body
    )
}

fn render_auth_status_page() -> String {
    render_auth_status_page_with_message(
        "Auth status",
        "Checking current Harper session via /auth/me...",
    )
}

fn render_auth_status_page_with_message(title: &str, message: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <title>{title}</title>
    <style>
      body {{
        font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        margin: 2rem;
        color: #111827;
        background: #f9fafb;
      }}
      .card {{
        max-width: 48rem;
        background: white;
        border: 1px solid #e5e7eb;
        border-radius: 12px;
        padding: 1.25rem 1.5rem;
        box-shadow: 0 4px 20px rgba(0, 0, 0, 0.04);
      }}
      code {{
        background: #f3f4f6;
        padding: 0.1rem 0.3rem;
        border-radius: 6px;
      }}
      pre {{
        overflow-x: auto;
        background: #111827;
        color: #f9fafb;
        padding: 1rem;
        border-radius: 10px;
      }}
      details {{
        margin-top: 1rem;
      }}
      summary {{
        cursor: pointer;
        color: #4b5563;
      }}
      .muted {{ color: #6b7280; }}
      .ok {{ color: #065f46; }}
      .bad {{ color: #991b1b; }}
    </style>
  </head>
  <body>
    <div class="card">
      <h1>{title}</h1>
      <p id="status" class="muted">{message}</p>
      <p id="next" class="muted"></p>
      <details>
        <summary>Show details</summary>
        <pre id="payload">{{}}</pre>
      </details>
    </div>
    <script>
      async function loadAuthState() {{
        const status = document.getElementById('status');
        const next = document.getElementById('next');
        const payload = document.getElementById('payload');
        try {{
          const response = await fetch('/auth/me', {{
            method: 'GET',
            credentials: 'include',
            headers: {{ 'Accept': 'application/json' }}
          }});
          const body = await response.json();
          payload.textContent = JSON.stringify(body, null, 2);
          if (response.ok && body.authenticated) {{
            const email = body.user && body.user.email ? body.user.email : body.user && body.user.user_id;
            status.textContent = `Signed in as ${{email}}`;
            status.className = 'ok';
            next.textContent = 'You can close this tab and return to Harper.';
          }} else {{
            status.textContent = 'No active Harper session';
            status.className = 'bad';
            next.textContent = '';
          }}
        }} catch (error) {{
          payload.textContent = JSON.stringify({{ error: String(error) }}, null, 2);
          status.textContent = 'Failed to query /auth/me';
          status.className = 'bad';
          next.textContent = '';
        }}
      }}
      loadAuthState();
    </script>
  </body>
</html>"#,
        title = title,
        message = message
    )
}

pub async fn run_server(
    addr: &str,
    conn: Arc<Mutex<Connection>>,
    api_config: ApiConfig,
    exec_policy: ExecPolicyConfig,
    supabase_auth: Option<SupabaseAuthConfig>,
) -> HarperResult<()> {
    let router = create_router(conn, api_config, exec_policy, supabase_auth);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("Harper API server running on {}", addr);

    axum::serve(listener, router).await?;
    Ok(())
}

pub async fn list_pending_approvals(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let conn = state.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {:?}", e),
        )
    })?;

    let mut stmt = conn.prepare(
        "SELECT id, command, source, created_at FROM command_logs WHERE session_id = ?1 AND status = 'pending' AND requires_approval = 1"
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let rows = stmt
        .query_map([&session_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "command": row.get::<_, String>(1)?,
                "source": row.get::<_, String>(2)?,
                "created_at": row.get::<_, String>(3)?,
            }))
        })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let results: Vec<serde_json::Value> = rows.filter_map(|r| r.ok()).collect();

    Ok(Json(results))
}

pub async fn approve_command(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(payload): Json<ApprovalRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let conn = state.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {:?}", e),
        )
    })?;

    let status = if payload.approved {
        "completed"
    } else {
        "rejected"
    };

    conn.execute(
        "UPDATE command_logs SET approved = ?1, status = ?2 WHERE session_id = ?3 AND status = 'pending' AND requires_approval = 1 ORDER BY id DESC LIMIT 1",
        params![payload.approved as i32, status, session_id],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "session_id": session_id,
        "approved": payload.approved,
    })))
}

pub async fn approve_pending_tool(
    State(state): State<Arc<ServerState>>,
    Path(pending_id): Path<String>,
    Json(payload): Json<ApprovalRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    let conn = state.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock error: {:?}", e),
        )
    })?;

    let tool = crate::memory::storage::get_pending_tool(&conn, &pending_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Pending tool not found".to_string()))?;
    let session_id = tool.session_id.clone();

    if !payload.approved {
        crate::memory::storage::delete_pending_tool(&conn, &pending_id)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let _ = save_message(&conn, &session_id, "assistant", "Tool execution rejected");
        drop(conn);
        return Ok(Json(ChatResponse {
            message: "Tool execution rejected".to_string(),
            session_id,
            status: "rejected".to_string(),
            pending_id: None,
            pending_tools: None,
        }));
    }

    let args: serde_json::Value = serde_json::from_str(&tool.args)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = match tool.tool.as_str() {
        "run_command" => {
            if let Some(cmd) = args.get("command").and_then(|c| c.as_str()) {
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                let exit_code = output.status.code();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let output_str = if exit_code == Some(0) {
                    format!("$ {}\n{}", cmd, stdout)
                } else {
                    format!("$ {}\nError (exit {:?}):\n{}", cmd, exit_code, stderr)
                };
                let record = CommandLogRecord {
                    session_id: Some(session_id.clone()),
                    command: cmd.to_string(),
                    source: "http".to_string(),
                    requires_approval: true,
                    approved: true,
                    status: "completed".to_string(),
                    exit_code,
                    duration_ms: None,
                    stdout_preview: Some(stdout.to_string()),
                    stderr_preview: if stderr.is_empty() {
                        None
                    } else {
                        Some(stderr.to_string())
                    },
                    error_message: None,
                };
                let _ = crate::memory::storage::insert_command_log(&conn, &record);
                output_str
            } else {
                "Invalid command".to_string()
            }
        }
        "read_file" => {
            if let Some(path) = args.get("filePath").and_then(|p| p.as_str()) {
                let content = std::fs::read_to_string(path).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Read error: {}", e),
                    )
                })?;
                if content.len() > 50000 {
                    format!(
                        "{}...\n(truncated {} bytes)",
                        &content[..50000],
                        content.len()
                    )
                } else {
                    content
                }
            } else {
                "Invalid path".to_string()
            }
        }
        "write_file" => {
            if let Some(path) = args.get("filePath").and_then(|p| p.as_str()) {
                if let Some(content) = args.get("content").and_then(|c| c.as_str()) {
                    std::fs::write(path, content).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Write error: {}", e),
                        )
                    })?;
                    format!("Written {} bytes to {}", content.len(), path)
                } else {
                    "Invalid content".to_string()
                }
            } else {
                "Invalid path".to_string()
            }
        }
        "grep" => {
            if let Some(pattern) = args.get("pattern").and_then(|p| p.as_str()) {
                if let Some(path) = args.get("path").and_then(|p| p.as_str()) {
                    let hits =
                        crate::tools::search::search_files(std::path::Path::new(path), pattern);
                    let mut stdout = String::new();
                    for (file, lines) in &hits {
                        for (line_no, line) in lines {
                            stdout.push_str(&format!("{}:{}:{}\n", file.display(), line_no, line));
                        }
                    }
                    if stdout.is_empty() {
                        format!("No matches found for '{}' in {}", pattern, path)
                    } else {
                        format!("{}\n(matched {} lines)", stdout, stdout.lines().count())
                    }
                } else {
                    "Invalid path".to_string()
                }
            } else {
                "Invalid pattern".to_string()
            }
        }
        _ => "Unknown tool".to_string(),
    };

    crate::memory::storage::delete_pending_tool(&conn, &pending_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = save_message(&conn, &session_id, "assistant", &result);
    drop(conn);

    Ok(Json(ChatResponse {
        message: result,
        session_id,
        status: "completed".to_string(),
        pending_id: None,
        pending_tools: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        CodeReviewFinding, CodeSuggestion, ReviewRange, ReviewRequest, ServerState,
        SupabaseAuthConfig, TuiAuthFlowState, TuiRefreshRequest, auth_me, auth_tui_poll,
        auth_tui_refresh, build_authorize_url, delete_session, extract_json_payload, get_session,
        get_session_plan, get_session_plan_stream, list_sessions, normalize_finding_range,
        render_auth_status_page, render_auth_success, review_code,
    };
    use crate::core::auth::{AuthSession, AuthenticatedUser, UserAuthProvider};
    use crate::core::{ApiConfig, ApiProvider};
    use crate::memory::storage::{save_message, save_session, save_session_for_user};
    use crate::runtime::config::ExecPolicyConfig;
    use axum::Json;
    use axum::body::to_bytes;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION};
    use axum::response::IntoResponse;
    use chrono::{Duration, Utc};
    use jsonwebtoken::{EncodingKey, Header, encode};
    use reqwest::Client;
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn extracts_json_from_fenced_block() {
        let raw = "```json\n{\"summary\":\"ok\",\"findings\":[]}\n```";
        assert_eq!(
            extract_json_payload(raw),
            "{\"summary\":\"ok\",\"findings\":[]}"
        );
    }

    #[test]
    fn clamps_review_ranges_to_file_bounds() {
        let mut finding = CodeReviewFinding {
            title: "Issue".to_string(),
            severity: "warning".to_string(),
            message: "Details".to_string(),
            range: ReviewRange {
                start_line: 0,
                start_column: 0,
                end_line: 99,
                end_column: 0,
            },
            suggestion: Some(CodeSuggestion {
                description: "Fix".to_string(),
                replacement: "value".to_string(),
            }),
        };

        normalize_finding_range(&mut finding, "first\nsecond");

        assert_eq!(finding.range.start_line, 1);
        assert_eq!(finding.range.start_column, 1);
        assert_eq!(finding.range.end_line, 2);
        assert_eq!(finding.range.end_column, 1);
    }

    #[test]
    fn auth_status_page_uses_auth_me_on_load() {
        let html = render_auth_status_page();
        assert!(html.contains("fetch('/auth/me'"));
        assert!(html.contains("Checking current Harper session via /auth/me"));
    }

    #[test]
    fn auth_success_page_verifies_session_with_auth_me() {
        let html = render_auth_success(&AuthSession {
            access_token: "token".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: None,
            user: AuthenticatedUser {
                user_id: "user-a".to_string(),
                email: Some("user-a@example.com".to_string()),
                display_name: None,
                provider: Some(UserAuthProvider::Github),
            },
        });
        assert!(html.contains("Signed in to Harper"));
        assert!(html.contains("Signed in as user-a@example.com"));
        assert!(html.contains("You can close this tab and return to Harper."));
        assert!(html.contains("<summary>Show details</summary>"));
        assert!(html.contains("fetch('/auth/me'"));
    }

    #[test]
    fn authorize_url_uses_redirect_param_for_harper_flow() {
        let state = test_server_state(Some(SupabaseAuthConfig {
            project_url: Some("https://project.supabase.co".to_string()),
            anon_key: Some("anon".to_string()),
            jwt_secret: Some("secret".to_string()),
            redirect_url: Some("http://127.0.0.1:8081/auth/callback".to_string()),
            allowed_providers: Some(vec!["github".to_string()]),
        }));

        let authorize_url =
            build_authorize_url(&state, "github", Some("tui-flow".to_string())).expect("url");
        let parsed = reqwest::Url::parse(&authorize_url).expect("valid authorize url");
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert!(!params.contains_key("state"));
        let redirect_to = params.get("redirect_to").expect("redirect_to");
        assert!(redirect_to.contains("/auth/callback?harper_flow="));
    }

    #[tokio::test]
    async fn review_endpoint_rejects_empty_content() {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");

        let state = Arc::new(ServerState {
            conn: Arc::new(Mutex::new(conn)),
            api_config: ApiConfig {
                provider: ApiProvider::OpenAI,
                api_key: "test-key".to_string(),
                base_url: "https://api.openai.com/v1/chat/completions".to_string(),
                model_name: "gpt-5.5".to_string(),
            },
            client: Client::new(),
            exec_policy: crate::runtime::config::ExecPolicyConfig::default(),
            supabase_auth: None,
            oauth_states: Arc::new(Mutex::new(HashMap::new())),
            tui_auth_flows: Arc::new(Mutex::new(HashMap::new())),
        });

        let request = ReviewRequest {
            file_path: "src/main.rs".to_string(),
            content: String::new(),
            language: Some("rust".to_string()),
            workspace_root: None,
            instructions: None,
            selection: None,
            max_findings: Some(5),
        };

        let result = review_code(State(state), Json(request)).await;
        let error = result.expect_err("request should be rejected");
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(error.1.contains("content must not be empty"));
    }

    fn test_server_state(supabase_auth: Option<SupabaseAuthConfig>) -> Arc<ServerState> {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::memory::storage::init_db(&conn).expect("init db");

        Arc::new(ServerState {
            conn: Arc::new(Mutex::new(conn)),
            api_config: ApiConfig {
                provider: ApiProvider::OpenAI,
                api_key: "test-key".to_string(),
                base_url: "https://api.openai.com/v1/chat/completions".to_string(),
                model_name: "gpt-5.5".to_string(),
            },
            client: Client::new(),
            exec_policy: ExecPolicyConfig::default(),
            supabase_auth,
            oauth_states: Arc::new(Mutex::new(HashMap::new())),
            tui_auth_flows: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn auth_headers(secret: &str, user_id: &str, email: &str) -> HeaderMap {
        auth_headers_with_exp(secret, user_id, email, Utc::now() + Duration::hours(1))
    }

    fn auth_headers_with_exp(
        secret: &str,
        user_id: &str,
        email: &str,
        exp: chrono::DateTime<Utc>,
    ) -> HeaderMap {
        #[derive(serde::Serialize)]
        struct TestClaims {
            sub: String,
            email: Option<String>,
            role: Option<String>,
            aud: Option<String>,
            exp: usize,
        }

        let token = encode(
            &Header::default(),
            &TestClaims {
                sub: user_id.to_string(),
                email: Some(email.to_string()),
                role: Some("authenticated".to_string()),
                aud: Some("authenticated".to_string()),
                exp: exp.timestamp() as usize,
            },
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("jwt encode");

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).expect("bearer header"),
        );
        headers
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn local_mode_get_session_allows_unowned_session_without_auth() {
        let state = test_server_state(None);
        {
            let conn = state.conn.lock().expect("db lock");
            save_session(&conn, "local-session").expect("save session");
            save_message(&conn, "local-session", "user", "hello").expect("save message");
        }

        let response = get_session(
            State(state),
            HeaderMap::new(),
            Path("local-session".to_string()),
        )
        .await
        .expect("local session should be readable");

        let body = response.0;
        assert_eq!(body["session_id"], "local-session");
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert!(body["user_id"].is_null());
    }

    #[tokio::test]
    async fn get_session_includes_plan_runtime_jobs() {
        let state = test_server_state(None);
        {
            let conn = state.conn.lock().expect("db lock");
            save_session(&conn, "job-session").expect("save session");
            crate::memory::storage::save_plan_state(
                &conn,
                "job-session",
                &crate::core::plan::PlanState {
                    explanation: Some("Track runtime".to_string()),
                    items: vec![crate::core::plan::PlanItem {
                        step: "Run command".to_string(),
                        status: crate::core::plan::PlanStepStatus::InProgress,
                        job_id: Some("job-1".to_string()),
                    }],
                    runtime: Some(crate::core::plan::PlanRuntime {
                        active_tool: Some("run_command".to_string()),
                        active_command: Some("echo hi".to_string()),
                        active_status: Some("running".to_string()),
                        active_job_id: Some("job-1".to_string()),
                        jobs: vec![crate::core::plan::PlanJobRecord {
                            job_id: "job-1".to_string(),
                            tool: "run_command".to_string(),
                            command: Some("echo hi".to_string()),
                            status: crate::core::plan::PlanJobStatus::Running,
                            output_transcript: "echo hi\n".to_string(),
                            output_preview: Some("echo hi".to_string()),
                            has_error_output: false,
                        }],
                        followup: None,
                        ..Default::default()
                    }),
                    updated_at: None,
                },
            )
            .expect("save plan state");
        }

        let response = get_session(
            State(state),
            HeaderMap::new(),
            Path("job-session".to_string()),
        )
        .await
        .expect("job session should be readable");

        let body = response.0;
        assert_eq!(body["plan"]["runtime"]["active_job_id"], "job-1");
        assert_eq!(body["plan"]["items"][0]["job_id"], "job-1");
        assert_eq!(body["plan"]["runtime"]["jobs"][0]["status"], "running");
        assert_eq!(body["plan"]["runtime"]["jobs"][0]["command"], "echo hi");
    }

    #[tokio::test]
    async fn get_session_plan_returns_runtime_jobs_only() {
        let state = test_server_state(None);
        {
            let conn = state.conn.lock().expect("db lock");
            save_session(&conn, "plan-session").expect("save session");
            crate::memory::storage::save_plan_state(
                &conn,
                "plan-session",
                &crate::core::plan::PlanState {
                    explanation: Some("Track runtime".to_string()),
                    items: vec![crate::core::plan::PlanItem {
                        step: "Run command".to_string(),
                        status: crate::core::plan::PlanStepStatus::InProgress,
                        job_id: Some("job-1".to_string()),
                    }],
                    runtime: Some(crate::core::plan::PlanRuntime {
                        active_tool: Some("run_command".to_string()),
                        active_command: Some("echo hi".to_string()),
                        active_status: Some("running".to_string()),
                        active_job_id: Some("job-1".to_string()),
                        jobs: vec![crate::core::plan::PlanJobRecord {
                            job_id: "job-1".to_string(),
                            tool: "run_command".to_string(),
                            command: Some("echo hi".to_string()),
                            status: crate::core::plan::PlanJobStatus::Running,
                            output_transcript: "echo hi\n".to_string(),
                            output_preview: Some("echo hi".to_string()),
                            has_error_output: false,
                        }],
                        followup: None,
                        ..Default::default()
                    }),
                    updated_at: None,
                },
            )
            .expect("save plan state");
        }

        let response = get_session_plan(
            State(state),
            HeaderMap::new(),
            Path("plan-session".to_string()),
        )
        .await
        .expect("plan should be readable");

        let body = response.0;
        assert_eq!(body["runtime"]["active_job_id"], "job-1");
        assert_eq!(body["runtime"]["jobs"][0]["output_preview"], "echo hi");
        assert_eq!(body["items"][0]["job_id"], "job-1");
    }

    #[tokio::test]
    async fn get_session_plan_stream_responds_with_sse() {
        let state = test_server_state(None);
        {
            let conn = state.conn.lock().expect("db lock");
            save_session(&conn, "plan-stream-session").expect("save session");
            crate::memory::storage::save_plan_state(
                &conn,
                "plan-stream-session",
                &crate::core::plan::PlanState {
                    explanation: Some("Track runtime".to_string()),
                    items: vec![crate::core::plan::PlanItem {
                        step: "Run command".to_string(),
                        status: crate::core::plan::PlanStepStatus::InProgress,
                        job_id: Some("job-1".to_string()),
                    }],
                    runtime: Some(crate::core::plan::PlanRuntime {
                        active_tool: Some("run_command".to_string()),
                        active_command: Some("echo hi".to_string()),
                        active_status: Some("running".to_string()),
                        active_job_id: Some("job-1".to_string()),
                        jobs: vec![crate::core::plan::PlanJobRecord {
                            job_id: "job-1".to_string(),
                            tool: "run_command".to_string(),
                            command: Some("echo hi".to_string()),
                            status: crate::core::plan::PlanJobStatus::Running,
                            output_transcript: "echo hi\n".to_string(),
                            output_preview: Some("echo hi".to_string()),
                            has_error_output: false,
                        }],
                        followup: None,
                        ..Default::default()
                    }),
                    updated_at: None,
                },
            )
            .expect("save plan state");
        }

        let response = get_session_plan_stream(
            State(state),
            HeaderMap::new(),
            Path("plan-stream-session".to_string()),
        )
        .await
        .expect("stream should be readable")
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .expect("content-type header")
                .to_str()
                .ok(),
            Some("text/event-stream")
        );
    }

    #[tokio::test]
    async fn authenticated_list_sessions_returns_only_owned_sessions() {
        let secret = "test-secret";
        let state = test_server_state(Some(SupabaseAuthConfig {
            jwt_secret: Some(secret.to_string()),
            ..SupabaseAuthConfig::default()
        }));
        {
            let conn = state.conn.lock().expect("db lock");
            save_session_for_user(&conn, "session-a", "user-a").expect("session a");
            save_session_for_user(&conn, "session-b", "user-b").expect("session b");
        }

        let response = list_sessions(
            State(state),
            auth_headers(secret, "user-a", "user-a@example.com"),
        )
        .await
        .expect("list should succeed");

        let sessions = response.0;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "session-a");
        assert_eq!(sessions[0].user_id.as_deref(), Some("user-a"));
    }

    #[tokio::test]
    async fn authenticated_get_session_denies_other_users_session() {
        let secret = "test-secret";
        let state = test_server_state(Some(SupabaseAuthConfig {
            jwt_secret: Some(secret.to_string()),
            ..SupabaseAuthConfig::default()
        }));
        {
            let conn = state.conn.lock().expect("db lock");
            save_session_for_user(&conn, "session-a", "owner").expect("owned session");
            save_message(&conn, "session-a", "user", "hello").expect("save message");
        }

        let error = get_session(
            State(state),
            auth_headers(secret, "intruder", "intruder@example.com"),
            Path("session-a".to_string()),
        )
        .await
        .expect_err("other user should not read session");

        assert_eq!(error, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn authenticated_delete_session_denies_other_users_session() {
        let secret = "test-secret";
        let state = test_server_state(Some(SupabaseAuthConfig {
            jwt_secret: Some(secret.to_string()),
            ..SupabaseAuthConfig::default()
        }));
        {
            let conn = state.conn.lock().expect("db lock");
            save_session_for_user(&conn, "session-a", "owner").expect("owned session");
        }

        let status = delete_session(
            State(state.clone()),
            auth_headers(secret, "intruder", "intruder@example.com"),
            Path("session-a".to_string()),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);

        let conn = state.conn.lock().expect("db lock");
        let still_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                ["session-a"],
                |row| row.get(0),
            )
            .expect("session count");
        assert_eq!(still_exists, 1);
    }

    #[tokio::test]
    async fn auth_me_returns_authenticated_user_for_valid_token() {
        let secret = "test-secret";
        let state = test_server_state(Some(SupabaseAuthConfig {
            jwt_secret: Some(secret.to_string()),
            ..SupabaseAuthConfig::default()
        }));

        let response = auth_me(
            State(state),
            auth_headers(secret, "user-a", "user-a@example.com"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["authenticated"], true);
        assert_eq!(body["user"]["user_id"], "user-a");
        assert_eq!(body["user"]["email"], "user-a@example.com");
    }

    #[tokio::test]
    async fn auth_me_clears_invalid_auth_cookies() {
        let secret = "test-secret";
        let state = test_server_state(Some(SupabaseAuthConfig {
            jwt_secret: Some(secret.to_string()),
            ..SupabaseAuthConfig::default()
        }));

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static(
                "harper_access_token=broken-token; harper_refresh_token=refresh-token",
            ),
        );

        let response = auth_me(State(state), headers).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let set_cookies: Vec<_> = response
            .headers()
            .get_all(axum::http::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect();
        assert!(
            set_cookies
                .iter()
                .any(|cookie| cookie.contains("harper_access_token=;")
                    && cookie.contains("Max-Age=0"))
        );
        assert!(set_cookies.iter().any(
            |cookie| cookie.contains("harper_refresh_token=;") && cookie.contains("Max-Age=0")
        ));

        let body = response_json(response).await;
        assert_eq!(body["authenticated"], false);
        assert!(body["user"].is_null());
    }

    #[tokio::test]
    async fn tui_auth_poll_returns_pending_for_unfinished_flow() {
        let state = test_server_state(None);
        {
            let mut flows = state.tui_auth_flows.lock().expect("flow lock");
            flows.insert(
                "flow-1".to_string(),
                TuiAuthFlowState {
                    session: None,
                    created_at: std::time::Instant::now(),
                },
            );
        }

        let response = auth_tui_poll(State(state), Path("flow-1".to_string()))
            .await
            .expect("poll should succeed");
        assert_eq!(response.0.status, "pending");
        assert!(response.0.session.is_none());
    }

    #[tokio::test]
    async fn auth_tui_refresh_requires_supabase_config() {
        let state = test_server_state(None);
        let error = auth_tui_refresh(
            State(state),
            Json(TuiRefreshRequest {
                refresh_token: "refresh-token".to_string(),
            }),
        )
        .await
        .expect_err("refresh should require supabase config");

        assert_eq!(error.0, StatusCode::NOT_IMPLEMENTED);
        assert!(error.1.contains("Supabase auth is not configured"));
    }
}
