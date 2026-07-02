//! Sensitivity-scan exclude configuration: a committed `.shore/sensitivity.json`
//! plus a git-excluded `.shore/sensitivity.local.json` supply gitignore-style
//! path globs the worktree sensitivity scan skips — the targeted alternative to
//! the blanket `--include-ephemeral` override when a repo's own test fixtures
//! carry scanner-triggering strings. Kept in its own module so the scan's
//! vocabulary (kinds/severities/outcomes) stays untouched here.
//!
//! Glob semantics are a documented gitignore-style subset, matched against the
//! repo-relative `/`-separated path the scan iterates:
//!
//! - A leading `/`, or any interior `/`, makes the pattern **rooted** against
//!   the whole relative path (`tests/**`, `src/lib.rs`, `/data`).
//! - A pattern with no `/` (ignoring a sole trailing one) matches **any path
//!   component at any depth** — a matching directory component covers
//!   everything beneath it, a matching final component is the file itself
//!   (`*.pem`, `data`).
//! - A trailing `/` marks a **directory** pattern: it matches only paths
//!   *inside* a matching directory (`fixtures/` matches `fixtures/sample.txt`
//!   and, being slash-free, `deep/fixtures/sample.txt` — never a plain file
//!   named `fixtures`). A rooted directory pattern behaves as `dir/**`.
//! - `**` spans path segments: zero or more interior (`src/**/store` matches
//!   `src/store`), one or more trailing (`tests/**` matches things inside
//!   `tests`, never `tests` itself). `*` and `?` match within a segment (never
//!   `/`).
//!
//! Negation (`!`) and empty patterns are rejected at load — the config gates a
//! protection, so a misread must never silently change coverage.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShoreError};

/// Repo-relative paths to the sensitivity-config files: the committed default
/// and the git-excluded private override (covered by `.shore/.gitignore`'s
/// `*.local.json` line).
pub(crate) const SENSITIVITY_CONFIG_REL_PATH: &str = ".shore/sensitivity.json";
pub(crate) const SENSITIVITY_CONFIG_LOCAL_REL_PATH: &str = ".shore/sensitivity.local.json";

const SENSITIVITY_CONFIG_SCHEMA: &str = "shore.sensitivity-config";
const SENSITIVITY_CONFIG_VERSION: u32 = 1;

/// The persisted sensitivity-config document. Modeled on `StoreConfig`
/// (schema + version + body) so an unsupported schema/version is a loud error,
/// never a silent misread.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SensitivityConfig {
    schema: String,
    version: u32,
    exclude_globs: Vec<String>,
}

/// The effective exclude-glob list for `worktree_root`: the committed
/// `.shore/sensitivity.json` unioned with the git-excluded
/// `.shore/sensitivity.local.json` (committed order first, then novel local
/// entries). Union — not the local-replaces-committed rule `store.json` /
/// `delegates.json` use — because this is a LIST: replace would force copying
/// the whole committed list to add one local entry, union grants nothing
/// replace couldn't (a replacing local file could always supply a wider list),
/// and the scan's audit counts make any widening visible. Both files absent →
/// empty (scan everything; opt-in only). Malformed JSON, an unsupported
/// schema/version, a negated (`!`) pattern, or an empty pattern is a hard
/// error naming the offending file — the config gates a protection, so a
/// misread must never silently change coverage.
pub(crate) fn resolve_sensitivity_excludes(worktree_root: &Path) -> Result<Vec<String>> {
    let committed = load_sensitivity_config(&worktree_root.join(SENSITIVITY_CONFIG_REL_PATH))?;
    let local = load_sensitivity_config(&worktree_root.join(SENSITIVITY_CONFIG_LOCAL_REL_PATH))?;
    let mut globs: Vec<String> = Vec::new();
    for config in [committed, local].into_iter().flatten() {
        for glob in config.exclude_globs {
            if !globs.contains(&glob) {
                globs.push(glob);
            }
        }
    }
    Ok(globs)
}

