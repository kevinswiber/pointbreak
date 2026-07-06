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
#[derive(Debug)]
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

/// One representative scope per token kind. These query the THEME's rules and
/// are independent of the tokenizer's scopes — the classify step
/// (`src/highlight/tokenize.rs`) stays the single mapping from source text to
/// kinds; themes only recolor kinds.
const REPRESENTATIVE_SCOPES: [(TokenKind, &str); 10] = [
    (TokenKind::Keyword, "keyword"),
    (TokenKind::String, "string"),
    (TokenKind::Comment, "comment"),
    (TokenKind::Number, "constant.numeric"),
    (TokenKind::Type, "storage.type"),
    (TokenKind::Function, "entity.name.function"),
    (TokenKind::Constant, "constant"),
    (TokenKind::Operator, "keyword.operator"),
    (TokenKind::Punctuation, "punctuation"),
    (TokenKind::Variable, "variable"),
];

/// syntect `Color` → foreground SGR, honoring bat's alpha sentinels (used by
/// the embedded `ansi`/`base16` themes): `a==0` → ANSI palette index in `r`
/// (basic 30–37 below 8, else 256-color), `a==1` → terminal default (no
/// escape), otherwise 24-bit truecolor.
fn sgr_fg_for_color(color: syntect::highlighting::Color) -> String {
    match color.a {
        0 if color.r < 8 => format!("\x1b[3{}m", color.r),
        0 => format!("\x1b[38;5;{}m", color.r),
        1 => String::new(),
        _ => format!("\x1b[38;2;{};{};{}m", color.r, color.g, color.b),
    }
}

impl DiffPalette {
    /// Derive a palette from an embedded theme: per-kind foreground from the
    /// theme's style for a representative scope; emph tints from `mode`
    /// (delta's constants, not theme-derived — matching delta itself). A
    /// scope with no theme rule inherits the theme's default foreground — an
    /// explicit fg SGR that approximates plain text; acceptable.
    pub(super) fn from_theme(theme: &syntect::highlighting::Theme, mode: DiffMode) -> DiffPalette {
        use syntect::highlighting::Highlighter;
        use syntect::parsing::Scope;
        let highlighter = Highlighter::new(theme);
        let sgr_for_kind = |kind: TokenKind| -> Cow<'static, str> {
            let (_, scope) = REPRESENTATIVE_SCOPES
                .iter()
                .find(|(k, _)| *k == kind)
                .expect("every non-Plain kind has a representative scope");
            // Literal scopes in REPRESENTATIVE_SCOPES always parse.
            let scope = Scope::new(scope).expect("representative scope parses");
            let style = highlighter.style_for_stack(&[scope]);
            Cow::Owned(sgr_fg_for_color(style.foreground))
        };
        let base = DiffPalette::builtin_for(mode); // supplies the emph pair
        DiffPalette {
            keyword: sgr_for_kind(TokenKind::Keyword),
            string: sgr_for_kind(TokenKind::String),
            comment: sgr_for_kind(TokenKind::Comment),
            number: sgr_for_kind(TokenKind::Number),
            r#type: sgr_for_kind(TokenKind::Type),
            function: sgr_for_kind(TokenKind::Function),
            constant: sgr_for_kind(TokenKind::Constant),
            operator: sgr_for_kind(TokenKind::Operator),
            punctuation: sgr_for_kind(TokenKind::Punctuation),
            variable: sgr_for_kind(TokenKind::Variable),
            emph_add_bg: base.emph_add_bg,
            emph_del_bg: base.emph_del_bg,
        }
    }
}

/// Terminal gate: the terminal may be queried for its background only when
/// ANSI color is actually being emitted, stdout is a real TTY (piped output
/// must stay deterministic and non-interactive), and the truecolor lane is
/// active (the named-16 lane has nothing to select). Deliberately stricter
/// than bat, which queries even when `NO_COLOR` has turned colors off. The
/// caller additionally requires the resolved preference to be `Auto` — an
/// explicit mode or theme-name choice never queries, while an explicitly
/// requested `auto` detects like the default (bat semantics).
pub(super) fn detection_allowed(colored: bool, stdout_is_tty: bool, truecolor: bool) -> bool {
    colored && stdout_is_tty && truecolor
}

