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

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::Frame;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};

use super::app::{
    AppState, ApprovalState, LineSelection, ReviewState, SessionInfo, TuiApp, UiMessage,
    MAIN_MENU_ITEM_COUNT,
};
use super::settings;
use super::theme::Theme;
use harper_core::core::plan::{
    PlanFollowup, PlanJobRecord, PlanJobStatus, PlanLoopOutcome, PlanLoopStage,
};
use harper_core::{PlanRuntime, PlanState, PlanStepStatus, ResolvedAgents};

const MAX_COMPLETION_POPUP_HEIGHT: u16 = 12;
const MENU_BLOCK_VERTICAL_OVERHEAD: u16 = 3;
const FOOTER_HEIGHT: u16 = 1;
const SIDEBAR_WIDTH: u16 = 22;
const MAX_COMMAND_OUTPUT_LINES: usize = 4;
const COMPACT_HEIGHT_THRESHOLD: u16 = 36;
const COMPACT_WIDTH_THRESHOLD: u16 = 100;
const COMPACT_COMMAND_OUTPUT_LINES: usize = 3;
const MAIN_MENU_ITEMS_WIDTH: u16 = 22;
const MAIN_MENU_LOGO_WIDTH: u16 = 38;
const MAIN_MENU_LOGO_HEIGHT: u16 = 11;
const MAIN_MENU_LOGO_COLOR: Color = Color::Rgb(255, 122, 36);
const MAIN_MENU_LOGO_ROWS: [&str; MAIN_MENU_LOGO_HEIGHT as usize] = [
    "   ⣀⠴⠶⠛⢻⣄",
    "⠰⠛⠛⠙⢷⣦⡀ ⠹⣧⡀",
    "     ⠈⠻⣦⡀⢈⣷⣄⣀⡀",
    "      ⢀⣼⣿⣿⣿⣿⣿⣿⣦",
    "     ⢠⣿⣿⣅⣼⣿⣿⣿⣿⣿⢇⣴⣾⣿⣿⣷⣦⣄    ⢀⣀⣀⣀⣀",
    "     ⢸⣿⣿⣿⣿⣿⣿⣿⠿⠋⢸⣿⣿⣿⣿⣿⣿⣿⣷⡀⢠⣾⣿⣿⣿⣿⣿⣿⣦⡀",
    "     ⠈⠛⠻⠟⠛⠛⢉⣴⢶⣤⣈⠻⣿⣿⣿⣿⢟⡛⢿⡇⣿⡿⢟⠛⢿⣿⣿⣿⣿⣿⣦",
    "          ⣠⡿⠁⢀⣤⣭⣥⣬⣿⠟⠁⣾⣷ ⣠⣤⡶⣿⣿⣌⠻⣿⣿⣿⣿⣿⣧",
    "         ⣴⠟ ⢀⣿⠋⠉⠉⠉⠁   ⢹⣧⠉⠁ ⠈⠻⢿⣦⡙⣿⣿⣿⣿⡿",
    "       ⢠⡾⠃  ⣾⠃         ⢻⡆     ⠘⢷⣌⠉⠉⠁",
    "      ⣰⡟⠁  ⠸⠏          ⠈⢿⠄      ⠻⣦",
];

// Refined shortcuts for a cleaner footer
const CHAT_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[
        ("G", "Help"),
        ("W", "Search"),
        ("B", "Sidebar"),
        ("A", "Agents"),
        ("F", "Findings"),
        ("M", "Msgs"),
        ("C", "ID"),
        ("Ctrl+O", "Output"),
    ],
    &[
        ("X", "Exit"),
        ("R", "Load"),
        ("L", "Preview"),
        ("U", "Paste"),
        ("T", "Enter"),
        ("Y", "Prev"),
        ("V", "Next"),
        ("Esc", "Back"),
        ("P", "Jobs"),
    ],
];

const HOME_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[
        ("↑↓", "Move"),
        ("Enter", "Open"),
        ("Ctrl+G", "Help"),
        ("Q", "Quit"),
    ],
    &[("J/K", "Move"), ("L", "Open")],
];

const SESSIONS_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[
        ("↑↓", "Move"),
        ("Enter", "Resume"),
        ("→", "Preview"),
        ("D", "Delete"),
        ("Esc", "Back"),
    ],
    &[("J/K", "Move"), ("L", "Preview"), ("Q", "Back")],
];

const EXPORT_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[("↑↓", "Move"), ("Enter", "Export"), ("Esc", "Back")],
    &[("J/K", "Move")],
];

const PREVIEW_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[("↑↓", "Scroll"), ("Enter", "Resume"), ("Esc", "Back")],
    &[("J/K", "Scroll")],
];

const SETTINGS_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[("↑↓", "Move"), ("Enter", "Open"), ("Esc", "Back")],
    &[("J/K", "Move")],
];

const PROFILE_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[("↑↓", "Move"), ("Enter", "Run"), ("Esc", "Back")],
    &[("J/K", "Move")],
];

const EXECUTION_POLICY_FOOTER_SHORTCUTS: &[&[(&str, &str)]] = &[
    &[("↑↓", "Move"), ("Enter", "Change"), ("Esc", "Back")],
    &[("J/K", "Move")],
];

const STATS_FOOTER_SHORTCUTS: &[&[(&str, &str)]] =
    &[&[("Esc", "Back"), ("Q", "Back"), ("Ctrl+G", "Help")]];

use crate::plugins::syntax::highlight_code_lines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

enum MessageSegment {
    Paragraph(String),
    Heading(String),
    StructuredLine(String),
    CodeBlock {
        language: Option<String>,
        content: String,
    },
    Blank,
}

fn wrapped_line_count(lines: &[Line<'static>], width: u16) -> usize {
    if width == 0 {
        return 0;
    }

    lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width as usize)
            }
        })
        .sum()
}

fn input_area_height(input: &str, width: u16, available_height: u16) -> u16 {
    let compact = width <= COMPACT_WIDTH_THRESHOLD;
    let content_width = width.saturating_sub(4).max(1);
    let lines = if input.is_empty() {
        vec![Line::raw("› Type / for commands, or write a message")]
    } else {
        input_lines(input, Style::default())
    };
    let content_height = wrapped_line_count(&lines, content_width) as u16;
    let multiline = input.contains('\n');
    let max_height = if multiline {
        available_height.saturating_sub(4).clamp(4, 12)
    } else if compact {
        4
    } else {
        8
    };
    content_height.saturating_add(1).clamp(3, max_height)
}

fn compact_layout(area: Rect) -> bool {
    area.height <= COMPACT_HEIGHT_THRESHOLD || area.width <= COMPACT_WIDTH_THRESHOLD
}

fn input_lines(input: &str, input_style: Style) -> Vec<Line<'static>> {
    input
        .split('\n')
        .map(|line| Line::from(vec![Span::styled(line.to_string(), input_style)]))
        .collect()
}

fn input_display_lines(
    input: &str,
    input_style: Style,
    visible_capacity: usize,
) -> Vec<Line<'static>> {
    let mut lines = input_lines(input, input_style)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix = if index == 0 { "› " } else { "  " };
            let mut spans = vec![Span::styled(prefix, Style::default())];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    if visible_capacity > 0 && lines.len() > visible_capacity {
        let hidden = lines.len() - visible_capacity;
        lines = lines.split_off(hidden);
        if let Some(first) = lines.first_mut() {
            first.spans.insert(0, Span::raw("… "));
        }
    }

    lines
}

pub fn parse_content_with_code(
    syntax_set: &SyntaxSet,
    theme_set: &ThemeSet,
    content: &str,
    theme: &Theme,
    default_color: Color,
    syntax_theme: &str,
) -> Vec<Line<'static>> {
    render_message_segments(
        parse_message_segments(content),
        syntax_set,
        theme_set,
        theme,
        default_color,
        syntax_theme,
    )
}

fn theme_render_cache_key(theme: &Theme) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{}",
        theme.input, theme.output, theme.foreground, theme.accent, theme.title, theme.syntax_theme
    )
}

pub fn refresh_chat_render_cache(chat_state: &mut super::app::ChatState, theme: &Theme) {
    let theme_key = theme_render_cache_key(theme);
    let visible_messages = chat_state
        .messages
        .iter()
        .filter(|msg| msg.role != "system")
        .collect::<Vec<_>>();

    let theme_changed = chat_state.render_cache_theme_key != theme_key;
    let cache_changed = theme_changed
        || chat_state.rendered_message_cache.len() != visible_messages.len()
        || visible_messages
            .iter()
            .zip(chat_state.rendered_message_cache.iter())
            .any(|(msg, cached)| msg.role != cached.role || msg.content != cached.content);

    if !cache_changed {
        return;
    }

    let mut rendered_message_cache = Vec::with_capacity(visible_messages.len());

    for msg in visible_messages.iter() {
        let mut lines = Vec::new();
        append_rendered_message_lines(&mut lines, msg, theme, theme.input, theme.output);

        rendered_message_cache.push(super::app::RenderedMessageBlock {
            role: msg.role.clone(),
            content: msg.content.clone(),
            lines,
        });
    }

    chat_state.rendered_message_cache = rendered_message_cache;
    chat_state.rendered_transcript_lines.clear();
    chat_state.render_cache_theme_key = theme_key;
}

pub fn draw(frame: &mut Frame, app: &TuiApp, theme: &Theme) {
    let area = frame.area();
    let compact = compact_layout(area);
    let compact_menu =
        matches!(app.state, AppState::Menu(_)) && area.height <= MAIN_MENU_ITEM_COUNT as u16 + 4;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(FOOTER_HEIGHT)])
        .split(area);

    let hide_footer = !matches!(app.state, AppState::Chat(_)) && (compact || compact_menu);
    let main_area = if hide_footer { area } else { chunks[0] };
    let footer_area = chunks[1];

    match &app.state {
        AppState::Menu(selected) => {
            draw_zen_menu(frame, *selected, theme, main_area, app.show_menu_logo)
        }
        AppState::Chat(chat_state) => {
            let show_sidebar = chat_state.sidebar_visible && area.width > COMPACT_WIDTH_THRESHOLD;
            let chat_area = if show_sidebar {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
                    .split(area);
                draw_zen_sidebar(frame, &chat_state.sidebar_sections, theme, chunks[0]);
                chunks[1]
            } else {
                area
            };

            let mut has_plan = chat_state
                .active_plan
                .as_ref()
                .is_some_and(|plan| !plan.items.is_empty() || plan.explanation.is_some());
            let agents_focused = matches!(
                chat_state.navigation_focus,
                super::app::NavigationFocus::Agents
            );
            let has_agent_sources = chat_state
                .active_agents
                .as_ref()
                .is_some_and(|agents| !agents.sources.is_empty());
            let mut has_agents =
                chat_state.agents_panel_expanded || agents_focused || has_agent_sources;
            let mut has_review = chat_state.active_review.is_some();
            let command_output_display = derive_command_output_display(chat_state);
            let mut has_command_output = command_output_display.is_some();
            let mut has_chat_loop =
                !has_plan && should_render_chat_loop_panel(&chat_state.loop_state);
            chat_state.command_output_area.set(None);
            let mut plan_height = chat_state
                .active_plan
                .as_ref()
                .map(plan_panel_height)
                .map(|height| if compact { height.min(5) } else { height })
                .unwrap_or(0);
            let mut command_output_height = command_output_display
                .as_ref()
                .map(|output| command_output_panel_height(output, compact))
                .unwrap_or(0);
            let mut chat_loop_height = if has_chat_loop {
                let height = chat_loop_panel_height(&chat_state.loop_state);
                if compact {
                    height.min(3)
                } else {
                    height
                }
            } else {
                0
            };
            let mut agents_height = chat_state
                .active_agents
                .as_ref()
                .map(|agents| {
                    let height = agents_panel_height(
                        agents,
                        chat_state.agents_panel_expanded,
                        agents_focused,
                        compact,
                    );
                    if chat_state.input.contains('\n') && !agents_focused {
                        height.min(1)
                    } else {
                        height
                    }
                })
                .unwrap_or(if chat_state.agents_panel_expanded {
                    if compact {
                        4
                    } else {
                        6
                    }
                } else {
                    1
                });
            let mut review_height = chat_state
                .active_review
                .as_ref()
                .map(|review| review_panel_height(review, chat_state.review_selected))
                .map(|height| if compact { height.min(5) } else { height })
                .unwrap_or(0);
            let input_height =
                input_area_height(&chat_state.input, chat_area.width, chat_area.height);

            let summary_height = if compact { 1 } else { 2 };
            let message_min_height = if compact { 3 } else { 5 };
            let fixed_height = summary_height
                + message_min_height
                + input_height
                + if has_review { review_height } else { 0 }
                + if has_command_output {
                    command_output_height
                } else {
                    0
                }
                + if has_chat_loop { chat_loop_height } else { 0 }
                + if has_plan { plan_height } else { 0 }
                + if has_agents { agents_height } else { 0 };
            let mut overflow = fixed_height.saturating_sub(chat_area.height);
            if overflow > 0 && has_agents && !agents_focused {
                overflow = overflow.saturating_sub(agents_height);
                has_agents = false;
                agents_height = 0;
            }
            if overflow > 0 && has_chat_loop {
                overflow = overflow.saturating_sub(chat_loop_height);
                has_chat_loop = false;
                chat_loop_height = 0;
            }
            if overflow > 0
                && has_plan
                && !matches!(
                    chat_state.navigation_focus,
                    super::app::NavigationFocus::PlanSteps | super::app::NavigationFocus::PlanJobs
                )
            {
                overflow = overflow.saturating_sub(plan_height);
                has_plan = false;
                plan_height = 0;
            }
            if overflow > 0
                && has_review
                && !matches!(
                    chat_state.navigation_focus,
                    super::app::NavigationFocus::Review
                )
            {
                overflow = overflow.saturating_sub(review_height);
                has_review = false;
                review_height = 0;
            }
            if overflow > 0 && has_command_output && !chat_state.command_output_expanded {
                has_command_output = false;
                command_output_height = 0;
            }

            let mut constraints = vec![
                Constraint::Length(summary_height),
                Constraint::Min(message_min_height),
            ];
            if has_review {
                constraints.push(Constraint::Length(review_height));
            }
            if has_command_output {
                constraints.push(Constraint::Length(command_output_height));
            }
            if has_chat_loop {
                constraints.push(Constraint::Length(chat_loop_height));
            }
            if has_plan {
                constraints.push(Constraint::Length(plan_height));
            }
            if has_agents {
                constraints.push(Constraint::Length(agents_height));
            }
            constraints.push(Constraint::Length(input_height));

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(chat_area);

            draw_chat_summary(frame, app, chat_state, theme, chunks[0], compact);

            let mut message_lines: Vec<Line<'static>> = Vec::new();
            let divider_width = chunks[1].width.saturating_sub(4);
            for (index, cached) in chat_state.rendered_message_cache.iter().enumerate() {
                message_lines.extend(cached.lines.iter().cloned());
                let has_following_message = index + 1 < chat_state.rendered_message_cache.len();
                if cached.role == "assistant" && has_following_message {
                    append_message_divider(&mut message_lines, theme, divider_width);
                }
            }

            if chat_state.awaiting_response {
                message_lines.push(Line::from(vec![Span::styled(
                    "Harper ›",
                    Style::default()
                        .fg(theme.title)
                        .add_modifier(Modifier::BOLD),
                )]));
                message_lines.push(Line::from(vec![
                    Span::styled(
                        activity_spinner_frame(app),
                        Style::default().fg(theme.accent),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        "Thinking...",
                        theme.muted_style().add_modifier(Modifier::ITALIC),
                    ),
                ]));
                message_lines.push(Line::raw(""));
            }

            let chat_block = if compact {
                Block::default().borders(Borders::NONE)
            } else {
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::horizontal(1))
            };
            let vertical_padding = 0;
            let horizontal_padding = if compact { 0 } else { 2 };
            let visible_line_capacity = chunks[1].height.saturating_sub(vertical_padding) as usize;
            let content_width = chunks[1].width.saturating_sub(horizontal_padding);
            let total_wrapped_lines = wrapped_line_count(&message_lines, content_width);
            let messages_widget = Paragraph::new(message_lines)
                .block(chat_block)
                .wrap(Wrap { trim: false });
            let max_scroll_offset = total_wrapped_lines.saturating_sub(visible_line_capacity);
            let effective_scroll_offset = chat_state.scroll_offset.min(max_scroll_offset);
            let paragraph_scroll = total_wrapped_lines
                .saturating_sub(visible_line_capacity + effective_scroll_offset)
                as u16;
            let messages_widget = messages_widget.scroll((paragraph_scroll, 0));
            chat_state.messages_area.set(Some(chunks[1]));
            frame.render_widget(messages_widget, chunks[1]);

            let mut next_panel_index = 2;
            if has_review {
                if let Some(review) = &chat_state.active_review {
                    draw_review_panel(
                        frame,
                        review,
                        chat_state.review_selected,
                        matches!(
                            chat_state.navigation_focus,
                            super::app::NavigationFocus::Review
                        ),
                        theme,
                        chunks[next_panel_index],
                    );
                }
                next_panel_index += 1;
            }
            if has_command_output {
                if let Some(output) = command_output_display.as_ref() {
                    draw_command_output_panel(
                        frame,
                        output,
                        theme,
                        chunks[next_panel_index],
                        compact,
                    );
                }
                next_panel_index += 1;
            }
            if has_chat_loop {
                draw_chat_loop_panel(
                    frame,
                    &chat_state.loop_state,
                    theme,
                    chunks[next_panel_index],
                );
                next_panel_index += 1;
            }
            if has_plan {
                if let Some(plan) = &chat_state.active_plan {
                    draw_plan_panel(
                        frame,
                        plan,
                        chat_state.plan_step_selected,
                        chat_state.plan_job_selected,
                        PlanPanelFocus {
                            focused_steps: matches!(
                                chat_state.navigation_focus,
                                super::app::NavigationFocus::PlanSteps
                            ),
                            focused_jobs: matches!(
                                chat_state.navigation_focus,
                                super::app::NavigationFocus::PlanJobs
                            ),
                            compact,
                        },
                        theme,
                        chunks[next_panel_index],
                    );
                }
                next_panel_index += 1;
            }
            if has_agents {
                draw_agents_panel(
                    frame,
                    chat_state.active_agents.as_ref(),
                    theme,
                    chunks[next_panel_index],
                    AgentsPanelViewState {
                        context_enabled: app.agents_context_enabled,
                        expanded: chat_state.agents_panel_expanded,
                        scroll_offset: chat_state.agents_scroll_offset,
                        focused: agents_focused,
                        compact,
                    },
                );
            }

            let input_block = Block::default()
                .borders(Borders::TOP)
                .border_style(theme.muted_style())
                .padding(Padding::horizontal(1));

            let input_index = 2
                + usize::from(has_review)
                + usize::from(has_command_output)
                + usize::from(has_chat_loop)
                + usize::from(has_plan)
                + usize::from(has_agents);
            let input_text = if chat_state.input.is_empty() {
                vec![Line::from(vec![
                    Span::styled("› ", Style::default().fg(theme.accent)),
                    Span::styled(
                        "Type / for commands, or write a message",
                        theme.muted_style().add_modifier(Modifier::ITALIC),
                    ),
                ])]
            } else {
                input_display_lines(
                    &chat_state.input,
                    Style::default().fg(theme.input),
                    chunks[input_index].height.saturating_sub(1) as usize,
                )
                .into_iter()
                .map(|line| {
                    let spans = line
                        .spans
                        .into_iter()
                        .enumerate()
                        .map(|(index, span)| {
                            if index == 0 {
                                Span::styled(
                                    span.content.into_owned(),
                                    Style::default().fg(theme.accent),
                                )
                            } else {
                                span
                            }
                        })
                        .collect::<Vec<_>>();
                    Line::from(spans)
                })
                .collect()
            };

            let input_widget = Paragraph::new(input_text)
                .block(input_block)
                .wrap(Wrap { trim: false });
            frame.render_widget(input_widget, chunks[input_index]);

            let completion_height = completion_popup_height(
                chat_state,
                chunks[input_index].y.saturating_sub(chat_area.y),
            );
            if completion_height > 0 {
                let popup_area = Rect {
                    x: chunks[input_index].x,
                    y: chunks[input_index].y.saturating_sub(completion_height),
                    width: chunks[input_index].width.min(48),
                    height: completion_height,
                };
                frame.render_widget(Clear, popup_area);
                draw_completion_popup(frame, chat_state, theme, popup_area);
            }
        }
        AppState::Sessions(sessions, selected) => draw_sessions(
            frame,
            sessions,
            *selected,
            app.auth_session.is_some() && app.auth_server_base_url.is_some(),
            theme,
            main_area,
        ),
        AppState::ExportSessions(sessions, selected) => {
            draw_export_sessions(frame, sessions, *selected, theme, main_area)
        }
        AppState::Settings(selected) => draw_settings(frame, *selected, theme, main_area),
        AppState::Appearance(selected) => draw_appearance(frame, app, *selected, theme, main_area),
        AppState::Profile(selected) => draw_profile(frame, app, *selected, theme, main_area),
        AppState::ExecutionPolicy(selected) => {
            draw_execution_policy(frame, app, *selected, theme, main_area)
        }
        AppState::ViewSession(name, messages, selected) => {
            draw_view_session(frame, name, messages, *selected, theme, main_area)
        }
        AppState::Stats(stats) => draw_stats(frame, stats, theme, main_area),
    }

    if !matches!(app.state, AppState::Chat(_)) && !hide_footer {
        draw_zen_footer(frame, app, theme, footer_area);
    }

    if let Some(approval) = &app.pending_approval {
        draw_approval(frame, approval, theme);
    }

    if let AppState::Chat(chat_state) = &app.state {
        if chat_state.plan_steps_expanded {
            draw_plan_steps_browser(frame, chat_state, theme);
        }
        if chat_state.plan_jobs_expanded {
            draw_plan_jobs_browser(frame, chat_state, theme);
        }
        if chat_state.command_output_expanded {
            draw_command_output_browser(frame, chat_state, theme);
        }
    }

    if let Some(msg) = &app.message {
        draw_message_overlay(frame, msg, app.help_selected, theme);
    }
}

