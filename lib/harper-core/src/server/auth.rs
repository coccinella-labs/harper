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

use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header, errors::ErrorKind, jwk::JwkSet,
};
use reqwest::Client;
use std::str::FromStr;

use crate::core::auth::{AuthenticatedUser, UserAuthClaims, UserAuthProvider};
use crate::runtime::config::SupabaseAuthConfig;

pub const ACCESS_TOKEN_COOKIE: &str = "harper_access_token";
pub const REFRESH_TOKEN_COOKIE: &str = "harper_refresh_token";

#[derive(Debug)]
pub enum AuthError {
    MissingToken,
    MissingConfig(String),
    ExpiredToken,
    InvalidToken(String),
}

pub async fn authenticate_request_with_client(
    headers: &HeaderMap,
    supabase: Option<&SupabaseAuthConfig>,
    client: &Client,
) -> Result<AuthenticatedUser, AuthError> {
    let supabase = supabase.ok_or_else(|| {
        AuthError::MissingConfig("Supabase authentication is not configured".to_string())
    })?;

    let token = extract_bearer_token(headers).ok_or(AuthError::MissingToken)?;
    decode_access_token_with_client(&token, supabase, client).await
}

pub fn decode_access_token(
    token: &str,
    supabase: &SupabaseAuthConfig,
) -> Result<AuthenticatedUser, AuthError> {
    let jwt_secret = supabase.jwt_secret.as_deref().ok_or_else(|| {
        AuthError::MissingConfig("Supabase JWT secret is not configured".to_string())
    })?;

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;

    let decoded = decode::<UserAuthClaims>(
        token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &validation,
    )
    .map_err(|err| match err.kind() {
        ErrorKind::ExpiredSignature => AuthError::ExpiredToken,
        _ => AuthError::InvalidToken(format!("Invalid Supabase token: {}", err)),
    })?;

    let claims = decoded.claims;
    let provider = inferred_provider(&claims);
    let display_name = inferred_display_name(&claims);

    Ok(AuthenticatedUser {
        user_id: claims.sub,
        email: claims.email,
        display_name,
        provider,
    })
}

pub async fn decode_access_token_with_client(
    token: &str,
    supabase: &SupabaseAuthConfig,
    client: &Client,
) -> Result<AuthenticatedUser, AuthError> {
    let header = decode_header(token)
        .map_err(|err| AuthError::InvalidToken(format!("Invalid header: {}", err)))?;

    match header.alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
            decode_access_token(token, supabase)
        }
        Algorithm::ES256
        | Algorithm::ES384
        | Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512 => {
            decode_access_token_with_jwks(token, supabase, client, header.alg, header.kid).await
        }
        other => Err(AuthError::InvalidToken(format!(
            "Unsupported Supabase token algorithm: {:?}",
            other
        ))),
    }
}

async fn decode_access_token_with_jwks(
    token: &str,
    supabase: &SupabaseAuthConfig,
    client: &Client,
    algorithm: Algorithm,
    kid: Option<String>,
) -> Result<AuthenticatedUser, AuthError> {
    let project_url = supabase.project_url.as_deref().ok_or_else(|| {
        AuthError::MissingConfig("Supabase project URL is not configured".to_string())
    })?;
    let kid =
        kid.ok_or_else(|| AuthError::InvalidToken("Missing key id in token header".to_string()))?;

    let jwks_url = format!(
        "{}/auth/v1/.well-known/jwks.json",
        project_url.trim_end_matches('/')
    );
    let jwks = client
        .get(&jwks_url)
        .send()
        .await
        .map_err(|err| AuthError::InvalidToken(format!("Failed to fetch Supabase JWKS: {}", err)))?
        .error_for_status()
        .map_err(|err| AuthError::InvalidToken(format!("Supabase JWKS request failed: {}", err)))?
        .json::<JwkSet>()
        .await
        .map_err(|err| {
            AuthError::InvalidToken(format!("Invalid Supabase JWKS payload: {}", err))
        })?;

    let jwk = jwks
        .keys
        .iter()
        .find(|jwk| jwk.common.key_id.as_deref() == Some(kid.as_str()))
        .ok_or_else(|| {
            AuthError::InvalidToken(format!(
                "No matching Supabase signing key found for kid '{}'",
                kid
            ))
        })?;

    let mut validation = Validation::new(algorithm);
    validation.validate_aud = false;

    let decoded = decode::<UserAuthClaims>(
        token,
        &DecodingKey::from_jwk(jwk)
            .map_err(|err| AuthError::InvalidToken(format!("Invalid Supabase JWK: {}", err)))?,
        &validation,
    )
    .map_err(|err| match err.kind() {
        ErrorKind::ExpiredSignature => AuthError::ExpiredToken,
        _ => AuthError::InvalidToken(format!("Invalid Supabase token: {}", err)),
    })?;

    let claims = decoded.claims;
    let provider = inferred_provider(&claims);
    let display_name = inferred_display_name(&claims);

    Ok(AuthenticatedUser {
        user_id: claims.sub,
        email: claims.email,
        display_name,
        provider,
    })
}

