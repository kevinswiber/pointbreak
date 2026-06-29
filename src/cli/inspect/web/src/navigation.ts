// The ref-chip resolution layer: turn a clicked reference chip into a router
// navigation (or a diff open), and reveal an event/revision by clearing the
// filters that would hide it. Ported from the served app.js `resolveRef` /
// `revealEvent` / `revealBy` / `navigateToUnit` (→ `navigateToRevision`) /
// `navigateToTrack`, in the revision vocabulary.
//
// Everything routes through `router.navigate` (commit → the store subscriber
// repaints); navigation never calls render. It owns the single `document`
// `click→resolveRef` delegate (`onDocumentClick`, registered once by the
// composition root): chips render across timeline / detail / diff / cards, so it
// must stay one global listener. Per the detail layer's deferral, that same
// delegate also resolves the `data-reveal-revision` "show in timeline" button.

import { openDiff } from "./diff/controller";
import { navigate } from "./router";
import { getState } from "./store";
import type { HistoryEntry } from "./types";

/** Scope the timeline to a single revision via the shareable `revision:<id>` query. */
export function navigateToRevision(id: string): void {
  navigate({
    lens: "timeline",
    filterText: `revision:${id}`,
    filterTrack: "",
    filterObject: "",
  });
}

/** Scope the timeline to a single track, dismissing any open diff. */
export function navigateToTrack(id: string): void {
  navigate({
    lens: "timeline",
    filterTrack: id,
    diff: null,
    diffHash: null,
    focus: null,
  });
}

// Make an event visible (clearing every filter that could hide it, including the
// track filter — a cross-track chip would otherwise select a hidden row) and select
// it, all through the router so the URL stays the single source of truth.
/** Reveal and select an event on the timeline, clearing hiding filters. */
export function revealEvent(eventId: string): void {
  const e = (getState().history?.entries ?? []).find(
    (x) => x.eventId === eventId,
  );
  if (!e) return;
  const types = new Set(getState().enabledTypes);
  types.add(e.eventType);
  navigate({
    lens: "timeline",
    selected: { kind: "event", id: eventId },
    filterText: "",
    filterTrack: "",
    filterObject: "",
    enabledTypes: types,
    diff: null,
    diffHash: null,
    focus: null,
  });
}

/** Reveal the first event matching a predicate (e.g. the fact that recorded an id). */
export function revealBy(predicate: (e: HistoryEntry) => boolean): void {
  const e = (getState().history?.entries ?? []).find(predicate);
  if (e?.eventId) revealEvent(e.eventId);
}

// A reference chip resolves to a navigation through the router (set the selection /
// scope and push a hash), never an in-place filter mutation. Navigating to a named
// reference also dismisses any open diff overlay.
/** Route a clicked reference chip to its resource by kind. */
export function resolveRef(kind: string, id: string): void {
  switch (kind) {
    // The revision and the (retired) review-unit prefix both address a revision's
    // composite — their identity is unified onto the revision id.
    case "rev":
    case "review-unit":
      navigate({
        selected: { kind: "revision", id },
        diff: null,
        diffHash: null,
        focus: null,
      });
      break;
    case "track":
      navigateToTrack(id);
      break;
    case "snap":
      openDiff(id);
      break;
    case "obs":
      revealBy((e) => e.summary?.observationId === id);
      break;
    case "assess":
      revealBy((e) => e.summary?.assessmentId === id);
      break;
    case "input-request":
      revealBy(
        (e) =>
          e.eventType === "input_request_opened" &&
          e.summary?.inputRequestId === id,
      );
      break;
    case "evt":
      revealEvent(id);
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
