use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::projection::{BaseEntry, BaseHistoryProjection};
use super::search::{event_type_wire, matches_query, parse_search_query};
use super::summary::ReviewHistoryEntry;
use crate::model::EventId;
use crate::session::ProjectionDiagnostic;

/// The display order of the queried history page. The base is stored ascending
/// `(occurred_at, event_id)`; `Desc` reverses the filtered set before windowing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HistoryOrder {
    #[default]
    Asc,
    Desc,
}

/// The query applied to the base projection: full-text `q`, the exact `track` and
/// `object` params, the enabled event-type page set (`None` => all types), and the
/// display order. Pure data — `apply_history_query` runs it over a cached base.
#[derive(Clone, Debug, Default)]
pub struct HistoryQuery {
    pub q: String,
    pub track: Option<String>,
    pub object: Option<String>,
    pub types: Option<BTreeSet<String>>,
    pub order: HistoryOrder,
}

/// The query-path window spec. Precedence `at` › `offset`; a bare `limit`
/// (offset/at both `None`) is the first page. Positional by design — the
/// inspector is a random-access virtual list that needs backward paging and
/// reveal-to-position, which a forward-only cursor cannot express. The CLI keeps
/// the opaque `HistoryCursor` (plan 0092 `HistoryWindow`), which this path does
/// not touch.
#[derive(Clone, Debug, Default)]
pub struct HistoryPage {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub at: Option<EventId>,
}

/// The result of `apply_history_query`: the windowed page plus the facet counts,
/// the full filtered size (`match_count`), the window start (`offset`), the located
/// index for an `at` request (`match_index`), and the FULL-set identity (never the
/// filtered set — plan 0092 INV-5).
pub struct QueriedHistory {
    pub entries: Vec<ReviewHistoryEntry>,
    pub facets: BTreeMap<String, usize>,
    pub match_count: usize,
    pub offset: usize,
    pub match_index: Option<usize>,
    pub event_set_hash: String,
    pub event_count: usize,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

/// Run the query over the cached base projection, purely (no I/O): filter (q +
/// track + object + types page set) → facets (excluding the types page set, INV-3)
/// → order → window (`at` › `offset`, INV-2). Identity is always the full replayed
/// set (INV-2).
pub fn apply_history_query(
    base: &BaseHistoryProjection,
    query: &HistoryQuery,
    page: &HistoryPage,
) -> QueriedHistory {
    let clauses = parse_search_query(&query.q);
    // The facet predicate is q + track + object, EXCLUDING the `types` page filter
    // (INV-3). The page predicate additionally applies the `types` set.
    let facet_match = |entry: &BaseEntry| {
        track_object_match(entry, query) && matches_query(&entry.record, &clauses)
    };

    let mut facets: BTreeMap<String, usize> = BTreeMap::new();
    for entry in base.entries.iter().filter(|&entry| facet_match(entry)) {
        *facets.entry(event_type_wire(&entry.entry)).or_default() += 1;
    }

    let mut filtered: Vec<&BaseEntry> = base
        .entries
        .iter()
        .filter(|&entry| facet_match(entry) && type_set_match(entry, query.types.as_ref()))
        .collect();
    // The base is ascending `(occurred_at, event_id)`; `Desc` reverses the ordered
    // filtered set so windowing runs in display order (INV-2).
    if matches!(query.order, HistoryOrder::Desc) {
        filtered.reverse();
    }
    let match_count = filtered.len();

    let (range, match_index) = resolve_window(&filtered, page);
    let entries = filtered[range.clone()]
        .iter()
        .map(|entry| entry.entry.clone())
        .collect();

    QueriedHistory {
        entries,
        facets,
        match_count,
        offset: range.start,
        match_index,
        event_set_hash: base.event_set_hash.clone(),
        event_count: base.event_count,
        diagnostics: base.diagnostics.clone(),
    }
}

/// The `track=` and `object=` params: exact matches against the entry's record
/// fields (mirrors the client's `entryTrack === filterTrack` / object equality).
/// An absent param does not constrain.
fn track_object_match(entry: &BaseEntry, query: &HistoryQuery) -> bool {
    if let Some(track) = &query.track
        && entry.record.field("track") != Some(track.as_str())
    {
        return false;
    }
    if let Some(object) = &query.object
        && entry.record.field("object") != Some(object.as_str())
    {
        return false;
    }
    true
}

/// The `types` page filter (the enabled event-type set): `None` => all types;
/// `Some(set)` keeps only entries whose event-type wire id is in the set.
fn type_set_match(entry: &BaseEntry, types: Option<&BTreeSet<String>>) -> bool {
    match types {
        None => true,
        Some(set) => set.contains(&event_type_wire(&entry.entry)),
    }
}

/// The exclusive window end for `start` under `limit` (`None` => to `len`),
/// saturating so an attacker-supplied huge `limit` cannot overflow. `Some(0)`
/// yields an empty `start..start` — never a divide-by-zero (F6).
fn page_end(start: usize, limit: Option<usize>, len: usize) -> usize {
    match limit {
        Some(limit) => start.saturating_add(limit).min(len),
        None => len,
    }
}

/// Resolve the window over the filtered + display-ordered set. Precedence
/// `at` › `offset` (a bare `limit` is the first page). Returns the index range
/// and, for an `at` request, the located index (`match_index`).
fn resolve_window(filtered: &[&BaseEntry], page: &HistoryPage) -> (Range<usize>, Option<usize>) {
    let len = filtered.len();
    if let Some(at) = &page.at {
        let Some(index) = filtered
            .iter()
            .position(|entry| &entry.entry.event_id == at)
        else {
            // The located event is filtered out — an empty page, no index (INV-2).
            return (0..0, None);
        };
        // Page-align the window on the located index. Division by the limit happens
        // only in the `limit > 0` arm, so a zero/absent limit never divides (F6).
        let range = match page.limit {
            Some(0) => 0..0,
            None => 0..len,
            Some(limit) => {
                let start = (index / limit) * limit;
                start..page_end(start, Some(limit), len)
            }
        };
        return (range, Some(index));
    }
    let start = page.offset.unwrap_or(0).min(len);
    (start..page_end(start, page.limit, len), None)
}

#[cfg(test)]
mod tests {
    use super::super::search::{SearchRecord, build_haystack};
    use super::super::summary::ReviewHistorySummary;
    use super::*;
    use crate::model::{ObservationId, ReviewTargetRef, RevisionId};
    use crate::session::event::{EventType, Writer};

