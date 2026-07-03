//! Output-lane selection for document-emitting commands.
//!
//! One decision point per command: the format enum, the shared `--format` args,
//! precedence resolution, the write entry, the JSON-fallback text lane, and the
//! id/byte display helpers. Every lane-related choice funnels through here so the
//! machine contract and the text rendering can never be selected by accident.

use std::io::Write;

use crate::cli::json;

/// The resolved output lane. The machine lanes (`Json`, `JsonPretty`) route
/// through the shared `write_json` primitive; `Text` renders a disposable view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutputFormat {
    Json,
    JsonPretty,
    Text,
}

/// The `--format` flag values. clap renders these as `json | json-pretty | text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(super) enum FormatArg {
    Json,
    JsonPretty,
    Text,
}

impl From<FormatArg> for OutputFormat {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Json => OutputFormat::Json,
            FormatArg::JsonPretty => OutputFormat::JsonPretty,
            FormatArg::Text => OutputFormat::Text,
        }
    }
}

/// Shared `--format` argument, flattened into every document-emitting command.
#[derive(Debug, Default, clap::Args)]
pub(super) struct FormatArgs {
    /// Output format: text | json | json-pretty.
    #[arg(long, value_enum)]
    pub(super) format: Option<FormatArg>,
}

impl FormatArgs {
    /// `--format` wins; otherwise fold the command's legacy `--pretty` selection.
    pub(super) fn explicit(&self, legacy_pretty: bool) -> Option<OutputFormat> {
        self.format
            .map(OutputFormat::from)
            .or_else(|| legacy_pretty.then_some(OutputFormat::JsonPretty))
    }
}

/// A resolved lane plus how it was chosen. `defaulted` is true when neither the
/// flag nor `SHORE_FORMAT` selected the lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResolvedFormat {
    pub(super) format: OutputFormat,
    pub(super) defaulted: bool,
}

/// Precedence: explicit `--format` flag > `SHORE_FORMAT` env > `default`.
/// The only reader of `SHORE_FORMAT`.
pub(super) fn resolve_format(
    explicit: Option<OutputFormat>,
    default: OutputFormat,
) -> Result<ResolvedFormat, Box<dyn std::error::Error>> {
    Ok(resolve_format_from(explicit, env_format()?, default))
}

/// The pure precedence rule behind [`resolve_format`]: explicit flag > env
/// selection > `default`. Split out from the environment read so it is
/// unit-testable without touching (or racing on) the process environment.
fn resolve_format_from(
    explicit: Option<OutputFormat>,
    env: Option<OutputFormat>,
    default: OutputFormat,
) -> ResolvedFormat {
    match explicit.or(env) {
        Some(format) => ResolvedFormat {
            format,
            defaulted: false,
        },
        None => ResolvedFormat {
            format: default,
            defaulted: true,
        },
    }
}

/// Read `SHORE_FORMAT` from the environment. Unset or empty yields no selection;
/// an invalid value is a hard error. This is the single `SHORE_FORMAT` read site.
fn env_format() -> Result<Option<OutputFormat>, Box<dyn std::error::Error>> {
    match std::env::var("SHORE_FORMAT") {
        Ok(value) if !value.is_empty() => Ok(Some(parse_format_value(&value)?)),
        _ => Ok(None),
    }
}

/// Lane dispatch: `Json` and `JsonPretty` emit the machine document byte-for-byte
/// through `write_json`; `Text` writes the rendered view.
pub(super) fn write_document<T, F>(
    stdout: &mut dyn Write,
    format: ResolvedFormat,
    document: &T,
    render_text: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    T: serde::Serialize,
    F: FnOnce() -> String,
{
    match format.format {
        OutputFormat::Json => json::write_json(stdout, document, false)?,
        OutputFormat::JsonPretty => json::write_json(stdout, document, true)?,
        OutputFormat::Text => writeln!(stdout, "{}", render_text())?,
    }
    Ok(())
}

/// Interim text lane for commands with no bespoke digest yet: indented JSON.
pub(super) fn write_document_json_fallback<T: serde::Serialize>(
    stdout: &mut dyn Write,
    format: ResolvedFormat,
    document: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    write_document(stdout, format, document, || {
        serde_json::to_string_pretty(document).unwrap_or_default()
    })
}

/// Git-style short form of an opaque id, ported from the inspector's `shortRef`
/// (`src/cli/inspect/web/src/refs.ts:36-44`). Prefixed content ids collapse to
/// `<prefix>:<8 hex>`, bare `sha256:<hex>` to `sha256:<8 hex>`, a raw 40-hex git
/// OID to its first 10 chars; anything else passes through unchanged.
pub(super) fn short_ref(id: &str) -> String {
    if let Some(short) = prefixed_sha_short(id) {
        return short;
    }
    if let Some(hex) = strip_prefix_ci(id, "sha256:")
        && hex.len() >= 8
        && is_hex(hex)
    {
        return format!("sha256:{}", &hex[..8]);
    }
    if id.len() == 40 && is_hex(id) {
        return id[..10].to_string();
    }
    id.to_string()
}

/// `<prefix>:(git:)?sha256:<hex>=6+>` -> `<prefix>:<first 8 hex>`, else `None`.
fn prefixed_sha_short(id: &str) -> Option<String> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphabetic() || bytes[end] == b'-') {
        end += 1;
    }
    if end >= bytes.len() || bytes[end] != b':' {
        return None;
    }
    let prefix = &id[..end];
    let mut rest = &id[end + 1..];
    if let Some(after_git) = strip_prefix_ci(rest, "git:") {
        rest = after_git;
    }
    let hex = strip_prefix_ci(rest, "sha256:")?;
    if hex.len() >= 6 && is_hex(hex) {
        Some(format!("{prefix}:{}", &hex[..8.min(hex.len())]))
    } else {
        None
    }
}

