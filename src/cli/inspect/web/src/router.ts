// The URL-fragment router: the hash grammar plus the navigate / apply choke point.
//
// `location.hash` is the single serialization of {lens|entity, selection, filters,
// diff overlay}. The fragment never reaches the GET-only server, so routing is
// entirely client-side. Theme and density are reader-local localStorage prefs and
// are deliberately NOT part of the fragment (not shareable view state).
//
//   #/<lens>                   lens-primary (lens ∈ timeline | list | threads)
//   #/<lens>?sel=<id>          a selection within the lens
//   #/revision/<revisionId>    entity-primary: the named revision is selected
//   #/event/<eventId>          entity-primary: the named event is selected
//   ?lens=<lens>               the master lens behind an entity-primary path
//   ?track= ?object=           cross-lens scope (survive a lens switch)
//   ?order= ?types= ?q=        per-lens timeline controls
//   ?diff=<snapshotId> ?focus=<factId> ?diffHash=<snapshotContentHash>
//                              the route-preserving diff overlay
//   ?v=1                       grammar version (reserved; parsed-but-ignored)
//   ?journal= ?asof=           reserved: reported as unsupported live-state input
//
// Ported from the served app.js router cluster. `parseHash`/`serializeState` are a
// pure round-trip — `parseHash` takes the present types, `serializeState` takes a
// state snapshot — so the grammar is unit-testable without a store. `navigate`/
// `applyHash` mutate through `store.commit` and `history` and never call render:
// the store subscriber is the only repaint path, so importing render here would be
// the router↔render cycle. This module imports neither render nor any lens.

import { fetchRevealPage, revealPatch } from "./data";
import { $ } from "./dom";
import {
  eventExists,
  presentTypes,
  revisionExists,
  revisionInAnyThread,
} from "./model";
import { refInfo, shortRef } from "./refs";
import { commit, getState, type Selection, type State } from "./store";

const LENSES = ["timeline", "list", "threads"];
const DEFAULT_LENS = "timeline";

/**
 * A parsed fragment: a full state patch plus the transient route seam. Absent
 * params resolve to their defaults so the fragment fully determines the
 * filter/selection state (Back/Forward to a barer fragment clears what it omits).
 * The seam fields (`unsupported*`/`unknownPath`) are read by `resolve`/
 * `liveStateDiagnostic` and are never committed — `resolve` returns a clean
 * `Partial<State>` that omits them.
 */
export interface RoutePatch {
  lens: string;
  selected: Selection;
  filterTrack: string;
  filterSnapshot: string;
  order: string;
  filterText: string;
  enabledTypes: Set<string>;
  diff: string | null;
  diffHash: string | null;
  focus: string | null;
  // Reserved forward-compat seam: a reserved param surfaces a live-state notice.
  unsupportedAsOf: string | boolean | null;
  unsupportedJournal: string | boolean | null;
  // Set only when the path is unrecognized; resolve() surfaces a visible fallback.
  unknownPath: string | null;
}

/**
 * The state slice `serializeState` reads. `State` satisfies it structurally, so the
 * router passes `getState()`; tests pass a literal. Narrowing to this slice keeps
 * serialization a pure function of the route fields (it cannot read history/objects).
 */
export interface SerializeSnapshot {
  lens: string;
  selected: Selection;
  filterTrack: string;
  filterSnapshot: string;
  order: string;
  enabledTypes: Set<string>;
  filterText: string;
  diff: string | null;
  diffHash: string | null;
  focus: string | null;
}

/** Options for {@link navigate}: `replace` swaps the history entry for a refinement. */
export interface NavigateOptions {
  replace?: boolean;
}

// Classify a selection id as a revision or an event. A `rev:` id is a revision; the
// legacy `review-unit:` prefix is preserved as a revision too. Anything else is an
// event selection.
export function selectionKind(id: string): "event" | "revision" {
  const info = refInfo(id);
  if (info && (info.kind === "rev" || info.kind === "review-unit"))
    return "revision";
  return "event";
}

/** Decode an `&`-separated `key=value` query string; a bare key maps to "". */
export function parseQuery(
  queryString: string,
): Record<string, string | undefined> {
  const params: Record<string, string | undefined> = {};
  for (const pair of queryString.split("&")) {
    if (!pair) continue;
    const eq = pair.indexOf("=");
    const key = decodeURIComponent(eq < 0 ? pair : pair.slice(0, eq));
    params[key] = eq < 0 ? "" : decodeURIComponent(pair.slice(eq + 1));
  }
  return params;
}

/**
 * Parse a fragment into a complete route patch. `presentTypes` seeds the default
 * `enabledTypes` (all present) when no `types=` is given, so the function stays pure.
 */
