// The resolver ships its whole API but the commands adopt it family-by-family,
// so the not-yet-wired wrappers (and their `IdKind` variants) are dead until
// their family flattens. `IdKind::Validation` has no wrapper by design — no
// argument takes a `validation:` id as input — so it stays unconstructed
// permanently. Kept until the last family wires the last wrapper (revision
// family); the removal point narrows this to a targeted allow on
// `IdKind::Validation`.
#![allow(dead_code)]

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use shoreline::session::{StoreIdIndex, store_id_index};

/// Minimum accepted abbreviated-id fragment length (ADR-0031 Decision 4). Four
/// keeps memory-typed fragments short (git's default feel) while staying at or
/// below the inspector's `shortRef` display width, so any id the product
/// displays is still re-enterable verbatim. A shorter fragment is rejected even
/// when unique; an ambiguous one errors with the full candidate list and never
/// auto-picks, so a low floor only makes a retry more likely, never wrong.
pub const MIN_ID_FRAGMENT: usize = 4;

/// The id kinds an argument can accept. `prefix()` is the frozen ADR-0028 token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdKind {
    Revision,
    Object,
    Event,
    Observation,
    Assessment,
    InputRequest,
    Validation,
    CommitAssociation,
    RefAssociation,
}

impl IdKind {
    pub fn prefix(self) -> &'static str {
        match self {
            IdKind::Revision => "rev",
            IdKind::Object => "obj",
            IdKind::Event => "evt",
            IdKind::Observation => "obs",
            IdKind::Assessment => "assess",
            IdKind::InputRequest => "input-request",
            IdKind::Validation => "validation",
            IdKind::CommitAssociation => "assoc-commit",
            IdKind::RefAssociation => "assoc-ref",
        }
    }

    fn set(self, index: &StoreIdIndex) -> &std::collections::BTreeSet<String> {
        match self {
            IdKind::Revision => &index.revisions,
            IdKind::Object => &index.objects,
            IdKind::Event => &index.events,
            IdKind::Observation => &index.observations,
            IdKind::Assessment => &index.assessments,
            IdKind::InputRequest => &index.input_requests,
            IdKind::Validation => &index.validations,
            IdKind::CommitAssociation => &index.commit_associations,
            IdKind::RefAssociation => &index.ref_associations,
        }
    }
}

/// Resolves id-taking arguments against one lazily-built store id index. The
/// index builds at most once per invocation, and only the first time an
/// *abbreviated* id needs resolving — full ids never trigger a build.
pub struct IdResolver {
    repo: PathBuf,
    index: OnceCell<StoreIdIndex>,
}

type CliResult<T> = Result<T, Box<dyn std::error::Error>>;

impl IdResolver {
    pub fn new(repo: &Path) -> Self {
        Self {
            repo: repo.to_path_buf(),
            index: OnceCell::new(),
        }
    }

    /// Resolve `input` to a full id, accepting exactly one of the given `kinds`.
    pub fn resolve(&self, kinds: &[IdKind], input: &str) -> CliResult<String> {
        match classify(kinds, input)? {
            Resolution::Full(full) => Ok(full),
            Resolution::Fragment { kind, hex } => match_fragment(self.index()?, kind, &hex),
        }
    }

    // Convenience wrappers (exact spellings owned by this task).
    pub fn rev(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::Revision], input)
    }
    pub fn object(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::Object], input)
    }
    pub fn event(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::Event], input)
    }
    pub fn observation(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::Observation], input)
    }
    pub fn assessment(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::Assessment], input)
    }
    pub fn input_request(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::InputRequest], input)
    }
    /// Union resolution for `association withdraw <ID>`: prefixed-required
    /// (`assoc-commit:` | `assoc-ref:`); the resolved prefix selects the axis.
    pub fn association(&self, input: &str) -> CliResult<String> {
        self.resolve(&[IdKind::CommitAssociation, IdKind::RefAssociation], input)
    }

    /// Lazily build the index the first time an abbreviated id needs it. Single
    /// threaded per invocation, so a plain check-then-set is sufficient (the std
    /// `OnceCell::get_or_try_init` is unstable; this avoids it).
    fn index(&self) -> CliResult<&StoreIdIndex> {
        if let Some(index) = self.index.get() {
            return Ok(index);
        }
        let built = store_id_index(&self.repo)?;
        let _ = self.index.set(built);
        Ok(self.index.get().expect("index set above"))
    }
}