/// Case-insensitive prefix strip; the remainder is a valid `str` slice because a
/// match implies the leading bytes were ASCII.
fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let len = prefix.len();
    if value.len() >= len && value.as_bytes()[..len].eq_ignore_ascii_case(prefix.as_bytes()) {
        Some(&value[len..])
    } else {
        None
    }
}

fn is_hex(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Readable byte size, e.g. `24588565` -> `"24.6 MB"`. SI (1000-based) units;
/// exact rounding is disposable. Consumed by the store-status text digest.
pub(super) fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < MB {
        format!("{:.1} KB", value / KB)
    } else if value < GB {
        format!("{:.1} MB", value / MB)
    } else {
        format!("{:.1} GB", value / GB)
    }
}

/// Parse a `SHORE_FORMAT` value; an unknown value is a hard error naming the variable.
fn parse_format_value(value: &str) -> Result<OutputFormat, Box<dyn std::error::Error>> {
    match value {
        "json" => Ok(OutputFormat::Json),
        "json-pretty" => Ok(OutputFormat::JsonPretty),
        "text" => Ok(OutputFormat::Text),
        other => Err(format!(
            "invalid SHORE_FORMAT value {other:?}: expected one of json, json-pretty, text"
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_format_prefers_flag_over_env_over_default() {
        // Tests the pure precedence rule with the env selection passed in, so the
        // assertion never depends on the test process's ambient SHORE_FORMAT.
        // Explicit flag wins over both env and default, and is not `defaulted`.
        let flag = resolve_format_from(
            Some(OutputFormat::Text),
            Some(OutputFormat::Json),
            OutputFormat::Json,
        );
        assert_eq!(flag.format, OutputFormat::Text);
        assert!(!flag.defaulted);
        // No flag: the env selection wins over the default, and is not `defaulted`.
        let env = resolve_format_from(None, Some(OutputFormat::JsonPretty), OutputFormat::Json);
        assert_eq!(env.format, OutputFormat::JsonPretty);
        assert!(!env.defaulted);
        // Neither flag nor env: the default wins and is marked defaulted.
        let fallback = resolve_format_from(None, None, OutputFormat::Json);
        assert_eq!(fallback.format, OutputFormat::Json);
        assert!(fallback.defaulted);
    }

    #[test]
    fn format_json_is_byte_identical_to_legacy_compact_default() {
        let document = serde_json::json!({"schema": "shore.test", "version": 1, "value": 7});
        let json_lane = ResolvedFormat {
            format: OutputFormat::Json,
            defaulted: false,
        };
        let mut via_seam = Vec::new();
        write_document(&mut via_seam, json_lane, &document, String::new).unwrap();
        let mut via_legacy = Vec::new();
        crate::cli::json::write_json(&mut via_legacy, &document, false).unwrap();
        assert_eq!(via_seam, via_legacy);
    }

    #[test]
    fn short_ref_matches_the_inspector_semantics() {
        assert_eq!(
            short_ref(
                "rev:sha256:1ace028b0000000000000000000000000000000000000000000000000000ffff"
            ),
            "rev:1ace028b"
        );
        // Raw 40-hex git OID -> first 10 chars (refs.ts:42).
        assert_eq!(
            short_ref("57ace44f0a57ace44f0a57ace44f0a57ace44f0a"),
            "57ace44f0a"
        );
        // Unmatched values pass through unchanged (refs.ts:43).
        assert_eq!(short_ref("plainvalue"), "plainvalue");
    }

    #[test]
    fn invalid_format_value_is_a_hard_error_at_the_parser() {
        assert!(parse_format_value("bogus").is_err());
        assert_eq!(parse_format_value("json").unwrap(), OutputFormat::Json);
        assert_eq!(
            parse_format_value("json-pretty").unwrap(),
            OutputFormat::JsonPretty
        );
        assert_eq!(parse_format_value("text").unwrap(), OutputFormat::Text);
    }

    #[test]
    fn format_bytes_renders_representative_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(24_588_565), "24.6 MB");
    }
}