/// Look up an embedded theme by its bat-compatible name (case-sensitive).
/// The single two-face read site. `EmbeddedLazyThemeSet::get` takes the
/// `EmbeddedThemeName` enum, so by-name lookup goes through the inner
/// `LazyThemeSet` (via the provided `From` impl), whose `get(&str)` is the
/// name-keyed accessor.
pub(super) fn theme_by_name(name: &str) -> Option<syntect::highlighting::Theme> {
    let set: two_face::theme::LazyThemeSet = two_face::theme::extra().into();
    set.get(name).cloned()
}

/// Every embedded theme name, for unknown-name error messages. Sorted for
/// stable output.
pub(super) fn available_theme_names() -> Vec<String> {
    let mut names: Vec<String> = two_face::theme::EmbeddedLazyThemeSet::theme_names()
        .iter()
        .map(|name| name.as_name().to_string())
        .collect();
    names.sort();
    names
}

/// Which emph pair a named theme gets: BT.601 integer luminance of the
/// theme's background. `None` (or a bat alpha-sentinel background) yields no
/// verdict — the caller falls back to Dark without a terminal query.
pub(super) fn classify_theme_mode(theme: &syntect::highlighting::Theme) -> Option<DiffMode> {
    let bg = theme.settings.background?;
    if bg.a == 0 || bg.a == 1 {
        return None;
    }
    let luma = (299 * u32::from(bg.r) + 587 * u32::from(bg.g) + 114 * u32::from(bg.b)) / 1000;
    Some(if luma >= 128 {
        DiffMode::Light
    } else {
        DiffMode::Dark
    })
}

/// A resolved truecolor palette plus at most one warning line for stderr
/// (e.g. an unknown inherited BAT_THEME that was ignored).
#[derive(Debug)]
pub(super) struct PaletteChoice {
    pub(super) palette: DiffPalette,
    pub(super) warning: Option<String>,
}

/// Query the terminal background via `terminal-colorsaurus` (the same crate
/// bat and delta use; OSC 10+11 with the crate's default bounded timeout,
/// kept for bat/delta parity). The ONLY crate call site. Callers must apply
/// [`detection_allowed`] first — this function unconditionally queries.
pub(super) fn detect_mode() -> Option<DiffMode> {
    use terminal_colorsaurus::{QueryOptions, ThemeMode, theme_mode};
    match theme_mode(QueryOptions::default()).ok()? {
        ThemeMode::Dark => Some(DiffMode::Dark),
        ThemeMode::Light => Some(DiffMode::Light),
    }
}

