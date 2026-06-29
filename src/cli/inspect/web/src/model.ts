// The state-bound model layer: the revision / supersession / object derivations
// over the loaded `/api/*` documents on the store, plus the state-bound timeline
// filter predicates. Ported from the served app.js model/supersession cluster and
// the per-render filter functions. State-reading (via `getState()`) but DOM-free —
// it returns ids, classifications, annotation records, and ready-to-insert badge
// HTML strings; the render/lenses/detail layers turn those into the live DOM.
//
// The filter predicates (currentClauses/matchesFilters/facetCounts) live here, not
// in the pure `query` grammar, because they need `eventMatchesObject` (a model
// derivation) while `model` already imports `query`'s grammar — housing them in
// `query` would close a cycle. So `query` stays pure and one-directional, and the
// state-bound predicates own a module-local parse cache here.

import { CLASS } from "./classNames";
import type { Annotation } from "./diff/render";
import { escapeHtml } from "./escape";
import {
  assessmentCue,
  assessmentDisplayLabel,
  attentionCues,
  entryRevisionId,
  entryTrack,
  type Revision,
  revisionSearchIndex,
} from "./projection";
import { matchesQuery, parseSearchQuery, type QueryClause } from "./query";
import { linkify, shortId, targetDisplayLabel } from "./refs";
import { getState } from "./store";
import {
  type HistoryEntry,
  type Overview,
  type SearchIndex,
  SUPERSEDABLE_FACT_TYPES,
  TYPE_MAP,
  TYPES,
} from "./types";

// A supersession thread (a connected component of the supersession DAG) as laid
// out server-side. Only the fields the model reads are typed; deeper renderers
// extend this view as they consume more of the payload.
/** One laid-out node of a thread's supersession DAG (geometry + supersession state). */
export interface ThreadNode {
  id?: string;
  x?: number;
  y?: number;
  // Box dimensions and head/superseded state the DAG painter reads.
  w?: number;
  h?: number;
  isHead?: boolean;
  isSuperseded?: boolean;
}

/** The normalized (0,0)-origin bounds of a thread's laid-out graph. */
export interface ThreadBounds {
  w?: number;
  h?: number;
}

/** A routed supersession edge: the superseding `from`, the `to` it supersedes, and its polyline. */
export interface ThreadEdge {
  from?: string;
  to?: string;
  path?: number[][];
}

/** A thread's server-computed layout (the placed supersession nodes, edges, and bounds). */
export interface ThreadLayout {
  nodes?: ThreadNode[];
  edges?: ThreadEdge[];
  bounds?: ThreadBounds;
}

/** A supersession thread: its member revisions, heads, and their laid-out positions. */
export interface Thread {
  revisions?: string[];
  laidOut?: ThreadLayout | null;
  // The thread's current heads, its superseded members, and whether it forks
  // (competing heads), read by the threads-lens card + DAG painter.
  heads?: string[];
  superseded?: string[];
  competing?: boolean;
}

/** The server-computed per-revision supersession classification. */
export interface RevisionClassification {
  state?: string;
  supersededBy?: string[];
  supersedes?: string[];
}

/** A selectable lens entry: an event row or a revision card, with its id. */
export interface LensEntry {
  kind: "event" | "revision";
  id: string;
}

// The fallback search record for an entry that has not been indexed yet, so the
// matcher never has to read off a missing index.
const EMPTY_SEARCH_INDEX: SearchIndex = { text: "", type: "" };

// ---------------------------------------------------------------------------
// Type distribution
// ---------------------------------------------------------------------------

/** The event types present in the history, known ones first (canonical order). */
export function presentTypes(): string[] {
  const present = new Set(
    (getState().history?.entries ?? []).map((e) => e.eventType),
  );
  const ordered = TYPES.map((t) => t.id).filter((id) => present.has(id));
  for (const id of present) if (!TYPE_MAP[id]) ordered.push(id);
  return ordered;
}

// ---------------------------------------------------------------------------
// Threads + supersession classification (from /api/objects)
// ---------------------------------------------------------------------------

/** The laid-out supersession threads, or []. */
export function objectThreads(): Thread[] {
  return (getState().objects?.threads ?? []) as Thread[];
}

