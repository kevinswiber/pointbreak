use std::collections::BTreeMap;

use serde::Serialize;

use super::summary::{ReviewHistoryEntry, ReviewHistorySummary};
use crate::model::{ReviewEndpoint, ReviewTargetRef};
use crate::session::event::EventType;

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
    /// still sourced from the shared `object_id` document field.
    pub fn from_entry(entry: &ReviewHistoryEntry, snapshot: &str) -> Self {
        let mut fields = BTreeMap::new();
        fields.insert("type".to_owned(), event_type_wire(entry));
        fields.insert("track".to_owned(), entry_track(entry));
        fields.insert("revision".to_owned(), entry_revision_id(entry));
        fields.insert("snapshot".to_owned(), snapshot.to_owned());
        fields.insert("status".to_owned(), summary_status(entry));
        Self {
            text: build_haystack(entry),
            fields,
        }
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
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

/// Tokenize a raw query string, then classify each token into a negatable field
/// or free-text clause — parity with `web/src/query.ts parseSearchQuery` (INV-4).
/// A `field:value` whose field is in `QUERY_FIELDS` is a field clause (value
/// lowercased, surrounding quotes stripped); everything else is free text.
pub fn parse_search_query(query: &str) -> Vec<QueryClause> {
    let mut clauses = Vec::new();
    for raw in tokenize_query(query) {
        let mut token = raw.as_str();
        let mut negate = false;
        // A leading `-` negates, but only when something follows it.
        if token.len() > 1 && token.starts_with('-') {
            negate = true;
            token = &token[1..];
        }
        let colon = token.find(':');
        let field = match colon {
            Some(index) if index > 0 => token[..index].to_lowercase(),
            _ => String::new(),
        };
        // Legacy `object:` aliases to the renamed `snapshot` field during the
        // transition (#334); the token is user-typed, so old queries keep working.
        let field = if field == "object" {
            "snapshot".to_owned()
        } else {
            field
        };
        if !field.is_empty() && QUERY_FIELDS.contains(&field.as_str()) {
            let value = strip_wrapping_quotes(&token[colon.expect("field implies a colon") + 1..])
                .to_lowercase();
            clauses.push(QueryClause::Field {
                field,
                value,
                negate,
            });
        } else {
            let value = strip_wrapping_quotes(token).to_lowercase();
            if !value.is_empty() {
                clauses.push(QueryClause::Text { value, negate });
            }
        }
    }
    clauses
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

/// The grammar fields a `field:value` clause may address (mirrors web
/// `QUERY_FIELDS`). `attention` is in the client list but is never populated in a
/// history record, so a bare `attention:` clause matches "" — preserved.
const QUERY_FIELDS: &[&str] = &[
    "type",
    "track",
    "revision",
    "snapshot",
    "status",
    "attention",
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

/// Match one field clause against a record (mirrors `fieldMatches`). The `type`
/// field compares exactly to the resolved wire id (label-or-id); every other
/// field substring-matches the record's value lowercased at match time (the
/// value is already lowercased by `parse_search_query`).
fn field_matches(record: &SearchRecord, field: &str, value: &str) -> bool {
    if field == "type" {
        return record.field("type").unwrap_or("") == resolve_type_value(value);
    }
    record
        .field(field)
        .unwrap_or("")
        .to_lowercase()
        .contains(value)
}

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
/// field fold exactly what the TS client received over the wire.
fn enum_wire<T: Serialize>(value: &T) -> String {
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

/// The lane an entry belongs to — its explicit track, else its writer's actor id
/// (mirrors `web/src/projection.ts entryTrack`). `pub(super)` for reuse by the
/// query path (3.1).
pub(super) fn entry_track(entry: &ReviewHistoryEntry) -> String {
    match &entry.track_id {
        Some(track) if !track.as_str().is_empty() => track.as_str().to_owned(),
        _ => entry.writer.actor_id.as_str().to_owned(),
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

/// The `status` record field — the validation status wire string, else ""
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
        ReviewHistorySummary::RevisionCaptured { base, .. } => {
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
    match value {
        "accepted" => "accepted",
        "accepted_with_follow_up" => "accepted-with-follow-up",
        "needs_changes" => "needs-changes",
        "needs_clarification" => "needs-clarification",
        other => other,
    }
    .to_owned()
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
        let record = SearchRecord::from_entry(&entry, "");
        assert_eq!(record.field("type"), Some("validation_check_recorded"));
        assert_eq!(record.field("status"), Some("passed"));
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
        assert_eq!(
            parse_search_query("-status:failed"),
            vec![QueryClause::Field {
                field: "status".into(),
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
            &[("track", "agent:codex"), ("revision", "rev:sha256:abcdef")],
            "",
        );
        assert!(field_matches(&record, "revision", "abcd"));
        assert!(field_matches(&record, "track", "codex"));
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
            &[("track", "agent:codex")],
            "",
        );
        assert!(matches_query(&record, &parse_search_query("track:codex")));
        assert!(!matches_query(&record, &parse_search_query("track:claude")));
    }

    #[test]
    fn non_type_field_match_is_case_insensitive_on_the_record_value() {
        // A mixed-case stored field still matches a lowercased query value (parity
        // with the TS `idx[field].toLowerCase().includes(value)`).
        let record = record(
            "review_observation_recorded",
            &[("track", "agent:Codex")],
            "",
        );
        assert!(matches_query(&record, &parse_search_query("track:codex")));
    }
}