/// The outcome of parsing an input string against the accepted kinds — computed
/// without the index, so full ids never trigger a build.
enum Resolution {
    Full(String),
    Fragment { kind: IdKind, hex: String },
}

/// Parse `input` into either a full id (pass through) or a (kind, hex-fragment)
/// pair to resolve. Applies the floor, the lowercase-hex rule, the prefix rule,
/// and the single-kind rule for bare fragments — all index-free.
fn classify(kinds: &[IdKind], input: &str) -> CliResult<Resolution> {
    // Prefixed form: `<kind>:…`. The leading token must be one of `kinds`.
    for &kind in kinds {
        let head = format!("{}:", kind.prefix());
        if let Some(rest) = input.strip_prefix(&head) {
            let hex = strip_to_fragment(rest);
            if hex.len() == 64 && is_lower_hex(hex) {
                return Ok(Resolution::Full(input.to_owned())); // full id, verbatim
            }
            check_fragment(hex)?;
            return Ok(Resolution::Fragment {
                kind,
                hex: hex.to_owned(),
            });
        }
    }
    // A colon that matched no accepted kind prefix is a wrong-kind / unknown-prefix input.
    if input.contains(':') {
        return Err(format!(
            "id {input:?} does not name one of the expected kinds ({})",
            kind_prefixes(kinds)
        )
        .into());
    }
    // Bare fragment: only when the argument implies exactly one kind.
    if kinds.len() != 1 {
        return Err(format!(
            "ambiguous id {input:?}: prefix it with one of {} to say which kind you mean",
            kind_prefixes(kinds)
        )
        .into());
    }
    check_fragment(input)?;
    Ok(Resolution::Fragment {
        kind: kinds[0],
        hex: input.to_owned(),
    })
}

/// Scan one kind's id set for digest-prefix matches. Zero → not-found; one →
/// resolved; more than one → hard error listing every full candidate (INV-6).
fn match_fragment(index: &StoreIdIndex, kind: IdKind, hex: &str) -> CliResult<String> {
    let matches: Vec<&String> = kind
        .set(index)
        .iter()
        .filter(|id| digest_hex(id).is_some_and(|digest| digest.starts_with(hex)))
        .collect();
    match matches.len() {
        0 => Err(format!("no {} id matches {hex:?}", kind.prefix()).into()),
        1 => Ok(matches[0].clone()), // matches[0]: &String → String
        n => {
            let listed = matches
                .iter()
                .map(|id| format!("  {id}"))
                .collect::<Vec<_>>()
                .join("\n");
            Err(format!(
                "id fragment {hex:?} is ambiguous — {n} {} ids match; \
                 use more characters or a full id:\n{listed}",
                kind.prefix()
            )
            .into())
        }
    }
}

/// Strip the optional `git:`/`worktree:` infix then the optional `sha256:` tag,
/// leaving the leading digest fragment. `rev:worktree:sha256:<hex>` after the
/// `rev:` head → `<hex>`; `rev:40c47f97` → `40c47f97`.
fn strip_to_fragment(rest: &str) -> &str {
    let rest = rest
        .strip_prefix("git:")
        .or_else(|| rest.strip_prefix("worktree:"))
        .unwrap_or(rest);
    rest.strip_prefix("sha256:").unwrap_or(rest)
}

/// The digest hex of a full id: the tail after the final `sha256:` segment.
fn digest_hex(id: &str) -> Option<&str> {
    id.rsplit_once("sha256:").map(|(_, hex)| hex)
}

fn check_fragment(hex: &str) -> CliResult<()> {
    if !is_lower_hex(hex) {
        return Err(format!("id fragment {hex:?} must be lowercase hex").into());
    }
    if hex.len() < MIN_ID_FRAGMENT {
        return Err(format!(
            "id fragment {hex:?} is too short; use at least {MIN_ID_FRAGMENT} hex characters"
        )
        .into());
    }
    Ok(())
}