fn render_diff_lines(content: &str, theme: &Theme) -> Vec<Line<'static>> {
    content
        .lines()
        .map(|line| {
            let style = if line.starts_with("+++") || line.starts_with("---") {
                theme.info_style().add_modifier(Modifier::BOLD)
            } else if line.starts_with('+') {
                Style::default().fg(theme.success)
            } else if line.starts_with('-') {
                Style::default().fg(theme.error)
            } else if line.starts_with("@@") {
                theme.accent_style().add_modifier(Modifier::BOLD)
            } else if line.starts_with("diff --git") || line.starts_with("index ") {
                theme.muted_style().add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            Line::styled(line.to_string(), style)
        })
        .collect()
}

fn parse_message_segments(content: &str) -> Vec<MessageSegment> {
    let raw_lines: Vec<&str> = content.lines().collect();
    let mut segments = Vec::new();
    let mut paragraph = Vec::new();
    let mut idx = 0usize;

    let flush_paragraph = |paragraph: &mut Vec<&str>, segments: &mut Vec<MessageSegment>| {
        if paragraph.is_empty() {
            return;
        }
        let text = paragraph.join("\n");
        if !text.trim().is_empty() {
            segments.push(MessageSegment::Paragraph(text));
        }
        paragraph.clear();
    };

    while idx < raw_lines.len() {
        let raw_line = raw_lines[idx];
        let trimmed = raw_line.trim_end();
        let normalized = trimmed.trim();

        if normalized.is_empty() {
            flush_paragraph(&mut paragraph, &mut segments);
            if !matches!(segments.last(), Some(MessageSegment::Blank)) {
                segments.push(MessageSegment::Blank);
            }
            idx += 1;
            continue;
        }

        if let Some(rest) = normalized.strip_prefix("```") {
            flush_paragraph(&mut paragraph, &mut segments);
            let language = if rest.trim().is_empty() {
                None
            } else {
                Some(rest.trim().to_string())
            };
            let mut code_lines = Vec::new();
            idx += 1;
            while idx < raw_lines.len() && raw_lines[idx].trim_end() != "```" {
                code_lines.push(raw_lines[idx].trim_end());
                idx += 1;
            }
            if idx < raw_lines.len() {
                idx += 1;
            }
            segments.push(MessageSegment::CodeBlock {
                language,
                content: code_lines.join("\n"),
            });
            continue;
        }

        if normalized.starts_with('#') {
            flush_paragraph(&mut paragraph, &mut segments);
            segments.push(MessageSegment::Heading(
                normalized.trim_start_matches('#').trim().to_string(),
            ));
            idx += 1;
            continue;
        }

        let is_structured_line = normalized.starts_with("- ")
            || normalized.starts_with("* ")
            || normalized.starts_with("> ")
            || normalized
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
                && normalized.contains(". ");
        if is_structured_line {
            flush_paragraph(&mut paragraph, &mut segments);
            segments.push(MessageSegment::StructuredLine(normalized.to_string()));
            idx += 1;
            continue;
        }

        if let Some((language, code, consumed)) = infer_unfenced_code_lines(&raw_lines[idx..]) {
            flush_paragraph(&mut paragraph, &mut segments);
            segments.push(MessageSegment::CodeBlock {
                language: Some(language.to_string()),
                content: code,
            });
            idx += consumed;
            continue;
        }

        paragraph.push(trimmed);
        idx += 1;
    }

    flush_paragraph(&mut paragraph, &mut segments);
    segments
}

fn render_message_segments(
    segments: Vec<MessageSegment>,
    syntax_set: &SyntaxSet,
    theme_set: &ThemeSet,
    theme: &Theme,
    default_color: Color,
    syntax_theme: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for segment in segments {
        match segment {
            MessageSegment::Paragraph(text) => {
                lines.extend(normalize_plain_message_lines(&text, default_color, theme));
            }
            MessageSegment::Heading(text) => {
                lines.push(parse_inline_markdown_line(&text, default_color, true));
            }
            MessageSegment::StructuredLine(text) => {
                lines.push(parse_inline_markdown_line(&text, default_color, false));
            }
            MessageSegment::CodeBlock { language, content } => match language.as_deref() {
                Some(language) if language.eq_ignore_ascii_case("diff") => {
                    lines.extend(render_diff_lines(&content, theme));
                }
                Some(language) => {
                    lines.extend(highlight_code_lines(
                        syntax_set,
                        theme_set,
                        language,
                        &content,
                        syntax_theme,
                    ));
                }
                None => {
                    lines.extend(normalize_plain_message_lines(
                        &content,
                        default_color,
                        theme,
                    ));
                }
            },
            MessageSegment::Blank => lines.push(Line::raw("")),
        }
    }
    lines
}

fn normalize_plain_message_lines(content: &str, color: Color, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let raw_lines: Vec<&str> = content.lines().collect();

    let flush_paragraph = |paragraph: &mut Vec<&str>, lines: &mut Vec<Line<'static>>| {
        if paragraph.is_empty() {
            return;
        }
        let normalized = paragraph.join(" ");
        let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
        if !normalized.is_empty() {
            if let Some((language, code)) = infer_unfenced_code_block(&normalized) {
                lines.extend(highlight_code_lines(
                    &theme.syntax_set,
                    &theme.theme_set,
                    language,
                    code,
                    &theme.syntax_theme,
                ));
            } else if let Some((prefix, language, code, suffix)) =
                split_inline_code_paragraph(&normalized)
            {
                if !prefix.is_empty() {
                    lines.push(parse_inline_markdown_line(prefix, color, false));
                }
                lines.extend(highlight_code_lines(
                    &theme.syntax_set,
                    &theme.theme_set,
                    language,
                    code,
                    &theme.syntax_theme,
                ));
                if !suffix.is_empty() {
                    lines.push(parse_inline_markdown_line(suffix, color, false));
                }
            } else {
                lines.push(parse_inline_markdown_line(&normalized, color, false));
            }
        }
        paragraph.clear();
    };

    let mut idx = 0usize;
    while idx < raw_lines.len() {
        let raw_line = raw_lines[idx];
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut lines);
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            idx += 1;
            continue;
        }

        let is_structured_line = trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("> ")
            || trimmed.starts_with('#')
            || trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                && trimmed.contains(". ");

        if looks_like_code_intro(trimmed)
            && raw_lines
                .get(idx + 1)
                .is_some_and(|next| infer_unfenced_code_lines(std::slice::from_ref(next)).is_some())
        {
            flush_paragraph(&mut paragraph, &mut lines);
            lines.push(parse_inline_markdown_line(trimmed, color, false));
            idx += 1;
            continue;
        }

        if let Some((language, code, consumed)) = infer_unfenced_code_lines(&raw_lines[idx..]) {
            flush_paragraph(&mut paragraph, &mut lines);
            lines.extend(highlight_code_lines(
                &theme.syntax_set,
                &theme.theme_set,
                language,
                &code,
                &theme.syntax_theme,
            ));
            idx += consumed;
            continue;
        }

        if is_structured_line {
            flush_paragraph(&mut paragraph, &mut lines);
            lines.push(parse_inline_markdown_line(
                trimmed,
                color,
                trimmed.starts_with('#'),
            ));
            idx += 1;
            continue;
        }

        paragraph.push(trimmed);
        idx += 1;
    }

    flush_paragraph(&mut paragraph, &mut lines);
    lines
}

fn parse_inline_markdown_line(content: &str, color: Color, heading: bool) -> Line<'static> {
    let base_style = if heading {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    let mut spans = Vec::new();
    let mut remaining = content;
    while !remaining.is_empty() {
        let bold_index = remaining.find("**");
        let code_index = remaining.find('`');
        let next_marker = match (bold_index, code_index) {
            (Some(bold), Some(code)) => {
                if bold < code {
                    Some((bold, "bold"))
                } else {
                    Some((code, "code"))
                }
            }
            (Some(bold), None) => Some((bold, "bold")),
            (None, Some(code)) => Some((code, "code")),
            (None, None) => None,
        };

        let Some((index, kind)) = next_marker else {
            if !remaining.is_empty() {
                spans.push(Span::styled(remaining.to_string(), base_style));
            }
            break;
        };

        if index > 0 {
            spans.push(Span::styled(remaining[..index].to_string(), base_style));
        }

        remaining = &remaining[index..];
        match kind {
            "bold" => {
                if let Some(end) = remaining[2..].find("**") {
                    let text = &remaining[2..2 + end];
                    spans.push(Span::styled(
                        text.to_string(),
                        base_style.add_modifier(Modifier::BOLD),
                    ));
                    remaining = &remaining[2 + end + 2..];
                } else {
                    spans.push(Span::styled(remaining.to_string(), base_style));
                    break;
                }
            }
            "code" => {
                if let Some(end) = remaining[1..].find('`') {
                    let text = &remaining[1..1 + end];
                    spans.push(Span::styled(
                        text.to_string(),
                        base_style
                            .fg(Color::Rgb(193, 223, 173))
                            .add_modifier(Modifier::ITALIC),
                    ));
                    remaining = &remaining[1 + end + 1..];
                } else {
                    spans.push(Span::styled(remaining.to_string(), base_style));
                    break;
                }
            }
            _ => break,
        }
    }

    Line::from(spans)
}

fn infer_unfenced_code_lines(lines: &[&str]) -> Option<(&'static str, String, usize)> {
    let first = lines.first()?.trim_end();
    if first.trim().is_empty() || !is_code_like_line(first.trim()) {
        return None;
    }
    let language = detect_code_language(first.trim())?;

    let mut collected = vec![first];
    let mut consumed = 1usize;
    while let Some(next) = lines.get(consumed) {
        let trimmed = next.trim_end();
        if trimmed.trim().is_empty() {
            break;
        }
        if is_code_like_continuation(trimmed) || is_code_like_line(trimmed.trim()) {
            collected.push(trimmed);
            consumed += 1;
        } else {
            break;
        }
    }

    Some((language, collected.join("\n"), consumed))
}

fn infer_unfenced_code_block(normalized: &str) -> Option<(&'static str, &str)> {
    let language = detect_code_language(normalized)?;
    if is_code_like_line(normalized) {
        Some((language, normalized))
    } else {
        None
    }
}

fn split_inline_code_paragraph(normalized: &str) -> Option<(&str, &'static str, &str, &str)> {
    let colon = normalized.find(':')?;
    let (prefix, after_prefix) = normalized.split_at(colon + 1);
    if !looks_like_code_intro(prefix.trim()) {
        return None;
    }

    let after = after_prefix.trim();
    if let Some(rest) = after.strip_prefix("print(") {
        let mut depth = 1i32;
        let mut end_index = "print(".len();
        for ch in rest.chars() {
            end_index += ch.len_utf8();
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let code = &after[..end_index];
                        let suffix = after[end_index..].trim_start();
                        return Some((prefix.trim_end(), "py", code, suffix));
                    }
                }
                _ => {}
            }
        }
    }

    None
}

fn looks_like_code_intro(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower.starts_with("here is")
        || lower.contains("script:")
        || lower.contains("code:")
        || lower.contains("command:")
}

