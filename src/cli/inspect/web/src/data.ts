// The data-loading layer: fetch the `/api/*` documents, index each timeline
// entry, and commit the payloads to the store. Ported from the served app.js
// `load` / `pollFreshness` / `showError`.
//
// The cycle cut (data must not depend on render): `load` builds every entry's
// `__search` index and then `commit`s — it never calls a render function. The
// store subscriber repaints in response to the commit, so the data layer has no
// import edge to render. Building the index *before* the commit means a subscriber
// repaint never observes an un-indexed entry.

import { $ } from "./dom";
import { fetchJSON } from "./http";
import { entryRevisionId, entryTrack } from "./projection";
import { buildHaystack } from "./query";
import {
  commit,
  getState,
  type HistoryDoc,
  type ObjectsDoc,
  type RevisionsDoc,
} from "./store";

// The `/api/freshness` probe: the event-log head marker (the event count) the
// poller diffs against the stored baseline. Not store state (only its deltas are
// committed, via a reload).
interface FreshnessDoc {
  eventCount?: number;
}

// The object id a revision captured, resolved against the freshly-fetched
// revisions payload (so the index can be built before anything is committed —
// mirrors model.objectIdForRevision, but reads the payload rather than the store).
function objectIdForRevisionIn(
  revisions: RevisionsDoc,
  revisionId: string,
): string {
  return (
    revisions.entries.find((r) => r.revisionId === revisionId)?.objectId ?? ""
  );
}

// Build the per-entry search index in place: a lowercased haystack plus the small
// structured projection the query grammar matches by field. Done once per load,
// not per keystroke.
function indexEntries(history: HistoryDoc, revisions: RevisionsDoc): void {
  for (const e of history.entries ?? []) {
    const revision = entryRevisionId(e);
    e.__search = {
      text: buildHaystack(e),
      type: e.eventType,
      track: entryTrack(e),
      revision,
      object: objectIdForRevisionIn(revisions, revision),
      status: e.summary?.status ?? "",
    };
  }
}

/**
 * Show `message` in `#error` (prefixed), or hide and clear the banner when given
 * no message.
 */
export function showError(message: string | null): void {
  const el = $("#error");
  if (!el) return;
  if (!message) {
    el.classList.add("hidden");
    el.textContent = "";
    return;
  }
  el.textContent = `error: ${message}`;
  el.classList.remove("hidden");
}

/**
 * Load the three review documents, index every timeline entry, then commit the
 * payloads + freshness baselines to the store in one shot. Never calls render —
 * the store subscriber repaints. A load failure surfaces in `#error` rather than
 * throwing.
 */
export async function load(): Promise<void> {
  try {
    // Take the freshness marker BEFORE the documents, so the baseline can never be
    // newer than what was loaded. If an event lands during the document fetch, the
    // marker the next poll reads is higher than this baseline and triggers a reload
    // (at worst a redundant one) rather than masking the append. Fetched in parallel
    // with the documents, the marker could come back newer than them and the poll
    // would then report "unchanged" forever. Seeding from the probe (not
    // `history.eventCount`) also keeps a retired/skipped event — where the event-file
    // marker exceeds the post-skip history count — from forcing a reload every tick.
    const freshness = (await fetchJSON("/api/freshness")) as FreshnessDoc;
    const [historyRaw, revisionsRaw, objectsRaw] = await Promise.all([
      fetchJSON("/api/history"),
      fetchJSON("/api/revisions"),
      fetchJSON("/api/objects"),
    ]);
    const history = historyRaw as HistoryDoc;
    const revisions = revisionsRaw as RevisionsDoc;
    const objects = objectsRaw as ObjectsDoc;
    // Index before committing so a subscriber repaint never sees an un-indexed
    // entry.
    indexEntries(history, revisions);
    showError(null);
    commit({
      history,
      revisions,
      objects,
      lastEventCount: freshness.eventCount ?? null,
    });
  } catch (err) {
    showError(err instanceof Error ? err.message : String(err));
  }
}

/**
 * Probe `/api/freshness` and reload when the event-log head marker changed,
 * updating the `#refresh` indicator. A probe failure marks it stalled.
 */
export async function pollFreshness(): Promise<void> {
  try {
    const f = (await fetchJSON("/api/freshness")) as FreshnessDoc;
    const refresh = $("#refresh");
    const s = getState();
    const changed = (f.eventCount ?? null) !== s.lastEventCount;
    if (changed) {
      if (refresh) {
        refresh.textContent = "updated";
        refresh.classList.add("live");
      }
      await load();
      setTimeout(() => {
        if (refresh) {
          refresh.textContent = "watching";
          refresh.classList.remove("live");
        }
      }, 1200);
    } else if (refresh) {
      refresh.textContent = "watching";
    }
  } catch {
    const refresh = $("#refresh");
    if (refresh) refresh.textContent = "stalled";
  }
}
