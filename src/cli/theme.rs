//! Theme selection for `shore diff`'s truecolor lane: preference grammar,
//! precedence, palettes, and terminal-background detection.

// Consumed by the diff render path once the wiring lands; until then the
// items here are dead code to the binary. Remove this allow with that wiring.
#![allow(dead_code)]

use std::borrow::Cow;

use shoreline::highlight::TokenKind;

/// Resolved lightness class for the truecolor palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiffMode {
    Light,
    Dark,
}

/// A parsed theme preference: detect, force a mode's built-in palette, or a
/// named embedded theme. Parsing is infallible — unknown names are resolved
/// (and rejected or warned about) later, with source-aware posture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ThemePreference {
    Auto,
    Mode(DiffMode),
    Named(String),
}

/// The theme value grammar: `auto` (and any `auto:*` variant, for BAT_THEME
/// compatibility), `light`, `dark`, `default` (bat back-compat: the dark
/// default), else a verbatim theme name. Keywords are case-sensitive
/// lowercase; theme names are case-sensitive too, so anything unrecognized
/// passes through untouched.
pub(super) fn parse_theme_value(value: &str) -> ThemePreference {
    let value = value.trim();
    if value == "auto" || value.starts_with("auto:") {
        return ThemePreference::Auto;
    }
    match value {
        "light" => ThemePreference::Mode(DiffMode::Light),
        "dark" | "default" => ThemePreference::Mode(DiffMode::Dark),
        other => ThemePreference::Named(other.to_string()),
    }
}

/// The truecolor lane's palette: one foreground SGR per token kind plus the
/// two intraline-emphasis background tints. Built-ins are `'static`; a palette
/// derived from an embedded theme owns its strings — hence `Cow`.
pub(super) struct DiffPalette {
    pub(super) keyword: Cow<'static, str>,
    pub(super) string: Cow<'static, str>,
    pub(super) comment: Cow<'static, str>,
    pub(super) number: Cow<'static, str>,
    pub(super) r#type: Cow<'static, str>,
    pub(super) function: Cow<'static, str>,
    pub(super) constant: Cow<'static, str>,
    pub(super) operator: Cow<'static, str>,
    pub(super) punctuation: Cow<'static, str>,
    pub(super) variable: Cow<'static, str>,
    /// Background tint for an emphasized segment on an Added row.
    pub(super) emph_add_bg: Cow<'static, str>,
    /// Background tint for an emphasized segment on a Removed row.
    pub(super) emph_del_bg: Cow<'static, str>,
}

impl DiffPalette {
    pub(super) fn sgr_for(&self, kind: TokenKind) -> &str {
        match kind {
            TokenKind::Keyword => &self.keyword,
            TokenKind::String => &self.string,
            TokenKind::Comment => &self.comment,
            TokenKind::Number => &self.number,
            TokenKind::Type => &self.r#type,
            TokenKind::Function => &self.function,
            TokenKind::Constant => &self.constant,
            TokenKind::Operator => &self.operator,
            TokenKind::Punctuation => &self.punctuation,
            TokenKind::Variable => &self.variable,
            TokenKind::Plain => "",
        }
    }

    pub(super) fn builtin_for(mode: DiffMode) -> DiffPalette {
        match mode {
            DiffMode::Dark => Self::builtin_dark(),
            DiffMode::Light => Self::builtin_light(),
        }
    }