fn inferred_provider(claims: &UserAuthClaims) -> Option<UserAuthProvider> {
    if let Some(app_metadata) = &claims.app_metadata {
        if let Some(provider) = app_metadata
            .provider
            .as_deref()
            .and_then(|provider| UserAuthProvider::from_str(provider).ok())
        {
            return Some(provider);
        }

        if let Some(provider) = app_metadata
            .providers
            .iter()
            .find_map(|provider| UserAuthProvider::from_str(provider).ok())
        {
            return Some(provider);
        }
    }

    None
}

fn inferred_display_name(claims: &UserAuthClaims) -> Option<String> {
    let metadata = claims.user_metadata.as_ref()?;
    metadata
        .full_name
        .as_ref()
        .or(metadata.name.as_ref())
        .or(metadata.user_name.as_ref())
        .cloned()
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    if let Some(auth_header) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = auth_header
            .strip_prefix("Bearer ")
            .or_else(|| auth_header.strip_prefix("bearer "))
        {
            return Some(token.to_string());
        }
    }

    cookie_value(headers, ACCESS_TOKEN_COOKIE)
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookie_header.split(';').find_map(|pair| {
        let mut parts = pair.trim().splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        (key == name).then(|| value.to_string())
    })
}

pub fn refresh_token(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, REFRESH_TOKEN_COOKIE)
}

impl AuthError {
    pub fn into_http_error(self) -> (StatusCode, String) {
        match self {
            Self::MissingToken => (
                StatusCode::UNAUTHORIZED,
                "Missing bearer token in Authorization header or auth cookie".to_string(),
            ),
            Self::MissingConfig(message) => (StatusCode::UNAUTHORIZED, message),
            Self::ExpiredToken => (
                StatusCode::UNAUTHORIZED,
                "Supabase access token expired".to_string(),
            ),
            Self::InvalidToken(message) => (StatusCode::UNAUTHORIZED, message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{inferred_display_name, inferred_provider};
    use crate::core::auth::{
        UserAppMetadataClaims, UserAuthClaims, UserAuthProvider, UserMetadataClaims,
    };

    #[test]
    fn infers_provider_from_app_metadata() {
        let claims = UserAuthClaims {
            sub: "user-1".to_string(),
            email: Some("user@example.com".to_string()),
            role: None,
            aud: None,
            app_metadata: Some(UserAppMetadataClaims {
                provider: Some("github".to_string()),
                providers: vec!["github".to_string()],
            }),
            user_metadata: None,
        };

        assert_eq!(inferred_provider(&claims), Some(UserAuthProvider::Github));
    }

    #[test]
    fn infers_display_name_from_user_metadata() {
        let claims = UserAuthClaims {
            sub: "user-1".to_string(),
            email: Some("user@example.com".to_string()),
            role: None,
            aud: None,
            app_metadata: None,
            user_metadata: Some(UserMetadataClaims {
                full_name: Some("Example User".to_string()),
                name: None,
                user_name: None,
            }),
        };

        assert_eq!(
            inferred_display_name(&claims).as_deref(),
            Some("Example User")
        );
    }
}