fn is_lower_hex(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn kind_prefixes(kinds: &[IdKind]) -> String {
    kinds
        .iter()
        .map(|kind| format!("{}:", kind.prefix()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Index-injected seam used by unit tests: identical to `IdResolver::resolve`
/// but scans a supplied index rather than building one.
#[cfg(test)]
fn resolve_within(index: &StoreIdIndex, kinds: &[IdKind], input: &str) -> CliResult<String> {
    match classify(kinds, input)? {
        Resolution::Full(full) => Ok(full),
        Resolution::Fragment { kind, hex } => match_fragment(index, kind, &hex),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use shoreline::session::StoreIdIndex;

    use super::{IdKind, IdResolver, MIN_ID_FRAGMENT, resolve_within};

    /// A full rev id whose 64-hex digest begins with `head` (padded with `fill`).
    /// The length must be exactly 64 or the full-id passthrough test is invalid.
    fn rev_full(head: &str, fill: char) -> String {
        let digest: String = head
            .chars()
            .chain(std::iter::repeat(fill))
            .take(64)
            .collect();
        format!("rev:sha256:{digest}")
    }

    fn index_with_revisions(ids: &[&str]) -> StoreIdIndex {
        StoreIdIndex {
            revisions: ids
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<BTreeSet<_>>(),
            ..Default::default()
        }
    }

    #[test]
    fn full_id_passes_through_without_building_the_index() {
        // A path that cannot resolve a store: if a full id triggered a build, this
        // would error. It must not.
        let rev_a = rev_full("40c47f97", '0');
        let resolver = IdResolver::new(Path::new("/definitely/missing/shore-store"));
        let out = resolver.resolve(&[IdKind::Revision], &rev_a).unwrap();
        assert_eq!(out, rev_a);
    }

    #[test]
    fn prefixed_short_resolves_to_the_unique_full_id() {
        let rev_a = rev_full("40c47f97", '0');
        let index = index_with_revisions(&[rev_a.as_str()]);
        // Digest tag optional: `rev:40c47f97` and `rev:sha256:40c47f97` are equivalent.
        assert_eq!(
            resolve_within(&index, &[IdKind::Revision], "rev:40c47f97").unwrap(),
            rev_a
        );
        assert_eq!(
            resolve_within(&index, &[IdKind::Revision], "rev:sha256:40c47f97").unwrap(),
            rev_a
        );
    }

    #[test]
    fn bare_fragment_resolves_only_for_a_single_kind() {
        let rev_a = rev_full("40c47f97", '0');
        let index = index_with_revisions(&[rev_a.as_str()]);
        assert_eq!(
            resolve_within(&index, &[IdKind::Revision], "40c47f97").unwrap(),
            rev_a
        );
    }

    #[test]
    fn bare_fragment_is_rejected_when_more_than_one_kind_is_possible() {
        let index = StoreIdIndex::default();
        let err = resolve_within(
            &index,
            &[IdKind::CommitAssociation, IdKind::RefAssociation],
            "40c47f97",
        )
        .unwrap_err();
        assert!(err.to_string().contains("prefix"), "err: {err}");
    }

    #[test]
    fn a_fragment_below_the_floor_is_rejected_as_too_short() {
        assert_eq!(MIN_ID_FRAGMENT, 4);
        let rev_a = rev_full("abcd1234", '0');
        let index = index_with_revisions(&[rev_a.as_str()]);
        // Three hex chars is under the four-char floor even though it is a unique
        // prefix; "abc" carries no '4', so the message assertion checks the floor.
        let err = resolve_within(&index, &[IdKind::Revision], "abc").unwrap_err();
        assert!(err.to_string().contains("4"), "err: {err}");
    }

    #[test]
    fn a_four_hex_fragment_resolves_at_the_floor() {
        let rev_a = rev_full("40c47f97", '0');
        let index = index_with_revisions(&[rev_a.as_str()]);
        assert_eq!(
            resolve_within(&index, &[IdKind::Revision], "40c4").unwrap(),
            rev_a
        );
    }

    #[test]
    fn an_ambiguous_fragment_lists_every_full_candidate() {
        // Two distinct full ids whose digests share the `40c47f97` prefix.
        let rev_a = rev_full("40c47f97", '0');
        let rev_b = rev_full("40c47f97", '1');
        let index = index_with_revisions(&[rev_a.as_str(), rev_b.as_str()]);
        let err = resolve_within(&index, &[IdKind::Revision], "40c47f97").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains(rev_a.as_str()),
            "lists candidate A: {message}"
        );
        assert!(
            message.contains(rev_b.as_str()),
            "lists candidate B: {message}"
        );
    }

    #[test]
    fn a_zero_match_fragment_is_not_found() {
        let rev_a = rev_full("40c47f97", '0');
        let index = index_with_revisions(&[rev_a.as_str()]);
        let err = resolve_within(&index, &[IdKind::Revision], "deadbeef").unwrap_err();
        assert!(err.to_string().to_lowercase().contains("no"), "err: {err}");
    }
}
