use std::collections::BTreeMap;

use serde::Serialize;

use super::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::model::{ReviewEndpoint, ReviewTargetRef};
use crate::session::event::EventType;
use crate::session::identity::instant::normalize_instant_to_iso_millis;

/// A once-built search record for one history entry: a lowercased free-text
/// haystack plus the small structured field projection the query grammar matches
/// by name. Mirrors the retired client `SearchIndex` (web/src/types.ts).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchRecord {
    pub text: String,
    pub fields: BTreeMap<String, String>,
}

impl SearchRecord {
    /// Build the record for `entry`. `snapshot` is the content-object id the
    /// entry's revision captured (resolved by the caller against the
    /// revision->object map); "" when the entry addresses no captured object.
    /// The grammar key is `snapshot` (renamed from `object` in #334); the value is
    /// still sourced from the shared `object_id` document field. `extras` carries
    /// the per-entry inputs the build cannot derive from the entry alone.
    pub fn from_entry(
        entry: &ReviewHistoryEntry,
        snapshot: &str,
        extras: &EventRecordExtras,
    ) -> Self {
        let mut fields = BTreeMap::new();
        fields.insert("type".to_owned(), event_type_wire(entry));
        fields.insert("track".to_owned(), wrap_token(entry_track(entry)));
        fields.insert("actor".to_owned(), wrap_token(entry_actor(entry)));
        fields.insert("revision".to_owned(), entry_revision_id(entry));
        fields.insert("snapshot".to_owned(), snapshot.to_owned());
        fields.insert("check".to_owned(), summary_status(entry));
        fields.insert("assessment".to_owned(), entry_assessment(entry));
        fields.insert("tag".to_owned(), entry_tag_set(entry));
        fields.insert("is".to_owned(), extras.is_set());
        fields.insert(
            RANGE_ANCHOR_FIELD.to_owned(),
            normalize_instant_to_iso_millis(&entry.occurred_at).unwrap_or_default(),
        );
        Self {
            text: build_haystack(entry),
            fields,
        }
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }
}

/// Per-entry inputs the record build cannot derive from the entry alone. Today:
/// the input-request lifecycle standing for the `is:` set, computed once per base
/// in the projection loop and passed in. `Default` yields an empty set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventRecordExtras {
    /// `Some(true)` = an open input-request-opened entry (→ `is:open`);
    /// `Some(false)` = a responded one (→ `is:answered`); `None` = any other kind.
    pub is_open: Option<bool>,
}

impl EventRecordExtras {
    /// The space-wrapped `is:` set for this entry: `" open "`, `" answered "`, or "".
    fn is_set(&self) -> String {
        match self.is_open {
            Some(true) => " open ".to_owned(),
            Some(false) => " answered ".to_owned(),
            None => String::new(),
        }
    }
}

/// The lowercased haystack of an entry's human-relevant fields — parity with
/// `web/src/query.ts buildHaystack` (INV-4). Folds the same field set the TS
/// client did (title, body, summary, assessment, outcome, reasonCode, eventId,
/// revisionId, the per-fact ids, track, anchor, checkName, command, tags),
/// lowercased and space-joined with empties dropped.
pub fn build_haystack(entry: &ReviewHistoryEntry) -> String {
    let mut parts: Vec<String> = vec![
        entry_title(entry),
        entry.event_id.as_str().to_owned(),
        entry_revision_id(entry),
        entry_track(entry),
        entry_actor(entry),
        entry_anchor(entry),
    ];
    push_summary_searchables(&mut parts, entry);
    parts.retain(|part| !part.is_empty());
    parts.join(" ").to_lowercase()
}

/// A parsed query clause: a `field:value` equality or a free-text term, each
/// negatable. Mirrors `web/src/query.ts QueryClause`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueryClause {
    Field {
        field: String,
        value: String,
        negate: bool,
    },
    Text {
        value: String,
        negate: bool,
    },
}

/// The surface a query is parsed for — the per-surface key sets and value sets
/// hang off this. Mirrors `web/src/types.ts QuerySurface`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuerySurface {
    Event,
    Revision,
}

/// Why a clause was diagnosed. Serializes kebab-case, matching the TS union
/// (`unsupported-qualifier` / `deprecated-qualifier` / `unsupported-value`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryDiagnosticCode {
    UnsupportedQualifier,
    DeprecatedQualifier,
    UnsupportedValue,
}

/// One parse diagnostic: the code, the user-typed key it concerns, and a
/// human-readable message. Mirrors `web/src/types.ts QueryDiagnostic`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryDiagnostic {
    pub code: QueryDiagnosticCode,
    pub key: String,
    pub message: String,
}

/// The surface-aware parse result: the surviving clauses plus any diagnostics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParsedQuery {
    pub clauses: Vec<QueryClause>,
    pub diagnostics: Vec<QueryDiagnostic>,
}

/// Parse a query for a surface. A supported qualifier becomes a field clause
/// (aliases applied); a known-but-unsupported one, or an out-of-set `is:`/`attention:`
/// value, yields a diagnostic and drops the clause (never a silent-empty match); an
/// unknown key stays free text.
pub fn parse_search_query_for(query: &str, surface: QuerySurface) -> ParsedQuery {
    let mut clauses = Vec::new();
    let mut diagnostics = Vec::new();
    for raw in tokenize_query(query) {
        let mut token = raw.as_str();
        let mut negate = false;
        // A leading `-` negates, but only when something follows it.
        if token.len() > 1 && token.starts_with('-') {
            negate = true;
            token = &token[1..];
        }
        let colon = token.find(':');
        let key = match colon {
            Some(index) if index > 0 => token[..index].to_lowercase(),
            _ => String::new(),
        };
        if key.is_empty() {
            push_text(&mut clauses, token, negate);
            continue;
        }
        let value =
            strip_wrapping_quotes(&token[colon.expect("key implies a colon") + 1..]).to_lowercase();
        let (field, deprecated_from) = resolve_alias(&key, surface);
        if surface_fields(surface).contains(&field.as_str()) {
            if let Some(allowed) = value_set(surface, &field)
                && !allowed.contains(&value.as_str())
            {
                diagnostics.push(QueryDiagnostic::unsupported_value(&field, &value, allowed));
                continue;
            }
            if let Some(from) = deprecated_from {
                diagnostics.push(QueryDiagnostic::deprecated(&from, &field));
            }
            let value = canonicalize_field_value(&field, value);
            clauses.push(QueryClause::Field {
                field,
                value,
                negate,
            });
        } else if KNOWN_QUERY_KEYS.contains(&key.as_str()) {
            diagnostics.push(QueryDiagnostic::unsupported_qualifier(&key, surface));
        } else {
            push_text(&mut clauses, token, negate);
        }
    }
    ParsedQuery {
        clauses,
        diagnostics,
    }
}

/// The legacy event-surface parse — delegates to [`parse_search_query_for`],
/// discarding diagnostics, so existing callers keep compiling.
pub fn parse_search_query(query: &str) -> Vec<QueryClause> {
    parse_search_query_for(query, QuerySurface::Event).clauses
}

/// Canonicalize a qualifier value after alias resolution. Actor ids come in two
/// shapes (`is_valid_actor_id`): the `actor:` scheme or a self-certifying Ed25519
/// `did:key`. The `actor:` qualifier accepts a scheme id with or without its
/// prefix (`actor:agent:codex` ≡ `actor:actor:agent:codex`) and canonicalizes to
/// the prefixed form the record stores; a `did:key` value is already fully
/// qualified and passes through untouched. Mirrors `web/src/query.ts`.
fn canonicalize_field_value(field: &str, value: String) -> String {
    if field == "actor"
        && !value.is_empty()
        && !value.starts_with("actor:")
        && !value.starts_with("did:key:")
    {
        format!("actor:{value}")
    } else {
        value
    }
}

