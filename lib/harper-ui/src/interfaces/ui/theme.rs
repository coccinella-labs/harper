// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use ratatui::style::{Color, Modifier, Style};
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub accent: Color,
    pub border: Color,
    pub title: Color,
    pub input: Color,
    pub output: Color,
    pub error: Color,
    pub success: Color,
    pub warning: Color,
    pub info: Color,
    pub muted: Color,
    pub highlight: Color,
    pub selection: Color,
    pub syntax_theme: String,
    pub syntax_set: SyntaxSet,
    pub theme_set: ThemeSet,
}

impl Default for Theme {
    fn default() -> Self {
        Self::minimal()
    }
}

impl Theme {
    pub fn minimal() -> Self {
        Self {
            background: Color::Rgb(15, 15, 15),    // Deep, soft black
            foreground: Color::Rgb(220, 220, 220), // Soft white
            accent: Color::Rgb(130, 150, 180),     // Muted steel blue
            border: Color::Rgb(40, 40, 40),        // Subtle borders
            title: Color::Rgb(180, 180, 180),      // Gray titles
            input: Color::Rgb(255, 255, 255),      // Pure white for active input
            output: Color::Rgb(200, 200, 200),     // Off-white for responses
            error: Color::Rgb(180, 100, 100),      // Muted red
            success: Color::Rgb(100, 150, 100),    // Muted green
            warning: Color::Rgb(180, 150, 100),    // Muted gold
            info: Color::Rgb(130, 150, 180),       // Match accent
            muted: Color::Rgb(80, 80, 80),         // Dimmed text
            highlight: Color::Rgb(240, 240, 240),  // Bright highlight
            selection: Color::Rgb(45, 45, 45),     // Subtle background selection
            syntax_theme: "base16-ocean.dark".to_string(),
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "minimal" => Self::minimal(),
            _ => Self::default(),
        }
    }

    #[allow(dead_code)]
    pub fn input_style(&self) -> Style {
        Style::default().fg(self.input)
    }

    #[allow(dead_code)]
    pub fn output_style(&self) -> Style {
        Style::default().fg(self.output)
    }

    #[allow(dead_code)]
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error)
    }

    pub fn title_style(&self) -> Style {
        Style::default().fg(self.title)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    #[allow(dead_code)]
    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent)
    }

    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.warning)
    }

    pub fn info_style(&self) -> Style {
        Style::default().fg(self.info)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn highlight_style(&self) -> Style {
        Style::default()
            .fg(self.highlight)
            .add_modifier(Modifier::BOLD)
    }

    pub fn selection_style(&self) -> Style {
        Style::default().bg(self.selection).fg(self.background)
    }
}