fn is_code_like_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("fn ")
        || trimmed.starts_with("pub fn ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("trait ")
        || trimmed.starts_with("impl ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("async fn ")
        || trimmed.starts_with("#!/bin/")
        || trimmed.starts_with("print(")
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.contains(" = ")
        || trimmed.ends_with('{')
}

fn is_code_like_continuation(line: &str) -> bool {
    let trimmed = line.trim();
    line.starts_with(' ')
        || line.starts_with('\t')
        || trimmed.starts_with('}')
        || trimmed.starts_with(')')
        || trimmed.starts_with("else")
        || trimmed.starts_with("elif")
        || trimmed.starts_with("return ")
        || trimmed.starts_with("println!(")
        || trimmed.starts_with("print(")
}

fn detect_code_language(content: &str) -> Option<&'static str> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("diff --git")
        || trimmed.starts_with("@@")
        || trimmed.contains("\n@@ ")
        || (trimmed.starts_with('+') || trimmed.starts_with('-')) && trimmed.lines().count() > 1
    {
        return Some("diff");
    }
    if trimmed.starts_with("fn ")
        || trimmed.starts_with("pub fn ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("trait ")
        || trimmed.starts_with("impl ")
        || trimmed.contains("println!(")
    {
        return Some("rs");
    }
    if trimmed.starts_with("def ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("print(")
    {
        return Some("py");
    }
    if trimmed.starts_with("#!/bin/")
        || trimmed.starts_with("echo ")
        || trimmed.starts_with("cargo ")
        || trimmed.starts_with("git ")
        || trimmed.starts_with("cd ")
    {
        return Some("sh");
    }
    if trimmed.starts_with('[') && trimmed.contains(']') || trimmed.contains(" = ") {
        return Some("toml");
    }
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        return Some("json");
    }
    None
}

fn append_rendered_message_lines(
    message_lines: &mut Vec<Line<'static>>,
    msg: &harper_core::core::Message,
    theme: &Theme,
    user_color: Color,
    assistant_color: Color,
) {
    let label = match msg.role.as_str() {
        "user" => "User ›",
        "assistant" => "Harper ›",
        _ => "System ›",
    };
    let label_style = match msg.role.as_str() {
        "user" => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        "assistant" => Style::default()
            .fg(theme.title)
            .add_modifier(Modifier::BOLD),
        _ => theme.muted_style(),
    };
    let default_color = if msg.role == "user" {
        user_color
    } else {
        assistant_color
    };

    message_lines.push(Line::from(vec![Span::styled(label, label_style)]));
    message_lines.extend(parse_content_with_code(
        &theme.syntax_set,
        &theme.theme_set,
        &msg.content,
        theme,
        default_color,
        &theme.syntax_theme,
    ));
    message_lines.push(Line::raw(""));
}

fn append_message_divider(message_lines: &mut Vec<Line<'static>>, theme: &Theme, width: u16) {
    let width = width.max(8) as usize;
    message_lines.push(Line::from(vec![Span::styled(
        "─".repeat(width),
        theme.muted_style(),
    )]));
    message_lines.push(Line::raw(""));
}

fn review_panel_height(review: &ReviewState, selected: usize) -> u16 {
    let findings = review.findings.len().min(3);
    let model_line = usize::from(review.model.is_some());
    let detail_lines = review
        .findings
        .get(selected)
        .map(|finding| 2 + finding.message.lines().count().min(3))
        .unwrap_or(0);
    (findings + model_line + detail_lines + 4) as u16
}

fn header_widget_enabled(app: &TuiApp, widget: super::app::HeaderWidget) -> bool {
    app.header_widgets.contains(&widget)
}

fn push_header_separator(spans: &mut Vec<Span<'static>>, _theme: &Theme) {
    if !spans.is_empty() {
        spans.push(Span::raw("  "));
    }
}

fn draw_chat_summary(
    frame: &mut Frame,
    app: &TuiApp,
    chat_state: &super::app::ChatState,
    theme: &Theme,
    area: Rect,
    compact: bool,
) {
    let plan_status = chat_state
        .active_plan
        .as_ref()
        .map_or("plan: none".to_string(), |plan| {
            let total = plan.items.len();
            let completed = plan
                .items
                .iter()
                .filter(|item| matches!(item.status, PlanStepStatus::Completed))
                .count();
            let blocked = plan
                .items
                .iter()
                .filter(|item| matches!(item.status, PlanStepStatus::Blocked))
                .count();
            let in_progress = plan
                .items
                .iter()
                .any(|item| matches!(item.status, PlanStepStatus::InProgress));
            if total == 0 {
                "plan: empty".to_string()
            } else if blocked > 0 {
                format!("plan: {}/{} done, {} blocked", completed, total, blocked)
            } else if in_progress {
                format!("plan: {}/{} done, active", completed, total)
            } else {
                format!("plan: {}/{} done", completed, total)
            }
        });
    let agents_status = if !app.agents_context_enabled {
        "agents: off".to_string()
    } else {
        chat_state
            .active_agents
            .as_ref()
            .map_or("agents: none".to_string(), |agents| {
                format!("agents: {} sections", agents.effective_rule_sections.len())
            })
    };
    let web_status = if chat_state.web_search_enabled {
        "web: on".to_string()
    } else {
        "web: off".to_string()
    };
    let auth_status = app.auth_status_label();
    let focus_status = format!("focus: {}", chat_state.navigation_focus_label());
    let model_status = if app.model_label.is_empty() {
        None
    } else {
        Some(format!("model: {}", app.model_label))
    };
    let cwd_status = if app.current_working_dir.is_empty() {
        None
    } else {
        Some(format!("cwd: {}", app.current_working_dir))
    };
    let strategy_status = format!(
        "strategy: {}",
        settings::execution_strategy_name(app.execution_strategy)
    );
    let approval_status = if app.pending_approval.is_some() {
        Some("approval: pending".to_string())
    } else {
        None
    };
    let update_status = app.update_status.clone();
    let activity_status = app.activity_status.as_ref().map(|status| {
        format!(
            "activity: {} {}",
            activity_spinner_frame(app),
            truncate_chat_summary(status, 32)
        )
    });
    let last_action = latest_action_summary(app, chat_state);
    let last_rule_source =
        if chat_state.active_agents.is_some() && !chat_state.agents_panel_expanded {
            latest_rule_source(chat_state)
        } else {
            None
        };

    let mut spans: Vec<Span<'static>> = Vec::new();
    if header_widget_enabled(app, super::app::HeaderWidget::Session) {
        spans.push(Span::styled("session ", theme.muted_style()));
        spans.push(Span::styled(
            truncate_chat_summary(&chat_state.session_id, 12),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Plan) {
        push_header_separator(&mut spans, theme);
        spans.push(Span::styled(plan_status, theme.muted_style()));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Agents) {
        push_header_separator(&mut spans, theme);
        spans.push(Span::styled(agents_status, theme.muted_style()));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Web) {
        push_header_separator(&mut spans, theme);
        spans.push(Span::styled(web_status, theme.muted_style()));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Auth) {
        push_header_separator(&mut spans, theme);
        spans.push(Span::styled(auth_status, theme.muted_style()));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Focus) {
        push_header_separator(&mut spans, theme);
        spans.push(Span::styled(focus_status, theme.muted_style()));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Model) {
        if let Some(status) = model_status {
            push_header_separator(&mut spans, theme);
            spans.push(Span::styled(
                status,
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Cwd) {
        if let Some(status) = cwd_status {
            push_header_separator(&mut spans, theme);
            spans.push(Span::styled(status, theme.muted_style()));
        }
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Strategy) {
        push_header_separator(&mut spans, theme);
        spans.push(Span::styled(
            strategy_status,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Approval) {
        if let Some(status) = approval_status {
            push_header_separator(&mut spans, theme);
            spans.push(Span::styled(
                status,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Update) {
        if let Some(status) = update_status {
            push_header_separator(&mut spans, theme);
            spans.push(Span::styled(
                status,
                Style::default()
                    .fg(theme.muted)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    if header_widget_enabled(app, super::app::HeaderWidget::Activity) {
        if let Some(status) = activity_status {
            push_header_separator(&mut spans, theme);
            spans.push(Span::styled(
                status,
                Style::default()
                    .fg(theme.output)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    let line = Line::from(spans);
    if compact {
        frame.render_widget(
            Paragraph::new(line)
                .style(Style::default().fg(theme.foreground))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let mut detail_spans = Vec::new();
    if let Some(action) = last_action {
        detail_spans.push(Span::styled("last ", theme.muted_style()));
        detail_spans.push(Span::styled(
            truncate_chat_summary(&action, 36),
            Style::default().fg(theme.foreground),
        ));
    }
    if let Some(source) = last_rule_source {
        if !detail_spans.is_empty() {
            detail_spans.push(Span::raw("  "));
        }
        detail_spans.push(Span::styled(
            truncate_chat_summary(&source, 24),
            theme.muted_style().add_modifier(Modifier::ITALIC),
        ));
    }

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme.muted_style())
        .padding(Padding::horizontal(1));
    let mut lines = vec![line];
    if !detail_spans.is_empty() {
        lines.push(Line::from(detail_spans));
    }
    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

fn truncate_chat_summary(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        value.to_string()
    } else {
        format!("{}...", &value[..max_len])
    }
}

fn activity_spinner_frame(app: &TuiApp) -> &'static str {
    const FRAMES: [&str; 4] = ["·", "◜", "◝", "◞"];
    let elapsed = app
        .activity_started_at
        .map(|started| started.elapsed().as_millis() / 120)
        .unwrap_or(0);
    FRAMES[(elapsed as usize) % FRAMES.len()]
}

fn latest_action_summary(app: &TuiApp, chat_state: &super::app::ChatState) -> Option<String> {
    if let Some(approval) = &app.pending_approval {
        return Some(approval.command.clone());
    }

    if let Some(command_output) = &chat_state.command_output {
        return Some(command_output.command.clone());
    }

    for msg in chat_state.messages.iter().rev() {
        for line in msg.content.lines().rev() {
            let trimmed = line.trim();
            if let Some(command) = trimmed.strip_prefix("$ ") {
                return Some(command.to_string());
            }
        }
    }

    None
}

fn latest_rule_source(chat_state: &super::app::ChatState) -> Option<String> {
    chat_state.active_agents.as_ref().and_then(|agents| {
        agents
            .effective_rule_sections
            .iter()
            .rev()
            .find_map(|section| {
                section.rules.last().map(|rule| {
                    let source_path = &rule.source_path;
                    std::env::current_dir()
                        .ok()
                        .and_then(|cwd| {
                            source_path
                                .strip_prefix(cwd)
                                .ok()
                                .map(|path| path.display().to_string())
                        })
                        .or_else(|| {
                            source_path
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                        })
                        .unwrap_or_else(|| source_path.display().to_string())
                })
            })
    })
}

fn plan_panel_height(plan: &PlanState) -> u16 {
    let mut lines = plan.items.len().min(4) + 2;
    if plan.explanation.is_some() {
        lines += 1;
    }
    if plan
        .runtime
        .as_ref()
        .is_some_and(PlanRuntime::has_active_state)
    {
        lines += 1;
    }
    if let Some(runtime) = &plan.runtime {
        if runtime.loop_stage.is_some()
            || runtime.last_outcome.is_some()
            || runtime.last_feedback.is_some()
        {
            lines += plan_loop_state_line_count(runtime);
        }
        if runtime.followup.is_some() {
            lines += 2;
        }
        if !runtime.followup_history.is_empty() {
            lines += runtime.followup_history.len().min(2) + 1;
        }
        lines += runtime.jobs.len().min(3);
        if runtime.jobs.len() > 3 {
            lines += 1;
        }
        if !runtime.jobs.is_empty() {
            lines += 1;
            if runtime.jobs.iter().any(|job| {
                job.output_preview
                    .as_ref()
                    .is_some_and(|preview| !preview.is_empty())
            }) {
                lines += 2;
            }
        }
    }
    lines as u16
}

fn completion_popup_height(chat_state: &super::app::ChatState, available_height: u16) -> u16 {
    if chat_state.input.starts_with('/') && !chat_state.completion_candidates.is_empty() {
        ((chat_state.completion_candidates.len() as u16) + 2)
            .min(MAX_COMPLETION_POPUP_HEIGHT)
            .min(available_height)
    } else {
        0
    }
}

fn completion_visible_range(
    total: usize,
    selected: usize,
    visible_capacity: usize,
) -> (usize, usize) {
    selected_visible_range(total, selected, visible_capacity)
}

fn selected_visible_range(
    total: usize,
    selected: usize,
    visible_capacity: usize,
) -> (usize, usize) {
    if total == 0 || visible_capacity == 0 {
        return (0, 0);
    }

    let selected = selected.min(total.saturating_sub(1));
    let visible_capacity = visible_capacity.min(total);
    let half_window = visible_capacity / 2;
    let max_start = total.saturating_sub(visible_capacity);
    let start = selected.saturating_sub(half_window).min(max_start);
    let end = start + visible_capacity;

    (start, end)
}

fn draw_completion_popup(
    frame: &mut Frame,
    chat_state: &super::app::ChatState,
    theme: &Theme,
    area: Rect,
) {
    let selected_index = chat_state
        .completion_candidates
        .iter()
        .position(|candidate| candidate == &chat_state.input)
        .unwrap_or(chat_state.completion_index)
        .min(chat_state.completion_candidates.len().saturating_sub(1));
    let visible_capacity = area.height.saturating_sub(2) as usize;
    let (window_start, window_end) = completion_visible_range(
        chat_state.completion_candidates.len(),
        selected_index,
        visible_capacity,
    );
    let items = chat_state
        .completion_candidates
        .iter()
        .skip(window_start)
        .take(window_end.saturating_sub(window_start))
        .map(|candidate| {
            let style = if candidate == &chat_state.input {
                theme
                    .selection_style()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(theme.background).fg(theme.foreground)
            };
            ListItem::new(candidate.clone()).style(style)
        })
        .collect::<Vec<_>>();

    let widget = List::new(items).block(
        Block::default()
            .title(" Slash Commands ")
            .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(theme.accent_style())
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.background)),
    );
    frame.render_widget(widget, area);
}

#[derive(Clone)]
struct CommandOutputDisplay {
    command: String,
    content: String,
    has_error: bool,
    status_label: String,
    total_line_count: usize,
}

fn derive_command_output_display(
    chat_state: &super::app::ChatState,
) -> Option<CommandOutputDisplay> {
    if let Some(output) = &chat_state.command_output {
        let trimmed = output.content.trim_end();
        let content = if trimmed.trim().is_empty() {
            "No output yet".to_string()
        } else {
            trimmed.to_string()
        };
        let status_label = chat_state
            .active_plan
            .as_ref()
            .and_then(|plan| plan.runtime.as_ref())
            .and_then(|runtime| runtime.active_status.clone())
            .unwrap_or_else(|| {
                if output.done {
                    "done".to_string()
                } else {
                    "live".to_string()
                }
            });
        let total_line_count = content.lines().count().max(1);
        return Some(CommandOutputDisplay {
            command: output.command.clone(),
            content,
            has_error: output.has_error,
            status_label,
            total_line_count,
        });
    }

    let runtime = chat_state
        .active_plan
        .as_ref()
        .and_then(|plan| plan.runtime.as_ref())?;
    let job = runtime
        .active_job_id
        .as_deref()
        .and_then(|job_id| runtime.jobs.iter().rev().find(|job| job.job_id == job_id))
        .or_else(|| runtime.jobs.last())?;

    let transcript = job.output_transcript.trim_end();
    let preview = job.output_preview.as_deref().unwrap_or_default().trim_end();
    let (content, total_line_count) = if transcript.trim().is_empty() {
        let preview_content = if preview.trim().is_empty() {
            "No output recorded yet".to_string()
        } else {
            preview.to_string()
        };
        let preview_lines = preview_content.lines().count().max(1);
        (preview_content, preview_lines)
    } else {
        let transcript_content = transcript.to_string();
        let transcript_lines = transcript_content.lines().count().max(1);
        (transcript_content, transcript_lines)
    };

    Some(CommandOutputDisplay {
        command: job.command.clone().unwrap_or_else(|| job.tool.clone()),
        content,
        has_error: job.has_error_output,
        status_label: format!("{:?}", job.status).to_ascii_lowercase(),
        total_line_count,
    })
}

fn command_output_line_limit(compact: bool) -> usize {
    if compact {
        COMPACT_COMMAND_OUTPUT_LINES
    } else {
        MAX_COMMAND_OUTPUT_LINES
    }
}

fn command_output_panel_height(output: &CommandOutputDisplay, compact: bool) -> u16 {
    let line_limit = command_output_line_limit(compact);
    let visible_lines = output.total_line_count.clamp(1, line_limit);
    let truncation_line = usize::from(output.total_line_count > line_limit);
    let chrome = if compact { 3 } else { 4 };
    (visible_lines + truncation_line + chrome) as u16
}

fn draw_command_output_panel(
    frame: &mut Frame,
    output: &CommandOutputDisplay,
    theme: &Theme,
    area: Rect,
    compact: bool,
) {
    let title = format!(
        " $ Command Output ({}) • Ctrl+O maximize ",
        output.status_label
    );
    let line_limit = command_output_line_limit(compact);
    let mut lines = if compact {
        Vec::new()
    } else {
        vec![Line::styled(
            truncate_chat_summary(&output.command, 96),
            theme.muted_style().add_modifier(Modifier::ITALIC),
        )]
    };
    if output.total_line_count > line_limit {
        lines.push(Line::styled(
            format!(
                "showing last {} of {} lines",
                line_limit, output.total_line_count
            ),
            theme.muted_style(),
        ));
    }
    lines.extend(command_output_lines(output, theme, Some(line_limit)));
    let block = Block::default()
        .title(title)
        .borders(Borders::TOP)
        .border_style(theme.muted_style())
        .padding(Padding::horizontal(1));
    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

fn chat_loop_panel_height(loop_state: &super::app::ChatLoopState) -> u16 {
    (loop_state_line_count(
        loop_state.stage.as_ref(),
        loop_state.last_outcome.as_ref(),
        loop_state.last_feedback.as_deref(),
    ) + 2) as u16
}

fn should_render_chat_loop_panel(loop_state: &super::app::ChatLoopState) -> bool {
    if !loop_state.has_state() {
        return false;
    }

    !matches!(
        (loop_state.stage.as_ref(), loop_state.last_outcome.as_ref()),
        (
            Some(PlanLoopStage::Responding),
            Some(PlanLoopOutcome::Responded)
        )
    )
}

fn draw_chat_loop_panel(
    frame: &mut Frame,
    loop_state: &super::app::ChatLoopState,
    theme: &Theme,
    area: Rect,
) {
    let widget = Paragraph::new(loop_state_lines(
        loop_state.stage.as_ref(),
        loop_state.last_outcome.as_ref(),
        loop_state.last_feedback.as_deref(),
        theme,
    ))
    .block(
        Block::default()
            .title(" Loop State ")
            .borders(Borders::TOP)
            .border_style(theme.muted_style())
            .padding(Padding::horizontal(1)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

fn detect_command_output_language(command: &str, content: &str) -> Option<&'static str> {
    let normalized_command = command.to_ascii_lowercase();
    if normalized_command.contains("git diff")
        || content.starts_with("Git diff:\n")
        || content.contains("diff --git ")
        || content.contains("\n@@ ")
    {
        return Some("diff");
    }

    if normalized_command.starts_with("cat ") || normalized_command.contains(" read_file ") {
        if normalized_command.ends_with(".rs") {
            return Some("rs");
        }
        if normalized_command.ends_with(".toml") {
            return Some("toml");
        }
        if normalized_command.ends_with(".py") {
            return Some("py");
        }
        if normalized_command.ends_with(".ts") {
            return Some("ts");
        }
        if normalized_command.ends_with(".js") {
            return Some("js");
        }
        if normalized_command.ends_with(".json") {
            return Some("json");
        }
        if normalized_command.ends_with(".md") {
            return Some("md");
        }
        if normalized_command.ends_with(".sh") {
            return Some("sh");
        }
    }

    detect_code_language(content)
}

fn strip_command_output_prefix(content: &str) -> &str {
    content.strip_prefix("Git diff:\n").unwrap_or(content)
}

fn command_output_lines(
    output: &CommandOutputDisplay,
    theme: &Theme,
    line_limit: Option<usize>,
) -> Vec<Line<'static>> {
    let color = if output.has_error {
        Color::Rgb(245, 158, 11)
    } else {
        theme.output
    };
    let content = if output.content.trim().is_empty() {
        "No output yet"
    } else {
        output.content.trim_end()
    };
    if let Some(language) = detect_command_output_language(&output.command, content) {
        let code = strip_command_output_prefix(content);
        let highlighted = if language == "diff" {
            render_diff_lines(code, theme)
        } else {
            highlight_code_lines(
                &theme.syntax_set,
                &theme.theme_set,
                language,
                code,
                &theme.syntax_theme,
            )
        };
        if let Some(limit) = line_limit {
            let window = highlighted.len().saturating_sub(limit);
            highlighted.into_iter().skip(window).collect()
        } else {
            highlighted
        }
    } else {
        let content_lines = content.lines().collect::<Vec<_>>();
        let start = line_limit
            .map(|limit| content_lines.len().saturating_sub(limit))
            .unwrap_or(0);
        content_lines
            .into_iter()
            .skip(start)
            .map(|line| Line::styled(line.to_string(), Style::default().fg(color)))
            .collect()
    }
}

fn draw_review_panel(
    frame: &mut Frame,
    review: &ReviewState,
    selected: usize,
    focused: bool,
    theme: &Theme,
    area: Rect,
) {
    let mut lines = Vec::new();
    lines.push(Line::styled(review.summary.as_str(), theme.muted_style()));
    if let Some(model) = &review.model {
        lines.push(Line::styled(
            format!("model {}", model),
            theme.muted_style().add_modifier(Modifier::ITALIC),
        ));
    }
    for (index, finding) in review.findings.iter().take(3).enumerate() {
        let severity_color = match finding.severity.as_str() {
            "error" => Color::Rgb(239, 68, 68),
            "warning" => Color::Rgb(245, 158, 11),
            _ => theme.accent,
        };
        let marker = if index == selected { "▸" } else { "•" };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(severity_color)),
            Span::raw(" "),
            Span::styled(
                format!("{}: {}", finding.severity, finding.title),
                if index == selected {
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.foreground)
                },
            ),
        ]));
        if index == selected {
            for detail_line in finding.message.lines().take(3) {
                lines.push(Line::styled(
                    format!("    {}", truncate_chat_summary(detail_line, 88)),
                    theme.muted_style(),
                ));
            }
        } else {
            lines.push(Line::styled(
                format!("  {}", truncate_chat_summary(&finding.message, 88)),
                theme.muted_style(),
            ));
        }
    }
    if review.findings.len() > 3 {
        lines.push(Line::styled(
            format!("{} more findings", review.findings.len() - 3),
            theme.muted_style(),
        ));
    }

    let title = if focused {
        " Review Findings (focused • Y/V or ↑/↓) "
    } else {
        " Review Findings (Ctrl+F focus) "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::TOP)
        .border_style(theme.muted_style())
        .padding(Padding::horizontal(1));
    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

fn draw_plan_panel(
    frame: &mut Frame,
    plan: &PlanState,
    selected_step: usize,
    selected_job: usize,
    focus: PlanPanelFocus,
    theme: &Theme,
    area: Rect,
) {
    let mut lines = Vec::new();

    if let Some(explanation) = plan.explanation.as_ref().filter(|_| !focus.compact) {
        lines.push(Line::styled(explanation.as_str(), theme.muted_style()));
    }
    if let Some(runtime) = plan
        .runtime
        .as_ref()
        .filter(|runtime| runtime.has_active_state())
    {
        lines.push(Line::styled(
            format_plan_runtime(runtime),
            theme.muted_style().add_modifier(Modifier::ITALIC),
        ));
    }
    if let Some(runtime) = plan.runtime.as_ref().filter(|_| !focus.compact) {
        lines.extend(plan_loop_state_lines(runtime, theme));
        lines.extend(plan_followup_lines(runtime, theme));
        lines.extend(plan_followup_history_lines(runtime, theme));
        lines.extend(plan_job_lines(
            runtime,
            selected_job,
            focus.focused_jobs,
            theme,
        ));
        if let Some(selected) = runtime
            .jobs
            .get(selected_job.min(runtime.jobs.len().saturating_sub(1)))
        {
            lines.push(Line::styled(
                format!(
                    "job {} • {}",
                    &selected.job_id[..selected.job_id.len().min(8)],
                    selected.tool
                ),
                theme.muted_style(),
            ));
            if let Some(preview) = selected
                .output_preview
                .as_ref()
                .filter(|preview| !preview.is_empty())
            {
                let preview_color = if selected.has_error_output {
                    Color::Rgb(245, 158, 11)
                } else {
                    theme.output
                };
                for line in preview.lines().take(2) {
                    lines.push(Line::styled(
                        format!("  {}", truncate_chat_summary(line, 88)),
                        Style::default().fg(preview_color),
                    ));
                }
            }
        }
    }

    let step_capacity = if focus.compact { 2 } else { 4 };
    let step_window_start = selected_step
        .saturating_sub(step_capacity / 2)
        .min(plan.items.len().saturating_sub(step_capacity));
    let step_window_end = (step_window_start + step_capacity).min(plan.items.len());
    for (offset, item) in plan.items[step_window_start..step_window_end]
        .iter()
        .enumerate()
    {
        let is_selected = focus.focused_steps && step_window_start + offset == selected_step;
        let (marker, color) = match item.status {
            PlanStepStatus::Pending => ("○", theme.muted),
            PlanStepStatus::InProgress => ("◐", theme.accent),
            PlanStepStatus::Completed => ("●", theme.output),
            PlanStepStatus::Blocked => ("■", Color::Rgb(245, 158, 11)),
        };
        let text_style = if is_selected {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.foreground)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(item.step.as_str(), text_style),
        ]));
    }

    if plan.items.len() > step_capacity {
        lines.push(Line::styled(
            format!("{} more steps", plan.items.len() - step_capacity),
            theme.muted_style(),
        ));
    }

    let title = if focus.compact {
        " Plan "
    } else if focus.focused_steps {
        " Plan (Ctrl+S • C complete • I in-progress • B blocked • R retry • U replan • K ack • X clear) "
    } else if focus.focused_jobs {
        " Plan (jobs focused • Y/V or ↑/↓) "
    } else {
        " Plan (Ctrl+S steps • P jobs) "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::TOP)
        .border_style(theme.muted_style())
        .padding(Padding::horizontal(1));
    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

#[derive(Clone, Copy)]
struct PlanPanelFocus {
    focused_steps: bool,
    focused_jobs: bool,
    compact: bool,
}

#[derive(Clone, Copy)]
struct AgentsPanelViewState {
    context_enabled: bool,
    expanded: bool,
    scroll_offset: usize,
    focused: bool,
    compact: bool,
}

fn agents_panel_height(
    agents: &ResolvedAgents,
    expanded: bool,
    focused: bool,
    compact: bool,
) -> u16 {
    if !expanded && !focused {
        return 1;
    }

    if compact {
        return if expanded { 4 } else { 2 };
    }

    if expanded {
        return 7;
    }

    let section_lines: usize = agents
        .effective_rule_sections
        .iter()
        .take(2)
        .map(|section| 2 + usize::from(!section.rules.is_empty()))
        .sum();
    let overflow_lines = usize::from(agents.effective_rule_sections.len() > 2);
    (section_lines + overflow_lines + 2) as u16
}

fn draw_agents_panel(
    frame: &mut Frame,
    agents: Option<&ResolvedAgents>,
    theme: &Theme,
    area: Rect,
    view_state: AgentsPanelViewState,
) {
    let lines = if let Some(agents) = agents {
        if !view_state.expanded && !view_state.focused {
            Vec::new()
        } else if view_state.expanded && !view_state.compact {
            expanded_agents_lines(agents, theme, view_state.scroll_offset, area.height)
        } else {
            compact_agents_lines(agents, theme, view_state.compact)
        }
    } else {
        if view_state.context_enabled {
            vec![
                Line::styled(
                    "No active AGENTS sources yet.",
                    Style::default().fg(theme.foreground),
                ),
                Line::styled(
                    "Send a message or open a session with resolved AGENTS context.",
                    theme.muted_style(),
                ),
            ]
        } else {
            vec![
                Line::styled(
                    "AGENTS context is disabled.",
                    Style::default().fg(theme.foreground),
                ),
                Line::styled(
                    "Use /agents on to enable cwd and file-scoped AGENTS resolution.",
                    theme.muted_style(),
                ),
            ]
        }
    };

    let title = if !view_state.expanded && !view_state.focused {
        " AGENTS (Ctrl+A open) "
    } else if view_state.compact {
        " AGENTS "
    } else if view_state.expanded && view_state.focused {
        " AGENTS (focused • Y/V or ↑/↓) "
    } else if view_state.expanded {
        " AGENTS (expanded • Ctrl+A focus) "
    } else {
        " AGENTS (Ctrl+A open) "
    };
    if !view_state.expanded && !view_state.focused {
        frame.render_widget(
            Paragraph::new(Line::styled(
                title.trim(),
                theme.muted_style().add_modifier(Modifier::ITALIC),
            ))
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let block = Block::default()
        .title(title)
        .borders(Borders::TOP)
        .border_style(theme.muted_style())
        .padding(Padding::horizontal(1));
    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

fn compact_agents_lines(
    agents: &ResolvedAgents,
    theme: &Theme,
    compact: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let section_limit = if compact { 1 } else { 2 };
    for section in agents.effective_rule_sections.iter().take(section_limit) {
        lines.push(Line::from(vec![
            Span::styled("•", Style::default().fg(theme.accent)),
            Span::raw(" "),
            Span::styled(
                section
                    .heading
                    .clone()
                    .unwrap_or_else(|| "General".to_string()),
                Style::default().fg(theme.foreground),
            ),
        ]));
        if let Some(preview) = section.rules.first().filter(|_| !compact) {
            lines.push(Line::styled(
                format!("  {}", truncate_agents_rule(&preview.text)),
                theme.muted_style(),
            ));
        }
    }
    if agents.effective_rule_sections.len() > section_limit {
        lines.push(Line::styled(
            format!(
                "{} more sections",
                agents.effective_rule_sections.len() - section_limit
            ),
            theme.muted_style(),
        ));
    }
    lines
}

fn expanded_agents_lines(
    agents: &ResolvedAgents,
    theme: &Theme,
    scroll_offset: usize,
    panel_height: u16,
) -> Vec<Line<'static>> {
    let mut full_lines = Vec::new();
    for section in &agents.effective_rule_sections {
        full_lines.push(Line::from(vec![
            Span::styled("•", Style::default().fg(theme.accent)),
            Span::raw(" "),
            Span::styled(
                section
                    .heading
                    .clone()
                    .unwrap_or_else(|| "General".to_string()),
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for rule in section.rules.iter().take(5) {
            full_lines.push(Line::styled(
                format!("  {}", rule.text),
                theme.muted_style(),
            ));
            full_lines.push(Line::styled(
                format!("    source: {}", rule.source_path.display()),
                theme.muted_style().add_modifier(Modifier::ITALIC),
            ));
        }
        full_lines.push(Line::raw(""));
    }

    let visible_height = panel_height.saturating_sub(2) as usize;
    let max_offset = full_lines.len().saturating_sub(visible_height.max(1));
    let offset = scroll_offset.min(max_offset);
    let end = (offset + visible_height.max(1)).min(full_lines.len());
    full_lines[offset..end].to_vec()
}

fn truncate_agents_rule(rule: &str) -> String {
    const MAX_LEN: usize = 72;
    if rule.len() <= MAX_LEN {
        rule.to_string()
    } else {
        format!("{}...", &rule[..MAX_LEN])
    }
}

fn format_plan_runtime(runtime: &PlanRuntime) -> String {
    let status = runtime
        .active_status
        .as_deref()
        .unwrap_or("running")
        .replace('_', " ");
    let target = runtime
        .active_command
        .as_deref()
        .or(runtime.active_tool.as_deref())
        .unwrap_or("task");
    format!("{}: {}", status, target)
}

fn plan_loop_state_lines(runtime: &PlanRuntime, theme: &Theme) -> Vec<Line<'static>> {
    loop_state_lines(
        runtime.loop_stage.as_ref(),
        runtime.last_outcome.as_ref(),
        runtime.last_feedback.as_deref(),
        theme,
    )
}

fn plan_loop_state_line_count(runtime: &PlanRuntime) -> usize {
    loop_state_line_count(
        runtime.loop_stage.as_ref(),
        runtime.last_outcome.as_ref(),
        runtime.last_feedback.as_deref(),
    )
}

fn loop_state_lines(
    stage: Option<&PlanLoopStage>,
    outcome: Option<&PlanLoopOutcome>,
    feedback: Option<&str>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(stage) = stage {
        lines.push(Line::styled(
            format!("loop: {}", format_loop_stage(stage)),
            theme.muted_style().add_modifier(Modifier::ITALIC),
        ));
    }
    if let Some(outcome) = outcome {
        let color = match outcome {
            PlanLoopOutcome::Succeeded
            | PlanLoopOutcome::Checkpointed
            | PlanLoopOutcome::Responded => theme.output,
            PlanLoopOutcome::WaitingApproval | PlanLoopOutcome::RetryPending => {
                Color::Rgb(245, 158, 11)
            }
            PlanLoopOutcome::Failed
            | PlanLoopOutcome::ReplanRequired
            | PlanLoopOutcome::BackendUnavailable
            | PlanLoopOutcome::MaxToolRounds
            | PlanLoopOutcome::Rejected => Color::Rgb(239, 68, 68),
            PlanLoopOutcome::Duplicate | PlanLoopOutcome::Cancelled => Color::Rgb(156, 163, 175),
        };
        lines.push(Line::styled(
            format!("last outcome: {}", format_loop_outcome(outcome)),
            Style::default().fg(color),
        ));
    }
    if let Some(feedback) = feedback.filter(|value| !value.trim().is_empty()) {
        lines.push(Line::styled(
            format!("  {}", truncate_chat_summary(feedback, 72)),
            theme.muted_style(),
        ));
    }
    lines
}

fn loop_state_line_count(
    stage: Option<&PlanLoopStage>,
    outcome: Option<&PlanLoopOutcome>,
    feedback: Option<&str>,
) -> usize {
    usize::from(stage.is_some())
        + usize::from(outcome.is_some())
        + usize::from(feedback.is_some_and(|value| !value.trim().is_empty()))
}

fn format_loop_stage(stage: &PlanLoopStage) -> &'static str {
    match stage {
        PlanLoopStage::Planning => "plan",
        PlanLoopStage::Inspecting => "inspect",
        PlanLoopStage::Executing => "execute",
        PlanLoopStage::Feedback => "feedback",
        PlanLoopStage::RetryPending => "retry",
        PlanLoopStage::ReplanRequired => "replan",
        PlanLoopStage::Responding => "respond",
        PlanLoopStage::Interrupted => "interrupted",
    }
}

fn format_loop_outcome(outcome: &PlanLoopOutcome) -> &'static str {
    match outcome {
        PlanLoopOutcome::WaitingApproval => "waiting approval",
        PlanLoopOutcome::Succeeded => "succeeded",
        PlanLoopOutcome::Failed => "failed",
        PlanLoopOutcome::Checkpointed => "checkpointed",
        PlanLoopOutcome::RetryPending => "retry suggested",
        PlanLoopOutcome::ReplanRequired => "replan required",
        PlanLoopOutcome::Responded => "responded",
        PlanLoopOutcome::BackendUnavailable => "backend unavailable",
        PlanLoopOutcome::MaxToolRounds => "max tool rounds",
        PlanLoopOutcome::Duplicate => "duplicate tool call",
        PlanLoopOutcome::Rejected => "rejected",
        PlanLoopOutcome::Cancelled => "cancelled",
    }
}

fn plan_followup_lines(runtime: &PlanRuntime, theme: &Theme) -> Vec<Line<'static>> {
    let Some(followup) = runtime.followup.as_ref() else {
        return vec![];
    };

    match followup {
        PlanFollowup::Checkpoint { step, next_step } => {
            let mut lines = vec![Line::styled(
                format!("checkpoint: {}", truncate_chat_summary(step, 72)),
                theme.muted_style().add_modifier(Modifier::ITALIC),
            )];
            if let Some(next_step) = next_step {
                lines.push(Line::styled(
                    format!("  next: {}", truncate_chat_summary(next_step, 72)),
                    theme.muted_style(),
                ));
            }
            lines
        }
        PlanFollowup::RetryOrReplan {
            step,
            command,
            retry_count,
        } => vec![
            Line::styled(
                format!("retry {}: {}", retry_count, truncate_chat_summary(step, 72)),
                Style::default().fg(Color::Rgb(245, 158, 11)),
            ),
            Line::styled(
                format!(
                    "  command: {}",
                    truncate_chat_summary(command.as_deref().unwrap_or(step), 72)
                ),
                theme.muted_style(),
            ),
        ],
    }
}

fn plan_followup_history_lines(runtime: &PlanRuntime, theme: &Theme) -> Vec<Line<'static>> {
    if runtime.followup_history.is_empty() {
        return vec![];
    }

    let mut lines = vec![Line::styled(
        "recent followups",
        theme.muted_style().add_modifier(Modifier::ITALIC),
    )];
    for followup in runtime.followup_history.iter().rev().take(2) {
        lines.push(format_followup_history_line(followup, theme));
    }
    lines
}

fn format_followup_history_line(followup: &PlanFollowup, theme: &Theme) -> Line<'static> {
    match followup {
        PlanFollowup::Checkpoint { step, next_step } => {
            let summary = if let Some(next_step) = next_step {
                format!(
                    "checkpoint: {} → {}",
                    truncate_chat_summary(step, 28),
                    truncate_chat_summary(next_step, 28)
                )
            } else {
                format!("checkpoint: {}", truncate_chat_summary(step, 60))
            };
            Line::styled(summary, theme.muted_style())
        }
        PlanFollowup::RetryOrReplan {
            step, retry_count, ..
        } => Line::styled(
            format!("retry {}: {}", retry_count, truncate_chat_summary(step, 60)),
            Style::default().fg(Color::Rgb(245, 158, 11)),
        ),
    }
}

fn plan_job_lines(
    runtime: &PlanRuntime,
    selected_job: usize,
    focused: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let jobs = &runtime.jobs;
    let selected_job = selected_job.min(jobs.len().saturating_sub(1));
    let window_end = (selected_job + 1).max(3).min(jobs.len());
    let window_start = window_end.saturating_sub(3);

    for (index, job) in jobs
        .iter()
        .enumerate()
        .skip(window_start)
        .take(window_end.saturating_sub(window_start))
    {
        let (marker, color) = match job.status {
            PlanJobStatus::WaitingApproval => ("◌", theme.muted),
            PlanJobStatus::Running => ("◐", theme.accent),
            PlanJobStatus::Blocked => ("■", Color::Rgb(245, 158, 11)),
            PlanJobStatus::Succeeded => ("●", theme.output),
            PlanJobStatus::Failed => ("✕", Color::Rgb(239, 68, 68)),
        };
        let is_selected = focused && index == selected_job;
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(
                format_plan_job(job),
                if is_selected {
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.muted_style().add_modifier(Modifier::ITALIC)
                },
            ),
        ]));
    }

    if runtime.jobs.len() > window_end {
        lines.push(Line::styled(
            format!("{} more jobs", runtime.jobs.len() - window_end),
            theme.muted_style(),
        ));
    }

    lines
}

fn format_plan_job(job: &PlanJobRecord) -> String {
    let target = job.command.as_deref().unwrap_or(&job.tool);
    let status = match job.status {
        PlanJobStatus::WaitingApproval => "waiting approval",
        PlanJobStatus::Running => "running",
        PlanJobStatus::Blocked => "blocked",
        PlanJobStatus::Succeeded => "succeeded",
        PlanJobStatus::Failed => "failed",
    };
    format!("{}: {}", status, truncate_chat_summary(target, 64))
}

fn draw_stats(
    frame: &mut Frame,
    stats: &harper_core::memory::session_service::GlobalStats,
    theme: &Theme,
    area: Rect,
) {
    let stats_lines = vec![
        Line::from(vec![
            Span::styled("Total Sessions   ", theme.muted_style()),
            Span::styled(
                stats.total_sessions.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Total Messages   ", theme.muted_style()),
            Span::styled(
                stats.total_messages.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Total Commands   ", theme.muted_style()),
            Span::styled(
                stats.total_commands.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Approved Commands ", theme.muted_style()),
            Span::styled(
                stats.approved_commands.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Avg Duration     ", theme.muted_style()),
            Span::styled(
                format!("{:.2} ms", stats.avg_command_duration_ms),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let compact = compact_layout(area);
    let area = if compact {
        centered_rect_width(60, area)
    } else {
        centered_rect(50, 40, area)
    };
    let stats_widget = Paragraph::new(stats_lines).block(
        Block::default()
            .title(" Usage Statistics ")
            .title_style(
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            )
            .padding(if compact {
                Padding::horizontal(1)
            } else {
                Padding::uniform(2)
            }),
    );

    frame.render_widget(stats_widget, area);
}

fn draw_zen_sidebar(
    frame: &mut Frame,
    sections: &[super::app::SidebarSection],
    theme: &Theme,
    area: Rect,
) {
    let mut items = Vec::new();

    for (section_index, section) in sections.iter().enumerate() {
        if section_index > 0 {
            items.push(ListItem::new(Line::from(vec![Span::raw(" ")])));
        }

        items.push(ListItem::new(Line::from(vec![Span::styled(
            section.title.clone(),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        )])));

        for entry in &section.entries {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!("• {entry}"),
                theme.muted_style(),
            )])));
        }
    }

    let sidebar = List::new(items).block(
        Block::default()
            .borders(Borders::RIGHT)
            .border_style(theme.muted_style())
            .padding(Padding::horizontal(1)),
    );

    frame.render_widget(sidebar, area);
}

fn draw_zen_menu(
    frame: &mut Frame,
    selected: usize,
    theme: &Theme,
    area: Rect,
    show_menu_logo: bool,
) {
    let menu_items = [
        "New Conversation",
        "History",
        "Export",
        "Statistics",
        "Settings",
        "Quit",
    ];
    debug_assert_eq!(menu_items.len(), MAIN_MENU_ITEM_COUNT);

    let show_logo =
        show_menu_logo && area.height >= main_menu_min_height() && area.width >= main_menu_width();
    let compact = area.height <= MAIN_MENU_ITEM_COUNT as u16 + MENU_BLOCK_VERTICAL_OVERHEAD;
    let area = if compact {
        centered_rect_width(40, area)
    } else if show_logo {
        main_menu_area(area)
    } else {
        centered_fixed_width_rect_with_min_height(
            MAIN_MENU_ITEMS_WIDTH,
            45,
            MAIN_MENU_ITEM_COUNT as u16 + MENU_BLOCK_VERTICAL_OVERHEAD,
            area,
        )
    };
    let visible_capacity = if compact {
        area.height as usize
    } else if show_logo {
        area.height
            .saturating_sub(MENU_BLOCK_VERTICAL_OVERHEAD + main_menu_logo_height()) as usize
    } else {
        area.height.saturating_sub(MENU_BLOCK_VERTICAL_OVERHEAD) as usize
    };
    let selected = selected.min(menu_items.len().saturating_sub(1));
    let (window_start, window_end) =
        selected_visible_range(menu_items.len(), selected, visible_capacity);

    let items: Vec<ListItem> = menu_items
        .iter()
        .enumerate()
        .skip(window_start)
        .take(window_end.saturating_sub(window_start))
        .map(|(i, label)| {
            let (prefix, style) = if i == selected {
                (
                    "› ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("  ", Style::default().fg(theme.muted))
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(*label, style),
            ]))
        })
        .collect();

    if compact {
        let menu = List::new(items);
        frame.render_widget(menu, area);
    } else if show_logo {
        let chunks = main_menu_chunks(area);
        draw_main_menu_logo(frame, chunks[0], theme);

        let menu = List::new(items).block(
            Block::default()
                .title("Harper")
                .title_alignment(Alignment::Center)
                .title_style(
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD),
                )
                .padding(Padding::uniform(1)),
        );

        let menu_area = centered_fixed_width_rect(MAIN_MENU_ITEMS_WIDTH, chunks[1]);
        frame.render_widget(menu, menu_area);
    } else {
        let menu = List::new(items).block(
            Block::default()
                .title("Harper")
                .title_alignment(Alignment::Center)
                .title_style(
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(Modifier::BOLD),
                )
                .padding(Padding::uniform(1)),
        );

        frame.render_widget(menu, area);
    }
}

fn main_menu_area(area: Rect) -> Rect {
    centered_fixed_width_rect_with_min_height(main_menu_width(), 45, main_menu_min_height(), area)
}

fn main_menu_min_height() -> u16 {
    MAIN_MENU_ITEM_COUNT as u16 + MENU_BLOCK_VERTICAL_OVERHEAD + main_menu_logo_height()
}

fn main_menu_chunks(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(main_menu_logo_height()),
            Constraint::Min(MAIN_MENU_ITEM_COUNT as u16),
        ])
        .split(area)
}

fn draw_main_menu_logo(frame: &mut Frame, area: Rect, theme: &Theme) {
    let logo = main_menu_logo_lines(theme.background);
    let logo_area = centered_fixed_width_rect(MAIN_MENU_LOGO_WIDTH, area);
    frame.render_widget(Paragraph::new(logo), logo_area);
}

fn main_menu_logo_height() -> u16 {
    MAIN_MENU_LOGO_HEIGHT
}

fn main_menu_width() -> u16 {
    MAIN_MENU_LOGO_WIDTH.max(MAIN_MENU_ITEMS_WIDTH)
}

fn main_menu_logo_lines(_background: Color) -> Vec<Line<'static>> {
    MAIN_MENU_LOGO_ROWS
        .iter()
        .map(|row| {
            let spans = row
                .chars()
                .map(|character| match character {
                    ' ' => Span::raw(" "),
                    _ => Span::styled(
                        character.to_string(),
                        Style::default().fg(MAIN_MENU_LOGO_COLOR),
                    ),
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn centered_rect_width(percent_x: u16, area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(area)[1]
}

#[cfg(test)]
fn centered_rect_with_min_height(
    percent_x: u16,
    percent_y: u16,
    min_height: u16,
    area: Rect,
) -> Rect {
    let rect = centered_rect(percent_x, percent_y, area);
    let height = rect.height.max(min_height).min(area.height);
    let y = area.y + area.height.saturating_sub(height) / 2;

    Rect { y, height, ..rect }
}

fn centered_fixed_width_rect_with_min_height(
    width: u16,
    percent_y: u16,
    min_height: u16,
    area: Rect,
) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area)[1];
    let height = vertical.height.max(min_height).min(area.height);
    let y = area.y + area.height.saturating_sub(height) / 2;
    let width = width.min(area.width);
    let x = area.x + area.width.saturating_sub(width) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}

fn centered_fixed_width_rect(width: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let x = area.x + area.width.saturating_sub(width) / 2;

    Rect { x, width, ..area }
}

fn footer_shortcuts_for_state(
    state: &AppState,
) -> &'static [&'static [(&'static str, &'static str)]] {
    match state {
        AppState::Menu(_) => HOME_FOOTER_SHORTCUTS,
        AppState::Sessions(_, _) => SESSIONS_FOOTER_SHORTCUTS,
        AppState::ExportSessions(_, _) => EXPORT_FOOTER_SHORTCUTS,
        AppState::ViewSession(_, _, _) => PREVIEW_FOOTER_SHORTCUTS,
        AppState::Settings(_) => SETTINGS_FOOTER_SHORTCUTS,
        AppState::Profile(_) | AppState::Appearance(_) => PROFILE_FOOTER_SHORTCUTS,
        AppState::ExecutionPolicy(_) => EXECUTION_POLICY_FOOTER_SHORTCUTS,
        AppState::Stats(_) => STATS_FOOTER_SHORTCUTS,
        _ => CHAT_FOOTER_SHORTCUTS,
    }
}

fn draw_zen_footer(frame: &mut Frame, app: &TuiApp, theme: &Theme, area: Rect) {
    let shortcuts = footer_shortcuts_for_state(&app.state);
    let Some(row) = shortcuts.first() else {
        return;
    };

    let num_cols = row.len().max(1) as u16;
    let col_width = (area.width / num_cols).max(1);
    for (col_idx, (key, label)) in row.iter().enumerate() {
        let shortcut_area = Rect {
            x: area.x + col_idx as u16 * col_width,
            y: area.y,
            width: col_width,
            height: 1,
        };

        let shortcut_text = Line::from(vec![
            Span::styled(*key, Style::default().fg(theme.accent)),
            Span::styled(format!(" {}", label), theme.muted_style()),
        ]);

        frame.render_widget(Paragraph::new(shortcut_text), shortcut_area);
    }
}

fn draw_approval(frame: &mut Frame, state: &ApprovalState, theme: &Theme) {
    let content = format!(
        "{}\n\nCommand:\n{}\n\nControls: Y approve • N/Esc reject • ↑/↓ or J/K scroll",
        state.prompt, state.command
    );
    let area = frame.area();
    let overlay_width = (content.len() as u16 + 4).min(area.width * 3 / 4).max(40);
    let overlay_height = (content.lines().count() as u16 + 4)
        .min(area.height / 2)
        .max(10);

    let overlay_area = Rect {
        x: (area.width - overlay_width) / 2,
        y: (area.height - overlay_height) / 2,
        width: overlay_width,
        height: overlay_height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.muted_style())
        .title(" Approval Required ")
        .title_style(theme.warning_style())
        .style(Style::default().bg(theme.background));

    let paragraph = Paragraph::new(content)
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
        .scroll((state.scroll_offset, 0));

    frame.render_widget(Clear, overlay_area);
    frame.render_widget(paragraph, overlay_area);
}

fn draw_sessions(
    frame: &mut Frame,
    sessions: &[SessionInfo],
    selected: usize,
    remote_mode: bool,
    theme: &Theme,
    area: Rect,
) {
    let compact = compact_layout(area);
    if sessions.is_empty() {
        let detail = if remote_mode {
            "No remote sessions were found for the signed-in user. Local-only sessions remain available under Export."
        } else {
            "Start a conversation first, then return here to resume it."
        };
        let empty = Paragraph::new(vec![
            Line::from(vec![Span::styled(
                "No sessions found",
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(detail, theme.muted_style())]),
        ])
        .block(
            Block::default()
                .title(if remote_mode {
                    " Remote Sessions "
                } else {
                    " Sessions "
                })
                .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(theme.accent_style())
                .padding(Padding::horizontal(1))
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().bg(theme.background).fg(theme.foreground))
        .wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    }

    let visible_item_capacity = if compact {
        area.height.saturating_sub(2).max(1) as usize
    } else {
        area.height.saturating_sub(2).max(2) as usize / 2
    };
    let visible_item_capacity = visible_item_capacity.max(1);
    let selected = selected.min(sessions.len().saturating_sub(1));
    let max_scroll_start = sessions.len().saturating_sub(visible_item_capacity);
    let scroll_start = if sessions.len() > visible_item_capacity {
        selected
            .saturating_sub(visible_item_capacity / 2)
            .min(max_scroll_start)
    } else {
        0
    };

    let items: Vec<ListItem> = sessions
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(visible_item_capacity)
        .map(|(i, session)| {
            let style = if i == selected {
                theme
                    .selection_style()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(theme.background).fg(theme.foreground)
            };
            let mut lines = vec![Line::from(vec![
                Span::styled(session.name.clone(), style),
                Span::styled("  ", theme.muted_style()),
                Span::styled(session.created_at.clone(), theme.muted_style()),
            ])];
            if !compact {
                lines.push(Line::from(vec![Span::styled(
                    truncate_chat_summary(&session.id, 48),
                    theme.muted_style(),
                )]));
            }
            ListItem::new(lines).style(style)
        })
        .collect();

    let position = format!("{}/{}", selected + 1, sessions.len());
    let sessions_list = List::new(items).block(
        Block::default()
            .title(if remote_mode {
                format!(" Remote Sessions ({position}) ")
            } else {
                format!(" Sessions ({position}) ")
            })
            .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(theme.accent_style())
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.background)),
    );

    frame.render_widget(sessions_list, area);
}

fn draw_export_sessions(
    frame: &mut Frame,
    sessions: &[SessionInfo],
    selected: usize,
    theme: &Theme,
    area: Rect,
) {
    let compact = compact_layout(area);
    if sessions.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(vec![Span::styled(
                "No sessions available to export",
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Create a conversation first, then return here to export it.",
                theme.muted_style(),
            )]),
        ])
        .block(
            Block::default()
                .title(" Export Sessions ")
                .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(theme.accent_style())
                .padding(Padding::horizontal(1))
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().bg(theme.background).fg(theme.foreground))
        .wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    }

    let visible_item_capacity = if compact {
        area.height.saturating_sub(2).max(1) as usize
    } else {
        area.height.saturating_sub(2).max(2) as usize / 2
    };
    let visible_item_capacity = visible_item_capacity.max(1);
    let selected = selected.min(sessions.len().saturating_sub(1));
    let max_scroll_start = sessions.len().saturating_sub(visible_item_capacity);
    let scroll_start = if sessions.len() > visible_item_capacity {
        selected
            .saturating_sub(visible_item_capacity / 2)
            .min(max_scroll_start)
    } else {
        0
    };

    let items: Vec<ListItem> = sessions
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(visible_item_capacity)
        .map(|(i, session)| {
            let style = if i == selected {
                theme
                    .selection_style()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(theme.background).fg(theme.foreground)
            };
            let mut lines = vec![Line::from(vec![
                Span::styled(display_session_name(session), style),
                Span::styled("  ", theme.muted_style()),
                Span::styled(session.created_at.clone(), theme.muted_style()),
            ])];
            if !compact {
                lines.push(Line::from(vec![Span::styled(
                    truncate_chat_summary(&session.id, 48),
                    theme.muted_style(),
                )]));
            }
            ListItem::new(lines).style(style)
        })
        .collect();

    let position = format!("{}/{}", selected + 1, sessions.len());
    let sessions_list = List::new(items).block(
        Block::default()
            .title(format!(" Export Local Sessions ({position}) "))
            .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(theme.accent_style())
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.background)),
    );

    frame.render_widget(sessions_list, area);
}

fn display_session_name(session: &SessionInfo) -> String {
    let trimmed = session.name.trim();
    if !trimmed.is_empty() && trimmed != session.id {
        return trimmed.to_string();
    }

    if session.id.len() >= 8 {
        return format!("Session {}", &session.id[..8]);
    }

    "Untitled session".to_string()
}

fn draw_settings(frame: &mut Frame, selected: usize, theme: &Theme, area: Rect) {
    let compact = compact_layout(area);
    let tools = [
        "Profile",
        "Appearance",
        "Execution Policy",
        "Search",
        "System",
        "Processes",
    ];
    let items: Vec<ListItem> = tools
        .iter()
        .enumerate()
        .map(|(i, tool)| {
            let style = if i == selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            ListItem::new(*tool).style(style)
        })
        .collect();

    let constraints = if compact {
        [Constraint::Min(5), Constraint::Length(1)]
    } else {
        [Constraint::Min(8), Constraint::Length(3)]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    let tools_list =
        List::new(items).block(Block::default().title(" Settings ").padding(if compact {
            Padding::horizontal(1)
        } else {
            Padding::uniform(1)
        }));
    let context = Paragraph::new(settings_context(selected))
        .style(theme.muted_style())
        .block(Block::default().padding(Padding::horizontal(1)))
        .wrap(Wrap { trim: true });

    frame.render_widget(tools_list, chunks[0]);
    frame.render_widget(context, chunks[1]);
}

fn settings_context(selected: usize) -> &'static str {
    match selected {
        0 => "Manage local sign-in state.",
        1 => "Control visual options for the home menu.",
        2 => "Configure command approval, sandboxing, retries, and header widgets.",
        3 => "Check web search availability.",
        4 => "Show a local system information snapshot.",
        5 => "Show a short local process snapshot.",
        _ => "",
    }
}

fn draw_appearance(frame: &mut Frame, app: &TuiApp, selected: usize, theme: &Theme, area: Rect) {
    let rows = [
        format!(
            "Menu Logo: {}",
            if app.show_menu_logo { "On" } else { "Off" }
        ),
        format!(
            "Mouse Capture: {}",
            if app.mouse_capture { "On" } else { "Off" }
        ),
        "Save".to_string(),
    ];
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = if index == selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            ListItem::new(row.clone()).style(style)
        })
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(2)])
        .split(area);
    let list = List::new(items).block(
        Block::default()
            .title(" Appearance ")
            .padding(Padding::uniform(1)),
    );
    let context_text = match selected {
        0 => "Menu logo shows the Harper logo on the home screen.",
        1 => "Mouse capture enables wheel/drag scrolling. Off keeps normal text selection.",
        2 => "Save keeps these appearance settings for next time.",
        _ => "",
    };
    let context = Paragraph::new(context_text)
        .style(theme.muted_style())
        .block(Block::default().padding(Padding::horizontal(1)))
        .wrap(Wrap { trim: true });

    frame.render_widget(list, chunks[0]);
    frame.render_widget(context, chunks[1]);
}

fn draw_execution_policy(
    frame: &mut Frame,
    app: &TuiApp,
    selected: usize,
    theme: &Theme,
    area: Rect,
) {
    let allowed = if app.allowed_commands.is_empty() {
        "(none)".to_string()
    } else {
        app.allowed_commands.join(", ")
    };
    let blocked = if app.blocked_commands.is_empty() {
        "(none)".to_string()
    } else {
        app.blocked_commands.join(", ")
    };
    let mut rows = vec![
        format!(
            "Approval: {}",
            settings::approval_profile_name(app.approval_profile)
        ),
        format!(
            "Strategy: {}",
            settings::execution_strategy_name(app.execution_strategy)
        ),
        format!(
            "Sandbox: {}",
            settings::sandbox_profile_name(app.sandbox_profile)
        ),
        format!("Retries: {}", app.retry_max_attempts),
        format!(
            "Header: {}",
            settings::header_widgets_summary(&app.header_widgets)
        ),
        format!("Allow: {allowed}"),
        format!("Block: {blocked}"),
        "Save".to_string(),
        format!(
            "Updates: {}",
            update_status_label(app.update_status.as_deref())
        ),
    ];
    if let Some(path) = &app.homebrew_path_fix {
        rows.push(format!("Fix Homebrew PATH: {}", path.display()));
    }
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let style = if index == selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            ListItem::new(label.clone()).style(style)
        })
        .collect();

    let context = execution_policy_context(selected);
    let context_widget = Paragraph::new(context)
        .style(theme.muted_style())
        .block(Block::default().padding(Padding::horizontal(1)))
        .wrap(Wrap { trim: true });

    let editor_height = match app
        .execution_policy_editor
        .as_ref()
        .map(|editor| editor.field)
    {
        Some(super::app::ExecutionPolicyListField::HeaderWidgets) => 18,
        Some(_) => 4,
        None => 0,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(editor_height),
        ])
        .split(area);

    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Policy ")
                .padding(Padding::uniform(1)),
        ),
        chunks[0],
    );
    frame.render_widget(context_widget, chunks[1]);
    if let Some(editor) = &app.execution_policy_editor {
        match editor.field {
            super::app::ExecutionPolicyListField::HeaderWidgets => {
                let editor_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(8),
                        Constraint::Length(3),
                        Constraint::Length(1),
                    ])
                    .split(chunks[2]);
                let widgets = settings::available_header_widgets();
                let visible_item_capacity =
                    editor_chunks[0].height.saturating_sub(2).max(1) as usize;
                let selected_index = editor.selected_index.min(widgets.len().saturating_sub(1));
                let max_scroll_start = widgets.len().saturating_sub(visible_item_capacity);
                let scroll_start = if widgets.len() > visible_item_capacity {
                    selected_index
                        .saturating_sub(visible_item_capacity / 2)
                        .min(max_scroll_start)
                } else {
                    0
                };
                let items = widgets
                    .iter()
                    .enumerate()
                    .skip(scroll_start)
                    .take(visible_item_capacity)
                    .map(|(index, widget)| {
                        let enabled = app.header_widgets.contains(widget);
                        let marker = if enabled { "[x]" } else { "[ ]" };
                        let locked = if *widget == super::app::HeaderWidget::Model {
                            " (required)"
                        } else {
                            ""
                        };
                        let style = if index == editor.selected_index {
                            if editor.text_input_focused {
                                Style::default().bg(theme.background).fg(theme.foreground)
                            } else {
                                theme
                                    .selection_style()
                                    .fg(theme.accent)
                                    .add_modifier(Modifier::BOLD)
                            }
                        } else {
                            Style::default().bg(theme.background).fg(theme.foreground)
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(format!("{marker} "), style),
                            Span::styled(settings::header_widget_name(*widget), style),
                            Span::styled(locked, theme.muted_style()),
                        ]))
                        .style(style)
                    })
                    .collect::<Vec<_>>();

                let editor_widget = List::new(items).block(
                    Block::default()
                        .title(" Header Widgets ")
                        .border_style(if editor.text_input_focused {
                            theme.muted_style()
                        } else {
                            theme.accent_style()
                        })
                        .borders(Borders::ALL)
                        .padding(Padding::horizontal(1))
                        .style(Style::default().bg(theme.background)),
                );
                frame.render_widget(editor_widget, editor_chunks[0]);

                let summary_text = if editor.text_input_focused {
                    format!("{}▌", editor.input)
                } else {
                    editor.input.clone()
                };
                let summary_widget = Paragraph::new(summary_text).block(
                    Block::default()
                        .title(" Saved As ")
                        .border_style(if editor.text_input_focused {
                            theme.accent_style()
                        } else {
                            theme.muted_style()
                        })
                        .borders(Borders::ALL)
                        .padding(Padding::horizontal(1))
                        .style(Style::default().bg(theme.background)),
                );
                frame.render_widget(
                    summary_widget
                        .style(Style::default().bg(theme.background).fg(theme.foreground)),
                    editor_chunks[1],
                );
                frame.render_widget(
                    Paragraph::new("Space toggles • Tab comma edit • S saves")
                        .style(theme.muted_style()),
                    editor_chunks[2],
                );
            }
            super::app::ExecutionPolicyListField::AllowedCommands => {
                let editor_widget = Paragraph::new(editor.input.as_str()).block(
                    Block::default()
                        .title(" Allow ")
                        .padding(Padding::horizontal(1)),
                );
                frame.render_widget(editor_widget, chunks[2]);
            }
            super::app::ExecutionPolicyListField::BlockedCommands => {
                let editor_widget = Paragraph::new(editor.input.as_str()).block(
                    Block::default()
                        .title(" Block ")
                        .padding(Padding::horizontal(1)),
                );
                frame.render_widget(editor_widget, chunks[2]);
            }
        }
    }
}