/// Turn a selection into the truecolor palette. The detector is injected so
/// the gate (and tests) control querying; it is invoked at most once, and
/// only where the Auto behavior applies — never for an explicit mode or a
/// known theme name. Unknown names are judged by provenance: explicit
/// sources fail hard with the valid vocabulary, the inherited BAT_THEME
/// warns and falls back to the Auto behavior.
pub(super) fn resolve_truecolor_palette(
    selection: &ThemeSelection,
    detect: impl FnOnce() -> Option<DiffMode>,
) -> Result<PaletteChoice, Box<dyn std::error::Error>> {
    fn auto_choice(detected: Option<DiffMode>) -> PaletteChoice {
        PaletteChoice {
            palette: DiffPalette::builtin_for(detected.unwrap_or(DiffMode::Dark)),
            warning: None,
        }
    }
    match &selection.preference {
        ThemePreference::Auto => Ok(auto_choice(detect())),
        ThemePreference::Mode(mode) => Ok(PaletteChoice {
            palette: DiffPalette::builtin_for(*mode),
            warning: None,
        }),
        ThemePreference::Named(name) => match theme_by_name(name) {
            Some(theme) => {
                // A theme with no classifiable background gets the dark pair
                // without an extra terminal query.
                let mode = classify_theme_mode(&theme).unwrap_or(DiffMode::Dark);
                Ok(PaletteChoice {
                    palette: DiffPalette::from_theme(&theme, mode),
                    warning: None,
                })
            }
            None if selection.source == ThemeSource::Inherited => {
                let mut choice = auto_choice(detect());
                choice.warning = Some(format!(
                    "ignoring unknown BAT_THEME value {name:?}; using the built-in palette"
                ));
                Ok(choice)
            }
            None => Err(format!(
                "unknown theme {name:?}: expected auto, light, dark, default, or one of: {}",
                available_theme_names().join(", ")
            )
            .into()),
        },
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

    fn color(r: u8, g: u8, b: u8, a: u8) -> syntect::highlighting::Color {
        syntect::highlighting::Color { r, g, b, a }
    }

    fn theme_with_background(
        bg: Option<syntect::highlighting::Color>,
    ) -> syntect::highlighting::Theme {
        let mut theme = syntect::highlighting::Theme::default();
        theme.settings.background = bg;
        theme
    }

    fn selection(preference: ThemePreference, source: ThemeSource) -> ThemeSelection {
        ThemeSelection { preference, source }
    }

    #[test]
    fn auto_uses_the_detector_and_falls_back_dark() {
        let sel = selection(ThemePreference::Auto, ThemeSource::Default);
        let light = resolve_truecolor_palette(&sel, || Some(DiffMode::Light)).unwrap();
        assert_eq!(
            light.palette.sgr_for(TokenKind::Keyword),
            "\x1b[38;2;122;68;212m"
        );
        assert!(light.warning.is_none());
        // No verdict (unsupported terminal, timeout, gate closed) → Dark.
        let dark = resolve_truecolor_palette(&sel, || None).unwrap();
        assert_eq!(
            dark.palette.sgr_for(TokenKind::Keyword),
            "\x1b[38;2;179;136;255m"
        );
    }

    #[test]
    fn explicit_auto_detects_like_the_default() {
        // Provenance does not gate Auto: `--theme auto` / `SHORE_THEME=auto`
        // is an explicit request to detect, bat semantics.
        let sel = selection(ThemePreference::Auto, ThemeSource::Explicit);
        let light = resolve_truecolor_palette(&sel, || Some(DiffMode::Light)).unwrap();
        assert_eq!(
            light.palette.sgr_for(TokenKind::Keyword),
            "\x1b[38;2;122;68;212m"
        );
    }

    #[test]
    fn explicit_mode_never_calls_the_detector() {
        let sel = selection(
            ThemePreference::Mode(DiffMode::Light),
            ThemeSource::Explicit,
        );
        let choice = resolve_truecolor_palette(&sel, || panic!("must not detect")).unwrap();
        assert_eq!(
            choice.palette.sgr_for(TokenKind::Keyword),
            "\x1b[38;2;122;68;212m"
        );
    }

    #[test]
    fn named_theme_derives_and_never_calls_the_detector() {
        let sel = selection(
            ThemePreference::Named("Monokai Extended".to_string()),
            ThemeSource::Explicit,
        );
        let choice = resolve_truecolor_palette(&sel, || panic!("must not detect")).unwrap();
        // Derived palette: theme fg, dark emph pair (Monokai classifies Dark).
        assert!(
            choice
                .palette
                .sgr_for(TokenKind::Keyword)
                .starts_with("\x1b[38;2;")
        );
        assert_eq!(choice.palette.emph_add_bg, "\x1b[48;2;0;96;0m");
    }

    #[test]
    fn unknown_name_errors_hard_when_explicit() {
        let sel = selection(
            ThemePreference::Named("no-such-theme".to_string()),
            ThemeSource::Explicit,
        );
        let err = resolve_truecolor_palette(&sel, || None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no-such-theme"));
        assert!(err.contains("auto, light, dark, default"));
        assert!(err.contains("Monokai Extended")); // lists the embedded names
    }

    #[test]
    fn unknown_name_warns_and_falls_back_when_inherited() {
        // BAT_THEME may name a user-cached bat theme shore does not embed —
        // warn once and behave as Auto.
        let sel = selection(
            ThemePreference::Named("no-such-theme".to_string()),
            ThemeSource::Inherited,
        );
        let choice = resolve_truecolor_palette(&sel, || Some(DiffMode::Light)).unwrap();
        assert_eq!(
            choice.palette.sgr_for(TokenKind::Keyword),
            "\x1b[38;2;122;68;212m"
        );
        let warning = choice.warning.expect("carries a warning");
        assert!(warning.contains("BAT_THEME"));
        assert!(warning.contains("no-such-theme"));
    }

    #[test]
    fn detection_requires_color_on_tty_and_truecolor() {
        // The one allowed combination.
        assert!(detection_allowed(true, true, true));
        // Colors off (NO_COLOR / --color never / auto+piped) → never query.
        assert!(!detection_allowed(false, true, true));
        // Piped stdout (even --color always / CLICOLOR_FORCE) → never query:
        // piped output stays deterministic and non-interactive.
        assert!(!detection_allowed(true, false, true));
        // Named-16 terminal → nothing to select; never query.
        assert!(!detection_allowed(true, true, false));
        // All off.
        assert!(!detection_allowed(false, false, false));
    }

    #[test]
    fn looks_up_embedded_themes_by_their_bat_names() {
        // two-face names are bat's lookup vocabulary (verified identical).
        assert!(theme_by_name("Monokai Extended").is_some());
        assert!(theme_by_name("OneHalfLight").is_some());
        assert!(theme_by_name("Solarized (dark)").is_some());
        assert!(theme_by_name("no-such-theme").is_none());
        // Names are case-sensitive (bat semantics).
        assert!(theme_by_name("monokai extended").is_none());
    }

    #[test]
    fn lists_available_names_for_error_messages() {
        let names = available_theme_names();
        assert!(names.iter().any(|n| n == "Monokai Extended"));
        assert!(names.iter().any(|n| n == "OneHalfLight"));
        assert!(names.len() >= 20); // the embedded set is substantial
    }

    #[test]
    fn classifies_theme_mode_from_background_luminance() {
        // BT.601 integer luminance: (299r + 587g + 114b) / 1000 >= 128 → Light.
        let light = theme_with_background(Some(color(0xff, 0xff, 0xff, 0xff)));
        assert_eq!(classify_theme_mode(&light), Some(DiffMode::Light));
        let dark = theme_with_background(Some(color(0x27, 0x28, 0x22, 0xff))); // Monokai bg
        assert_eq!(classify_theme_mode(&dark), Some(DiffMode::Dark));
        // No background, or a sentinel alpha → no verdict.
        assert_eq!(classify_theme_mode(&theme_with_background(None)), None);
        let sentinel = theme_with_background(Some(color(0, 0, 0, 0)));
        assert_eq!(classify_theme_mode(&sentinel), None);
    }

    #[test]
    fn embedded_light_and_dark_themes_classify_correctly() {
        let one_half_light = theme_by_name("OneHalfLight").unwrap();
        assert_eq!(classify_theme_mode(&one_half_light), Some(DiffMode::Light));
        let monokai = theme_by_name("Monokai Extended").unwrap();
        assert_eq!(classify_theme_mode(&monokai), Some(DiffMode::Dark));
    }

    #[test]
    fn derives_a_palette_from_an_embedded_theme() {
        // "Monokai Extended" is embedded via two-face; loading is pure (no I/O).
        let theme = theme_by_name("Monokai Extended").expect("embedded theme");
        let p = DiffPalette::from_theme(&theme, DiffMode::Dark);
        // Monokai keywords are colored: a truecolor fg SGR, not empty.
        assert!(p.sgr_for(TokenKind::Keyword).starts_with("\x1b[38;2;"));
        // Keyword and string styles differ in any real theme.
        assert_ne!(p.sgr_for(TokenKind::Keyword), p.sgr_for(TokenKind::String));
        // Plain always stays uncolored.
        assert_eq!(p.sgr_for(TokenKind::Plain), "");
        // The emph pair comes from the passed mode (delta's dark constants).
        assert_eq!(p.emph_add_bg, "\x1b[48;2;0;96;0m");
        assert_eq!(p.emph_del_bg, "\x1b[48;2;144;16;17m");
    }

    #[test]
    fn alpha_sentinels_map_to_ansi_indices_and_terminal_default() {
        // bat's convention (bat src/terminal.rs::to_ansi_color): a==0 encodes
        // an ANSI palette index in r; a==1 means "terminal default" (no escape).
        assert_eq!(sgr_fg_for_color(color(5, 0, 0, 0)), "\x1b[35m"); // basic magenta
        assert_eq!(sgr_fg_for_color(color(42, 0, 0, 0)), "\x1b[38;5;42m"); // 256-index
        assert_eq!(sgr_fg_for_color(color(1, 2, 3, 1)), ""); // terminal default
        assert_eq!(
            sgr_fg_for_color(color(10, 20, 30, 255)),
            "\x1b[38;2;10;20;30m"
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
