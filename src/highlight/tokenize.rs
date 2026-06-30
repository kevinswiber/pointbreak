//! The stateful line tokenizer: drive syntect's parser line-by-line, classify scopes into
//! [`TokenKind`]s, and emit byte-offset [`TokenSpan`]s (non-`Plain` only).

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference};

use super::syntax::syntax_set;
use super::{TokenKind, TokenSpan};

/// Tokenizes an ordered sequence of source lines from one syntax, carrying syntect's parse state
/// and scope stack across lines so multi-line constructs (block comments, strings) classify
/// correctly on every line they span.
pub(crate) struct LineTokenizer {
    parse: ParseState,
    stack: ScopeStack,
}

impl LineTokenizer {
    pub(crate) fn new(syntax: &SyntaxReference) -> Self {
        Self {
            parse: ParseState::new(syntax),
            stack: ScopeStack::new(),
        }
    }

    /// Tokenize one raw line (no trailing newline). Returns non-`Plain` spans by byte offset into
    /// `raw`, sorted and non-overlapping. Best-effort: any parse error yields an empty `Vec` so the
    /// caller falls back to plain rendering.
    pub(crate) fn next_line(&mut self, raw: &str) -> Vec<TokenSpan> {
        // The `_newlines` syntax set matches end-of-line patterns, so feed a trailing newline and
        // clamp the emitted offsets back to `raw.len()` (dropping the synthetic `\n`).
        let with_nl = format!("{raw}\n");
        let ops = match self.parse.parse_line(&with_nl, syntax_set()) {
            Ok(ops) => ops,
            Err(_) => return Vec::new(),
        };
        let mut spans: Vec<TokenSpan> = Vec::new();
        let mut last = 0usize;
        for (offset, op) in ops {
            // The scope stack *before* applying this op governs the text up to `offset`.
            push_span(
                &mut spans,
                last,
                offset.min(raw.len()),
                classify(&self.stack),
            );
            let _ = self.stack.apply(&op); // ignore apply errors (best-effort)
            last = offset;
        }
        push_span(
            &mut spans,
            last.min(raw.len()),
            raw.len(),
            classify(&self.stack),
        );
        coalesce_drop_plain(spans)
    }
}

/// The ordered scope→kind table. More specific scopes precede their parents (e.g.
/// `constant.numeric` before `constant`, `keyword.operator` before `keyword`) so the first match
/// against a given stack scope is the most precise one.
fn classifier() -> &'static [(Scope, TokenKind)] {
    static TABLE: OnceLock<Vec<(Scope, TokenKind)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let s = |name: &str| Scope::new(name).expect("static classifier scope parses");
        vec![
            (s("comment"), TokenKind::Comment),
            (s("string"), TokenKind::String),
            (s("constant.numeric"), TokenKind::Number),
            (s("constant"), TokenKind::Constant),
            (s("keyword.operator"), TokenKind::Operator),
            (s("keyword"), TokenKind::Keyword),
            (s("storage"), TokenKind::Keyword),
            (s("entity.name.function"), TokenKind::Function),
            (s("support.function"), TokenKind::Function),
            (s("entity.name.type"), TokenKind::Type),
            (s("support.type"), TokenKind::Type),
            (s("entity.name.tag"), TokenKind::Type),
            (s("variable"), TokenKind::Variable),
            (s("entity.other.attribute-name"), TokenKind::Variable),
            (s("punctuation"), TokenKind::Punctuation),
        ]
    })
}

/// Classify a scope stack: the most specific (deepest) scope that matches a classifier entry wins,
/// with the table order breaking ties within a single scope. Unmatched stacks are `Plain`.
fn classify(stack: &ScopeStack) -> TokenKind {
    for scope in stack.as_slice().iter().rev() {
        for (cls, kind) in classifier() {
            if cls.is_prefix_of(*scope) {
                return *kind;
            }
        }
    }
    TokenKind::Plain
}

/// Push a span, skipping empty/reversed ranges.
fn push_span(spans: &mut Vec<TokenSpan>, start: usize, end: usize, kind: TokenKind) {
    if start >= end {
        return;
    }
    spans.push(TokenSpan { start, end, kind });
}