fn execution_policy_context(selected: usize) -> &'static str {
    match selected {
        0 => "Cycles how command approval is handled.",
        1 => "Cycles command and tool routing behavior.",
        2 => "Cycles filesystem and network sandbox limits.",
        3 => "Cycles the retry limit for autonomous recovery.",
        4 => "Opens the header widget selector.",
        5 => "Edits comma-separated commands allowed by policy.",
        6 => "Edits comma-separated commands blocked by policy.",
        7 => "Writes the current policy settings to config/local.toml.",
        8 => "Checks for a newer release and refreshes the cached update status.",
        9 => "Moves the shadowing local binary aside and points it to Homebrew's Harper.",
        _ => "",
    }
}

fn update_status_label(status: Option<&str>) -> &str {
    status
        .and_then(|status| status.strip_prefix("update: "))
        .unwrap_or("not checked")
}

fn draw_profile(frame: &mut Frame, app: &TuiApp, selected: usize, theme: &Theme, area: Rect) {
    let info_lines = if let Some(session) = app.auth_session.as_ref() {
        vec![
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(theme.foreground)),
                Span::styled("Signed in", Style::default().fg(theme.accent)),
            ]),
            Line::from(vec![
                Span::styled("Email: ", Style::default().fg(theme.foreground)),
                Span::styled(
                    session
                        .user
                        .email
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                    theme.muted_style(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Name: ", Style::default().fg(theme.foreground)),
                Span::styled(
                    session
                        .user
                        .display_name
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                    theme.muted_style(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Provider: ", Style::default().fg(theme.foreground)),
                Span::styled(
                    session
                        .user
                        .provider
                        .as_ref()
                        .map(|provider| format!("{provider:?}"))
                        .unwrap_or_else(|| "none".to_string()),
                    theme.muted_style(),
                ),
            ]),
            Line::from(vec![
                Span::styled("User ID: ", Style::default().fg(theme.foreground)),
                Span::styled(session.user.user_id.clone(), theme.muted_style()),
            ]),
            Line::from(""),
            Line::styled(
                "Session storage uses the local OS keychain/keyring.",
                theme.muted_style(),
            ),
            Line::styled(
                "History, Export, and Statistics are scoped to this signed-in account.",
                theme.muted_style(),
            ),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(theme.foreground)),
                Span::styled("Not signed in", theme.muted_style()),
            ]),
            Line::from(""),
            Line::styled(
                "Sign in with /auth login <provider> in chat to enable account-scoped History, Export, and Statistics.",
                theme.muted_style(),
            ),
            Line::styled(
                "Without sign-in, Harper stays in local-only mode on this machine.",
                theme.muted_style(),
            ),
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(6)])
        .split(area);

    let profile = Paragraph::new(info_lines)
        .block(
            Block::default()
                .title(" Profile ")
                .padding(Padding::uniform(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(profile, chunks[0]);

    let actions: Vec<&str> = if app.auth_session.is_some() {
        vec!["Logout", "Refresh Session"]
    } else {
        vec!["Login with GitHub", "Login with Google", "Login with Apple"]
    };
    let items: Vec<ListItem> = actions
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let style = if index == selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            ListItem::new(*label).style(style)
        })
        .collect();

    let actions_list = List::new(items).block(
        Block::default()
            .title(" Actions ")
            .padding(Padding::uniform(1)),
    );
    frame.render_widget(actions_list, chunks[1]);
}