/** A thread's revision ids in laid-out order (by node y, then x), missing ones last. */
export function threadRevisionOrder(thread: Thread): string[] {
  const revisions = thread.revisions ?? [];
  const nodes = thread.laidOut?.nodes ?? [];
  if (!nodes.length) return revisions;
  const known = new Set(revisions);
  const ordered = nodes
    .filter(
      (n): n is ThreadNode & { id: string } =>
        typeof n.id === "string" && known.has(n.id),
    )
    .slice()
    .sort((a, b) => (a.y ?? 0) - (b.y ?? 0) || (a.x ?? 0) - (b.x ?? 0))
    .map((n) => n.id);
  if (ordered.length === revisions.length) return ordered;
  const seen = new Set(ordered);
  return ordered.concat(revisions.filter((id) => !seen.has(id)));
}

/** The server supersession classification for a revision, or null. */
export function revisionClassification(
  revisionId: string,
): RevisionClassification | null {
  const map = getState().objects?.revisionClassification;
  const raw: unknown = map ? map[revisionId] : undefined;
  if (raw === null || typeof raw !== "object") return null;
  return raw as RevisionClassification;
}

/** The revisions that directly supersede the given one (fork-tolerant), or []. */
export function supersededByRevision(revisionId: string): string[] {
  return revisionClassification(revisionId)?.supersededBy ?? [];
}

/** The predecessors a revision directly supersedes, or []. */
export function supersedesRevision(revisionId: string): string[] {
  return revisionClassification(revisionId)?.supersedes ?? [];
}

/** Whether a revision is a current head (a live head or a lone isolated root). */
export function revisionIsHead(revisionId: string): boolean {
  const klass = revisionClassification(revisionId)?.state;
  return klass === "head" || klass === "isolated";
}

// ---------------------------------------------------------------------------
// Revision lookups (from /api/revisions)
// ---------------------------------------------------------------------------

/** The revision entry with the given id, or null. */
export function revisionForId(revisionId: string): Revision | null {
  return (
    (getState().revisions?.entries ?? []).find(
      (r) => r.revisionId === revisionId,
    ) ?? null
  );
}

/** The content object id a revision captured, or "". */
export function objectIdForRevision(revisionId: string): string {
  return revisionForId(revisionId)?.objectId ?? "";
}

/** The captured object artifact content hash for a revision, or "". */
export function objectArtifactHashForRevision(revisionId: string): string {
  return revisionForId(revisionId)?.objectArtifactContentHash ?? "";
}

/** The snapshot (object) id captured for a revision, or null. */
export function snapshotIdForRevision(revisionId: string): string | null {
  const revision = revisionForId(revisionId);
  return revision ? (revision.objectId ?? null) : null;
}

/** The revision that captured an object, disambiguated by content hash when given. */
export function revisionIdForObject(
  objectId: string,
  contentHash: string | null = null,
): string | null {
  const entries = getState().revisions?.entries ?? [];
  const revision =
    entries.find(
      (r) =>
        r.objectId === objectId &&
        (!contentHash || r.objectArtifactContentHash === contentHash),
    ) ?? entries.find((r) => r.objectId === objectId);
  return revision ? (revision.revisionId ?? null) : null;
}

/** The server review overview for a revision, or null. */
export function overviewForRevision(revisionId: string): Overview | null {
  return revisionForId(revisionId)?.overview ?? null;
}

/** Whether an event addresses the revision that captured a given object. */
export function eventMatchesObject(e: HistoryEntry, objectId: string): boolean {
  if (!objectId) return true;
  return objectIdForRevision(entryRevisionId(e)) === objectId;
}

// ---------------------------------------------------------------------------
// Supersession badges (ready-to-insert HTML; reader-relative, advisory)
// ---------------------------------------------------------------------------

/** Whether an event records a review fact whose currency tracks its revision. */
export function isSupersedableFact(e: HistoryEntry): boolean {
  return SUPERSEDABLE_FACT_TYPES.has(e.eventType);
}