/// Load and validate a sensitivity-config file if present; absent → `None`.
fn load_sensitivity_config(path: &Path) -> Result<Option<SensitivityConfig>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ShoreError::Message(format!(
                "read sensitivity config {}: {error}",
                path.display()
            )));
        }
    };
    let config: SensitivityConfig = serde_json::from_slice(&bytes).map_err(|error| {
        ShoreError::Message(format!(
            "sensitivity config {} is malformed: {error}; expected a JSON document with an \
             \"excludeGlobs\" array of gitignore-style path globs",
            path.display()
        ))
    })?;
    if config.schema != SENSITIVITY_CONFIG_SCHEMA || config.version != SENSITIVITY_CONFIG_VERSION {
        return Err(ShoreError::Message(format!(
            "sensitivity config {} has unsupported schema/version {} v{} (expected {} v{})",
            path.display(),
            config.schema,
            config.version,
            SENSITIVITY_CONFIG_SCHEMA,
            SENSITIVITY_CONFIG_VERSION
        )));
    }
    for glob in &config.exclude_globs {
        if glob.is_empty() {
            return Err(ShoreError::Message(format!(
                "sensitivity config {} contains an empty exclude glob; list only non-empty \
                 paths/globs to exclude",
                path.display()
            )));
        }
        if glob.starts_with('!') {
            return Err(ShoreError::Message(format!(
                "sensitivity config {} contains the negated glob {glob:?}; negation is not \
                 supported — list only paths to exclude",
                path.display()
            )));
        }
    }
    Ok(Some(config))
}

/// Match a validated exclude pattern against a repo-relative, `/`-separated
/// path. Semantics are the documented gitignore-style subset in the module doc;
/// load-time validation has already rejected negation and empty patterns.
pub(crate) fn glob_matches(pattern: &str, relative_path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_prefix('/') {
        Some(stripped) => (stripped, true),
        None => (pattern, false),
    };
    let (pattern, dir_only) = match pattern.strip_suffix('/') {
        Some(stripped) => (stripped, true),
        None => (pattern, false),
    };
    let rooted = anchored || pattern.contains('/');
    let path: Vec<&str> = relative_path.split('/').collect();
    if rooted {
        let mut segments: Vec<&str> = pattern.split('/').collect();
        if dir_only {
            segments.push("**"); // dir/ ≡ dir/** (only what's inside)
        }
        segments_match(&segments, &path)
    } else if dir_only {
        // Slash-free directory pattern: matches at any depth, but only paths
        // INSIDE a matching directory — some non-final component matches.
        path.len() > 1
            && path[..path.len() - 1]
                .iter()
                .any(|segment| segment_matches(pattern, segment))
    } else {
        // Slash-free pattern: any component, gitignore-style — a matching
        // directory component covers everything beneath it; a matching final
        // component is the file itself.
        path.iter().any(|segment| segment_matches(pattern, segment))
    }
}

/// Recursive segment-list matcher: an interior `**` consumes zero or more path
/// segments; a trailing `**` consumes one or more (gitignore: `a/**` matches
/// what is inside `a`, never `a` itself); every other pattern segment must
/// match exactly one path segment.
fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            let min_skip = usize::from(rest.is_empty());
            (min_skip..=path.len()).any(|skip| segments_match(rest, &path[skip..]))
        }
        Some((first, rest)) => match path.split_first() {
            Some((segment, path_rest)) => {
                segment_matches(first, segment) && segments_match(rest, path_rest)
            }
            None => false,
        },
    }
}

/// Single-segment matcher: `*` any run (never `/`), `?` any one char, else
/// literal. Recursive backtracking is fine here: patterns are short
/// operator-authored config and segments are path components.
fn segment_matches(pattern: &str, segment: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let segment: Vec<char> = segment.chars().collect();
    chars_match(&pattern, &segment)
}