fn draw_view_session(
    frame: &mut Frame,
    name: &str,
    messages: &[harper_core::core::Message],
    scroll_offset: usize,
    theme: &Theme,
    area: Rect,
) {
    let mut message_lines: Vec<Line> = Vec::new();
    for msg in messages.iter().filter(|msg| msg.role != "system") {
        append_rendered_message_lines(&mut message_lines, msg, theme, theme.input, theme.output);
    }

    let title = format!(" Preview {} ", name);
    let view = Paragraph::new(message_lines)
        .block(
            Block::default()
                .title(title)
                .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(theme.accent_style())
                .padding(Padding::horizontal(1))
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().bg(theme.background).fg(theme.foreground))
        .scroll((scroll_offset as u16, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(view, area);
}

fn draw_message_overlay(
    frame: &mut Frame,
    message: &UiMessage,
    help_selected: usize,
    theme: &Theme,
) {
    if message.content.trim().is_empty() {
        return;
    }
    if matches!(message.message_type, super::app::MessageType::Help) {
        draw_help_overlay(frame, &message.content, help_selected, theme);
        return;
    }
    let area = frame.area();
    let (alignment, overlay_area) = match message.message_type {
        super::app::MessageType::Status | super::app::MessageType::Info => {
            let max_width = area.width.saturating_sub(4).clamp(24, 96);
            let content_width = message
                .content
                .lines()
                .map(|line| line.chars().count() as u16)
                .max()
                .unwrap_or(0);
            let overlay_width = (content_width + 8).clamp(24, max_width);
            let text_width = overlay_width.saturating_sub(4).max(1) as usize;
            let wrapped_line_count = message
                .content
                .lines()
                .map(|line| {
                    let len = line.chars().count();
                    usize::max(1, len.div_ceil(text_width))
                })
                .sum::<usize>() as u16;
            let overlay_height = (wrapped_line_count + 2).clamp(3, area.height.saturating_sub(3));
            let overlay_x = area.width.saturating_sub(overlay_width + 2);
            let overlay_y = 3.min(area.height.saturating_sub(overlay_height));
            let overlay_area = Rect {
                x: overlay_x,
                y: overlay_y,
                width: overlay_width,
                height: overlay_height,
            };
            (Alignment::Left, overlay_area)
        }
        super::app::MessageType::Error | super::app::MessageType::Help => {
            let max_overlay_width = area.width.saturating_mul(3) / 4;
            let content_width = message
                .content
                .lines()
                .map(|line| line.chars().count() as u16)
                .max()
                .unwrap_or(0);
            let overlay_width = (content_width + 8).clamp(24, max_overlay_width.max(24));
            let text_width = overlay_width.saturating_sub(4).max(1) as usize;
            let wrapped_line_count = message
                .content
                .lines()
                .map(|line| {
                    let len = line.chars().count();
                    usize::max(1, len.div_ceil(text_width))
                })
                .sum::<usize>() as u16;
            let overlay_height =
                (wrapped_line_count + 4).clamp(5, area.height.saturating_mul(3) / 4);
            let overlay_area = Rect {
                x: (area.width - overlay_width) / 2,
                y: (area.height - overlay_height) / 2,
                width: overlay_width,
                height: overlay_height,
            };
            (Alignment::Center, overlay_area)
        }
    };

    let border_style = match message.message_type {
        super::app::MessageType::Error => theme.warning_style(),
        super::app::MessageType::Help => theme.accent_style(),
        super::app::MessageType::Status => theme.muted_style(),
        super::app::MessageType::Info => theme.info_style(),
    };
    let block_padding = match message.message_type {
        super::app::MessageType::Status | super::app::MessageType::Info => Padding::horizontal(1),
        super::app::MessageType::Error | super::app::MessageType::Help => Padding::uniform(1),
    };

    let overlay = Paragraph::new(message.content.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .padding(block_padding),
        )
        .style(Style::default().bg(theme.background).fg(theme.foreground))
        .alignment(alignment)
        .wrap(Wrap { trim: true });

    frame.render_widget(Clear, overlay_area);
    frame.render_widget(overlay, overlay_area);
}

fn draw_help_overlay(frame: &mut Frame, content: &str, selected: usize, theme: &Theme) {
    let area = frame.area();
    let overlay_area = centered_rect(78, 34, area);
    frame.render_widget(Clear, overlay_area);

    let shortcuts = content
        .split('|')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(overlay_area);

    let total_rows = shortcuts.chunks(2).len();
    let visible_rows = chunks[1].height.max(1) as usize;
    let selected = selected.min(total_rows.saturating_sub(1));
    let max_scroll_start = total_rows.saturating_sub(visible_rows);
    let scroll_start = if total_rows > visible_rows {
        selected
            .saturating_sub(visible_rows / 2)
            .min(max_scroll_start)
    } else {
        0
    };
    let scroll_end = (scroll_start + visible_rows).min(total_rows);

    let row_items = shortcuts
        .chunks(2)
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end.saturating_sub(scroll_start))
        .map(|(index, chunk)| {
            let left = chunk.first().copied().unwrap_or_default();
            let right = chunk.get(1).copied().unwrap_or_default();
            let style = if index == selected {
                theme
                    .selection_style()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(theme.background).fg(theme.foreground)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{left:<34}"), style),
                Span::styled(right.to_string(), style),
            ]))
            .style(style)
        })
        .collect::<Vec<_>>();

    let header = Paragraph::new(Line::from(vec![Span::styled(
        "Keyboard shortcuts",
        theme.accent_style().add_modifier(Modifier::BOLD),
    )]))
    .block(
        Block::default()
            .title(" Help ")
            .title_style(theme.accent_style().add_modifier(Modifier::BOLD))
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(theme.accent_style())
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.background)),
    )
    .style(Style::default().bg(theme.background).fg(theme.foreground));
    frame.render_widget(header, chunks[0]);

    let body = List::new(row_items).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(theme.accent_style())
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.background)),
    );
    frame.render_widget(body, chunks[1]);

    let footer_hint = if total_rows > visible_rows {
        format!(
            "↑/↓ move • J/K move • Esc closes this help panel • {}/{}",
            selected + 1,
            total_rows
        )
    } else {
        "↑/↓ move • J/K move • Esc closes this help panel".to_string()
    };

    let footer = Paragraph::new(Line::from(vec![Span::styled(
        footer_hint,
        theme.muted_style().add_modifier(Modifier::ITALIC),
    )]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT)
            .border_style(theme.accent_style())
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(theme.background)),
    )
    .style(Style::default().bg(theme.background).fg(theme.foreground));
    frame.render_widget(footer, chunks[2]);
}