/** A "superseded by <successors>" badge for a fact on a superseded revision, or "". */
export function supersessionStaleBadge(e: HistoryEntry): string {
  if (!isSupersedableFact(e)) return "";
  const successors = supersededByRevision(entryRevisionId(e));
  if (!successors.length) return "";
  return `<span class="${CLASS.badge} ${CLASS.stale}">superseded by ${successors.map(linkify).join(" ")}</span>`;
}

/** A "supersedes <predecessors>" badge for a capture event, or "". */
export function captureSupersedesBadge(e: HistoryEntry): string {
  if (e.eventType !== "work_object_proposed") return "";
  const predecessors = supersedesRevision(entryRevisionId(e));
  if (!predecessors.length) return "";
  return `<span class="${CLASS.badge} ${CLASS.supersedes}">supersedes ${predecessors.map(linkify).join(" ")}</span>`;
}

/** The per-revision supersession status badge for a card or page, or "". */
export function supersessionBadge(revisionId: string): string {
  if (!revisionId) return "";
  if (revisionIsHead(revisionId))
    return `<span class="${CLASS.badge} ${CLASS.head}">current in thread</span>`;
  const successors = supersededByRevision(revisionId);
  if (successors.length)
    return `<span class="${CLASS.badge} ${CLASS.superseded}">superseded by ${successors.map(linkify).join(" ")}</span>`;
  return "";
}

// ---------------------------------------------------------------------------
// Review annotations gathered onto a revision
// ---------------------------------------------------------------------------

/** The observation / input-request / assessment facts on a revision, as annotations. */
export function annotationsForRevision(revisionId: string): Annotation[] {
  const out: Annotation[] = [];
  for (const e of getState().history?.entries ?? []) {
    if (entryRevisionId(e) !== revisionId) continue;
    const s = e.summary ?? {};
    if (e.eventType === "review_observation_recorded") {
      out.push({
        kind: "observation",
        id: s.observationId ?? e.eventId ?? "",
        title: s.title ?? "(observation)",
        body: s.body ?? "",
        bodyContentType: s.bodyContentType,
        track: e.trackId ?? "",
        tags: Array.isArray(s.tags) ? s.tags : [],
        target: s.target ?? {},
      });
    } else if (e.eventType === "input_request_opened") {
      const meta = [s.mode, s.reasonCode].filter(Boolean).join(" · ");
      out.push({
        kind: "input-request",
        id: s.inputRequestId ?? e.eventId ?? "",
        title: s.title ?? "(input request)",
        body: s.body ?? "",
        bodyContentType: s.bodyContentType,
        track: e.trackId ?? "",
        tags: meta ? [meta] : [],
        target: s.target ?? {},
      });
    } else if (e.eventType === "review_assessment_recorded") {
      const label = assessmentDisplayLabel(s.assessment ?? "");
      out.push({
        kind: "assessment",
        id: s.assessmentId ?? e.eventId ?? "",
        title: `assessment: ${label || "?"}`,
        body: s.summary ?? "",
        bodyContentType: s.summaryContentType,
        track: e.trackId ?? "",
        tags: [],
        target: s.target ?? {},
      });
    }
  }
  return out;
}

/** The compact thread-card overview (target, id, assessment, cues) for a revision, or "". */
export function renderThreadRevisionOverview(revisionId: string): string {
  const revision = revisionForId(revisionId);
  const overview = overviewForRevision(revisionId);
  if (!revision || !overview) return "";
  return `<div class="${CLASS.threadOverview}">
    <div><b>${targetDisplayLabel(revision.targetDisplay)}</b> <span>${escapeHtml(shortId(revisionId))}</span></div>
    ${assessmentCue(overview)}
    <div class="${CLASS.overviewCues}" aria-label="review cues"><span class="${CLASS.overviewLabel}">review cues</span>${attentionCues(overview)}</div>
  </div>`;
}

// ---------------------------------------------------------------------------
// State-bound filter predicates
// ---------------------------------------------------------------------------

// Parse the query once per filter string and memoize, since matchesFilters runs
// per timeline entry. This parse cache is a transient view-cache, not store state.
let queryCache: { raw: string | null; clauses: QueryClause[] } = {
  raw: null,
  clauses: [],
};