export function parseHash(
  hash: string,
  presentTypes: readonly string[],
): RoutePatch {
  const raw = hash.replace(/^#/, "");
  const q = raw.indexOf("?");
  const path = q < 0 ? raw : raw.slice(0, q);
  const p = parseQuery(q < 0 ? "" : raw.slice(q + 1));

  const patch: RoutePatch = {
    lens: DEFAULT_LENS,
    selected: { kind: null, id: null },
    filterTrack: p.track != null ? p.track : "",
    // The URL param stays `object` (shared history grammar); client state is
    // snapshot-named.
    filterSnapshot: p.object != null ? p.object : "",
    order: p.order === "asc" || p.order === "desc" ? p.order : "desc",
    filterText: p.q != null ? p.q : "",
    enabledTypes:
      p.types != null
        ? new Set(p.types.split(",").filter(Boolean))
        : new Set(presentTypes),
    diff: p.diff || null,
    diffHash: p.diffHash || null,
    focus: p.focus ? p.focus : null,
    unsupportedAsOf: p.asof != null ? p.asof || true : null,
    unsupportedJournal: p.journal != null ? p.journal || true : null,
    unknownPath: null,
  };

  const segs = path.split("/").filter(Boolean); // "/timeline" -> ["timeline"]
  const lensParam = p.lens ?? "";
  if (segs.length === 0) {
    patch.lens = DEFAULT_LENS;
  } else if (segs[0] === "revision" && segs[1]) {
    patch.selected = { kind: "revision", id: decodeURIComponent(segs[1]) };
    patch.lens = LENSES.includes(lensParam) ? lensParam : DEFAULT_LENS;
  } else if (segs[0] === "event" && segs[1]) {
    patch.selected = { kind: "event", id: decodeURIComponent(segs[1]) };
    patch.lens = LENSES.includes(lensParam) ? lensParam : DEFAULT_LENS;
  } else if (LENSES.includes(segs[0])) {
    patch.lens = segs[0];
    if (p.sel) patch.selected = { kind: selectionKind(p.sel), id: p.sel };
  } else {
    patch.lens = DEFAULT_LENS;
    patch.unknownPath = path; // resolve() surfaces a visible fallback diagnostic
  }
  return patch;
}

/**
 * Serialize a state snapshot into a fragment, omitting defaults to keep the URL
 * short. A selection is entity-primary (durable identity); the lens-primary `sel=`
 * form is the inverse of the parser's `?sel=` handling. `presentTypes` decides
 * whether a `types=` param is needed (only when a present type is disabled).
 */
export function serializeState(
  snapshot: SerializeSnapshot,
  presentTypes: readonly string[],
): string {
  const params: string[] = [];
  const sel = snapshot.selected ?? { kind: null, id: null };
  let path =
    snapshot.lens === DEFAULT_LENS ? "#/timeline" : `#/${snapshot.lens}`;
  if (sel.id && (sel.kind === "revision" || sel.kind === "event")) {
    path =
      sel.kind === "revision"
        ? `#/revision/${encodeURIComponent(sel.id)}`
        : `#/event/${encodeURIComponent(sel.id)}`;
    if (snapshot.lens && snapshot.lens !== DEFAULT_LENS)
      params.push(`lens=${encodeURIComponent(snapshot.lens)}`);
  } else if (sel.id) {
    params.push(`sel=${encodeURIComponent(sel.id)}`);
  }
  if (snapshot.filterTrack)
    params.push(`track=${encodeURIComponent(snapshot.filterTrack)}`);
  // The URL param stays `object` (shared history grammar); client state is
  // snapshot-named.
  if (snapshot.filterSnapshot)
    params.push(`object=${encodeURIComponent(snapshot.filterSnapshot)}`);
  if (snapshot.order && snapshot.order !== "desc")
    params.push(`order=${encodeURIComponent(snapshot.order)}`);
  if (presentTypes.some((id) => !snapshot.enabledTypes.has(id))) {
    params.push(
      `types=${encodeURIComponent(
        presentTypes.filter((id) => snapshot.enabledTypes.has(id)).join(","),
      )}`,
    );
  }
  if (snapshot.filterText)
    params.push(`q=${encodeURIComponent(snapshot.filterText)}`);
  if (snapshot.diff) params.push(`diff=${encodeURIComponent(snapshot.diff)}`);
  if (snapshot.diff && snapshot.diffHash)
    params.push(`diffHash=${encodeURIComponent(snapshot.diffHash)}`);
  if (snapshot.focus)
    params.push(`focus=${encodeURIComponent(snapshot.focus)}`);
  return params.length ? `${path}?${params.join("&")}` : path;
}

/**
 * The single mutation + history choke point. Commits the patch to the store (the
 * subscriber repaints), then pushes (or replaces, for a refinement) the serialized
 * state onto history. It never calls render — that is the router↔render cycle cut.
 */
export function navigate(
  patch: Partial<State>,
  opts: NavigateOptions = {},
): void {
  commit(patch);
  const hash = serializeState(getState(), presentTypes());
  if (opts.replace) history.replaceState({}, "", hash);
  else history.pushState({}, "", hash);
}

/**
 * Derive the whole view from the current fragment and commit it — the store
 * subscriber repaints. Called on boot and from the popstate / hashchange listeners
 * (Back/Forward + manual edits), which the composition root wires. An event
 * selection that is not in the loaded window is fetched-to-reveal asynchronously
 * (the history is server-paged, so the event may simply be off the loaded page).
 */
export function applyHash(): void {
  const patch = resolve(parseHash(location.hash, presentTypes()));
  commit(patch);
  const sel = getState().selected;
  if (sel.kind === "event" && sel.id && !eventExists(sel.id)) {
    void revealSelectedEvent(sel.id, patch.lens ?? DEFAULT_LENS);
  }
}

// Fetch-to-reveal an event a deep link named that is not in the loaded window:
// fetch the page containing it and commit the located window, or fall back with the
// existing "not in this store" diagnostic when it is genuinely absent from the set.
async function revealSelectedEvent(
  eventId: string,
  lens: string,
): Promise<void> {
  const page = await fetchRevealPage(eventId);
  if (!page) return;
  if (page.present) {
    commit(revealPatch(page, eventId));
    clearRouteDiagnostic();
    return;
  }
  commit({ selected: { kind: null, id: null } });
  showRouteDiagnostic(
    `fell back to the ${lens} lens — event ${shortRef(eventId)} is not in this store`,
  );
}

/**
 * Resolve a parsed patch against the loaded data, falling back up the hierarchy
 * (revision → its thread → the lens → timeline) with a visible diagnostic when a
 * deep link names an absent entity — never a 404, never a blank view. Returns a
 * clean `Partial<State>` that omits the transient route seam (the "cleaning" the
 * served code did by `delete`-ing fields off the patch).
 */
export function resolve(patch: RoutePatch): Partial<State> {
  const freshnessDiagnostic = liveStateDiagnostic(patch);
  const next = statePatchFrom(patch);
  if (patch.unknownPath != null) {
    showRouteDiagnostic(
      routeDiagnostic(
        `fell back to the timeline — unknown route ${patch.unknownPath}`,
        freshnessDiagnostic,
      ),
    );
    next.lens = DEFAULT_LENS;
    next.selected = { kind: null, id: null };
    return next;
  }
  const sel = patch.selected ?? { kind: null, id: null };
  if (sel.kind === "revision" && sel.id && !revisionExists(sel.id)) {
    if (revisionInAnyThread(sel.id)) {
      showRouteDiagnostic(
        routeDiagnostic(
          `fell back to the threads lens — revision ${shortRef(sel.id)} is not directly selectable`,
          freshnessDiagnostic,
        ),
      );
      next.lens = "threads";
    } else {
      // Keep the requested lens (only the selection was absent); name it in the
      // diagnostic so the message matches the lens actually shown.
      const lens = patch.lens || DEFAULT_LENS;
      showRouteDiagnostic(
        routeDiagnostic(
          `fell back to the ${lens} lens — revision ${shortRef(sel.id)} is not in this store`,
          freshnessDiagnostic,
        ),
      );
      next.lens = lens;
    }
    next.selected = { kind: null, id: null };
    return next;
  }
  // An event selection is not resolved against the loaded window here (the history
  // is server-paged, so the event may be off the loaded page). `applyHash`
  // fetches-to-reveal it and applies the "not in this store" fallback only when the
  // server confirms it is genuinely absent.
  if (freshnessDiagnostic) {
    showRouteDiagnostic(freshnessDiagnostic);
    return next;
  }
  clearRouteDiagnostic();
  return next;
}

/** The committable State fields of a route patch — the transient seam is dropped here. */
function statePatchFrom(patch: RoutePatch): Partial<State> {
  return {
    lens: patch.lens,
    selected: patch.selected,
    filterTrack: patch.filterTrack,
    filterSnapshot: patch.filterSnapshot,
    order: patch.order,
    filterText: patch.filterText,
    enabledTypes: patch.enabledTypes,
    diff: patch.diff,
    diffHash: patch.diffHash,
    focus: patch.focus,
  };
}

/**
 * The live-state notice for reserved (unsupported) links. Pure: it reads the seam
 * fields and returns the message; the patch is cleaned by {@link statePatchFrom}
 * never copying them, not by deleting them here.
 */
export function liveStateDiagnostic(patch: RoutePatch): string {
  const unsupported: string[] = [];
  if (patch.unsupportedAsOf != null)
    unsupported.push("as-of links are not supported by this server");
  if (patch.unsupportedJournal != null)
    unsupported.push("journal links are not supported by this server");
  return unsupported.length
    ? `showing live state — ${unsupported.join("; ")}`
    : "";
}

/** Join a primary diagnostic with an optional secondary clause. */
export function routeDiagnostic(primary: string, secondary: string): string {
  return secondary ? `${primary} — ${secondary}` : primary;
}

/** Show a route diagnostic in the live region and reveal it. */
export function showRouteDiagnostic(message: string): void {
  const el = $("#route-diagnostic");
  if (!el) return;
  el.textContent = message;
  el.classList.remove("hidden");
}

/** Clear the route diagnostic and re-hide the live region. */
export function clearRouteDiagnostic(): void {
  const el = $("#route-diagnostic");
  if (!el) return;
  el.textContent = "";
  el.classList.add("hidden");
}