/// Merge adjacent same-kind runs and drop `Plain` spans. Spans separated by a dropped `Plain` gap
/// are not merged (their byte ranges are not contiguous).
fn coalesce_drop_plain(spans: Vec<TokenSpan>) -> Vec<TokenSpan> {
    let mut out: Vec<TokenSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.kind == TokenKind::Plain {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.kind == span.kind
            && last.end == span.start
        {
            last.end = span.end;
            continue;
        }
        out.push(span);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::TokenKind;
    use crate::highlight::syntax::syntax_for_paths;

    #[test]
    fn tokenizes_a_rust_line_into_kinds_at_byte_ranges() {
        let syntax = syntax_for_paths(Some("a.rs"), None).unwrap();
        let mut tk = LineTokenizer::new(syntax);
        let line = "let x = 1; // x";
        let spans = tk.next_line(line);
        // a keyword span covering "let" (bytes 0..3), a number span over "1", a comment span.
        assert!(
            spans
                .iter()
                .any(|s| s.kind == TokenKind::Keyword && s.start == 0 && s.end == 3)
        );
        assert!(spans.iter().any(|s| s.kind == TokenKind::Number));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
        // non-plain only; spans sorted, non-overlapping, within line length.
        assert!(
            spans
                .iter()
                .all(|s| s.kind != TokenKind::Plain && s.end <= line.len())
        );
        assert!(spans.windows(2).all(|w| w[0].end <= w[1].start));
    }

    #[test]
    fn tokenizes_a_typescript_line() {
        // TypeScript is not in syntect's stock bundle; this exercises the two-face syntax set.
        let syntax = syntax_for_paths(Some("app.ts"), None).unwrap();
        let mut tk = LineTokenizer::new(syntax);
        let spans = tk.next_line("const x: number = 1;");
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword)); // `const`
        assert!(spans.iter().any(|s| s.kind == TokenKind::Number)); // `1`
    }

    #[test]
    fn carries_state_across_lines_for_block_comments() {
        let syntax = syntax_for_paths(Some("a.rs"), None).unwrap();
        let mut tk = LineTokenizer::new(syntax);
        let _ = tk.next_line("/* open");
        let spans = tk.next_line("still comment */"); // inside the block comment opened previously
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn classify_picks_most_specific_then_table_order() {
        use syntect::parsing::{Scope, ScopeStack};
        let stack = |scopes: &[&str]| {
            let mut s = ScopeStack::new();
            for name in scopes {
                s.push(Scope::new(name).unwrap());
            }
            s
        };
        // Deepest scope wins: a numeric constant inside source -> Number, not Constant.
        assert_eq!(
            classify(&stack(&["source.rust", "constant.numeric.integer.rust"])),
            TokenKind::Number
        );
        // keyword.operator beats the bare keyword entry for the same scope.
        assert_eq!(
            classify(&stack(&["source.rust", "keyword.operator.assignment.rust"])),
            TokenKind::Operator
        );
        // A function name entity classifies as Function.
        assert_eq!(
            classify(&stack(&["source.rust", "entity.name.function.rust"])),
            TokenKind::Function
        );
        // An unrecognized (meta-only) stack is Plain.
        assert_eq!(
            classify(&stack(&["source.rust", "meta.block.rust"])),
            TokenKind::Plain
        );
    }

    #[test]
    fn coalesce_merges_adjacent_same_kind_and_drops_plain() {
        let merged = coalesce_drop_plain(vec![
            TokenSpan {
                start: 0,
                end: 3,
                kind: TokenKind::Keyword,
            },
            TokenSpan {
                start: 3,
                end: 5,
                kind: TokenKind::Keyword,
            },
            TokenSpan {
                start: 5,
                end: 7,
                kind: TokenKind::Plain,
            },
            TokenSpan {
                start: 7,
                end: 9,
                kind: TokenKind::Number,
            },
        ]);
        assert_eq!(
            merged,
            vec![
                TokenSpan {
                    start: 0,
                    end: 5,
                    kind: TokenKind::Keyword,
                },
                TokenSpan {
                    start: 7,
                    end: 9,
                    kind: TokenKind::Number,
                },
            ]
        );
    }

    #[test]
    fn same_kind_spans_split_by_a_gap_are_not_merged() {
        let merged = coalesce_drop_plain(vec![
            TokenSpan {
                start: 0,
                end: 3,
                kind: TokenKind::Keyword,
            },
            // gap 3..5 is implicit Plain (not present), so the next keyword must stay separate.
            TokenSpan {
                start: 5,
                end: 8,
                kind: TokenKind::Keyword,
            },
        ]);
        assert_eq!(merged.len(), 2);
    }
}