    fn entry(
        occurred_at: &str,
        id: &str,
        event_type: EventType,
        title: &str,
        track: &str,
        revision: &str,
    ) -> ReviewHistoryEntry {
        let summary = match event_type {
            EventType::ReviewAssessmentRecorded => ReviewHistorySummary::ReviewAssessmentRecorded {
                assessment_id: crate::model::AssessmentId::new("assess:sha256:x"),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new(revision),
                },
                assessment: crate::session::event::ReviewAssessment::Accepted,
                summary: Some(title.to_owned()),
                summary_content_type: Default::default(),
                summary_byte_size: None,
                summary_content_hash: None,
                replaces: vec![],
                related_observations: vec![],
                related_input_requests: vec![],
            },
            _ => ReviewHistorySummary::ReviewObservationRecorded {
                observation_id: ObservationId::new("obs:sha256:x"),
                target: ReviewTargetRef::Revision {
                    revision_id: RevisionId::new(revision),
                },
                title: title.to_owned(),
                body: None,
                body_content_type: Default::default(),
                body_byte_size: None,
                body_content_hash: None,
                tags: vec![],
                confidence: None,
                supersedes: vec![],
            },
        };
        ReviewHistoryEntry {
            event_id: EventId::new(id),
            event_type,
            occurred_at: occurred_at.to_owned(),
            payload_hash: "sha256:x".to_owned(),
            journal_id: crate::model::JournalId::new("journal:default"),
            track_id: Some(crate::model::TrackId::new(track)),
            subject: Some(ReviewTargetRef::Revision {
                revision_id: RevisionId::new(revision),
            }),
            writer: Writer::shore_local("test"),
            verification_status: None,
            endorsements: vec![],
            principal: None,
            summary,
        }
    }

