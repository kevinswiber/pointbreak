// The central state container — the full app.js `state` object, typed, behind a
// minimal `getState`/`subscribe`/`commit` API. `commit` applies a patch, restores
// the invariants the served navigate() enforces, then notifies subscribers. It is
// the foundation a `render` subscriber and a `navigate` (commit + history) wrapper
// are layered onto by their owning modules; on its own it has no subscriber and no
// navigate — the container plus its tests stand alone.
//
// Ported from the served app.js `state` object and the navigate() choke point
// (the `if (!state.selected)` / `if (!state.diff)` reconciliation), with no DOM
// access and no behaviour beyond the container contract.

import type { Revision } from "./projection";
import type { HistoryEntry } from "./types";
import { TYPES } from "./types";

// The loaded `/api/*` documents the container holds. These are the fields the
// app reads off each payload; the entry views reuse the shared wire shapes
// (`HistoryEntry`, `Revision`). Sub-structures the renderers narrow at read
// time (history diagnostics, object threads, the per-revision classification map)
// stay `unknown`-typed dynamic JSON rather than re-declaring the wire here.

/** The `/api/history` document: timeline entries plus load-time diagnostics. */
export interface HistoryDoc {
  entries: HistoryEntry[];
  diagnostics: unknown[];
  // The event-set hash the data loader seeds as the freshness baseline (present
  // in the committed fixture; the poller compares the probe's hash against it).
  eventSetHash?: string;
}

/** The `/api/revisions` document: one entry per captured revision. */
export interface RevisionsDoc {
  entries: Revision[];
}

/** The `/api/objects` document: the laid-out threads plus the supersession map. */
export interface ObjectsDoc {
  threads: unknown[];
  revisionClassification: Record<string, unknown>;
}

/**
 * The single selection through-line. The detail pane is a pure projection of
 * this; `kind`/`id` are null when nothing is selected.
 */
export interface Selection {
  kind: "event" | "revision" | null;
  id: string | null;
}

/**
 * The complete review-view state: the loaded data, the route projection
 * (lens/order/diff/focus), the selection, the type/track/object/text filters,
 * and the freshness baselines the poller compares against. Every field the
 * loader commits and the render/model layer reads lives here; the transient view
 * caches that were never part of `state` stay with their owning modules.
 */
export interface State {
  history: HistoryDoc | null;
  revisions: RevisionsDoc | null;
  objects: ObjectsDoc | null;
  // The master-pane projection, serialized into the URL fragment by the router.
  lens: string;
  selected: Selection;
  enabledTypes: Set<string>;
  seenTypes: Set<string>;
  // The structured query string (serialized as q=): free-text terms plus
  // field:value clauses. The track/object filters mirror the active clauses.
  filterText: string;
  filterTrack: string;
  filterObject: string;
  // "desc" = newest first (default), "asc" = chronological.
  order: string;
  // The route-preserving diff overlay: the object id shown, its optional
  // event-bound artifact hash, and the single in-diff fact highlight.
  diff: string | null;
  diffHash: string | null;
  focus: string | null;
  // Freshness baselines the poller diffs against to surface a refresh cue.
  lastHash: string | null;
  lastDiagnosticCount: number | null;
}

// The initial state, ported verbatim from the served app.js `state` object.
const state: State = {
  history: null,
  revisions: null,
  objects: null,
  lens: "timeline",
  selected: { kind: null, id: null },
  enabledTypes: new Set(TYPES.map((t) => t.id)),
  seenTypes: new Set(TYPES.map((t) => t.id)),
  filterText: "",
  filterTrack: "",
  filterObject: "",
  order: "desc",
  diff: null,
  diffHash: null,
  focus: null,
  lastHash: null,
  lastDiagnosticCount: null,
};

const subscribers = new Set<() => void>();

/** The live state object. Reads are a projection of this single source of truth. */
export function getState(): State {
  return state;
}

/**
 * Register a callback fired once per `commit`. Returns an unsubscribe handle that
 * removes the callback.
 */
export function subscribe(fn: () => void): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

/**
 * Apply a partial patch, restore the invariants the served navigate() enforces
 * (an absent selection resets to the empty selection; a closed diff nulls its
 * hash), then notify every subscriber. Subscribers observe the already-applied,
 * already-reconciled state.
 */
export function commit(patch: Partial<State>): void {
  Object.assign(state, patch);
  if (!state.selected) state.selected = { kind: null, id: null };
  if (!state.diff) state.diffHash = null;
  for (const fn of subscribers) fn();
}
