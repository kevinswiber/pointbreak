// The global keydown layer: selection stepping, activation, search focus, two-key
// chords, the layered Escape, and the diff-local jump keys. Ported from the served
// app.js keyboard cluster (`onKey` / `handleEscape` / `stepSelection` /
// `activateSelection` / `focusSearch` / `setChord` / `isTypingTarget` +
// `pendingChord` / `chordTimer`).
//
// `keyboard` is top-of-graph — nothing imports it; the composition root wires
// `onKey` to `document.keydown`. Every state change routes through `router.navigate`
// (commit → the store subscriber repaints); it never calls render. Overlay handling
// goes through the manager: `handleEscape` closes whichever overlay is active
// (palette / diff / help are mutually exclusive), and the help toggle opens/closes
// `help` through the manager — so keyboard imports no sibling overlay module.
// `pendingChord` / `chordTimer` stay module-local.

import { jumpChange, jumpFact, openRevisionDiff } from "./diff/controller";
import { $ } from "./dom";
import { lensEntryIds } from "./model";
import { resolveRef } from "./navigation";
import {
  activeName,
  closeActive,
  open as openOverlay,
  trapFocus,
} from "./overlay";
import { toggle as togglePalette } from "./palette";
import { entryRevisionId } from "./projection";
import { navigate } from "./router";
import { getState } from "./store";

// A short-lived two-key chord (g-then-…), cleared after ~1s. Transient view-cache,
// never on the store.
let pendingChord: string | null = null;
let chordTimer: ReturnType<typeof setTimeout> | null = null;

function setChord(keyName: string): void {
  pendingChord = keyName;
  if (chordTimer) clearTimeout(chordTimer);
  chordTimer = setTimeout(() => {
    pendingChord = null;
  }, 1000);
}

/** Whether the element is a text-input context that should swallow shortcuts. */
function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    (el instanceof HTMLElement && el.isContentEditable)
  );
}

// Move the selection by delta within the active lens (replaceState — stepping a
// cursor is a refinement, not a distinct navigation).
function stepSelection(delta: number): void {
  const ids = lensEntryIds();
  if (!ids.length) return;
  let idx = ids.findIndex((x) => x.id === getState().selected.id);
  if (idx < 0) idx = delta > 0 ? -1 : 0;
  const next = Math.max(0, Math.min(ids.length - 1, idx + delta));
  navigate({ selected: ids[next] }, { replace: true });
}

// Open the selection's snapshot diff — a read affordance, never a gate.
function activateSelection(): void {
  const sel = getState().selected;
  if (sel.kind === "revision" && sel.id) {
    openRevisionDiff(sel.id);
  } else if (sel.kind === "event" && sel.id) {
    const event = (getState().history?.entries ?? []).find(
      (e) => e.eventId === sel.id,
    );
    const rev = event ? entryRevisionId(event) : "";
    if (rev) openRevisionDiff(rev);
  }
}

function focusSearch(): void {
  if (getState().lens !== "timeline") navigate({ lens: "timeline" });
  $<HTMLInputElement>("#filter-text")?.focus();
}

// Toggle the keyboard cheat sheet through the overlay manager (opening it tears
// down any other active overlay via mutual exclusion).
function toggleHelp(): void {
  if (activeName() === "help") closeActive();
  else openOverlay("help", "#key-help-close");
}

// Layered Escape: close the active overlay (diff / palette / help — mutually
// exclusive), then blur a field, then clear the query — one precedence chain.
function handleEscape(): void {
  if (activeName()) {
    closeActive();
    return;
  }
  const active = document.activeElement;
  if (isTypingTarget(active)) {
    if (active instanceof HTMLElement) active.blur();
    return;
  }
  if (getState().filterText) navigate({ filterText: "" }, { replace: true });
}

/** The single `document` keydown handler (wired once by the composition root). */
export function onKey(ev: KeyboardEvent): void {
  if (trapFocus(ev)) return;
  // A focused reference chip activates on Enter/Space (it carries role=link +
  // tabindex=0 but had no key handler), resolving the reference like a click.
  const target = ev.target;
  const chip =
    target instanceof Element
      ? target.closest<HTMLElement>("[data-ref-kind]")
      : null;
  if (chip && (ev.key === "Enter" || ev.key === " ")) {
    ev.preventDefault();
    resolveRef(chip.dataset.refKind ?? "", chip.dataset.refId ?? "");
    return;
  }
  // The command palette opens from anywhere, including a focused field.
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "k") {
    ev.preventDefault();
    togglePalette();
    return;
  }
  if (ev.ctrlKey && ev.shiftKey && ev.key.toLowerCase() === "p") {
    ev.preventDefault();
    togglePalette();
    return;
  }
  // Escape is global (it fires even while typing); everything else yields to a
  // focused text field.
  if (ev.key === "Escape") {
    handleEscape();
    return;
  }
  if (isTypingTarget(document.activeElement)) return;

  // Diff-local jumps, active only while the overlay is open: ]/[ step changes,
  // n/p step review facts.
  if (getState().diff) {
    if (ev.key === "]") {
      ev.preventDefault();
      jumpChange(1);
      return;
    }
    if (ev.key === "[") {
      ev.preventDefault();
      jumpChange(-1);
      return;
    }
    if (ev.key === "n") {
      ev.preventDefault();
      jumpFact(1);
      return;
    }
    if (ev.key === "p") {
      ev.preventDefault();
      jumpFact(-1);
      return;
    }
  }

  if (pendingChord === "g") {
    pendingChord = null;
    if (ev.key === "t") {
      navigate({ lens: "timeline" });
      return;
    }
    if (ev.key === "l") {
      navigate({ lens: "list" });
      return;
    }
    if (ev.key === "r") {
      navigate({ lens: "threads" });
      return;
    }
  }

  switch (ev.key) {
    case "g":
      setChord("g");
      return;
    case "/":
      ev.preventDefault();
      focusSearch();
      return;
    case "j":
    case "ArrowDown":
      ev.preventDefault();
      stepSelection(1);
      return;
    case "k":
    case "ArrowUp":
      ev.preventDefault();
      stepSelection(-1);
      return;
    case "Enter":
      activateSelection();
      return;
    case "?":
      ev.preventDefault();
      toggleHelp();
      return;
    default:
      return;
  }
}