fn push_text(clauses: &mut Vec<QueryClause>, token: &str, negate: bool) {
    let value = strip_wrapping_quotes(token).to_lowercase();
    if !value.is_empty() {
        clauses.push(QueryClause::Text { value, negate });
    }
}

fn surface_fields(surface: QuerySurface) -> &'static [&'static str] {
    match surface {
        QuerySurface::Event => EVENT_QUERY_FIELDS,
        QuerySurface::Revision => REVISION_QUERY_FIELDS,
    }
}

/// Rewrite a user-typed key to its canonical field. `object:`→`snapshot:` is
/// silent; `status:`→`check:`/`assessment:` carries a deprecation hint.
fn resolve_alias(key: &str, surface: QuerySurface) -> (String, Option<String>) {
    match key {
        "object" => ("snapshot".to_owned(), None),
        "status" => {
            let target = match surface {
                QuerySurface::Event => "check",
                QuerySurface::Revision => "assessment",
            };
            (target.to_owned(), Some("status".to_owned()))
        }
        other => (other.to_owned(), None),
    }
}

/// The closed value set a set-membership qualifier is validated against, per
/// surface; `None` means the qualifier's value is not enumerated.
fn value_set(surface: QuerySurface, field: &str) -> Option<&'static [&'static str]> {
    match (surface, field) {
        (QuerySurface::Event, "is") => Some(&["open", "answered"]),
        (QuerySurface::Revision, "is") => Some(&[
            "open",
            "answered",
            "unassessed",
            "stale",
            "follow-up",
            "contested",
            "superseded",
        ]),
        (QuerySurface::Revision, "attention") => Some(REVISION_ATTENTION_VALUES),
        _ => None,
    }
}

impl QuerySurface {
    fn label(self) -> &'static str {
        match self {
            QuerySurface::Event => "timeline",
            QuerySurface::Revision => "revisions",
        }
    }
}

impl QueryDiagnostic {
    fn unsupported_qualifier(key: &str, surface: QuerySurface) -> Self {
        Self {
            code: QueryDiagnosticCode::UnsupportedQualifier,
            key: key.to_owned(),
            message: format!("`{key}:` is not a filter on the {} view", surface.label()),
        }
    }
    fn deprecated(from: &str, to: &str) -> Self {
        Self {
            code: QueryDiagnosticCode::DeprecatedQualifier,
            key: from.to_owned(),
            message: format!("`{from}:` is deprecated; use `{to}:`"),
        }
    }
    fn unsupported_value(key: &str, value: &str, allowed: &[&str]) -> Self {
        Self {
            code: QueryDiagnosticCode::UnsupportedValue,
            key: key.to_owned(),
            message: format!("`{key}:{value}` — expected one of: {}", allowed.join(", ")),
        }
    }
}

/// AND every clause against a record, honoring negation — parity with
/// `web/src/query.ts matchesQuery` (INV-4).
pub fn matches_query(record: &SearchRecord, clauses: &[QueryClause]) -> bool {
    for clause in clauses {
        let (negate, hit) = match clause {
            QueryClause::Field {
                field,
                value,
                negate,
            } => (*negate, field_matches(record, field, value)),
            QueryClause::Text { value, negate } => (*negate, record.text.contains(value.as_str())),
        };
        let fails = if negate { hit } else { !hit };
        if fails {
            return false;
        }
    }
    true
}

/// The event-surface qualifier set (mirrors web `EVENT_QUERY_FIELDS`). The
/// per-surface sets are the single authority every consumer (server validator,
/// CLI, client surfaces, autocomplete) resolves supported keys from.
pub const EVENT_QUERY_FIELDS: &[&str] = &[
    "type",
    "track",
    "actor",
    "revision",
    "snapshot",
    "check",
    "assessment",
    "is",
    "tag",
    "before",
    "after",
];

/// The revision-surface qualifier set (mirrors web `REVISION_QUERY_FIELDS`).
/// `type:`/`check:` are event-only (known-but-unsupported here — diagnosed,
/// never silent-empty); every other key matches a revision-record slot.
pub const REVISION_QUERY_FIELDS: &[&str] = &[
    "track",
    "actor",
    "revision",
    "snapshot",
    "assessment",
    "is",
    "tag",
    "attention",
    "before",
    "after",
];

/// Every key the grammar knows across surfaces plus the aliases — the
/// known-but-unsupported distinction: a key here but not in a surface's set
/// diagnoses instead of falling back to free text.
pub const KNOWN_QUERY_KEYS: &[&str] = &[
    "type",
    "track",
    "actor",
    "revision",
    "snapshot",
    "check",
    "assessment",
    "is",
    "tag",
    "attention",
    "before",
    "after",
    "status",
    "object",
];

/// The revision `attention:` value set — the single source shared by the validator,
/// the revision record builders, and autocomplete. These are exactly the
/// projection.ts attentionTokens vocabulary.
pub const REVISION_ATTENTION_VALUES: &[&str] = &[
    "open-request",
    "unassessed",
    "validation-context",
    "follow-up",
    "stale-fact",
];

/// The event-type human-label ↔ wire-id table (mirrors `web/src/types.ts TYPES`).
/// Shared by `field_matches` (resolve a `type:` value) and `type_label`.
const TYPE_LABELS: &[(&str, &str)] = &[
    ("init", "review_initialized"),
    ("capture", "work_object_proposed"),
    ("observation", "review_observation_recorded"),
    ("assessment", "review_assessment_recorded"),
    ("request", "input_request_opened"),
    ("response", "input_request_responded"),
    ("validation", "validation_check_recorded"),
];

/// How a resolved qualifier compares its value against a record field, resolved
/// per key by [`match_kind_for`]; [`field_matches`] dispatches on it.
enum MatchKind {
    Exact,
    Substring,
    SetMember,
    RangeBefore,
    RangeAfter,
}

fn match_kind_for(field: &str) -> MatchKind {
    match field {
        "type" | "check" | "assessment" => MatchKind::Exact,
        "revision" | "snapshot" => MatchKind::Substring,
        "track" | "actor" | "is" | "tag" | "attention" => MatchKind::SetMember,
        "before" => MatchKind::RangeBefore,
        "after" => MatchKind::RangeAfter,
        _ => MatchKind::Substring,
    }
}

/// The record field the `before:`/`after:` range kinds compare against, holding the
/// entry's normalized ISO-8601 UTC time (event: `occurred_at`; revision: `capturedAt`).
/// One canonical key across both runtimes and both records — not itself a query key.
pub const RANGE_ANCHOR_FIELD: &str = "occurred_at";

/// Match one field clause against a record (mirrors `fieldMatches`), dispatching
/// per key on the resolved [`MatchKind`] (the value is already lowercased by the
/// parser). Set-membership fields store space-wrapped token lists; the range
/// kinds compare lexically over the normalized time anchor.
fn field_matches(record: &SearchRecord, field: &str, value: &str) -> bool {
    match match_kind_for(field) {
        MatchKind::Exact => exact_matches(record, field, value),
        MatchKind::Substring => record
            .field(field)
            .unwrap_or("")
            .to_lowercase()
            .contains(value),
        MatchKind::SetMember => record
            .field(field)
            .unwrap_or("")
            .contains(&format!(" {value} ")),
        MatchKind::RangeAfter => {
            let anchor = record
                .field(RANGE_ANCHOR_FIELD)
                .unwrap_or("")
                .to_lowercase();
            !anchor.is_empty() && anchor > range_bound(value)
        }
        MatchKind::RangeBefore => {
            let anchor = record
                .field(RANGE_ANCHOR_FIELD)
                .unwrap_or("")
                .to_lowercase();
            !anchor.is_empty() && anchor < range_bound(value)
        }
    }
}

