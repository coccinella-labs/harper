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

use rusqlite::Connection;
use std::env;

use harper_core::error::HarperError;

use std::io::IsTerminal;

use harper_core::runtime::config::{HarperConfig, should_enable_server};

mod auth;

fn exit_on_error<T, E: std::fmt::Display>(result: Result<T, E>, message: &str) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("{}: {}", message, e);
        std::process::exit(1);
    })
}

fn print_version() {
    println!("harper v{}", harper_ui::CLI_VERSION);
    std::process::exit(0);
}

fn get_api_key(config: &HarperConfig) -> String {
    let mut api_key = config.api.api_key.clone();
    if config.api.provider == "Gemini" {
        if let Ok(env_key) = std::env::var("GEMINI_API_KEY") {
            api_key = env_key;
        }
    } else if config.api.provider == "OpenAI" {
        if let Ok(env_key) = std::env::var("OPENAI_API_KEY") {
            api_key = env_key;
        }
    } else if config.api.provider == "Sambanova" {
        if let Ok(env_key) = std::env::var("SAMBASTUDIO_API_KEY") {
            api_key = env_key;
        }
    }
    if api_key == config.api.api_key && auth::is_placeholder_key(&api_key) {
        if let Some(keyring_key) = auth::load_keyring_key(&config.api.provider) {
            api_key = keyring_key;
        }
    }
    api_key
}

#[tokio::main]
async fn main() -> Result<(), HarperError> {
    // Load .env file if it exists
    let _ = dotenvy::dotenv();

    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        print_version();
    }
    if let Some(exit_code) = harper_ui::update::handle_update_command(&args).await {
        std::process::exit(exit_code);
    }
    if let Some(exit_code) = auth::handle_auth_command(&args) {
        std::process::exit(exit_code);
    }
    let config = exit_on_error(HarperConfig::new(), "Failed to load configuration");

    if !std::io::stdout().is_terminal() {
        eprintln!(
            "Harper requires an interactive terminal. Use harper-batch for non-interactive runs."
        );
        std::process::exit(2);
    }

    let api_key = get_api_key(&config);

    let api_config = harper_core::core::ApiConfig {
        provider: config.api.get_provider().map_err(|e| {
            eprintln!("Configuration error: {}", e);
            e
        })?,
        api_key,
        base_url: config.api.base_url.clone(),
        model_name: config.api.model_name.clone(),
    };

    // Ensure database directory exists
    if let Some(parent) = std::path::Path::new(&config.database.path).parent() {
        exit_on_error(
            std::fs::create_dir_all(parent),
            "Failed to create database directory",
        );
    }

    let conn = exit_on_error(
        Connection::open(&config.database.path),
        "Failed to open database",
    );
    exit_on_error(
        harper_core::memory::storage::init_db(&conn),
        "Failed to initialize database",
    );

    let exec_policy = config.exec_policy.clone();

    // Check for --no-server flag
    let args: Vec<String> = std::env::args().collect();
    let mut server_task = None;

    // Check for server mode
    let server_enabled = should_enable_server(config.server.enabled.unwrap_or(false), &args);
    if server_enabled {
        let host = config.server.host.as_deref().unwrap_or("127.0.0.1");
        let port = config.server.port.unwrap_or(8081);
        let addr = format!("{}:{}", host, port);

        let conn = std::sync::Arc::new(std::sync::Mutex::new(
            harper_core::memory::storage::create_connection(&config.database.path)
                .expect("Failed to create database connection"),
        ));

        println!("Starting Harper API server on http://{}", addr);
        println!("Endpoints:");
        println!("  GET  /health          - Health check");
        println!("  GET  /api/sessions    - List sessions");
        println!("  GET  /api/sessions/{{id}} - Get session");
        println!("  POST /api/chat        - Send chat message");
        println!("  POST /api/review      - Review file content");

        let conn_clone = conn.clone();
        let api_config_clone = api_config.clone();
        let exec_policy_clone = exec_policy.clone();
        let supabase_auth_clone = config.auth.supabase.clone();
        server_task = Some(tokio::spawn(async move {
            if let Err(e) = harper_core::server::run_server(
                &addr,
                conn_clone,
                api_config_clone,
                exec_policy_clone,
                supabase_auth_clone,
            )
            .await
            {
                eprintln!("Server error: {}", e);
            }
        }));
    }

    let session_service = harper_core::memory::session_service::SessionService::new(&conn);

    // Create theme
    let theme = config
        .ui
        .theme
        .as_ref()
        .map(|t| harper_ui::interfaces::ui::Theme::from_name(t))
        .unwrap_or_default();

    let custom_commands = config.custom_commands.commands.clone().unwrap_or_default();
    let server_base_url = server_enabled.then(|| {
        format!(
            "http://{}:{}",
            config
                .server
                .host
                .clone()
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            config.server.port.unwrap_or(8081)
        )
    });

    if let Err(err) = harper_ui::interfaces::ui::run_tui(
        &conn,
        &api_config,
        &session_service,
        &theme,
        &exec_policy,
        &config.ui,
        harper_ui::interfaces::ui::tui::TuiRunOptions {
            custom_commands,
            server_base_url,
        },
    )
    .await
    {
        eprintln!("TUI failed: {}", err);
        std::process::exit(1);
    }

    if let Some(server_task) = server_task {
        server_task.abort();
    }

    Ok(())
}