    fn base_from(entries: Vec<(ReviewHistoryEntry, &str)>) -> BaseHistoryProjection {
        let count = entries.len();
        let base_entries = entries
            .into_iter()
            .map(|(entry, object)| {
                let record = SearchRecord::from_entry(&entry, object);
                BaseEntry { entry, record }
            })
            .collect();
        BaseHistoryProjection {
            entries: base_entries,
            event_set_hash: "sha256:test".to_owned(),
            event_count: count,
            diagnostics: Vec::new(),
        }
    }

    /// `n` ascending observation entries on one track/revision.
    fn base_of(n: usize) -> BaseHistoryProjection {
        let entries = (1..=n)
            .map(|i| {
                (
                    entry(
                        &format!("2026-05-13T10:00:{i:02}Z"),
                        &format!("evt:sha256:{i:02}"),
                        EventType::ReviewObservationRecorded,
                        &format!("entry {i}"),
                        "agent:codex",
                        "rev:sha256:one",
                    ),
                    "obj:sha256:one",
                )
            })
            .collect();
        base_from(entries)
    }

    fn base_with_titles(titles: &[&str]) -> BaseHistoryProjection {
        let entries = titles
            .iter()
            .enumerate()
            .map(|(i, title)| {
                (
                    entry(
                        &format!("2026-05-13T10:00:{:02}Z", i + 1),
                        &format!("evt:sha256:{:02}", i + 1),
                        EventType::ReviewObservationRecorded,
                        title,
                        "agent:codex",
                        "rev:sha256:one",
                    ),
                    "obj:sha256:one",
                )
            })
            .collect();
        base_from(entries)
    }

    /// Two observations and one assessment, ascending.
    fn mixed_base() -> BaseHistoryProjection {
        base_from(vec![
            (
                entry(
                    "2026-05-13T10:00:01Z",
                    "evt:sha256:01",
                    EventType::ReviewObservationRecorded,
                    "first observation",
                    "agent:codex",
                    "rev:sha256:one",
                ),
                "obj:sha256:one",
            ),
            (
                entry(
                    "2026-05-13T10:00:02Z",
                    "evt:sha256:02",
                    EventType::ReviewObservationRecorded,
                    "second observation",
                    "agent:codex",
                    "rev:sha256:one",
                ),
                "obj:sha256:one",
            ),
            (
                entry(
                    "2026-05-13T10:00:03Z",
                    "evt:sha256:03",
                    EventType::ReviewAssessmentRecorded,
                    "an assessment",
                    "human:kevin",
                    "rev:sha256:one",
                ),
                "obj:sha256:one",
            ),
        ])
    }

    fn page(limit: Option<usize>) -> HistoryPage {
        HistoryPage {
            limit,
            offset: None,
            at: None,
        }
    }

    fn offset_page(limit: usize, offset: usize) -> HistoryPage {
        HistoryPage {
            limit: Some(limit),
            offset: Some(offset),
            at: None,
        }
    }

    #[test]
    fn empty_query_unwindowed_equals_base_order_and_full_identity() {
        let base = base_of(5);
        let out = apply_history_query(&base, &HistoryQuery::default(), &HistoryPage::default());
        assert_eq!(out.entries.len(), 5);
        assert_eq!(out.match_count, 5);
        assert_eq!(out.offset, 0);
        assert_eq!(out.event_count, base.event_count);
        assert_eq!(out.event_set_hash, base.event_set_hash);
    }

    #[test]
    fn q_filters_page_and_match_count_over_filtered_set() {
        let base = base_with_titles(&["pinned alpha", "other", "pinned beta"]);
        let q = HistoryQuery {
            q: "pinned".into(),
            ..Default::default()
        };
        let out = apply_history_query(&base, &q, &HistoryPage::default());
        assert_eq!(out.match_count, 2);
        assert!(
            out.entries
                .iter()
                .all(|e| build_haystack(e).contains("pinned"))
        );
    }