    /// Today's truecolor palette (the inspector's dark `--tok-*` hues) — the
    /// compatibility-frozen default. Emph tints are delta's dark constants.
    pub(super) const fn builtin_dark() -> DiffPalette {
        DiffPalette {
            keyword: Cow::Borrowed("\x1b[38;2;179;136;255m"), // --assess
            string: Cow::Borrowed("\x1b[38;2;109;210;138m"),  // --success
            comment: Cow::Borrowed("\x1b[38;2;154;165;179m"), // --fg-dim
            number: Cow::Borrowed("\x1b[38;2;79;208;192m"),   // --teal
            r#type: Cow::Borrowed("\x1b[38;2;138;180;248m"),  // --info
            function: Cow::Borrowed("\x1b[38;2;90;169;230m"), // --accent
            constant: Cow::Borrowed("\x1b[38;2;240;183;90m"), // --warning
            operator: Cow::Borrowed("\x1b[38;2;215;221;229m"), // --fg
            punctuation: Cow::Borrowed("\x1b[38;2;154;165;179m"), // --fg-dim
            variable: Cow::Borrowed("\x1b[38;2;215;221;229m"), // --fg
            emph_add_bg: Cow::Borrowed("\x1b[48;2;0;96;0m"),  // delta dark #006000
            emph_del_bg: Cow::Borrowed("\x1b[48;2;144;16;17m"), // delta dark #901011
        }
    }

    /// The inspector's light-theme `--tok-*` hues
    /// (`src/cli/inspect/assets/tokens.css`, `[data-theme="light"]`). Emph
    /// tints are delta's light constants.
    pub(super) const fn builtin_light() -> DiffPalette {
        DiffPalette {
            keyword: Cow::Borrowed("\x1b[38;2;122;68;212m"), // --assess #7a44d4
            string: Cow::Borrowed("\x1b[38;2;26;127;55m"),   // --success #1a7f37
            comment: Cow::Borrowed("\x1b[38;2;76;97;115m"),  // --fg-dim #4c6173
            number: Cow::Borrowed("\x1b[38;2;15;111;102m"),  // --teal #0f6f66
            r#type: Cow::Borrowed("\x1b[38;2;9;105;218m"),   // --info #0969da
            function: Cow::Borrowed("\x1b[38;2;3;105;161m"), // --accent #0369a1
            constant: Cow::Borrowed("\x1b[38;2;138;93;0m"),  // --warning #8a5d00
            operator: Cow::Borrowed("\x1b[38;2;20;36;51m"),  // --fg #142433
            punctuation: Cow::Borrowed("\x1b[38;2;76;97;115m"), // --fg-dim #4c6173
            variable: Cow::Borrowed("\x1b[38;2;20;36;51m"),  // --fg #142433
            emph_add_bg: Cow::Borrowed("\x1b[48;2;160;239;160m"), // delta light #a0efa0
            emph_del_bg: Cow::Borrowed("\x1b[48;2;255;192;192m"), // delta light #ffc0c0
        }
    }
}

/// Where a theme selection came from — governs the unknown-name posture:
/// explicit sources fail hard, the inherited source warns and falls back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ThemeSource {
    Explicit,
    Inherited,
    Default,
}

/// A resolved preference plus its provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ThemeSelection {
    pub(super) preference: ThemePreference,
    pub(super) source: ThemeSource,
}

/// Pure precedence core: `--theme` flag > `SHORE_THEME` > `BAT_THEME` > Auto.
/// Blank (empty/whitespace) values are no selection. Injected values keep it
/// unit-testable without touching (or racing on) the process environment.
pub(super) fn resolve_theme_selection(
    flag: Option<&str>,
    shore_env: Option<&str>,
    bat_env: Option<&str>,
) -> ThemeSelection {
    fn pick(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
    }
    if let Some(value) = pick(flag).or(pick(shore_env)) {
        return ThemeSelection {
            preference: parse_theme_value(value),
            source: ThemeSource::Explicit,
        };
    }
    if let Some(value) = pick(bat_env) {
        return ThemeSelection {
            preference: parse_theme_value(value),
            source: ThemeSource::Inherited,
        };
    }
    ThemeSelection {
        preference: ThemePreference::Auto,
        source: ThemeSource::Default,
    }
}