fn draw_plan_jobs_browser(frame: &mut Frame, chat_state: &super::app::ChatState, theme: &Theme) {
    let Some(plan) = &chat_state.active_plan else {
        return;
    };
    let Some(runtime) = &plan.runtime else {
        return;
    };
    if runtime.jobs.is_empty() {
        return;
    }

    let selected_index = chat_state
        .plan_job_selected
        .min(runtime.jobs.len().saturating_sub(1));
    let selected = &runtime.jobs[selected_index];
    let overlay_area = centered_rect(80, 70, frame.area());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(8),
        ])
        .split(overlay_area);

    frame.render_widget(Clear, overlay_area);

    let title = Paragraph::new(format!(
        "Planner Jobs • job {} • {} • Esc close • Y/V select • ↑/↓ scroll",
        &selected.job_id[..selected.job_id.len().min(8)],
        selected.tool
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.muted_style())
            .style(Style::default().bg(theme.background)),
    )
    .style(Style::default().fg(theme.foreground));
    frame.render_widget(title, chunks[0]);

    let summary_lines = plan_job_lines(runtime, selected_index, true, theme);
    let summary = Paragraph::new(summary_lines)
        .block(
            Block::default()
                .title(" Recent Jobs ")
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(theme.muted_style())
                .style(Style::default().bg(theme.background)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(summary, chunks[1]);

    let detail_lines = plan_job_transcript_lines(selected, theme);
    let detail = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .title(" Output Transcript ")
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(theme.muted_style())
                .style(Style::default().bg(theme.background))
                .padding(Padding::horizontal(1)),
        )
        .scroll((chat_state.plan_job_output_scroll as u16, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, chunks[2]);
}

fn draw_command_output_browser(
    frame: &mut Frame,
    chat_state: &super::app::ChatState,
    theme: &Theme,
) {
    let Some(output) = derive_command_output_display(chat_state) else {
        return;
    };

    let overlay_area = centered_rect(90, 80, frame.area());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(overlay_area);

    frame.render_widget(Clear, overlay_area);

    let title = Paragraph::new(format!(
        "$ Command Output ({}) • drag select • Ctrl+Shift+C copy • Esc close • {}",
        output.status_label,
        truncate_chat_summary(&output.command, 80)
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.muted_style())
            .style(Style::default().bg(theme.background)),
    )
    .style(Style::default().fg(theme.foreground));
    frame.render_widget(title, chunks[0]);

    let detail_lines = apply_line_selection(
        command_output_lines(&output, theme, None),
        chat_state.command_output_selection,
        theme.selection_style(),
    );
    let detail = Paragraph::new(detail_lines)
        .block(
            Block::default()
                .title(format!(" Output • {} lines ", output.total_line_count))
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(theme.muted_style())
                .style(Style::default().bg(theme.background))
                .padding(Padding::horizontal(1)),
        )
        .scroll((chat_state.command_output_scroll as u16, 0))
        .wrap(Wrap { trim: false });
    chat_state.command_output_area.set(Some(chunks[1]));
    frame.render_widget(detail, chunks[1]);
}

fn apply_line_selection(
    mut lines: Vec<Line<'static>>,
    selection: Option<LineSelection>,
    selection_style: Style,
) -> Vec<Line<'static>> {
    let Some(selection) = selection else {
        return lines;
    };
    let (start, end) = selection.range();
    for (index, line) in lines.iter_mut().enumerate() {
        if index < start || index > end {
            continue;
        }
        for span in &mut line.spans {
            span.style = selection_style;
        }
    }
    lines
}

