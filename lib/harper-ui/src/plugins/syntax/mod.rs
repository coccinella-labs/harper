// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

// Note: We use syntect for syntax highlighting due to its comprehensive language support
// and theme system. The yaml-rust dependency is unmaintained but poses no security risk
// as it's only used for YAML theme file parsing. syntect is actively maintained and
// provides the best syntax highlighting experience. This warning is acknowledged and
// documented in deny.toml and CI configuration.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

pub fn highlight_code(
    syntax_set: &SyntaxSet,
    theme_set: &ThemeSet,
    language: &str,
    code: &str,
    theme_name: &str,
) -> Vec<Span<'static>> {
    let syntax = resolve_syntax(syntax_set, language);

    let theme = theme_set
        .themes
        .get(theme_name)
        .or_else(|| theme_set.themes.get("base16-ocean.dark"))
        .expect("Could not find the requested theme or the default 'base16-ocean.dark' theme.");
    let mut highlighter = HighlightLines::new(syntax, theme);

    LinesWithEndings::from(code)
        .flat_map(|line| {
            let ranges: Vec<(SynStyle, &str)> = highlighter
                .highlight_line(line, syntax_set)
                .unwrap_or_default();

            ranges
                .into_iter()
                .map(|(style, text)| {
                    let color = syntect_to_ratatui_color(style.foreground);
                    Span::styled(text.to_string(), Style::default().fg(color))
                })
                .collect::<Vec<Span>>()
        })
        .collect()
}

pub fn highlight_code_lines(
    syntax_set: &SyntaxSet,
    theme_set: &ThemeSet,
    language: &str,
    code: &str,
    theme_name: &str,
) -> Vec<Line<'static>> {
    let syntax = resolve_syntax(syntax_set, language);
    let theme = theme_set
        .themes
        .get(theme_name)
        .or_else(|| theme_set.themes.get("base16-ocean.dark"))
        .expect("Could not find the requested theme or the default 'base16-ocean.dark' theme.");
    let mut highlighter = HighlightLines::new(syntax, theme);

    LinesWithEndings::from(code)
        .map(|line| {
            let ranges: Vec<(SynStyle, &str)> = highlighter
                .highlight_line(line, syntax_set)
                .unwrap_or_default();
            let spans = ranges
                .into_iter()
                .map(|(style, text)| {
                    let color = syntect_to_ratatui_color(style.foreground);
                    Span::styled(text.to_string(), Style::default().fg(color))
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}

fn resolve_syntax<'a>(
    syntax_set: &'a SyntaxSet,
    language: &str,
) -> &'a syntect::parsing::SyntaxReference {
    syntax_set
        .find_syntax_by_extension(language)
        .or_else(|| syntax_set.find_syntax_by_token(language))
        .or_else(|| syntax_set.find_syntax_by_name(language))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text())
}

fn syntect_to_ratatui_color(color: syntect::highlighting::Color) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (SyntaxSet, ThemeSet) {
        (
            SyntaxSet::load_defaults_newlines(),
            ThemeSet::load_defaults(),
        )
    }

    #[test]
    fn test_highlight_code_rust() {
        let (syntax_set, theme_set) = setup();
        let code = "fn main() {\n    println!(\"Hello\");\n}";
        let spans = highlight_code(&syntax_set, &theme_set, "rs", code, "base16-ocean.dark");
        assert!(!spans.is_empty());
        // Check that spans contain expected text
        let all_text: String = spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_text.contains("fn main()"));
        assert!(all_text.contains("println!"));
    }

    #[test]
    fn test_highlight_code_unknown_language() {
        let (syntax_set, theme_set) = setup();
        let code = "print('hello')";
        let spans = highlight_code(
            &syntax_set,
            &theme_set,
            "unknown",
            code,
            "base16-ocean.dark",
        );
        assert!(!spans.is_empty());
        // Should fall back to plain text syntax
    }

    #[test]
    fn test_highlight_code_empty() {
        let (syntax_set, theme_set) = setup();
        let spans = highlight_code(&syntax_set, &theme_set, "rs", "", "base16-ocean.dark");
        assert!(spans.is_empty());
    }
}