/// Read `SHORE_THEME` / `BAT_THEME` and delegate to the pure core. The single
/// theme-env read site (the `SHORE_FORMAT` convention, `src/cli/output.rs`).
pub(super) fn theme_selection_from_env(flag: Option<&str>) -> ThemeSelection {
    let shore = std::env::var("SHORE_THEME").ok();
    let bat = std::env::var("BAT_THEME").ok();
    resolve_theme_selection(flag, shore.as_deref(), bat.as_deref())
}

#[cfg(test)]
mod tests {
    use shoreline::highlight::TokenKind;

    use super::*;

    #[test]
    fn builtin_dark_pins_todays_truecolor_bytes() {
        // The dark built-in is byte-identical to the landed truecolor palette.
        let p = DiffPalette::builtin_dark();
        assert_eq!(p.sgr_for(TokenKind::Keyword), "\x1b[38;2;179;136;255m");
        assert_eq!(p.sgr_for(TokenKind::String), "\x1b[38;2;109;210;138m");
        assert_eq!(p.sgr_for(TokenKind::Comment), "\x1b[38;2;154;165;179m");
        assert_eq!(p.sgr_for(TokenKind::Number), "\x1b[38;2;79;208;192m");
        assert_eq!(p.sgr_for(TokenKind::Type), "\x1b[38;2;138;180;248m");
        assert_eq!(p.sgr_for(TokenKind::Function), "\x1b[38;2;90;169;230m");
        assert_eq!(p.sgr_for(TokenKind::Constant), "\x1b[38;2;240;183;90m");
        assert_eq!(p.sgr_for(TokenKind::Operator), "\x1b[38;2;215;221;229m");
        assert_eq!(p.sgr_for(TokenKind::Punctuation), "\x1b[38;2;154;165;179m");
        assert_eq!(p.sgr_for(TokenKind::Variable), "\x1b[38;2;215;221;229m");
        assert_eq!(p.sgr_for(TokenKind::Plain), "");
    }

    #[test]
    fn builtin_light_mirrors_inspector_light_tokens() {
        // tokens.css [data-theme="light"]: --assess/#7a44d4, --success/#1a7f37,
        // --fg-dim/#4c6173, --teal/#0f6f66, --info/#0969da, --accent/#0369a1,
        // --warning/#8a5d00, --fg/#142433.
        let p = DiffPalette::builtin_light();
        assert_eq!(p.sgr_for(TokenKind::Keyword), "\x1b[38;2;122;68;212m");
        assert_eq!(p.sgr_for(TokenKind::String), "\x1b[38;2;26;127;55m");
        assert_eq!(p.sgr_for(TokenKind::Comment), "\x1b[38;2;76;97;115m");
        assert_eq!(p.sgr_for(TokenKind::Number), "\x1b[38;2;15;111;102m");
        assert_eq!(p.sgr_for(TokenKind::Type), "\x1b[38;2;9;105;218m");
        assert_eq!(p.sgr_for(TokenKind::Function), "\x1b[38;2;3;105;161m");
        assert_eq!(p.sgr_for(TokenKind::Constant), "\x1b[38;2;138;93;0m");
        assert_eq!(p.sgr_for(TokenKind::Operator), "\x1b[38;2;20;36;51m");
        assert_eq!(p.sgr_for(TokenKind::Punctuation), "\x1b[38;2;76;97;115m");
        assert_eq!(p.sgr_for(TokenKind::Variable), "\x1b[38;2;20;36;51m");
        assert_eq!(p.sgr_for(TokenKind::Plain), "");
    }

    #[test]
    fn builtin_emph_tints_are_deltas_constants() {
        // delta's fixed per-mode intraline emphasis backgrounds.
        let dark = DiffPalette::builtin_dark();
        assert_eq!(dark.emph_add_bg, "\x1b[48;2;0;96;0m"); // #006000
        assert_eq!(dark.emph_del_bg, "\x1b[48;2;144;16;17m"); // #901011
        let light = DiffPalette::builtin_light();
        assert_eq!(light.emph_add_bg, "\x1b[48;2;160;239;160m"); // #a0efa0
        assert_eq!(light.emph_del_bg, "\x1b[48;2;255;192;192m"); // #ffc0c0
    }