    #[test]
    fn facets_exclude_the_types_page_filter_but_honor_q() {
        let base = mixed_base();
        let mut types = BTreeSet::new();
        types.insert("review_observation_recorded".to_owned());
        let q = HistoryQuery {
            types: Some(types),
            ..Default::default()
        };
        let out = apply_history_query(&base, &q, &HistoryPage::default());
        assert_eq!(out.entries.len(), 2);
        assert_eq!(out.match_count, 2);
        assert_eq!(out.facets.get("review_observation_recorded"), Some(&2));
        assert_eq!(out.facets.get("review_assessment_recorded"), Some(&1));
    }

    #[test]
    fn type_clause_in_q_affects_facets_unlike_the_types_param() {
        let base = mixed_base();
        let q = HistoryQuery {
            q: "type:observation".into(),
            ..Default::default()
        };
        let out = apply_history_query(&base, &q, &HistoryPage::default());
        assert_eq!(out.facets.get("review_assessment_recorded"), None);
        assert_eq!(out.facets.get("review_observation_recorded"), Some(&2));
    }

    #[test]
    fn order_desc_reverses_and_window_pages_in_display_order() {
        let base = base_of(5);
        let q = HistoryQuery {
            order: HistoryOrder::Desc,
            ..Default::default()
        };
        let out = apply_history_query(&base, &q, &page(Some(2)));
        assert!(out.entries[0].occurred_at > out.entries[1].occurred_at);
        assert_eq!(out.offset, 0);
    }

    #[test]
    fn offset_paging_continues_the_filtered_set_without_overlap() {
        let base = base_of(5);
        let p1 = apply_history_query(&base, &HistoryQuery::default(), &page(Some(2)));
        let p2 = apply_history_query(&base, &HistoryQuery::default(), &offset_page(2, 2));
        assert_eq!(p1.entries.len(), 2);
        assert_eq!(p2.offset, 2);
        assert_ne!(
            p1.entries.last().unwrap().event_id,
            p2.entries.first().unwrap().event_id
        );
    }

    #[test]
    fn desc_offset_paging_continues_toward_older_entries() {
        let base = base_of(5);
        let q = HistoryQuery {
            order: HistoryOrder::Desc,
            ..Default::default()
        };
        let p1 = apply_history_query(&base, &q, &page(Some(2))); // the two newest
        let p2 = apply_history_query(&base, &q, &offset_page(2, 2)); // the next two, older
        assert!(p2.entries.first().unwrap().occurred_at < p1.entries.last().unwrap().occurred_at);
        assert_eq!(p2.offset, 2);
    }

    #[test]
    fn offset_windows_the_filtered_set() {
        let base = base_of(5);
        let out = apply_history_query(&base, &HistoryQuery::default(), &offset_page(2, 1));
        assert_eq!(out.offset, 1);
        assert_eq!(out.entries.len(), 2);
        assert_eq!(out.match_count, 5);
    }

    #[test]
    fn at_locates_the_page_containing_an_event_and_sets_match_index() {
        let base = base_of(10);
        let target = base.entries[7].entry.event_id.clone();
        let out = apply_history_query(
            &base,
            &HistoryQuery::default(),
            &HistoryPage {
                limit: Some(3),
                offset: None,
                at: Some(target.clone()),
            },
        );
        assert_eq!(out.offset, 6);
        assert_eq!(out.match_index, Some(7));
        assert!(out.entries.iter().any(|e| e.event_id == target));
    }

    #[test]
    fn at_absent_from_filtered_set_returns_empty_with_no_match_index() {
        let base = base_with_titles(&["alpha", "beta"]);
        let q = HistoryQuery {
            q: "alpha".into(),
            ..Default::default()
        };
        let missing = base.entries[1].entry.event_id.clone();
        let out = apply_history_query(
            &base,
            &q,
            &HistoryPage {
                limit: Some(5),
                offset: None,
                at: Some(missing),
            },
        );
        assert!(out.match_index.is_none());
    }

    #[test]
    fn at_with_zero_limit_is_empty_and_does_not_panic() {
        let base = base_of(5);
        let target = base.entries[3].entry.event_id.clone();
        let out = apply_history_query(
            &base,
            &HistoryQuery::default(),
            &HistoryPage {
                limit: Some(0),
                offset: None,
                at: Some(target),
            },
        );
        assert!(out.entries.is_empty());
        assert_eq!(out.match_index, Some(3));
    }
}