/// The lexical bound a `before:`/`after:` value compares as. A value that parses
/// as a full RFC 3339 instant normalizes to the anchor's fixed-width `.mmm` form
/// (a whole-second bound would otherwise misorder against millisecond anchors —
/// `.` sorts before `z`); a date/datetime prefix passes through and keeps raw
/// lexical prefix-compare semantics. The parser lowercased the value, so restore
/// ASCII case for the strict instant parse and re-lowercase the result.
fn range_bound(value: &str) -> String {
    let upper = value.to_ascii_uppercase();
    normalize_instant_to_iso_millis(&upper)
        // Round-trip guard: only a bound the normalizer re-emits verbatim
        // (through the seconds) counts as a recognized instant — anything the
        // normalization would MOVE (e.g. a leap-second roll-over) falls back to
        // the raw compare, keeping both runtimes' accepted value sets identical.
        .filter(|iso| iso.as_bytes()[..19] == upper.as_bytes()[..19])
        .map(|iso| iso.to_lowercase())
        .unwrap_or_else(|| value.to_owned())
}

/// Exact-match arm: `type` and `assessment` resolve label-or-wire (type also
/// comma-OR); every other exact field compares the lowercased record value.
fn exact_matches(record: &SearchRecord, field: &str, value: &str) -> bool {
    match field {
        "type" => {
            let record_type = record.field("type").unwrap_or("");
            value
                .split(',')
                .any(|v| resolve_type_value(v) == record_type)
        }
        "assessment" => record.field("assessment").unwrap_or("") == resolve_assessment_value(value),
        _ => record.field(field).unwrap_or("").to_lowercase() == value,
    }
}

/// Resolve an `assessment:` value to its wire form (display label or wire → wire),
/// mirroring `resolve_type_value`; passes unknowns through.
fn resolve_assessment_value(value: &str) -> &str {
    for (wire, label) in ASSESSMENT_LABEL_TABLE {
        if *wire == value || *label == value {
            return wire;
        }
    }
    value
}

/// The assessment wire-value ↔ display-label table, shared by the `assessment:`
/// value resolution and `assessment_display_label`.
const ASSESSMENT_LABEL_TABLE: &[(&str, &str)] = &[
    ("accepted", "accepted"),
    ("accepted_with_follow_up", "accepted-with-follow-up"),
    ("needs_changes", "needs-changes"),
    ("needs_clarification", "needs-clarification"),
];

/// Resolve a `type:` clause value: the wire id when the value names a known label
/// or wire id, else the raw value (mirrors `TYPES.find(t => t.label === value ||
/// t.id === value)`).
fn resolve_type_value(value: &str) -> &str {
    for (label, wire_id) in TYPE_LABELS {
        if *label == value || *wire_id == value {
            return wire_id;
        }
    }
    value
}

/// Strip at most one leading and one trailing `"` (mirrors the TS
/// `replace(/^"|"$/g, "")`).
fn strip_wrapping_quotes(value: &str) -> &str {
    let value = value.strip_prefix('"').unwrap_or(value);
    value.strip_suffix('"').unwrap_or(value)
}

/// Split a query into tokens, honoring `"quoted phrases"` (optionally negated /
/// field-prefixed) and bare runs (mirrors `tokenizeQuery`'s
/// `-?(?:[a-z]+:)?"[^"]*"|\S+` with the case-insensitive `i` flag).
fn tokenize_query(query: &str) -> Vec<String> {
    let chars: Vec<char> = query.chars().collect();
    let count = chars.len();
    let mut out = Vec::new();
    let mut index = 0;
    while index < count {
        if chars[index].is_whitespace() {
            index += 1;
            continue;
        }
        if let Some(end) = match_quoted_token(&chars, index) {
            out.push(chars[index..end].iter().collect());
            index = end;
        } else {
            let start = index;
            while index < count && !chars[index].is_whitespace() {
                index += 1;
            }
            out.push(chars[start..index].iter().collect());
        }
    }
    out
}

/// Try to match `-?(?:[a-z]+:)?"[^"]*"` (case-insensitive field prefix, `i` flag)
/// anchored at `start`; return the exclusive end index on success, else `None`
/// (so the caller falls back to a `\S+` bare run).
fn match_quoted_token(chars: &[char], start: usize) -> Option<usize> {
    let count = chars.len();
    let mut index = start;
    if index < count && chars[index] == '-' {
        index += 1;
    }
    // Optional `[A-Za-z]+:` field prefix; backtrack when there is no colon.
    let after_dash = index;
    while index < count && chars[index].is_ascii_alphabetic() {
        index += 1;
    }
    if index > after_dash && index < count && chars[index] == ':' {
        index += 1;
    } else {
        index = after_dash;
    }
    if index >= count || chars[index] != '"' {
        return None;
    }
    index += 1;
    while index < count && chars[index] != '"' {
        index += 1;
    }
    if index >= count {
        return None;
    }
    Some(index + 1)
}

/// The serde wire string for a small string-serializing enum (assessment,
/// outcome, reason code, validation status), so the haystack and the `status`
/// field fold exactly what the TS client received over the wire. `pub(crate)`
/// so the revision record builder derives its wire values the same way.
pub(crate) fn enum_wire<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The event-type wire string the client used as `e.eventType`
/// (`review_observation_recorded`, …). `pub(super)` so the object-map join (2.1)
/// and facet keying / `type:` matching (3.1) reuse one derivation.
pub(super) fn event_type_wire(entry: &ReviewHistoryEntry) -> String {
    entry.event_type.as_str().to_owned()
}

/// The entry's explicit review track only — empty when the entry has none. The
/// writer actor id is a separate slot (`entry_actor`); it is never folded into
/// the lane. Mirrors `web/src/projection.ts entryTrack`.
pub(super) fn entry_track(entry: &ReviewHistoryEntry) -> String {
    match &entry.track_id {
        Some(track) if !track.as_str().is_empty() => track.as_str().to_owned(),
        _ => String::new(),
    }
}

/// The writer actor id — the actor slot, distinct from the lane. Mirrors
/// `web/src/projection.ts entryActor`.
pub(super) fn entry_actor(entry: &ReviewHistoryEntry) -> String {
    entry.writer.actor_id.as_str().to_owned()
}

/// Wrap a single field value as a one-token space-wrapped set (lowercased so a
/// lowercased query value matches by whole-token membership); "" stays "".
fn wrap_token(value: String) -> String {
    if value.is_empty() {
        value
    } else {
        format!(" {} ", value.to_lowercase())
    }
}

/// The one space-wrapped set encoding every multi-valued record field stores:
/// tokens lowercased and space-joined with a leading/trailing space, so the
/// set-membership match kind hits by whole token; "" when there are no tokens.
/// `pub(crate)` so the revision record builder shares the encoding.
pub(crate) fn wrap_set(tokens: impl IntoIterator<Item = String>) -> String {
    let tokens: Vec<String> = tokens
        .into_iter()
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect();
    if tokens.is_empty() {
        String::new()
    } else {
        format!(" {} ", tokens.join(" "))
    }
}

/// The tag dual-index convention, defined once for every surface: each tag
/// contributes BOTH its first-colon key (`issue`) and its full string
/// (`issue:191`). Case is left to [`wrap_set`], which lowercases at wrap time.
pub(crate) fn tag_index_tokens<'a>(tags: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut tokens = Vec::new();
    for tag in tags {
        if let Some((key, _)) = tag.split_once(':')
            && !key.is_empty()
        {
            tokens.push(key.to_owned());
        }
        tokens.push(tag.clone());
    }
    tokens
}

