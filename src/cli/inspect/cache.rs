//! One-slot projection cache for the inspect server (#255).
//!
//! Server-side `q` search needs the full body-hydrated haystack, so it cannot
//! slice-before-hydrate; without a cache every `/api/history` query would re-read,
//! re-fold, and re-hydrate the whole log. This cache amortizes the full build to
//! once per store version: it detects change with the cheap monotonic
//! `event_log_head_marker` (plan 0090, no event-byte decode) and serves the cached
//! `Arc<BaseHistoryProjection>` (whose `eventSetHash` is the confirm stamp) until
//! the marker moves, then drops and rebuilds. Single-slot (one store version),
//! read-side lazy, no store-dir lock (INV-5). This is ADR-0024 D4's
//! detect-vs-confirm model applied WITHOUT building the deferred redb index.

use std::sync::{Arc, RwLock};

use pointbreak::session::BaseHistoryProjection;

/// A single-slot cache of the fully-hydrated base projection, keyed by the store
/// head marker.
pub(super) struct HistoryProjectionCache {
    slot: RwLock<Option<Cached>>,
}

struct Cached {
    marker: u64,
    base: Arc<BaseHistoryProjection>,
}

impl HistoryProjectionCache {
    pub(super) fn new() -> Self {
        Self {
            slot: RwLock::new(None),
        }
    }

    /// Return the cached base when `marker` matches; otherwise run `build`, store
    /// it under `marker`, and return it. A build error caches nothing.
    pub(super) fn get_or_build(
        &self,
        marker: u64,
        build: impl FnOnce() -> Result<BaseHistoryProjection, String>,
    ) -> Result<Arc<BaseHistoryProjection>, String> {
        if let Some(cached) = self.slot.read().unwrap().as_ref()
            && cached.marker == marker
        {
            return Ok(Arc::clone(&cached.base));
        }
        let mut guard = self.slot.write().unwrap();
        // Re-check under the write lock: another thread may have rebuilt between
        // the read-lock miss and acquiring the write lock.
        if let Some(cached) = guard.as_ref()
            && cached.marker == marker
        {
            return Ok(Arc::clone(&cached.base));
        }
        let base = Arc::new(build()?);
        *guard = Some(Cached {
            marker,
            base: Arc::clone(&base),
        });
        Ok(base)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn base_stub(tag: &str) -> BaseHistoryProjection {
        BaseHistoryProjection {
            entries: Vec::new(),
            event_set_hash: tag.to_owned(),
            event_count: 0,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn builds_once_and_reuses_on_unchanged_marker() {
        let cache = HistoryProjectionCache::new();
        let builds = AtomicUsize::new(0);
        let build = || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(base_stub("v1"))
        };

        let a = cache.get_or_build(7, build).unwrap();
        let b = cache.get_or_build(7, build).unwrap();
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "same marker -> built once"
        );
        assert!(Arc::ptr_eq(&a, &b), "same Arc reused");
    }

    #[test]
    fn rebuilds_when_marker_changes() {
        let cache = HistoryProjectionCache::new();
        let builds = AtomicUsize::new(0);

        let v1 = cache
            .get_or_build(7, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(base_stub("v1"))
            })
            .unwrap();
        let v2 = cache
            .get_or_build(8, || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(base_stub("v2"))
            })
            .unwrap();
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "changed marker -> rebuilt"
        );
        assert!(!Arc::ptr_eq(&v1, &v2));
        assert_eq!(v2.event_set_hash, "v2");
    }

    #[test]
    fn build_error_is_not_cached() {
        let cache = HistoryProjectionCache::new();
        assert!(cache.get_or_build(7, || Err("boom".to_owned())).is_err());
        // A subsequent good build at the same marker still runs (the error left no
        // entry behind).
        let ok = cache.get_or_build(7, || Ok(base_stub("v1")));
        assert!(ok.is_ok());
    }
}
