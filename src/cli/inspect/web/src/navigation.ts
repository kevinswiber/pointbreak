// The ref-chip resolution layer: turn a clicked reference chip into a router
// navigation (or a diff open), and reveal an event by fetching the page that
// contains it. Ported from the served app.js `resolveRef` / `revealEvent` /
// `revealBy` / `navigateToUnit` (→ `navigateToRevision`) / `navigateToTrack`, in
// the revision vocabulary.
//
// The history is server-paged now, so an event may be off the loaded window:
// revealing it fetches the page that contains it (`at=<id>`), and a structured id
// (observation / assessment / input-request) is resolved to its event by a server
// search before the reveal. The reveal helpers are async; `resolveRef` stays the
// sync entry point (it fire-and-forgets the async work), with `resolveRefAsync`
// for awaiting callers/tests. Everything routes through `router.navigate` (commit →
// the store subscriber repaints); navigation never calls render. It owns the single
// `document` `click→resolveRef` delegate (`onDocumentClick`, registered once by the
// composition root): chips render across timeline / detail / diff / cards, so it
// must stay one global listener. Per the detail layer's deferral, that same
// delegate also resolves the `data-reveal-revision` "show in timeline" button.

import { fetchEventIdForQuery, fetchRevealPage, revealPatch } from "./data";
import { DIFF_ROUTE_CLEARED, openDiff } from "./diff/controller";
import { navigate } from "./router";

/** Scope the timeline to a single revision via the shareable `revision:<id>` query. */
export function navigateToRevision(id: string): void {
  navigate({
    lens: "timeline",
    filterText: `revision:${id}`,
    filterTrack: "",
    filterSnapshot: "",
  });
}

/** Scope the timeline to a single track, dismissing any open diff. */
export function navigateToTrack(id: string): void {
  navigate({
    lens: "timeline",
    filterTrack: id,
    ...DIFF_ROUTE_CLEARED,
  });
}

// Make an event visible: fetch the page that contains it (`at=<id>`) under the
// reset query so nothing hides it, then select it through the router (URL stays the
// single source of truth). A genuinely absent event leaves the view unchanged.
/** Fetch the page containing an event, reset the filters, and select it. */
export async function revealEvent(eventId: string): Promise<void> {
  const page = await fetchRevealPage(eventId);
  if (!page?.present) return;
  // A chip reveal names a record destination: it exits the diff page too.
  // (`revealPatch` itself stays page-neutral — it also runs in applyHash's
  // deep-link reveal, which must not close a page the hash addressed.)
  navigate({ ...revealPatch(page, eventId), ...DIFF_ROUTE_CLEARED });
}

// Resolve a structured id (observation / assessment / input-request) to its event
// via a server search, then reveal that event.
/** Reveal the event carrying a structured id, resolved server-side. */
async function revealByQuery(id: string): Promise<void> {
  const eventId = await fetchEventIdForQuery(id);
  if (eventId) await revealEvent(eventId);
}

// A reference chip resolves to a navigation through the router (set the selection /
// scope and push a hash), never an in-place filter mutation. Navigating to a named
// reference also dismisses any open diff overlay.
/** Route a clicked reference chip to its resource by kind (fire-and-forget). */
export function resolveRef(kind: string, id: string): void {
  void resolveRefAsync(kind, id);
}

/** Route a clicked reference chip by kind, awaitable for reveal-fetching callers. */
export async function resolveRefAsync(kind: string, id: string): Promise<void> {
  switch (kind) {
    // The revision and the (retired) review-unit prefix both address a revision's
    // composite — their identity is unified onto the revision id.
    case "rev":
    case "review-unit":
      navigate({
        selected: { kind: "revision", id },
        ...DIFF_ROUTE_CLEARED,
      });
      break;
    case "track":
      navigateToTrack(id);
      break;
    case "snap":
      openDiff(id);
      break;
    case "obs":
    case "assess":
    case "input-request":
      await revealByQuery(id);
      break;
    case "evt":
      await revealEvent(id);
      break;
    default:
      break;
  }
}

/**
 * The single `document` click delegate: a clicked reference chip anywhere
 * navigates to the resource it names, and the detail "show in timeline" button
 * (`data-reveal-revision`) scopes the timeline to its revision. Registered once by
 * the composition root, never per render.
 */
export function onDocumentClick(ev: MouseEvent): void {
  const t = ev.target;
  if (!(t instanceof Element)) return;
  const ref = t.closest<HTMLElement>("[data-ref-kind]");
  if (ref) {
    ev.preventDefault();
    resolveRef(ref.dataset.refKind ?? "", ref.dataset.refId ?? "");
    return;
  }
  const reveal = t.closest<HTMLElement>("[data-reveal-revision]");
  if (reveal) {
    const id = reveal.dataset.revealRevision;
    if (id) navigateToRevision(id);
  }
}
