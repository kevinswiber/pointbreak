// The state-bound model layer: the revision / supersession / object derivations
// over the loaded `/api/*` documents on the store, plus the revisions/threads-lens
// filter predicates. Ported from the served app.js model/supersession cluster.
// State-reading (via `getState()`) but DOM-free — it returns ids, classifications,
// annotation records, and ready-to-insert badge HTML strings; the render/lenses/
// detail layers turn those into the live DOM.
//
// The history timeline query (search / filter / facet counts) moved to the server,
// so the timeline lens paints the server-filtered page window rather than matching
// client-side. The revisions/threads lenses still match over the fully-loaded
// supersession graph, so their predicates (matchesRevisionFilters and friends) use
// the pure `query` grammar here.

import { CLASS } from "./classNames";
import type { Annotation } from "./diff/render";
import { escapeHtml } from "./escape";
import {
  assessmentCue,
  assessmentDisplayLabel,
  attentionCues,
  entryRevisionId,
  type Revision,
  revisionSearchIndex,
} from "./projection";
import { matchesQuery, parseSearchQuery } from "./query";
import { linkify, shortId, targetDisplayLabel } from "./refs";
import { getState } from "./store";
import {
  type HistoryEntry,
  type Overview,
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
  /** The fact relation this edge encodes (`replaces`/`supersedes`); absent on revision edges. */
  kind?: string;
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

// ---------------------------------------------------------------------------
// Type distribution
// ---------------------------------------------------------------------------

/** The event types present in the history, known ones first (canonical order). */
export function presentTypes(): string[] {
  const history = getState().history;
  // The server facets enumerate every matching type in the store (even those on
  // an unloaded page), so they are the authority for which toggles to show; the
  // loaded entries are the fallback before the first facets arrive.
  const keys = history?.facets ? Object.keys(history.facets) : [];
  const present = new Set(
    keys.length ? keys : (history?.entries ?? []).map((e) => e.eventType),
  );
  const ordered = TYPES.map((t) => t.id).filter((id) => present.has(id));
  for (const id of present) if (!TYPE_MAP[id]) ordered.push(id);
  return ordered;
}

// ---------------------------------------------------------------------------
// Threads + supersession classification (from /api/threads)
// ---------------------------------------------------------------------------

/** The laid-out supersession threads, or []. */
export function currentThreads(): Thread[] {
  return (getState().threads?.threads ?? []) as Thread[];
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
  const map = getState().threads?.revisionClassification;
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

/** The snapshot id a revision captured, or "". */
export function snapshotIdForRevision(revisionId: string): string {
  return revisionForId(revisionId)?.snapshotId ?? "";
}

/** The captured snapshot content hash for a revision, or "". */
export function snapshotContentHashForRevision(revisionId: string): string {
  return revisionForId(revisionId)?.snapshotContentHash ?? "";
}

/** The revision that captured a snapshot, disambiguated by content hash when given. */
export function revisionIdForSnapshot(
  snapshotId: string,
  contentHash: string | null = null,
): string | null {
  const entries = getState().revisions?.entries ?? [];
  const revision =
    entries.find(
      (r) =>
        r.snapshotId === snapshotId &&
        (!contentHash || r.snapshotContentHash === contentHash),
    ) ?? entries.find((r) => r.snapshotId === snapshotId);
  return revision ? (revision.revisionId ?? null) : null;
}

/** The server review overview for a revision, or null. */
export function overviewForRevision(revisionId: string): Overview | null {
  return revisionForId(revisionId)?.overview ?? null;
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
// Revisions/threads-lens filter predicates
//
// The timeline history query is server-owned now (search / filter / facets are
// applied to `/api/history`), so there is no client history predicate. The
// revisions and threads lenses still filter client-side over the fully-loaded
// supersession graph, so they parse the active `filterText` here.
// ---------------------------------------------------------------------------

/** Whether a revision passes the object filter and the query clauses. */
export function matchesRevisionFilters(r: Revision): boolean {
  const s = getState();
  if (s.filterSnapshot && r.snapshotId !== s.filterSnapshot) return false;
  return matchesQuery(revisionSearchIndex(r), parseSearchQuery(s.filterText));
}

/** Whether any of a thread's revisions passes the active filters. */
export function threadMatchesRevisionFilters(thread: Thread): boolean {
  const revisions = thread.revisions ?? [];
  const s = getState();
  if (!s.filterText && !s.filterSnapshot) return true;
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
  if (!s.filterText && !s.filterSnapshot) return revisions;
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
    for (const t of currentThreads().filter(threadMatchesRevisionFilters)) {
      for (const r of filteredThreadRevisionIds(t, threadRevisionOrder(t))) {
        ids.push({ kind: "revision", id: r });
      }
    }
    return ids;
  }
  // The server filtered and ordered the timeline page; step the loaded window
  // as-is (paging past its edges is handled by the keyboard stepper).
  return (s.history?.entries ?? []).map(
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
  return currentThreads().some((t) => (t.revisions ?? []).includes(id));
}

/** Whether an event id exists in the loaded history. */
export function eventExists(id: string): boolean {
  return (getState().history?.entries ?? []).some((e) => e.eventId === id);
}