fn chars_match(pattern: &[char], segment: &[char]) -> bool {
    match pattern.split_first() {
        None => segment.is_empty(),
        Some(('*', rest)) => (0..=segment.len()).any(|skip| chars_match(rest, &segment[skip..])),
        Some(('?', rest)) => match segment.split_first() {
            Some((_, seg_rest)) => chars_match(rest, seg_rest),
            None => false,
        },
        Some((ch, rest)) => match segment.split_first() {
            Some((seg_ch, seg_rest)) => seg_ch == ch && chars_match(rest, seg_rest),
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_follows_the_documented_gitignore_subset() {
        let cases: [(&str, &str, bool); 21] = [
            // Interior '/': rooted against the whole relative path.
            ("tests/**", "tests/cli_store_status.rs", true),
            ("tests/**", "tests/acceptance/session_state.rs", true),
            ("tests/**", "src/tests/fixture.rs", false),
            ("tests/**", "tests", false), // trailing '**' needs something INSIDE tests
            (
                "src/session/store/sensitivity.rs",
                "src/session/store/sensitivity.rs",
                true,
            ),
            (
                "src/session/store/sensitivity.rs",
                "src/session/store/sensitivity_config.rs",
                false,
            ),
            // Leading '/' forces rooted, even with no interior slash.
            ("/tests/**", "tests/cli_store_status.rs", true),
            ("/data", "data", true),
            ("/data", "nested/data", false),
            // No '/': any path component at any depth, gitignore-style.
            ("*.pem", "keys/dev.pem", true),
            ("*.pem", "dev.pem", true),
            ("*.pem", "keys/dev.pem.txt", false),
            ("data", "data/state.json", true), // a matching dir component covers what's under it
            // Trailing '/': directory pattern — only paths INSIDE a matching dir.
            ("fixtures/", "fixtures/sample.txt", true),
            ("fixtures/", "deep/fixtures/sample.txt", true), // slash-free dir pattern: any depth
            ("fixtures/", "fixtures", false), // a plain file named fixtures is not inside it
            // '*' and '?' never cross '/'.
            ("src/*.rs", "src/lib.rs", true),
            ("src/*.rs", "src/session/mod.rs", false),
            ("src/?ib.rs", "src/lib.rs", true),
            // Interior '**' spans zero or more segments.
            ("src/**/store/*.rs", "src/session/store/bundle.rs", true),
            ("src/**/store/*.rs", "src/store/lib.rs", true),
        ];
        for (pattern, path, expected) in cases {
            assert_eq!(
                glob_matches(pattern, path),
                expected,
                "pattern {pattern:?} vs path {path:?}"
            );
        }
    }

    fn write(root: &std::path::Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    const COMMITTED_DOC: &str =
        r#"{"schema":"shore.sensitivity-config","version":1,"excludeGlobs":["tests/**","*.pem"]}"#;
    const LOCAL_DOC: &str = r#"{"schema":"shore.sensitivity-config","version":1,"excludeGlobs":["*.pem","scratch/**"]}"#;

    #[test]
    fn absent_config_files_resolve_to_an_empty_exclude_list() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_sensitivity_excludes(root.path()).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn committed_and_local_exclude_lists_union() {
        // Union, not local-replaces-committed: an exclude config is a list, and
        // a local file should ADD entries without copying the committed list.
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/sensitivity.json", COMMITTED_DOC);
        write(root.path(), ".shore/sensitivity.local.json", LOCAL_DOC);
        assert_eq!(
            resolve_sensitivity_excludes(root.path()).unwrap(),
            vec![
                "tests/**".to_owned(),
                "*.pem".to_owned(),
                "scratch/**".to_owned(),
            ],
            "committed order first, then novel local entries; duplicates collapse"
        );
    }

    #[test]
    fn either_config_file_alone_is_used() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/sensitivity.local.json", LOCAL_DOC);
        assert_eq!(
            resolve_sensitivity_excludes(root.path()).unwrap(),
            vec!["*.pem".to_owned(), "scratch/**".to_owned()]
        );
    }

    #[test]
    fn malformed_sensitivity_config_is_a_hard_error_naming_the_file() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), ".shore/sensitivity.json", "{ not json");
        let err = resolve_sensitivity_excludes(root.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("sensitivity.json"), "names the file: {err}");
        assert!(
            err.contains("excludeGlobs"),
            "names the expected shape: {err}"
        );
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            ".shore/sensitivity.local.json",
            r#"{"schema":"shore.sensitivity-config","version":999,"excludeGlobs":[]}"#,
        );
        let err = resolve_sensitivity_excludes(root.path())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("sensitivity.local.json"),
            "names the offending file: {err}"
        );
    }

    #[test]
    fn negated_and_empty_patterns_are_rejected_at_load() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            ".shore/sensitivity.json",
            r#"{"schema":"shore.sensitivity-config","version":1,"excludeGlobs":["!tests/**"]}"#,
        );
        let err = resolve_sensitivity_excludes(root.path())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("negation") || err.contains('!'),
            "explains negation is unsupported: {err}"
        );
        write(
            root.path(),
            ".shore/sensitivity.json",
            r#"{"schema":"shore.sensitivity-config","version":1,"excludeGlobs":[""]}"#,
        );
        let err = resolve_sensitivity_excludes(root.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty"), "rejects an empty pattern: {err}");
    }
}