/// The completion-vocabulary key for one tag: its first-colon key (the whole
/// string when it has no colon), lowercased; `None` for a leading-colon tag
/// (no usable key). Shared by the warm and cold distinct-value derivations so
/// the two paths cannot disagree — both read the raw domain string, never the
/// space-wrapped set encoding (a tag is a free string and may contain spaces).
pub(super) fn tag_completion_key(tag: &str) -> Option<String> {
    let key = tag.split(':').next().unwrap_or(tag);
    if key.is_empty() {
        None
    } else {
        Some(key.to_lowercase())
    }
}

/// The `assessment` field — the verdict wire value on assessment entries, else "".
fn entry_assessment(entry: &ReviewHistoryEntry) -> String {
    match &entry.summary {
        ReviewHistorySummary::ReviewAssessmentRecorded { assessment, .. } => enum_wire(assessment),
        _ => String::new(),
    }
}

/// The `tag` field — each observation tag contributes BOTH its full string and
/// its first-colon key, lowercased, in the space-wrapped set encoding. "" when
/// the entry carries no tags.
fn entry_tag_set(entry: &ReviewHistoryEntry) -> String {
    match &entry.summary {
        ReviewHistorySummary::ReviewObservationRecorded { tags, .. } => {
            wrap_set(tag_index_tokens(tags))
        }
        _ => String::new(),
    }
}

/// The revision id an entry addresses, read through its subject (mirrors
/// `web/src/projection.ts entryRevisionId`), or "". `pub(super)` for reuse by
/// the object-map join (2.1) and the query path (3.1).
pub(super) fn entry_revision_id(entry: &ReviewHistoryEntry) -> String {
    match &entry.subject {
        Some(target) => review_target_revision_id(target).to_owned(),
        None => String::new(),
    }
}

/// The `check` record field — the validation status wire string, else ""
/// (mirrors `e.summary?.status ?? ""`; only validation entries carry one).
fn summary_status(entry: &ReviewHistoryEntry) -> String {
    match &entry.summary {
        ReviewHistorySummary::ValidationCheckRecorded { status, .. } => enum_wire(status),
        _ => String::new(),
    }
}

/// A human title for a timeline entry (mirrors `web/src/projection.ts
/// entryTitle` precedence: title → assessment label → outcome → reasonCode →
/// type-specific capture/validation title → type label).
fn entry_title(entry: &ReviewHistoryEntry) -> String {
    match &entry.summary {
        ReviewHistorySummary::ReviewObservationRecorded { title, .. } => {
            if !title.is_empty() {
                return title.clone();
            }
        }
        ReviewHistorySummary::InputRequestOpened {
            title, reason_code, ..
        } => {
            if !title.is_empty() {
                return title.clone();
            }
            return enum_wire(reason_code);
        }
        ReviewHistorySummary::ReviewAssessmentRecorded { assessment, .. } => {
            return assessment_display_label(&enum_wire(assessment));
        }
        ReviewHistorySummary::InputRequestResponded { outcome, .. } => {
            return enum_wire(outcome);
        }
        ReviewHistorySummary::RevisionCaptured { summary, base, .. } => {
            if let Some(summary) = summary.as_ref().filter(|summary| !summary.is_empty()) {
                return summary.clone();
            }
            let commit_oid = match base {
                Some(ReviewEndpoint::GitCommit { commit_oid, .. }) => commit_oid.as_str(),
                _ => "",
            };
            return if commit_oid.is_empty() {
                "capture".to_owned()
            } else {
                format!("capture · base {}", short_id(commit_oid))
            };
        }
        ReviewHistorySummary::ValidationCheckRecorded {
            check_name, status, ..
        } => {
            let name = if check_name.is_empty() {
                "validation"
            } else {
                check_name.as_str()
            };
            let status = enum_wire(status);
            return if status.is_empty() {
                name.to_owned()
            } else {
                format!("{name} · {status}")
            };
        }
        _ => {}
    }
    type_label(entry.event_type)
}

/// Map an assessment wire value to its hyphenated display label (mirrors
/// `assessmentDisplayLabel` / `ASSESSMENT_LABELS`, passing through unknowns).
fn assessment_display_label(value: &str) -> String {
    for (wire, label) in ASSESSMENT_LABEL_TABLE {
        if *wire == value {
            return (*label).to_owned();
        }
    }
    value.to_owned()
}

/// The short tail of an id (mirrors `web/src/refs.ts shortId`): the segment after
/// the last `:`, truncated to 12 chars.
fn short_id(id: &str) -> String {
    let tail = id.rsplit(':').next().unwrap_or("");
    tail.chars().take(12).collect()
}

/// An event type's display label (mirrors `typeLabel` / `TYPES`), falling back to
/// the raw wire id for types the client never tabbed. Shares the `TYPE_LABELS`
/// table with the query grammar's `type:` resolution.
fn type_label(event_type: EventType) -> String {
    let wire = event_type.as_str();
    for (label, wire_id) in TYPE_LABELS {
        if *wire_id == wire {
            return (*label).to_owned();
        }
    }
    wire.to_owned()
}

/// A `file:start-end` anchor for an entry's file target, or "" (mirrors
/// `entryAnchor`: only file/range observation/request/assessment targets carry a
/// file path; note and validation targets do not).
fn entry_anchor(entry: &ReviewHistoryEntry) -> String {
    match &entry.summary {
        ReviewHistorySummary::ReviewObservationRecorded { target, .. }
        | ReviewHistorySummary::InputRequestOpened { target, .. }
        | ReviewHistorySummary::ReviewAssessmentRecorded { target, .. } => target_anchor(target),
        _ => String::new(),
    }
}

/// The `file:start-end` (or bare `file`) anchor of a review target, or "".
fn target_anchor(target: &ReviewTargetRef) -> String {
    match target {
        ReviewTargetRef::Range {
            file_path,
            start_line,
            end_line,
            ..
        } if *start_line != 0 => {
            let end = if *end_line != 0 {
                *end_line
            } else {
                *start_line
            };
            format!("{file_path}:{start_line}-{end}")
        }
        ReviewTargetRef::Range { file_path, .. } | ReviewTargetRef::File { file_path, .. } => {
            file_path.clone()
        }
        _ => String::new(),
    }
}

/// The revision id any review target keys on.
fn review_target_revision_id(target: &ReviewTargetRef) -> &str {
    match target {
        ReviewTargetRef::Revision { revision_id }
        | ReviewTargetRef::File { revision_id, .. }
        | ReviewTargetRef::Range { revision_id, .. }
        | ReviewTargetRef::Observation { revision_id, .. }
        | ReviewTargetRef::InputRequest { revision_id, .. }
        | ReviewTargetRef::Assessment { revision_id, .. }
        | ReviewTargetRef::Event { revision_id, .. } => revision_id.as_str(),
    }
}

