//! Language detection: map a diff file's paths to a syntect syntax, by extension only.

use std::sync::OnceLock;

use syntect::parsing::{SyntaxReference, SyntaxSet};

/// The syntax set, loaded once. Uses `two-face`'s bundled set (the `bat` syntaxes) rather than
/// syntect's stock bundle, so the long tail of languages syntect omits — TypeScript and TSX most
/// notably — tokenize. The `_newlines` variant lets the line tokenizer feed each line with a
/// trailing `\n` so end-of-line patterns match.
pub(crate) fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Detect a syntax from the diff paths (new then old), by extension only.
///
/// In-memory: never touches disk (we deliberately avoid `find_syntax_for_file`, which reads the
/// first line off disk). Unknown, absent, or stock-unsupported extensions (e.g. `.ts`) return
/// `None`, which the caller renders plain.
pub(crate) fn syntax_for_paths(
    new_path: Option<&str>,
    old_path: Option<&str>,
) -> Option<&'static SyntaxReference> {
    let ss = syntax_set();
    for path in [new_path, old_path].into_iter().flatten() {
        let p = std::path::Path::new(path);
        if let Some(ext) = p.extension().and_then(|e| e.to_str())
            && let Some(syntax) = ss.find_syntax_by_extension(ext)
        {
            return Some(syntax);
        }
        // Whole-name fallback for extensionless, name-keyed syntaxes (Makefile, Dockerfile, ...).
        if let Some(name) = p.file_name().and_then(|n| n.to_str())
            && let Some(syntax) = ss.find_syntax_by_extension(name)
        {
            return Some(syntax);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_extension() {
        assert!(syntax_for_paths(Some("src/main.rs"), None).is_some());
        assert!(syntax_for_paths(Some("a.py"), None).is_some());
    }

    #[test]
    fn unknown_or_missing_extension_is_none() {
        assert!(syntax_for_paths(Some("a.xyzzy"), None).is_none());
        assert!(syntax_for_paths(None, None).is_none());
    }

    #[test]
    fn detects_typescript_and_tsx() {
        // TypeScript/TSX are not in syntect's stock bundle; they are vendored and added to the set.
        assert!(syntax_for_paths(Some("app.ts"), None).is_some());
        assert!(syntax_for_paths(Some("component.tsx"), None).is_some());
    }

    #[test]
    fn prefers_new_path_then_old_path() {
        assert!(syntax_for_paths(None, Some("old.rs")).is_some());
    }
}