/** The parsed query clauses for the current filter text, memoized on the raw string. */
export function currentClauses(): QueryClause[] {
  const filterText = getState().filterText;
  if (queryCache.raw !== filterText) {
    queryCache = { raw: filterText, clauses: parseSearchQuery(filterText) };
  }
  return queryCache.clauses;
}

/** Whether a timeline event passes the type / track / object / query filters. */
export function matchesFilters(e: HistoryEntry): boolean {
  const s = getState();
  if (!s.enabledTypes.has(e.eventType)) return false;
  if (s.filterTrack && entryTrack(e) !== s.filterTrack) return false;
  if (s.filterObject && !eventMatchesObject(e, s.filterObject)) return false;
  return matchesQuery(e.__search ?? EMPTY_SEARCH_INDEX, currentClauses());
}

/** Per-type event counts under every filter except the type toggles. */
export function facetCounts(): Record<string, number> {
  const s = getState();
  const counts: Record<string, number> = {};
  const clauses = currentClauses();
  for (const e of s.history?.entries ?? []) {
    if (s.filterTrack && entryTrack(e) !== s.filterTrack) continue;
    if (s.filterObject && !eventMatchesObject(e, s.filterObject)) continue;
    if (!matchesQuery(e.__search ?? EMPTY_SEARCH_INDEX, clauses)) continue;
    counts[e.eventType] = (counts[e.eventType] ?? 0) + 1;
  }
  return counts;
}

/** Whether a revision passes the object filter and the query clauses. */
export function matchesRevisionFilters(r: Revision): boolean {
  const s = getState();
  if (s.filterObject && r.objectId !== s.filterObject) return false;
  return matchesQuery(revisionSearchIndex(r), currentClauses());
}

/** Whether any of a thread's revisions passes the active filters. */
export function threadMatchesRevisionFilters(thread: Thread): boolean {
  const revisions = thread.revisions ?? [];
  const s = getState();
  if (!s.filterText && !s.filterObject) return true;
  return revisions
    .map(revisionForId)
    .filter((r): r is Revision => r !== null)
    .some(matchesRevisionFilters);
}

/** A thread's revision ids that pass the active filters, in the given order. */
export function filteredThreadRevisionIds(
  thread: Thread,
  revisions: string[] = thread.revisions ?? [],
): string[] {
  const s = getState();
  if (!s.filterText && !s.filterObject) return revisions;
  return revisions.filter((revisionId) => {
    const revision = revisionForId(revisionId);
    return revision ? matchesRevisionFilters(revision) : false;
  });
}

// ---------------------------------------------------------------------------
// Lens cursor + existence predicates
// ---------------------------------------------------------------------------

/** The selectable entries of the active lens, in display order, for cursor stepping. */
export function lensEntryIds(): LensEntry[] {
  const s = getState();
  if (s.lens === "list") {
    return (s.revisions?.entries ?? [])
      .filter(matchesRevisionFilters)
      .map((r): LensEntry => ({ kind: "revision", id: r.revisionId ?? "" }));
  }
  if (s.lens === "threads") {
    const ids: LensEntry[] = [];
    for (const t of objectThreads().filter(threadMatchesRevisionFilters)) {
      for (const r of filteredThreadRevisionIds(t, threadRevisionOrder(t))) {
        ids.push({ kind: "revision", id: r });
      }
    }
    return ids;
  }
  let entries = (s.history?.entries ?? []).filter(matchesFilters);
  if (s.order === "desc") entries = entries.slice().reverse();
  return entries.map(
    (e): LensEntry => ({ kind: "event", id: e.eventId ?? "" }),
  );
}

/** The selected id when the single selection is an event, else null. */
export function selectedEventId(): string | null {
  const selected = getState().selected;
  return selected && selected.kind === "event" ? selected.id : null;
}

/** Whether a revision id exists in the loaded revisions list. */
export function revisionExists(id: string): boolean {
  return (getState().revisions?.entries ?? []).some((r) => r.revisionId === id);
}

/** Whether a revision id appears in any laid-out thread. */
export function revisionInAnyThread(id: string): boolean {
  return objectThreads().some((t) => (t.revisions ?? []).includes(id));
}

/** Whether an event id exists in the loaded history. */
export function eventExists(id: string): boolean {
  return (getState().history?.entries ?? []).some((e) => e.eventId === id);
}
