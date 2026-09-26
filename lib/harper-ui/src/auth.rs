// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_core::{HarperError, HarperResult};
use keyring::Entry;
use std::io::{self, Write};

const KEYRING_SERVICE: &str = "harper";

#[derive(Debug, Clone, Copy)]
pub enum Provider {
    OpenAI,
    Sambanova,
    Gemini,
}

impl Provider {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "openai" | "open_ai" => Some(Self::OpenAI),
            "sambanova" | "samba" => Some(Self::Sambanova),
            "gemini" => Some(Self::Gemini),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Sambanova => "Sambanova",
            Self::Gemini => "Gemini",
        }
    }

    fn account_name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Sambanova => "Sambanova",
            Self::Gemini => "Gemini",
        }
    }
}

pub fn handle_auth_command(args: &[String]) -> Option<i32> {
    if args.len() < 2 || args[1] != "auth" {
        return None;
    }

    if args.len() < 3 {
        print_usage();
        return Some(2);
    }

    let subcommand = args[2].as_str();
    let provider = match parse_provider(args) {
        Ok(provider) => provider,
        Err(message) => {
            eprintln!("{}", message);
            print_usage();
            return Some(2);
        }
    };

    match subcommand {
        "login" => {
            if let Err(err) = login(provider) {
                eprintln!("Auth login failed: {}", err);
                return Some(1);
            }
            println!(
                "Stored {} API key in your OS keychain.",
                provider.display_name()
            );
            Some(0)
        }
        "logout" => {
            if let Err(err) = logout(provider) {
                eprintln!("Auth logout failed: {}", err);
                return Some(1);
            }
            println!(
                "Removed {} API key from your OS keychain.",
                provider.display_name()
            );
            println!(
                "To fully revoke access, also delete the key in your {} dashboard.",
                provider.display_name()
            );
            Some(0)
        }
        _ => {
            print_usage();
            Some(2)
        }
    }
}

pub fn load_keyring_key(provider_name: &str) -> Option<String> {
    let provider = Provider::from_str(provider_name)?;
    let entry = Entry::new(KEYRING_SERVICE, provider.account_name()).ok()?;
    entry.get_password().ok()
}

pub fn is_placeholder_key(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed == "your_api_key_here" || trimmed == "your_gemini_api_key_here"
}

fn login(provider: Provider) -> HarperResult<()> {
    let api_key = prompt_for_key(provider)?;
    if api_key.trim().is_empty() {
        return Err(HarperError::Api("API key cannot be empty.".to_string()));
    }

    let entry = Entry::new(KEYRING_SERVICE, provider.account_name())
        .map_err(|e| HarperError::Crypto(format!("Failed to open keychain entry: {}", e)))?;
    entry
        .set_password(api_key.trim())
        .map_err(|e| HarperError::Crypto(format!("Failed to store key: {}", e)))?;

    Ok(())
}

fn logout(provider: Provider) -> HarperResult<()> {
    let entry = Entry::new(KEYRING_SERVICE, provider.account_name())
        .map_err(|e| HarperError::Crypto(format!("Failed to open keychain entry: {}", e)))?;
    entry
        .delete_credential()
        .map_err(|e| HarperError::Crypto(format!("Failed to remove key: {}", e)))?;
    Ok(())
}

fn prompt_for_key(provider: Provider) -> HarperResult<String> {
    println!(
        "Enter {} API key (input will be visible):",
        provider.display_name()
    );
    print!("> ");
    io::stdout()
        .flush()
        .map_err(|e| HarperError::Io(format!("Failed to flush stdout: {}", e)))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| HarperError::Io(format!("Failed to read input: {}", e)))?;
    Ok(input)
}

fn parse_provider(args: &[String]) -> HarperResult<Provider> {
    let mut provider_value: Option<String> = None;

    let mut iter = args.iter().skip(3);
    while let Some(arg) = iter.next() {
        if arg == "--provider" {
            if let Some(value) = iter.next() {
                provider_value = Some(value.to_string());
                break;
            }
        } else if !arg.starts_with("--") && provider_value.is_none() {
            provider_value = Some(arg.to_string());
            break;
        }
    }

    let provider_value = provider_value
        .ok_or_else(|| HarperError::Api("Missing provider. Use --provider <name>.".to_string()))?;

    Provider::from_str(&provider_value).ok_or_else(|| {
        HarperError::Api(format!(
            "Unknown provider '{}'. Use OpenAI, Sambanova, or Gemini.",
            provider_value
        ))
    })
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  harper auth login --provider <openai|sambanova|gemini>");
    eprintln!("  harper auth logout --provider <openai|sambanova|gemini>");
}

#[cfg(test)]
mod tests {
    use super::{Provider, is_placeholder_key, parse_provider};

    #[test]
    fn parse_provider_flag() {
        let args = vec![
            "harper".to_string(),
            "auth".to_string(),
            "login".to_string(),
            "--provider".to_string(),
            "openai".to_string(),
        ];
        let provider = parse_provider(&args).expect("provider should parse");
        assert!(matches!(provider, Provider::OpenAI));
    }

    #[test]
    fn parse_provider_positional() {
        let args = vec![
            "harper".to_string(),
            "auth".to_string(),
            "logout".to_string(),
            "gemini".to_string(),
        ];
        let provider = parse_provider(&args).expect("provider should parse");
        assert!(matches!(provider, Provider::Gemini));
    }

    #[test]
    fn detects_placeholder_keys() {
        assert!(is_placeholder_key("your_api_key_here"));
        assert!(is_placeholder_key("your_gemini_api_key_here"));
        assert!(!is_placeholder_key("sk-live-123"));
    }
}