fn draw_plan_steps_browser(frame: &mut Frame, chat_state: &super::app::ChatState, theme: &Theme) {
    let Some(plan) = &chat_state.active_plan else {
        return;
    };
    if plan.items.is_empty() {
        return;
    }

    let selected_index = chat_state
        .plan_step_selected
        .min(plan.items.len().saturating_sub(1));
    let overlay_area = centered_rect(80, 70, frame.area());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(overlay_area);

    frame.render_widget(Clear, overlay_area);

    let title = Paragraph::new(
        "Plan Steps • Esc close • Y/V select • C complete • I in-progress • B blocked • R retry • U replan • K ack • X clear",
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.muted_style())
            .style(Style::default().bg(theme.background)),
    )
    .style(Style::default().fg(theme.foreground));
    frame.render_widget(title, chunks[0]);

    let items: Vec<ListItem> = plan
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let (marker, color) = match item.status {
                PlanStepStatus::Pending => ("○", theme.muted),
                PlanStepStatus::InProgress => ("◐", theme.accent),
                PlanStepStatus::Completed => ("●", theme.output),
                PlanStepStatus::Blocked => ("■", Color::Rgb(245, 158, 11)),
            };
            let text_style = if index == selected_index {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(item.step.clone(), text_style),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Steps ")
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(theme.muted_style())
                .style(Style::default().bg(theme.background))
                .padding(Padding::horizontal(1)),
        ),
        chunks[1],
    );

    let selected = &plan.items[selected_index];
    let detail = Paragraph::new(
        vec![
            Line::styled(
                format!("status: {}", plan_step_status_label(selected.status)),
                theme.muted_style(),
            ),
            Line::styled(
                selected.step.as_str(),
                Style::default().fg(theme.foreground),
            ),
        ]
        .into_iter()
        .chain(selected_step_followup_lines(
            plan.runtime.as_ref(),
            selected.step.as_str(),
            theme,
        ))
        .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .title(" Selected Step ")
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(theme.muted_style())
            .style(Style::default().bg(theme.background))
            .padding(Padding::horizontal(1)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(detail, chunks[2]);
}

fn selected_step_followup_lines(
    runtime: Option<&PlanRuntime>,
    step: &str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let Some(runtime) = runtime else {
        return vec![];
    };
    let Some(followup) = runtime.followup.as_ref() else {
        return vec![];
    };
    match followup {
        PlanFollowup::Checkpoint {
            step: followup_step,
            next_step,
        } if followup_step == step => {
            let mut lines = vec![Line::styled("checkpoint pending", theme.muted_style())];
            if let Some(next_step) = next_step {
                lines.push(Line::styled(
                    format!("next step: {}", truncate_chat_summary(next_step, 72)),
                    theme.muted_style(),
                ));
            }
            lines
        }
        PlanFollowup::RetryOrReplan {
            step: followup_step,
            command,
            retry_count,
        } if followup_step == step => vec![
            Line::styled(
                format!("retry count: {}", retry_count),
                Style::default().fg(Color::Rgb(245, 158, 11)),
            ),
            Line::styled(
                format!(
                    "command: {}",
                    truncate_chat_summary(command.as_deref().unwrap_or(step), 72)
                ),
                theme.muted_style(),
            ),
        ],
        _ => vec![],
    }
}

fn plan_step_status_label(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "pending",
        PlanStepStatus::InProgress => "in_progress",
        PlanStepStatus::Completed => "completed",
        PlanStepStatus::Blocked => "blocked",
    }
}

