// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserAuthProvider {
    Github,
    Google,
    Apple,
}

impl UserAuthProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Google => "google",
            Self::Apple => "apple",
        }
    }
}

impl FromStr for UserAuthProvider {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_lowercase().as_str() {
            "github" => Ok(Self::Github),
            "google" => Ok(Self::Google),
            "apple" => Ok(Self::Apple),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub provider: Option<UserAuthProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub user: AuthenticatedUser,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserAuthClaims {
    pub sub: String,
    pub email: Option<String>,
    pub role: Option<String>,
    pub aud: Option<String>,
    #[serde(default)]
    pub app_metadata: Option<UserAppMetadataClaims>,
    #[serde(default)]
    pub user_metadata: Option<UserMetadataClaims>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UserAppMetadataClaims {
    pub provider: Option<String>,
    #[serde(default)]
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UserMetadataClaims {
    pub full_name: Option<String>,
    pub name: Option<String>,
    pub user_name: Option<String>,
}