/// Push an entry's summary-derived searchable tokens (body / summary / verdict /
/// per-fact ids / checkName / command / tags) — the parts of `buildHaystack`
/// that live on the typed summary variant.
fn push_summary_searchables(parts: &mut Vec<String>, entry: &ReviewHistoryEntry) {
    match &entry.summary {
        ReviewHistorySummary::ReviewObservationRecorded {
            observation_id,
            body,
            tags,
            ..
        } => {
            parts.push(observation_id.as_str().to_owned());
            if let Some(body) = body {
                parts.push(body.clone());
            }
            parts.extend(tags.iter().cloned());
        }
        ReviewHistorySummary::ReviewAssessmentRecorded {
            assessment_id,
            assessment,
            summary,
            ..
        } => {
            if let Some(summary) = summary {
                parts.push(summary.clone());
            }
            parts.push(enum_wire(assessment));
            parts.push(assessment_id.as_str().to_owned());
        }
        ReviewHistorySummary::InputRequestOpened {
            input_request_id,
            reason_code,
            body,
            ..
        } => {
            if let Some(body) = body {
                parts.push(body.clone());
            }
            parts.push(enum_wire(reason_code));
            parts.push(input_request_id.as_str().to_owned());
        }
        ReviewHistorySummary::InputRequestResponded {
            input_request_id,
            outcome,
            ..
        } => {
            parts.push(enum_wire(outcome));
            parts.push(input_request_id.as_str().to_owned());
        }
        ReviewHistorySummary::ValidationCheckRecorded {
            validation_check_id,
            summary,
            check_name,
            command,
            ..
        } => {
            if let Some(summary) = summary {
                parts.push(summary.clone());
            }
            parts.push(validation_check_id.as_str().to_owned());
            parts.push(check_name.clone());
            if let Some(command) = command {
                parts.push(command.clone());
            }
        }
        ReviewHistorySummary::ReviewInitialized {}
        | ReviewHistorySummary::ReviewNoteImported {}
        | ReviewHistorySummary::RevisionCaptured { .. }
        | ReviewHistorySummary::RevisionRefAssociated { .. }
        | ReviewHistorySummary::RevisionRefWithdrawn { .. }
        | ReviewHistorySummary::RevisionCommitAssociated { .. }
        | ReviewHistorySummary::RevisionCommitWithdrawn { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::summary::ReviewHistorySummary;
    use super::*;
    use crate::model::{
        EventId, JournalId, ObservationId, ReviewTargetRef, RevisionId, TrackId, ValidationCheckId,
        ValidationStatus, ValidationTarget, ValidationTrigger,
    };
    use crate::session::event::{EventType, Writer};

    fn observation_entry_with_body_tags(
        title: &str,
        body: &str,
        tags: &[&str],
    ) -> ReviewHistoryEntry {
        ReviewHistoryEntry {
            event_id: EventId::new("evt:sha256:obs"),
            event_type: EventType::ReviewObservationRecorded,
            occurred_at: "2026-05-13T10:00:01Z".to_owned(),
            payload_hash: "sha256:obs".to_owned(),
            journal_id: JournalId::new("journal:default"),
            track_id: Some(TrackId::new("agent:codex")),
            subject: Some(ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:one"),
            }),
            writer: Writer::shore_local("test"),
            verification_status: None,
            endorsements: vec![],
            principal: None,
            summary: ReviewHistorySummary::ReviewObservationRecorded {
                observation_id: ObservationId::new("obs:sha256:one"),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                title: title.to_owned(),
                body: Some(body.to_owned()),
                body_content_type: Default::default(),
                body_byte_size: Some(body.len() as u64),
                body_content_hash: Some("sha256:body".to_owned()),
                body_content_state: Default::default(),
                tags: tags.iter().map(|t| (*t).to_owned()).collect(),
                confidence: None,
                supersedes: vec![],
                responds_to: vec![],
            },
        }
    }

    fn validation_entry(
        check_name: &str,
        command: Option<&str>,
        status: &str,
    ) -> ReviewHistoryEntry {
        let status = match status {
            "passed" => ValidationStatus::Passed,
            "failed" => ValidationStatus::Failed,
            "errored" => ValidationStatus::Errored,
            _ => ValidationStatus::Skipped,
        };
        ReviewHistoryEntry {
            event_id: EventId::new("evt:sha256:val"),
            event_type: EventType::ValidationCheckRecorded,
            occurred_at: "2026-05-13T10:00:04Z".to_owned(),
            payload_hash: "sha256:val".to_owned(),
            journal_id: JournalId::new("journal:default"),
            track_id: Some(TrackId::new("agent:codex")),
            subject: Some(ReviewTargetRef::Revision {
                revision_id: RevisionId::new("rev:sha256:one"),
            }),
            writer: Writer::shore_local("test"),
            verification_status: None,
            endorsements: vec![],
            principal: None,
            summary: ReviewHistorySummary::ValidationCheckRecorded {
                validation_check_id: ValidationCheckId::new("validation:sha256:one"),
                target: ValidationTarget::Revision {
                    revision_id: RevisionId::new("rev:sha256:one"),
                },
                check_name: check_name.to_owned(),
                command: command.map(|c| c.to_owned()),
                status,
                exit_code: Some(0),
                trigger: ValidationTrigger::Manual,
                source_fingerprint: None,
                summary: None,
                summary_content_type: Default::default(),
                summary_content_hash: None,
                summary_content_state: Default::default(),
                started_at: None,
                completed_at: None,
                log_artifact_content_hashes: vec![],
            },
        }
    }

    #[test]
    fn haystack_folds_title_body_ids_track_tags_lowercased() {
        let entry = observation_entry_with_body_tags("Pinned Issue", "Body TEXT", &["Correctness"]);
        let h = build_haystack(&entry);
        assert!(h.contains("pinned issue"), "title folded: {h}");
        assert!(h.contains("body text"), "body folded: {h}");
        assert!(h.contains("correctness"), "tag folded: {h}");
        assert!(h.contains(entry.event_id.as_str()), "eventId folded: {h}");
        assert_eq!(h, h.to_lowercase(), "the haystack is lowercased");
    }

    #[test]
    fn haystack_includes_validation_check_name_command_and_status() {
        let entry = validation_entry("cargo test", Some("cargo test --all"), "passed");
        let h = build_haystack(&entry);
        assert!(h.contains("cargo test"), "checkName folded: {h}");
        assert!(h.contains("cargo test --all"), "command folded: {h}");
        assert!(h.contains("passed"), "status folded: {h}");
    }

    #[test]
    fn search_record_carries_type_track_revision_status_fields() {
        let entry = validation_entry("cargo test", None, "passed");
        let record = SearchRecord::from_entry(&entry, "", &EventRecordExtras::default());
        assert_eq!(record.field("type"), Some("validation_check_recorded"));
        assert_eq!(record.field("check"), Some("passed"));
        assert_eq!(record.field("track"), Some(" agent:codex "));
        assert!(record.field("revision").is_some());
        // The grammar key is `snapshot` (renamed from `object` in #334); the value
        // is supplied by the caller (the revision->object map), empty here.
        assert_eq!(record.field("snapshot"), Some(""));
        assert_eq!(
            record.field("object"),
            None,
            "the legacy key is gone from the record"
        );
    }

    #[test]
    fn entry_actor_returns_the_writer_actor_id() {
        // The actor gets its own slot at the record layer.
        let entry = observation_entry_with_body_tags("t", "b", &[]);
        assert_eq!(entry_actor(&entry), "actor:local"); // Writer::shore_local -> "actor:local"
    }

    #[test]
    fn entry_track_returns_the_explicit_track_only() {
        // The lane no longer folds the writer actor: no track means an empty lane,
        // and the actor is reachable through its own slot.
        let mut entry = observation_entry_with_body_tags("t", "b", &[]);
        entry.track_id = None;
        assert_eq!(entry_track(&entry), "");
        assert_eq!(entry_actor(&entry), "actor:local");
    }

    #[test]
    fn field_matches_before_after_are_lexical_over_the_range_anchor() {
        // ISO-8601 is lexically orderable; the anchor is lowercased at compare time so
        // it matches the lowercased query value.
        let record = record(
            "review_observation_recorded",
            &[(RANGE_ANCHOR_FIELD, "2026-05-13T10:00:01Z")],
            "",
        );
        assert!(field_matches(&record, "after", "2026-05-13t10:00:00z"));
        assert!(!field_matches(&record, "after", "2026-05-13t10:00:02z"));
        assert!(field_matches(&record, "before", "2026-05-13t10:00:02z"));
        assert!(!field_matches(&record, "before", "2026-05-13t10:00:00z"));
    }

    #[test]
    fn search_record_carries_actor_check_assessment_tag_and_is_fields() {
        // A validation entry: the check field is today's `status` source, renamed.
        let validation = validation_entry("cargo test", None, "passed");
        let record = SearchRecord::from_entry(&validation, "", &EventRecordExtras::default());
        assert_eq!(record.field("type"), Some("validation_check_recorded"));
        assert_eq!(record.field("check"), Some("passed"));
        assert_eq!(
            record.field("status"),
            None,
            "the status key is renamed to check"
        );
        assert_eq!(record.field("actor"), Some(" actor:local ")); // space-wrapped token set
        assert_eq!(record.field("is"), Some("")); // not a request entry, default extras

        // An observation with tags: the dual index carries both the full string and key.
        let obs = observation_entry_with_body_tags("Pinned", "body", &["issue:191"]);
        let record = SearchRecord::from_entry(&obs, "", &EventRecordExtras::default());
        assert!(field_matches(&record, "tag", "issue:191"));
        assert!(field_matches(&record, "tag", "issue"));
        assert_eq!(record.field("track"), Some(" agent:codex "));
        assert_eq!(record.field("actor"), Some(" actor:local "));
    }

    #[test]
    fn from_entry_encodes_the_is_set_from_extras() {
        // This exercises the is-set ENCODING only; which entries actually carry a value
        // is decided at the build loop, not here.
        let entry = observation_entry_with_body_tags("t", "b", &[]);
        let open = SearchRecord::from_entry(
            &entry,
            "",
            &EventRecordExtras {
                is_open: Some(true),
            },
        );
        assert_eq!(open.field("is"), Some(" open "));
        let answered = SearchRecord::from_entry(
            &entry,
            "",
            &EventRecordExtras {
                is_open: Some(false),
            },
        );
        assert_eq!(answered.field("is"), Some(" answered "));
    }

    #[test]
    fn from_entry_normalizes_the_time_slot_for_range_compare() {
        // The store mints BOTH unix-ms and RFC 3339 tokens; the record's range anchor is
        // the canonical fixed-width `.mmm` ISO form so before:/after: prefix-compare.
        let mut entry = observation_entry_with_body_tags("t", "b", &[]);
        entry.occurred_at = "unix-ms:1747130401250".to_owned(); // ~ 2025-05-13, .250 frac
        let record = SearchRecord::from_entry(&entry, "", &EventRecordExtras::default());
        let iso = record.field(RANGE_ANCHOR_FIELD).unwrap();
        assert!(
            iso.starts_with("2025-") && iso.ends_with(".250Z"),
            "got {iso}"
        );
        assert!(field_matches(&record, "after", "2025-01-01"));
        assert!(field_matches(&record, "before", "2027-01-01"));

        // An adapter-minted RFC 3339 token normalizes on this side too (round-trip).
        entry.occurred_at = "2026-05-13T10:00:01.500Z".to_owned();
        let record = SearchRecord::from_entry(&entry, "", &EventRecordExtras::default());
        assert_eq!(
            record.field(RANGE_ANCHOR_FIELD),
            Some("2026-05-13T10:00:01.500Z")
        );
    }

    #[test]
    fn build_haystack_folds_the_writer_actor_id() {
        // The actor id used to reach free text via the track fold; keep it discoverable
        // now that the lane no longer folds it.
        let entry = observation_entry_with_body_tags("t", "b", &[]);
        assert!(build_haystack(&entry).contains("actor:local"));
    }

    #[test]
    fn parse_search_query_reads_snapshot_and_aliases_legacy_object() {
        let snap = parse_search_query("snapshot:obj-1");
        assert_eq!(
            snap,
            vec![QueryClause::Field {
                field: "snapshot".to_owned(),
                value: "obj-1".to_owned(),
                negate: false,
            }]
        );
        // A user-typed legacy `object:` token aliases to the snapshot field (#334).
        assert_eq!(
            parse_search_query("object:obj-1"),
            snap,
            "legacy object: token aliases to the snapshot field"
        );
    }

    #[test]
    fn event_surface_flags_unsupported_qualifier_and_drops_the_clause() {
        // attention: is revision-only → diagnosed and dropped; the free text survives.
        let parsed = parse_search_query_for("attention:open free", QuerySurface::Event);
        assert_eq!(
            parsed.clauses,
            vec![QueryClause::Text {
                value: "free".into(),
                negate: false,
            }]
        );
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(
            parsed.diagnostics[0].code,
            QueryDiagnosticCode::UnsupportedQualifier
        );
        assert_eq!(parsed.diagnostics[0].key, "attention");
    }

    #[test]
    fn status_aliases_to_check_on_event_with_a_deprecation_hint() {
        let parsed = parse_search_query_for("status:passed", QuerySurface::Event);
        assert_eq!(
            parsed.clauses,
            vec![QueryClause::Field {
                field: "check".into(),
                value: "passed".into(),
                negate: false,
            }],
        );
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(
            parsed.diagnostics[0].code,
            QueryDiagnosticCode::DeprecatedQualifier
        );
    }

    #[test]
    fn status_aliases_to_assessment_on_revision() {
        let parsed = parse_search_query_for("status:accepted", QuerySurface::Revision);
        assert_eq!(
            parsed.clauses,
            vec![QueryClause::Field {
                field: "assessment".into(),
                value: "accepted".into(),
                negate: false,
            }],
        );
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.code == QueryDiagnosticCode::DeprecatedQualifier)
        );
    }

    #[test]
    fn actor_qualifier_accepts_the_id_with_or_without_its_prefix() {
        // Actor ids always carry the `actor:` prefix, so the qualifier value
        // canonicalizes a prefix-less spelling instead of forcing `actor:actor:…`.
        let expected = vec![QueryClause::Field {
            field: "actor".into(),
            value: "actor:agent:codex".into(),
            negate: false,
        }];
        let short = parse_search_query_for("actor:agent:codex", QuerySurface::Event);
        let full = parse_search_query_for("actor:actor:agent:codex", QuerySurface::Event);
        assert_eq!(short.clauses, expected);
        assert_eq!(full.clauses, expected);

        // Both spellings reach a real record.
        let entry = observation_entry_with_body_tags("t", "b", &[]);
        let record = SearchRecord::from_entry(&entry, "", &EventRecordExtras::default());
        assert!(matches_query(
            &record,
            &parse_search_query("actor:actor:local")
        ));
        assert!(matches_query(&record, &parse_search_query("actor:local")));
    }

    #[test]
    fn actor_qualifier_preserves_self_certifying_did_key_values() {
        // Actor ids are either actor:-scheme or a bare Ed25519 did:key
        // (is_valid_actor_id); a did:key value is already fully qualified and
        // must not be rewritten to actor:did:key:….
        let did = "did:key:z6MkehRgf7yJbgaGfYsdoAsKdBPE3dj2CYhowQdcjqSJgvVd";
        let parsed = parse_search_query_for(&format!("actor:{did}"), QuerySurface::Event);
        assert_eq!(
            parsed.clauses,
            vec![QueryClause::Field {
                field: "actor".into(),
                value: did.to_lowercase(),
                negate: false,
            }],
        );

        // End-to-end: the clause reaches a record whose writer id is the did:key.
        let mut entry = observation_entry_with_body_tags("t", "b", &[]);
        entry.writer.actor_id = crate::model::ActorId::new(did);
        let record = SearchRecord::from_entry(&entry, "", &EventRecordExtras::default());
        assert!(matches_query(
            &record,
            &parse_search_query(&format!("actor:{did}"))
        ));
    }

    #[test]
    fn object_aliases_to_snapshot_silently() {
        let parsed = parse_search_query_for("object:obj-1", QuerySurface::Event);
        assert_eq!(
            parsed.clauses,
            vec![QueryClause::Field {
                field: "snapshot".into(),
                value: "obj-1".into(),
                negate: false,
            }],
        );
        assert!(parsed.diagnostics.is_empty()); // silent alias, unchanged
    }

    #[test]
    fn unknown_key_stays_free_text_no_diagnostic() {
        let parsed = parse_search_query_for("foo:bar", QuerySurface::Event);
        assert_eq!(
            parsed.clauses,
            vec![QueryClause::Text {
                value: "foo:bar".into(),
                negate: false,
            }]
        );
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn is_value_is_validated_per_surface() {
        let bad = parse_search_query_for("is:contested", QuerySurface::Event); // event: open|answered
        assert!(bad.clauses.is_empty());
        assert_eq!(
            bad.diagnostics[0].code,
            QueryDiagnosticCode::UnsupportedValue
        );
        // The revision surface enumerates the full lifecycle set — contested is a
        // member there (the revision record carries the `is` slot).
        let contested = parse_search_query_for("is:contested", QuerySurface::Revision);
        assert_eq!(
            contested.clauses,
            vec![QueryClause::Field {
                field: "is".into(),
                value: "contested".into(),
                negate: false,
            }],
        );
        assert!(contested.diagnostics.is_empty());
        // The revision surface still validates enumerated values via attention:.
        let bad_attention = parse_search_query_for("attention:bogus", QuerySurface::Revision);
        assert!(bad_attention.clauses.is_empty());
        assert_eq!(
            bad_attention.diagnostics[0].code,
            QueryDiagnosticCode::UnsupportedValue
        );
        let ok = parse_search_query_for("attention:open-request", QuerySurface::Revision);
        assert_eq!(
            ok.clauses,
            vec![QueryClause::Field {
                field: "attention".into(),
                value: "open-request".into(),
                negate: false,
            }],
        );
        assert!(ok.diagnostics.is_empty());
    }

    #[test]
    fn revision_surface_supports_indexed_qualifiers_and_rejects_type_and_check() {
        // The revision record carries track/actor/tag/is slots now, so the
        // qualifiers parse to clauses (actor: canonicalizes its prefix-less
        // spelling, same as the event surface).
        for (query, field, value) in [
            ("track:agent:codex", "track", "agent:codex"),
            ("actor:actor:local", "actor", "actor:local"),
            ("actor:local", "actor", "actor:local"),
            ("tag:issue", "tag", "issue"),
            ("is:open", "is", "open"),
        ] {
            let parsed = parse_search_query_for(query, QuerySurface::Revision);
            assert_eq!(
                parsed.clauses,
                vec![QueryClause::Field {
                    field: field.into(),
                    value: value.into(),
                    negate: false,
                }],
                "{query}"
            );
            assert!(parsed.diagnostics.is_empty(), "{query}");
        }
        // type:/check: stay event-only — known-but-unsupported here, diagnosed,
        // never silent-empty.
        for query in ["type:observation", "check:passed"] {
            let parsed = parse_search_query_for(query, QuerySurface::Revision);
            assert!(
                parsed.clauses.is_empty(),
                "{query} must not produce a clause"
            );
            assert_eq!(
                parsed.diagnostics[0].code,
                QueryDiagnosticCode::UnsupportedQualifier,
                "{query}"
            );
        }
        // before:/after: stay supported — the revision index carries the anchor.
        let ranged = parse_search_query_for("after:2026-01-01", QuerySurface::Revision);
        assert_eq!(ranged.clauses.len(), 1);
        assert!(ranged.diagnostics.is_empty());
    }

    #[test]
    fn legacy_parse_search_query_delegates_to_the_event_surface() {
        assert_eq!(
            parse_search_query("status:passed"),
            vec![QueryClause::Field {
                field: "check".into(),
                value: "passed".into(),
                negate: false,
            }],
        );
    }

    // ---- 1.2 grammar parity corpus (mirrors web/src/test/query.test.ts) ----

    /// A bare search record with the given fields, for the grammar matchers.
    fn record(type_id: &str, fields: &[(&str, &str)], text: &str) -> SearchRecord {
        let mut map = BTreeMap::new();
        map.insert("type".to_owned(), type_id.to_owned());
        for (key, value) in fields {
            map.insert((*key).to_owned(), (*value).to_owned());
        }
        SearchRecord {
            text: text.to_owned(),
            fields: map,
        }
    }

    #[test]
    fn tokenize_splits_on_whitespace() {
        assert_eq!(tokenize_query("foo bar"), vec!["foo", "bar"]);
        assert!(tokenize_query("").is_empty());
    }

    #[test]
    fn tokenize_keeps_quoted_phrases_bare_negated_and_field_prefixed_intact() {
        assert_eq!(
            tokenize_query("type:observation \"quoted phrase\""),
            vec!["type:observation", "\"quoted phrase\""]
        );
        assert_eq!(tokenize_query("-\"neg phrase\""), vec!["-\"neg phrase\""]);
        assert_eq!(
            tokenize_query("track:\"agent codex\""),
            vec!["track:\"agent codex\""]
        );
        assert_eq!(tokenize_query("-status:failed"), vec!["-status:failed"]);
    }

    #[test]
    fn parses_field_clause_and_free_text() {
        let clauses = parse_search_query("type:observation pinned");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Field {
                    field: "type".into(),
                    value: "observation".into(),
                    negate: false,
                },
                QueryClause::Text {
                    value: "pinned".into(),
                    negate: false,
                },
            ]
        );
    }

    #[test]
    fn parses_known_field_lowercasing_field_and_value() {
        assert_eq!(
            parse_search_query("TYPE:Observation"),
            vec![QueryClause::Field {
                field: "type".into(),
                value: "observation".into(),
                negate: false,
            }]
        );
    }

    #[test]
    fn parses_negation_and_quoted_phrase() {
        let clauses = parse_search_query("-track:\"agent codex\" \"needs review\"");
        assert_eq!(
            clauses,
            vec![
                QueryClause::Field {
                    field: "track".into(),
                    value: "agent codex".into(),
                    negate: true,
                },
                QueryClause::Text {
                    value: "needs review".into(),
                    negate: false,
                },
            ]
        );
    }

    #[test]
    fn parses_leading_dash_as_negation_for_field_and_text() {
        // `status:` aliases to `check:` on the event surface; negation rides along.
        assert_eq!(
            parse_search_query("-status:failed"),
            vec![QueryClause::Field {
                field: "check".into(),
                value: "failed".into(),
                negate: true,
            }]
        );
        assert_eq!(
            parse_search_query("-observed"),
            vec![QueryClause::Text {
                value: "observed".into(),
                negate: true,
            }]
        );
    }

    #[test]
    fn strips_quotes_from_field_values_and_phrases() {
        assert_eq!(
            parse_search_query("track:\"agent:codex\""),
            vec![QueryClause::Field {
                field: "track".into(),
                value: "agent:codex".into(),
                negate: false,
            }]
        );
        assert_eq!(
            parse_search_query("\"quoted phrase\""),
            vec![QueryClause::Text {
                value: "quoted phrase".into(),
                negate: false,
            }]
        );
    }

    #[test]
    fn unknown_field_becomes_free_text() {
        // `foo:` is not a grammar field, so the whole token is a free-text term.
        assert_eq!(
            parse_search_query("foo:bar"),
            vec![QueryClause::Text {
                value: "foo:bar".into(),
                negate: false,
            }]
        );
        assert_eq!(
            parse_search_query("nope:value"),
            vec![QueryClause::Text {
                value: "nope:value".into(),
                negate: false,
            }]
        );
    }

    #[test]
    fn splits_bare_terms_and_empty_query() {
        assert_eq!(
            parse_search_query("free text"),
            vec![
                QueryClause::Text {
                    value: "free".into(),
                    negate: false,
                },
                QueryClause::Text {
                    value: "text".into(),
                    negate: false,
                },
            ]
        );
        assert!(parse_search_query("").is_empty());
    }

    #[test]
    fn field_matches_track_and_actor_by_whole_token_not_substring() {
        // The record stores track/actor space-wrapped, matched as whole tokens.
        let record = record(
            "review_observation_recorded",
            &[
                ("track", " agent:codex "),
                ("actor", " actor:agent:codex-loop "),
            ],
            "",
        );
        assert!(field_matches(&record, "track", "agent:codex"));
        assert!(!field_matches(&record, "track", "codex")); // a substring, not a whole token
        assert!(field_matches(&record, "actor", "actor:agent:codex-loop"));
        assert!(!field_matches(&record, "actor", "codex-loop"));
    }

    #[test]
    fn field_matches_is_and_tag_are_set_membership_not_substring() {
        // The space-wrapped token set: a value matches only as a whole token.
        let record = record(
            "input_request_opened",
            &[("is", " open answered "), ("tag", " issue issue:191 ")],
            "",
        );
        assert!(field_matches(&record, "is", "open"));
        assert!(field_matches(&record, "is", "answered"));
        assert!(!field_matches(&record, "is", "pen")); // a substring of "open", not a member
        assert!(field_matches(&record, "tag", "issue:191"));
        assert!(field_matches(&record, "tag", "issue")); // the first-colon key
        assert!(!field_matches(&record, "tag", "issues"));
    }

    #[test]
    fn field_matches_normalizes_full_rfc3339_range_bounds() {
        // A whole-second RFC 3339 bound must compare against the fixed-width
        // millisecond anchor by instant, not raw bytes ('.' sorts before 'z').
        let later = record(
            "review_observation_recorded",
            &[(RANGE_ANCHOR_FIELD, "2026-05-13T10:00:01.500Z")],
            "",
        );
        assert!(!field_matches(&later, "before", "2026-05-13t10:00:01z"));
        assert!(field_matches(&later, "after", "2026-05-13t10:00:01z"));
        // An anchor equal to the bound matches neither strict range.
        let equal = record(
            "review_observation_recorded",
            &[(RANGE_ANCHOR_FIELD, "2026-05-13T10:00:01.000Z")],
            "",
        );
        assert!(!field_matches(&equal, "before", "2026-05-13t10:00:01z"));
        assert!(!field_matches(&equal, "after", "2026-05-13t10:00:01z"));
        // A date/datetime prefix keeps raw lexical prefix-compare semantics.
        assert!(field_matches(&later, "before", "2026-05-14"));
        assert!(field_matches(&later, "after", "2026-05-13"));
    }

    #[test]
    fn field_matches_type_is_comma_list_or() {
        // The only comma-OR key: any listed type resolves-and-matches.
        let record = record("review_observation_recorded", &[], "");
        assert!(field_matches(&record, "type", "observation,assessment"));
        assert!(!field_matches(&record, "type", "assessment,response"));
    }

    #[test]
    fn field_matches_assessment_by_display_label_or_wire() {
        let record = record(
            "review_assessment_recorded",
            &[("assessment", "needs_changes")],
            "",
        );
        assert!(field_matches(&record, "assessment", "needs-changes")); // display label
        assert!(field_matches(&record, "assessment", "needs_changes")); // wire value
        assert!(!field_matches(&record, "assessment", "accepted"));
    }

    #[test]
    fn field_matches_type_by_label_or_raw_id_exactly() {
        let record = record(
            "review_observation_recorded",
            &[("track", "agent:codex")],
            "",
        );
        assert!(field_matches(&record, "type", "observation"));
        assert!(field_matches(
            &record,
            "type",
            "review_observation_recorded"
        ));
        assert!(!field_matches(&record, "type", "assessment"));
    }

    #[test]
    fn field_matches_non_type_as_substring_of_lowercased_record() {
        let record = record(
            "review_observation_recorded",
            &[
                ("track", " agent:codex "),
                ("revision", "rev:sha256:abcdef"),
            ],
            "",
        );
        assert!(field_matches(&record, "revision", "abcd"));
        // track is whole-token membership over the wrapped field, not substring.
        assert!(field_matches(&record, "track", "agent:codex"));
        assert!(!field_matches(&record, "track", "codex"));
        assert!(!field_matches(&record, "revision", "zzz"));
    }

    #[test]
    fn field_matches_missing_field_is_no_match() {
        let record = record("review_observation_recorded", &[], "");
        assert!(!field_matches(&record, "snapshot", "anything"));
    }

    #[test]
    fn matches_anding_clauses_with_negation() {
        let record = record(
            "review_observation_recorded",
            &[("track", "agent:codex")],
            "pinned correctness issue",
        );
        assert!(matches_query(&record, &parse_search_query("pinned")));
        assert!(matches_query(&record, &parse_search_query("-missing")));
        assert!(!matches_query(
            &record,
            &parse_search_query("pinned -correctness")
        ));
    }

    #[test]
    fn matches_empty_clause_set_is_true() {
        let record = record("review_observation_recorded", &[], "anything");
        assert!(matches_query(&record, &[]));
    }

    #[test]
    fn matches_ands_field_and_free_text() {
        let record = record(
            "review_observation_recorded",
            &[("track", "agent:codex")],
            "observed change in src/lib.rs",
        );
        assert!(matches_query(
            &record,
            &parse_search_query("type:observation observed")
        ));
        assert!(!matches_query(
            &record,
            &parse_search_query("type:observation missing")
        ));
    }

    #[test]
    fn type_field_accepts_label_or_raw_id() {
        let record = record("review_observation_recorded", &[], "");
        assert!(matches_query(
            &record,
            &parse_search_query("type:observation")
        ));
        assert!(matches_query(
            &record,
            &parse_search_query("type:review_observation_recorded")
        ));
        assert!(!matches_query(
            &record,
            &parse_search_query("type:assessment")
        ));
    }

    #[test]
    fn non_type_field_substring_matches_record_field() {
        let record = record(
            "review_observation_recorded",
            &[("track", " agent:codex ")],
            "",
        );
        assert!(matches_query(
            &record,
            &parse_search_query("track:agent:codex")
        ));
        assert!(!matches_query(&record, &parse_search_query("track:codex")));
        assert!(!matches_query(&record, &parse_search_query("track:claude")));
    }

    #[test]
    fn non_type_field_match_is_case_insensitive_on_the_record_value() {
        // Case-folding happens at BUILD time: membership compares the record
        // byte-for-byte, so a mixed-case track id lowercases into the wrapped token.
        let mut entry = observation_entry_with_body_tags("t", "b", &[]);
        entry.track_id = Some(TrackId::new("agent:Codex"));
        let record = SearchRecord::from_entry(&entry, "", &EventRecordExtras::default());
        assert!(field_matches(&record, "track", "agent:codex"));
    }
}