fn plan_job_transcript_lines(job: &PlanJobRecord, theme: &Theme) -> Vec<Line<'static>> {
    let transcript = if job.output_transcript.trim().is_empty() {
        "No output recorded yet".to_string()
    } else {
        job.output_transcript.clone()
    };
    let detail_color = if job.has_error_output {
        Color::Rgb(245, 158, 11)
    } else {
        theme.output
    };
    transcript
        .lines()
        .map(|line| Line::styled(line.to_string(), Style::default().fg(detail_color)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interfaces::ui::app;
    use harper_core::{core::Message, PlanItem};
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;
    use std::cell::Cell;

    fn setup() -> (SyntaxSet, ThemeSet) {
        (
            SyntaxSet::load_defaults_newlines(),
            ThemeSet::load_defaults(),
        )
    }

    #[test]
    fn input_lines_preserve_typed_text() {
        let lines = input_lines("first\nsecond", Style::default());

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].to_string(), "first");
        assert_eq!(lines[1].to_string(), "second");
    }

    #[test]
    fn input_display_lines_show_tail_when_multiline_input_overflows() {
        let lines = input_display_lines("one\ntwo\nthree\nfour", Style::default(), 2);

        assert_eq!(lines.len(), 2);
        assert!(lines[0].to_string().contains("three"));
        assert!(lines[0].to_string().starts_with("…"));
        assert!(lines[1].to_string().contains("four"));
    }

    #[test]
    fn multiline_input_height_uses_available_room() {
        let input = "one\ntwo\nthree\nfour\nfive\nsix";

        assert_eq!(input_area_height(input, 120, 20), 7);
        assert_eq!(input_area_height(input, 120, 9), 5);
    }

    fn empty_chat_state() -> app::ChatState {
        app::ChatState {
            session_id: "session-1".to_string(),
            messages: Vec::<Message>::new(),
            awaiting_response: false,
            active_plan: None,
            active_agents: None,
            active_review: None,
            review_selected: 0,
            plan_step_selected: 0,
            plan_steps_expanded: false,
            plan_job_selected: 0,
            plan_jobs_expanded: false,
            plan_job_output_scroll: 0,
            navigation_focus: app::NavigationFocus::Messages,
            command_output: None,
            command_output_expanded: false,
            command_output_scroll: 0,
            loop_state: app::ChatLoopState::default(),
            agents_panel_expanded: false,
            agents_scroll_offset: 0,
            input: String::new(),
            web_search: false,
            web_search_enabled: false,
            completion_candidates: Vec::new(),
            completion_index: 0,
            scroll_offset: 0,
            completion_prefix: None,
            sidebar_visible: false,
            sidebar_sections: Vec::new(),
            rendered_message_cache: Vec::new(),
            rendered_transcript_lines: Vec::new(),
            render_cache_theme_key: String::new(),
            messages_area: Cell::new(None),
            command_output_area: Cell::new(None),
            command_output_selection: None,
        }
    }

    #[test]
    fn draw_slash_completion_popup_fits_small_terminal() {
        let mut chat_state = empty_chat_state();
        chat_state.input = "/".to_string();
        chat_state.completion_candidates = (0..40)
            .map(|index| format!("/command-{index}"))
            .collect::<Vec<_>>();

        let mut app = app::TuiApp::default();
        app.state = app::AppState::Chat(Box::new(chat_state));

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("slash completion popup should render");
    }

    #[test]
    fn completion_visible_range_tracks_middle_selection() {
        assert_eq!(completion_visible_range(40, 25, 10), (20, 30));
    }

    #[test]
    fn completion_visible_range_clamps_near_end() {
        assert_eq!(completion_visible_range(40, 39, 10), (30, 40));
    }

    #[test]
    fn menu_rect_keeps_all_rows_visible_on_small_terminal() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 18,
        };

        let menu_area = centered_rect_with_min_height(
            40,
            35,
            MAIN_MENU_ITEM_COUNT as u16 + MENU_BLOCK_VERTICAL_OVERHEAD,
            area,
        );

        assert_eq!(menu_area.height, 9);
        assert_eq!(menu_area.y, 4);
    }

    #[test]
    fn menu_visible_range_tracks_selection_like_completion_popup() {
        assert_eq!(selected_visible_range(MAIN_MENU_ITEM_COUNT, 5, 3), (3, 6));
    }

    #[test]
    fn menu_visible_range_shows_all_items_when_capacity_allows() {
        assert_eq!(
            selected_visible_range(MAIN_MENU_ITEM_COUNT, 5, MAIN_MENU_ITEM_COUNT),
            (0, MAIN_MENU_ITEM_COUNT)
        );
    }

    #[test]
    fn draw_menu_fits_exact_item_height_terminal() {
        let mut app = app::TuiApp::default();
        app.state = app::AppState::Menu(MAIN_MENU_ITEM_COUNT - 1);

        let backend = TestBackend::new(80, MAIN_MENU_ITEM_COUNT as u16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("compact menu should render");
    }

    #[test]
    fn draw_menu_shows_quit_at_twenty_six_rows() {
        let mut app = app::TuiApp::default();
        app.state = app::AppState::Menu(MAIN_MENU_ITEM_COUNT - 1);

        let backend = TestBackend::new(80, 26);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("menu should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Quit"));
    }

    #[test]
    fn draw_menu_shows_logo_on_roomy_terminal() {
        let mut app = app::TuiApp::default();
        app.state = app::AppState::Menu(0);

        let backend = TestBackend::new(120, 48);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("menu should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(MAIN_MENU_LOGO_ROWS
            .iter()
            .any(|row| !row.trim().is_empty() && rendered.contains(row.trim())));
        assert!(rendered.contains("New Conversation"));
    }

    #[test]
    fn menu_logo_dimensions_match_static_rows() {
        assert_eq!(main_menu_logo_height(), MAIN_MENU_LOGO_HEIGHT);
        assert!(main_menu_width() >= MAIN_MENU_ITEMS_WIDTH);
        assert_eq!(main_menu_width(), MAIN_MENU_LOGO_WIDTH);
        assert_eq!(MAIN_MENU_LOGO_ROWS.len(), MAIN_MENU_LOGO_HEIGHT as usize);
        assert!(MAIN_MENU_LOGO_ROWS
            .iter()
            .all(|row| row.chars().count() <= MAIN_MENU_LOGO_WIDTH as usize));
    }

    #[test]
    fn compact_layout_detects_small_terminal_room() {
        assert!(compact_layout(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 26,
        }));
        assert!(!compact_layout(Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 48,
        }));
    }

    #[test]
    fn compact_command_output_height_reclaims_rows() {
        let output = CommandOutputDisplay {
            command: "cargo test".to_string(),
            content: (0..12)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
            has_error: false,
            status_label: "done".to_string(),
            total_line_count: 12,
        };

        assert!(
            command_output_panel_height(&output, true)
                < command_output_panel_height(&output, false)
        );
    }

    #[test]
    fn default_command_output_height_is_globally_capped() {
        let output = CommandOutputDisplay {
            command: "cargo test".to_string(),
            content: (0..12)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
            has_error: false,
            status_label: "done".to_string(),
            total_line_count: 12,
        };

        assert_eq!(command_output_panel_height(&output, false), 9);
    }

    #[test]
    fn footer_uses_one_row_globally() {
        let mut app = app::TuiApp::default();
        app.state = app::AppState::Sessions(Vec::new(), 0);

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("footer should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Enter"));
        assert!(rendered.contains("Resume"));
        assert!(!rendered.contains("J/K Move"));
    }

    #[test]
    fn draw_chat_compacts_crowded_terminal() {
        let mut chat_state = empty_chat_state();
        chat_state.messages.push(Message {
            role: "user".to_string(),
            content: "Keep the main transcript visible.".to_string(),
        });
        chat_state.sidebar_visible = true;
        chat_state.sidebar_sections = vec![app::SidebarSection {
            title: "Commands".to_string(),
            entries: vec!["cargo test".to_string()],
        }];
        chat_state.command_output = Some(app::CommandOutputState {
            command: "cargo test".to_string(),
            content: (0..20)
                .map(|index| format!("output line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
            has_error: false,
            done: true,
        });
        chat_state.active_plan = Some(PlanState {
            explanation: Some("Crowded terminal plan".to_string()),
            items: (0..6)
                .map(|index| PlanItem {
                    step: format!("Step {index}"),
                    status: PlanStepStatus::Pending,
                    job_id: None,
                })
                .collect(),
            runtime: None,
            updated_at: None,
        });

        let theme = Theme::default();
        refresh_chat_render_cache(&mut chat_state, &theme);

        let mut app = app::TuiApp::default();
        app.state = app::AppState::Chat(Box::new(chat_state));

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("crowded chat should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Keep the main transcript visible."));
        assert!(rendered.contains("Command Output"));
        assert!(rendered.contains("Ctrl+O"));
        assert!(!rendered.contains("Commands"));
    }

    #[test]
    fn chat_shows_only_agents_hint_until_opened() {
        let mut chat_state = empty_chat_state();
        chat_state.messages.push(Message {
            role: "user".to_string(),
            content: "Keep chat space available.".to_string(),
        });
        chat_state.active_agents = Some(ResolvedAgents {
            sources: vec![harper_core::core::agents::AgentsSource {
                path: std::path::PathBuf::from("AGENTS.md"),
                content: "Rules".to_string(),
                sections: Vec::new(),
            }],
            effective_sections: Vec::new(),
            effective_rule_sections: vec![harper_core::core::agents::EffectiveAgentsSection {
                heading: Some("Harper Agent Rules".to_string()),
                rules: Vec::new(),
            }],
        });

        let theme = Theme::default();
        refresh_chat_render_cache(&mut chat_state, &theme);

        let mut app = app::TuiApp::default();
        app.state = app::AppState::Chat(Box::new(chat_state));

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("chat should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Keep chat space available."));
        assert!(rendered.contains("AGENTS"));
        assert!(rendered.contains("Ctrl+A"));
        assert!(!rendered.contains("Harper Agent Rules"));
    }

    #[test]
    fn multiline_input_collapses_agents_to_preserve_input_room() {
        let mut chat_state = empty_chat_state();
        chat_state.input = "one\ntwo\nthree\nfour\nfive".to_string();
        chat_state.agents_panel_expanded = true;
        chat_state.active_agents = Some(ResolvedAgents {
            sources: vec![harper_core::core::agents::AgentsSource {
                path: std::path::PathBuf::from("AGENTS.md"),
                content: "Rules".to_string(),
                sections: Vec::new(),
            }],
            effective_sections: Vec::new(),
            effective_rule_sections: vec![harper_core::core::agents::EffectiveAgentsSection {
                heading: Some("Harper Agent Rules".to_string()),
                rules: vec![harper_core::core::agents::EffectiveAgentsRule {
                    text: "Do not occupy input space.".to_string(),
                    source_path: std::path::PathBuf::from("AGENTS.md"),
                }],
            }],
        });

        let mut app = app::TuiApp::default();
        app.state = app::AppState::Chat(Box::new(chat_state));

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("chat should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("one"));
        assert!(rendered.contains("five"));
        assert!(rendered.contains("AGENTS"));
        assert!(!rendered.contains("Do not occupy input space."));
    }

    #[test]
    fn long_input_wraps_inside_input_area() {
        let mut chat_state = empty_chat_state();
        chat_state.input =
            "This pasted request is intentionally long so it must wrap before VISIBLE_TAIL"
                .to_string();

        let mut app = app::TuiApp::default();
        app.state = app::AppState::Chat(Box::new(chat_state));

        let backend = TestBackend::new(54, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("chat should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("VISIBLE_TAIL"));
    }

    #[test]
    fn draw_sessions_compacts_items_on_small_terminal() {
        let sessions = (0..10)
            .map(|index| SessionInfo {
                id: format!("session-id-{index}"),
                name: format!("Session {index}"),
                created_at: "today".to_string(),
            })
            .collect::<Vec<_>>();
        let mut app = app::TuiApp::default();
        app.state = app::AppState::Sessions(sessions, 7);

        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::default();

        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .expect("compact sessions should render");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Session 7"));
        assert!(!rendered.contains("session-id-7"));
    }

    #[test]
    fn test_parse_content_with_code_no_code() {
        let (syntax_set, theme_set) = setup();
        let lines = parse_content_with_code(
            &syntax_set,
            &theme_set,
            "Hello",
            &Theme::default(),
            Color::White,
            "base16-ocean.dark",
        );
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn parse_content_with_code_preserves_fenced_diff_lines() {
        let (syntax_set, theme_set) = setup();
        let lines = parse_content_with_code(
            &syntax_set,
            &theme_set,
            "Git diff:\n```diff\ndiff --git a/foo.rs b/foo.rs\n@@ -1 +1 @@\n-old\n+new\n```",
            &Theme::default(),
            Color::White,
            "base16-ocean.dark",
        );

        assert!(lines.len() >= 5);
        assert_eq!(lines[0].spans[0].content.as_ref(), "Git diff:");
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content
                    .as_ref()
                    .contains("diff --git a/foo.rs b/foo.rs")
            })
        }));
        assert!(lines.iter().filter(|line| !line.spans.is_empty()).count() >= 4);
        let theme = Theme::default();
        assert!(lines.iter().any(|line| {
            line.to_string().contains("diff --git a/foo.rs b/foo.rs")
                && line.style.fg == Some(theme.muted)
        }));
        assert!(lines.iter().any(|line| {
            line.to_string().contains("@@ -1 +1 @@") && line.style.fg == Some(theme.accent)
        }));
        assert!(lines.iter().any(|line| {
            line.to_string().contains("-old") && line.style.fg == Some(theme.error)
        }));
        assert!(lines.iter().any(|line| {
            line.to_string().contains("+new") && line.style.fg == Some(theme.success)
        }));
    }

    #[test]
    fn normalize_plain_message_lines_reflows_wrapped_prose() {
        let theme = Theme::default();
        let lines = normalize_plain_message_lines(
            "I am a large language model.\nI do not have direct access to repositories.\n\nBut I can explain general behavior.",
            Color::White,
            &theme,
        );

        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0].spans[0].content.as_ref(),
            "I am a large language model. I do not have direct access to repositories."
        );
        assert!(lines[1].spans.is_empty());
        assert_eq!(
            lines[2].spans[0].content.as_ref(),
            "But I can explain general behavior."
        );
    }

    #[test]
    fn normalize_plain_message_lines_preserves_structured_lines() {
        let theme = Theme::default();
        let lines = normalize_plain_message_lines(
            "- first item\n- second item\n\n1. numbered item",
            Color::White,
            &theme,
        );

        assert_eq!(lines[0].spans[0].content.as_ref(), "- first item");
        assert_eq!(lines[1].spans[0].content.as_ref(), "- second item");
        assert_eq!(lines[3].spans[0].content.as_ref(), "1. numbered item");
    }

    #[test]
    fn parse_inline_markdown_line_strips_bold_markers() {
        let line = parse_inline_markdown_line("Hello **world**", Color::White, false);

        assert_eq!(line.to_string(), "Hello world");
        assert_eq!(line.spans.len(), 2);
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn parse_inline_markdown_line_strips_code_markers() {
        let line = parse_inline_markdown_line("Use `cargo test` now", Color::White, false);

        assert_eq!(line.to_string(), "Use cargo test now");
        assert_eq!(line.spans.len(), 3);
        assert!(line.spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn normalize_plain_message_lines_highlights_unfenced_python_code() {
        let theme = Theme::default();
        let lines = normalize_plain_message_lines("print(\"Hello Joy\")", Color::White, &theme);

        assert!(!lines.is_empty());
        assert_eq!(lines[0].to_string(), "print(\"Hello Joy\")");
    }

    #[test]
    fn normalize_plain_message_lines_preserves_multiline_unfenced_python_code() {
        let theme = Theme::default();
        let lines = normalize_plain_message_lines(
            "def greet():\n    print(\"Hello Joy\")\n\ngreet()",
            Color::White,
            &theme,
        );

        assert!(lines.len() >= 3);
        assert!(lines
            .iter()
            .any(|line| line.to_string().contains("def greet():")));
        assert!(lines
            .iter()
            .any(|line| line.to_string().contains("print(\"Hello Joy\")")));
        assert!(lines
            .iter()
            .any(|line| line.to_string().contains("greet()")));
    }

    #[test]
    fn normalize_plain_message_lines_handles_intro_followed_by_code_block() {
        let theme = Theme::default();
        let lines = normalize_plain_message_lines(
            "Here is the Python script:\ndef greet():\n    print(\"Hello Joy\")",
            Color::White,
            &theme,
        );

        assert!(lines.len() >= 3);
        assert_eq!(lines[0].to_string(), "Here is the Python script:");
        assert!(lines
            .iter()
            .any(|line| line.to_string().contains("def greet():")));
        assert!(lines
            .iter()
            .any(|line| line.to_string().contains("print(\"Hello Joy\")")));
    }

    #[test]
    fn normalize_plain_message_lines_splits_inline_code_paragraph() {
        let theme = Theme::default();
        let lines = normalize_plain_message_lines(
            "Here is the Python script: print(\"Hello Joy\") Save this content into hello_joy.py",
            Color::White,
            &theme,
        );

        assert!(lines.len() >= 3);
        assert!(lines[0].to_string().contains("Here is the Python script"));
        assert_eq!(lines[1].to_string(), "print(\"Hello Joy\")");
        assert!(lines[2].to_string().contains("Save this content"));
    }

    #[test]
    fn plan_panel_height_accounts_for_recent_jobs() {
        let plan = PlanState {
            explanation: Some("Track execution".to_string()),
            items: vec![PlanItem {
                step: "Run command".to_string(),
                status: PlanStepStatus::InProgress,
                job_id: None,
            }],
            runtime: Some(PlanRuntime {
                active_tool: Some("run_command".to_string()),
                active_command: Some("echo hi".to_string()),
                active_status: Some("running".to_string()),
                active_job_id: Some("job-1".to_string()),
                jobs: vec![
                    PlanJobRecord {
                        job_id: "job-1".to_string(),
                        tool: "run_command".to_string(),
                        command: Some("echo hi".to_string()),
                        status: PlanJobStatus::Running,
                        output_transcript: "line one\nline two".to_string(),
                        output_preview: Some("line one\nline two".to_string()),
                        has_error_output: false,
                    },
                    PlanJobRecord {
                        job_id: "job-2".to_string(),
                        tool: "run_command".to_string(),
                        command: Some("ls".to_string()),
                        status: PlanJobStatus::Succeeded,
                        output_transcript: String::new(),
                        output_preview: None,
                        has_error_output: false,
                    },
                ],
                followup: None,
                followup_history: Vec::new(),
                ..Default::default()
            }),
            updated_at: None,
        };

        assert_eq!(plan_panel_height(&plan), 10);
    }

    #[test]
    fn plan_job_lines_show_recent_jobs_and_overflow() {
        let runtime = PlanRuntime {
            active_tool: None,
            active_command: None,
            active_status: None,
            active_job_id: None,
            jobs: vec![
                PlanJobRecord {
                    job_id: "job-1".to_string(),
                    tool: "run_command".to_string(),
                    command: Some("echo one".to_string()),
                    status: PlanJobStatus::Succeeded,
                    output_transcript: String::new(),
                    output_preview: None,
                    has_error_output: false,
                },
                PlanJobRecord {
                    job_id: "job-2".to_string(),
                    tool: "run_command".to_string(),
                    command: Some("echo two".to_string()),
                    status: PlanJobStatus::Failed,
                    output_transcript: "boom".to_string(),
                    output_preview: Some("boom".to_string()),
                    has_error_output: true,
                },
                PlanJobRecord {
                    job_id: "job-3".to_string(),
                    tool: "run_command".to_string(),
                    command: Some("echo three".to_string()),
                    status: PlanJobStatus::Running,
                    output_transcript: String::new(),
                    output_preview: None,
                    has_error_output: false,
                },
                PlanJobRecord {
                    job_id: "job-4".to_string(),
                    tool: "run_command".to_string(),
                    command: Some("echo four".to_string()),
                    status: PlanJobStatus::WaitingApproval,
                    output_transcript: String::new(),
                    output_preview: None,
                    has_error_output: false,
                },
            ],
            followup: None,
            followup_history: Vec::new(),
            ..Default::default()
        };

        let lines = plan_job_lines(&runtime, 0, true, &Theme::default());

        assert_eq!(lines.len(), 4);
        assert!(lines[0].to_string().contains("succeeded: echo one"));
        assert!(lines[1].to_string().contains("failed: echo two"));
        assert!(lines[2].to_string().contains("running: echo three"));
        assert!(lines[3].to_string().contains("1 more jobs"));
    }

    #[test]
    fn format_plan_job_prefers_command_text() {
        let job = PlanJobRecord {
            job_id: "job-1".to_string(),
            tool: "run_command".to_string(),
            command: Some("echo hi".to_string()),
            status: PlanJobStatus::Blocked,
            output_transcript: String::new(),
            output_preview: None,
            has_error_output: false,
        };

        assert_eq!(format_plan_job(&job), "blocked: echo hi");
    }

    #[test]
    fn plan_job_transcript_lines_use_full_transcript_and_placeholder() {
        let theme = Theme::default();
        let job = PlanJobRecord {
            job_id: "job-1".to_string(),
            tool: "run_command".to_string(),
            command: Some("echo hi".to_string()),
            status: PlanJobStatus::Succeeded,
            output_transcript: "line one\nline two\nline three".to_string(),
            output_preview: Some("line one\nline two".to_string()),
            has_error_output: false,
        };
        let placeholder_job = PlanJobRecord {
            output_transcript: String::new(),
            ..job.clone()
        };

        let transcript_lines = plan_job_transcript_lines(&job, &theme);
        let placeholder_lines = plan_job_transcript_lines(&placeholder_job, &theme);

        assert_eq!(transcript_lines.len(), 3);
        assert_eq!(transcript_lines[0].to_string(), "line one");
        assert_eq!(transcript_lines[2].to_string(), "line three");
        assert_eq!(placeholder_lines[0].to_string(), "No output recorded yet");
    }

    #[test]
    fn plan_followup_lines_render_retry_metadata() {
        let runtime = PlanRuntime {
            followup: Some(PlanFollowup::RetryOrReplan {
                step: "Patch handler".to_string(),
                command: Some("cargo test".to_string()),
                retry_count: 2,
            }),
            followup_history: Vec::new(),
            ..Default::default()
        };

        let lines = plan_followup_lines(&runtime, &Theme::default());

        assert_eq!(lines.len(), 2);
        assert!(lines[0].to_string().contains("retry 2: Patch handler"));
        assert!(lines[1].to_string().contains("command: cargo test"));
    }

    #[test]
    fn selected_step_followup_lines_render_checkpoint_metadata() {
        let runtime = PlanRuntime {
            followup: Some(PlanFollowup::Checkpoint {
                step: "Inspect server file".to_string(),
                next_step: Some("Patch handler".to_string()),
            }),
            followup_history: Vec::new(),
            ..Default::default()
        };

        let lines =
            selected_step_followup_lines(Some(&runtime), "Inspect server file", &Theme::default());

        assert_eq!(lines.len(), 2);
        assert!(lines[0].to_string().contains("checkpoint pending"));
        assert!(lines[1].to_string().contains("next step: Patch handler"));
    }

    #[test]
    fn detect_command_output_language_identifies_git_diff() {
        let command = "git diff -- lib/harper-core/src/agent/chat.rs";
        let content = "Git diff:\ndiff --git a/file.rs b/file.rs\n@@ -1 +1 @@\n-old\n+new\n";

        assert_eq!(
            detect_command_output_language(command, content),
            Some("diff")
        );
    }

    #[test]
    fn detect_command_output_language_falls_back_to_content_shape() {
        let command = "show output";
        let content = "def greet():\n    print(\"Hello Joy\")";

        assert_eq!(detect_command_output_language(command, content), Some("py"));
    }

    #[test]
    fn derive_command_output_display_falls_back_to_plan_runtime_job_output() {
        let mut chat_state = empty_chat_state();
        chat_state.active_plan = Some(PlanState {
            explanation: None,
            items: Vec::new(),
            runtime: Some(PlanRuntime {
                active_tool: Some("run_command".to_string()),
                active_command: Some("cargo test".to_string()),
                active_status: Some("running".to_string()),
                active_job_id: Some("job-1".to_string()),
                jobs: vec![PlanJobRecord {
                    job_id: "job-1".to_string(),
                    tool: "run_command".to_string(),
                    command: Some("cargo test".to_string()),
                    status: PlanJobStatus::Running,
                    output_transcript: "line one\nline two\nline three".to_string(),
                    output_preview: Some("line two\nline three".to_string()),
                    has_error_output: false,
                }],
                followup: None,
                followup_history: Vec::new(),
                ..Default::default()
            }),
            updated_at: None,
        });

        let output = derive_command_output_display(&chat_state).expect("derived output");

        assert_eq!(output.command, "cargo test");
        assert_eq!(output.status_label, "running");
        assert_eq!(output.total_line_count, 3);
        assert!(output.content.contains("line one"));
        assert!(output.content.contains("line three"));
    }

    #[test]
    fn plan_followup_history_lines_render_recent_entries() {
        let runtime = PlanRuntime {
            followup: None,
            followup_history: vec![
                PlanFollowup::Checkpoint {
                    step: "Inspect output".to_string(),
                    next_step: Some("Patch handler".to_string()),
                },
                PlanFollowup::RetryOrReplan {
                    step: "Retry failing command".to_string(),
                    command: Some("cargo test".to_string()),
                    retry_count: 2,
                },
            ],
            ..Default::default()
        };

        let lines = plan_followup_history_lines(&runtime, &Theme::default());

        assert_eq!(lines.len(), 3);
        assert!(lines[0].to_string().contains("recent followups"));
        assert!(lines[1]
            .to_string()
            .contains("retry 2: Retry failing command"));
        assert!(lines[2]
            .to_string()
            .contains("checkpoint: Inspect output → Patch handler"));
    }

    #[test]
    fn plan_loop_state_lines_render_stage_outcome_and_feedback() {
        let runtime = PlanRuntime {
            loop_stage: Some(PlanLoopStage::Inspecting),
            last_outcome: Some(PlanLoopOutcome::RetryPending),
            last_feedback: Some("inspect relevant files before editing".to_string()),
            ..Default::default()
        };

        let lines = plan_loop_state_lines(&runtime, &Theme::default());

        assert_eq!(lines.len(), 3);
        assert!(lines[0].to_string().contains("loop: inspect"));
        assert!(lines[1]
            .to_string()
            .contains("last outcome: retry suggested"));
        assert!(lines[2]
            .to_string()
            .contains("inspect relevant files before editing"));
    }

    #[test]
    fn loop_state_lines_render_non_plan_chat_state() {
        let lines = loop_state_lines(
            Some(&PlanLoopStage::Responding),
            Some(&PlanLoopOutcome::Responded),
            Some("answered directly without planning"),
            &Theme::default(),
        );

        assert_eq!(lines.len(), 3);
        assert!(lines[0].to_string().contains("loop: respond"));
        assert!(lines[1].to_string().contains("last outcome: responded"));
        assert!(lines[2]
            .to_string()
            .contains("answered directly without planning"));
    }

    #[test]
    fn latest_action_summary_does_not_echo_plain_assistant_reply() {
        let mut chat_state = empty_chat_state();
        chat_state.messages.push(Message {
            role: "user".to_string(),
            content: "hi".to_string(),
        });
        chat_state.messages.push(Message {
            role: "assistant".to_string(),
            content: "Hello! How can I assist you today?".to_string(),
        });

        let app = app::TuiApp::default();
        assert!(latest_action_summary(&app, &chat_state).is_none());
    }

    #[test]
    fn respond_only_loop_panel_is_hidden() {
        let loop_state = app::ChatLoopState {
            stage: Some(PlanLoopStage::Responding),
            last_outcome: Some(PlanLoopOutcome::Responded),
            last_feedback: None,
        };

        assert!(!should_render_chat_loop_panel(&loop_state));
    }
}
