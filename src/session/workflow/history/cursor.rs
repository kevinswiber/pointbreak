use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::error::{Result, ShoreError};
use crate::model::EventId;

/// An opaque, deterministic continuation token for the review-history read path.
///
/// The token encodes the sort key `(occurred_at, event_id)` of the last entry a
/// caller has already seen. Clients treat it as opaque base64 and never parse
/// it; the server owns its meaning and may change the encoded shape later (for
/// example, to a future index key) without a client-visible change. The codec is
/// a pure function of its fields — no clock or randomness — so a given key always
/// yields the same token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryCursor {
    pub occurred_at: String,
    pub event_id: EventId,
}

impl HistoryCursor {
    /// Encode the cursor as a stable, opaque base64url (no-pad) token.
    pub fn encode(&self) -> String {
        // `occurred_at` and the event id never contain a newline, so '\n' is a
        // safe delimiter between the two fields (decode validates the split).
        let raw = format!("{}\n{}", self.occurred_at, self.event_id.as_str());
        URL_SAFE_NO_PAD.encode(raw.as_bytes())
    }

    /// Decode a token produced by [`HistoryCursor::encode`]. Returns a typed
    /// error on any malformed input (callers map it to an HTTP 400 / CLI usage
    /// error).
    pub fn decode(token: &str) -> Result<HistoryCursor> {
        let bytes = URL_SAFE_NO_PAD
            .decode(token.as_bytes())
            .map_err(|_| invalid_cursor())?;
        let raw = String::from_utf8(bytes).map_err(|_| invalid_cursor())?;
        let (occurred_at, event_id) = raw.split_once('\n').ok_or_else(invalid_cursor)?;
        Ok(HistoryCursor {
            occurred_at: occurred_at.to_owned(),
            event_id: EventId::new(event_id),
        })
    }
}

fn invalid_cursor() -> ShoreError {
    ShoreError::Message("invalid history cursor".to_owned())
}

/// The total order over the review-history stream: ascending `occurred_at`, then
/// ascending `event_id`. The projection sort and the window slicer both key on
/// this, so a continuation cursor lands between exactly the neighbours the sort
/// produced.
pub(super) fn cmp_key<'a>(occurred_at: &'a str, event_id: &'a str) -> (&'a str, &'a str) {
    (occurred_at, event_id)
}

/// A forward window over the filtered + sorted history stream: take at most
/// `limit` entries starting strictly after `after`. `None`/`None` (the default)
/// is the full unbounded result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryWindow {
    pub limit: Option<usize>,
    pub after: Option<HistoryCursor>,
}

/// The result of applying a [`HistoryWindow`]: the index range to hydrate and the
/// continuation token for the next page (if any entries remain after this one).
pub struct WindowSlice {
    pub range: std::ops::Range<usize>,
    pub next_cursor: Option<String>,
}

impl HistoryWindow {
    /// Apply the window to `keys`, the filtered set already sorted ascending by
    /// `(occurred_at, event_id)`. Pure: no I/O, no body hydration — the caller
    /// hydrates only the returned `range`.
    pub fn apply(&self, keys: &[HistoryCursor]) -> WindowSlice {
        // `start` is the first index strictly after `after` (0 when unset).
        let start = match &self.after {
            None => 0,
            Some(after) => keys.partition_point(|key| {
                cmp_key(&key.occurred_at, key.event_id.as_str())
                    <= cmp_key(&after.occurred_at, after.event_id.as_str())
            }),
        };
        let end = match self.limit {
            None => keys.len(),
            // `limit` is an attacker-controllable public query param, so a huge
            // value after a nonzero `start` must saturate rather than overflow.
            Some(limit) => start.saturating_add(limit).min(keys.len()),
        };
        let range = start..end;
        let next_cursor = next_cursor_for(keys, &range);
        WindowSlice { range, next_cursor }
    }
}

/// The forward continuation token for a windowed `range` over display-ordered
/// `keys`: emit only when at least one entry was taken AND at least one remains
/// after it (a full/last page, an empty window, and a cursor past the end all
/// yield `None`). `keys[range.end - 1]` is the display-ordered last entry, so the
/// token continues in the requested order (ascending or descending). This is
/// INV-1's single source for both the CLI window and the inspector query path.
pub(super) fn next_cursor_for(
    keys: &[HistoryCursor],
    range: &std::ops::Range<usize>,
) -> Option<String> {
    (range.end > range.start && range.end < keys.len()).then(|| keys[range.end - 1].encode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventId;

    #[test]
    fn cursor_round_trips() {
        let cursor = HistoryCursor {
            occurred_at: "unix-ms:1782800000000".into(),
            event_id: EventId::new("evt:sha256:abc"),
        };
        let token = cursor.encode();
        assert_eq!(HistoryCursor::decode(&token).unwrap(), cursor);
    }

    #[test]
    fn cursor_encode_is_deterministic() {
        let cursor = HistoryCursor {
            occurred_at: "unix-ms:1".into(),
            event_id: EventId::new("evt:sha256:x"),
        };
        assert_eq!(cursor.encode(), cursor.encode());
    }

    #[test]
    fn cursor_decode_rejects_malformed() {
        assert!(HistoryCursor::decode("not-base64!!").is_err());
        assert!(HistoryCursor::decode("").is_err());
    }

    fn keys(n: usize) -> Vec<HistoryCursor> {
        (0..n)
            .map(|i| HistoryCursor {
                occurred_at: format!("unix-ms:{i}"),
                event_id: EventId::new(format!("evt:sha256:{i}")),
            })
            .collect()
    }

    #[test]
    fn no_window_takes_all_no_cursor() {
        let k = keys(5);
        let window = HistoryWindow {
            limit: None,
            after: None,
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 0..5);
        assert!(slice.next_cursor.is_none());
    }

    #[test]
    fn limit_takes_prefix_and_emits_next_cursor() {
        let k = keys(5);
        let window = HistoryWindow {
            limit: Some(2),
            after: None,
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 0..2);
        assert_eq!(slice.next_cursor, Some(k[1].encode()));
    }

    #[test]
    fn cursor_skips_through_and_past_it() {
        let k = keys(5);
        let window = HistoryWindow {
            limit: Some(2),
            after: Some(k[1].clone()),
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 2..4);
        assert_eq!(slice.next_cursor, Some(k[3].encode()));
    }

    #[test]
    fn last_page_has_no_next_cursor() {
        let k = keys(3);
        let window = HistoryWindow {
            limit: Some(10),
            after: None,
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 0..3);
        assert!(slice.next_cursor.is_none());
    }

    #[test]
    fn cursor_past_end_is_empty() {
        let k = keys(2);
        let window = HistoryWindow {
            limit: Some(5),
            after: Some(k[1].clone()),
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 2..2);
        assert!(slice.next_cursor.is_none());
    }

    #[test]
    fn limit_zero_is_empty_window() {
        let k = keys(3);
        let window = HistoryWindow {
            limit: Some(0),
            after: None,
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 0..0);
        assert!(slice.next_cursor.is_none());
    }

    #[test]
    fn huge_limit_after_cursor_does_not_overflow() {
        let k = keys(3);
        let window = HistoryWindow {
            limit: Some(usize::MAX),
            after: Some(k[1].clone()),
        };
        let slice = window.apply(&k);
        assert_eq!(slice.range, 2..3);
        assert!(slice.next_cursor.is_none());
    }
}
