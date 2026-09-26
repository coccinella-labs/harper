// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

// UI interface for Harper
pub mod app;
pub mod auth;
pub mod events;
pub mod settings;
pub mod theme;
pub mod tui;
pub mod widgets;

pub use theme::Theme;
pub use tui::run_tui;