    #[test]
    fn builtin_for_mode_selects_the_matching_builtin() {
        assert_eq!(
            DiffPalette::builtin_for(DiffMode::Light).sgr_for(TokenKind::Keyword),
            DiffPalette::builtin_light().sgr_for(TokenKind::Keyword)
        );
        assert_eq!(
            DiffPalette::builtin_for(DiffMode::Dark).sgr_for(TokenKind::Keyword),
            DiffPalette::builtin_dark().sgr_for(TokenKind::Keyword)
        );
    }

    #[test]
    fn parses_keywords_and_names() {
        assert_eq!(parse_theme_value("auto"), ThemePreference::Auto);
        assert_eq!(
            parse_theme_value("light"),
            ThemePreference::Mode(DiffMode::Light)
        );
        assert_eq!(
            parse_theme_value("dark"),
            ThemePreference::Mode(DiffMode::Dark)
        );
        // bat back-compat: an explicit "default" always means the dark default.
        assert_eq!(
            parse_theme_value("default"),
            ThemePreference::Mode(DiffMode::Dark)
        );
        // bat's extended auto grammar collapses to Auto (shore's gate governs).
        assert_eq!(parse_theme_value("auto:always"), ThemePreference::Auto);
        assert_eq!(parse_theme_value("auto:system"), ThemePreference::Auto);
        // Anything else is a named theme, verbatim (names are case-sensitive).
        assert_eq!(
            parse_theme_value("Monokai Extended"),
            ThemePreference::Named("Monokai Extended".to_string())
        );
        // Keywords are case-sensitive lowercase; "Dark" is a (bogus) name, not a mode.
        assert_eq!(
            parse_theme_value("Dark"),
            ThemePreference::Named("Dark".to_string())
        );
    }

    #[test]
    fn precedence_flag_over_shore_env_over_bat_env() {
        let sel = resolve_theme_selection(Some("light"), Some("dark"), Some("Nord"));
        assert_eq!(sel.preference, ThemePreference::Mode(DiffMode::Light));
        assert_eq!(sel.source, ThemeSource::Explicit);

        let sel = resolve_theme_selection(None, Some("dark"), Some("Nord"));
        assert_eq!(sel.preference, ThemePreference::Mode(DiffMode::Dark));
        assert_eq!(sel.source, ThemeSource::Explicit);

        let sel = resolve_theme_selection(None, None, Some("Nord"));
        assert_eq!(sel.preference, ThemePreference::Named("Nord".to_string()));
        assert_eq!(sel.source, ThemeSource::Inherited);

        let sel = resolve_theme_selection(None, None, None);
        assert_eq!(sel.preference, ThemePreference::Auto);
        assert_eq!(sel.source, ThemeSource::Default);
    }

    #[test]
    fn empty_or_blank_values_are_no_selection() {
        // Unset and empty env are the same thing (SHORE_FORMAT precedent).
        let sel = resolve_theme_selection(None, Some(""), Some("  "));
        assert_eq!(sel.preference, ThemePreference::Auto);
        assert_eq!(sel.source, ThemeSource::Default);
        // An empty SHORE_THEME does not mask BAT_THEME.
        let sel = resolve_theme_selection(None, Some(""), Some("Nord"));
        assert_eq!(sel.source, ThemeSource::Inherited);
    }

    #[test]
    fn trims_surrounding_whitespace_only() {
        assert_eq!(
            parse_theme_value("  light "),
            ThemePreference::Mode(DiffMode::Light)
        );
        // Interior whitespace stays (theme names contain spaces).
        assert_eq!(
            parse_theme_value(" Solarized (dark) "),
            ThemePreference::Named("Solarized (dark)".to_string())
        );
    }
}
