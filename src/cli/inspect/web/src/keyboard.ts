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

import { fetchHistoryPage, HISTORY_PAGE } from "./data";
import { jumpChange, jumpFact, openRevisionDiff } from "./diff/controller";
import { $ } from "./dom";
import { loadedWindow } from "./lenses/timeline";
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
import { commit, getState } from "./store";

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
// cursor is a refinement, not a distinct navigation). Fire-and-forget the async
// timeline stepper so the key handler stays sync.
function stepSelection(delta: number): void {
  void stepSelectionAsync(delta);
}

// Step the fully-loaded revisions/threads lenses over their in-memory entries.
function stepList(delta: number): void {
  const ids = lensEntryIds();
  if (!ids.length) return;
  let idx = ids.findIndex((x) => x.id === getState().selected.id);
  if (idx < 0) idx = delta > 0 ? -1 : 0;
  const next = Math.max(0, Math.min(ids.length - 1, idx + delta));
  navigate({ selected: ids[next] }, { replace: true });
}

// Step the server-paged timeline. `lensEntryIds()` is only the loaded window, so a
// step past either edge fetches the adjacent page (offset-addressed) and then
// selects the target's global index; an in-window step selects directly.
async function stepTimeline(delta: number): Promise<void> {
  const state = getState();
  const { offset, count, matchCount } = loadedWindow(state);
  const ids = lensEntryIds();
  if (!ids.length || matchCount === 0) return;
  const local = ids.findIndex((x) => x.id === state.selected.id);
  if (local < 0) {
    // No selection (or an off-window one) → start at the first loaded row.
    navigate({ selected: ids[0] }, { replace: true });
    return;
  }
  const cur = offset + local;
  const target = Math.max(0, Math.min(matchCount - 1, cur + delta));
  if (target === cur) return; // clamped at an end of the matched set
  if (target >= offset && target < offset + count) {
    navigate({ selected: ids[target - offset] }, { replace: true });
    return;
  }
  await fetchHistoryPage({
    offset:
      target >= offset + count
        ? offset + count
        : Math.max(0, offset - HISTORY_PAGE),
  });
  const w = loadedWindow(getState());
  const loaded = lensEntryIds();
  const localAfter = target - w.offset;
  if (localAfter >= 0 && localAfter < loaded.length)
    navigate({ selected: loaded[localAfter] }, { replace: true });
}

/** Step the selection by delta, paging the timeline past its loaded edges. */
export async function stepSelectionAsync(delta: number): Promise<void> {
  if (getState().lens === "timeline") {
    await stepTimeline(delta);
    return;
  }
  stepList(delta);
}

// The Enter descend ladder: a parked cursor opens the detail pane; an open
// selection descends into its snapshot diff — a read affordance, never a gate.
function activateSelection(): void {
  const sel = getState().selected;
  if (!getState().open) {
    if (!sel.id) return;
    navigate({ open: true });
    return;
  }
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

// Layered Escape — one precedence chain, each press ascends one rung: close the
// active overlay (diff / palette / help — mutually exclusive), then blur a
// field, then restore the split from reading mode, then close the detail pane
// (the cursor stays parked), then clear the cursor, then clear the query.
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
  if (getState().reading) {
    // Reading mode is session-only state: restore through commit, not navigate.
    commit({ reading: false });
    return;
  }
  if (getState().open) {
    navigate({ open: false });
    return;
  }
  if (getState().selected.id) {
    navigate({ selected: { kind: null, id: null } });
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
    case "Enter": {
      // Native interactive targets keep their native Enter (a focused header
      // button or entity anchor would otherwise double-fire with the ladder).
      const t = ev.target;
      if (t instanceof Element && t.closest("a[href], button")) return;
      // preventDefault matters: the keydown's default action activates whatever
      // is focused AFTER handlers run — opening the diff focuses #diff-close,
      // and without this the same trusted keystroke "clicks" it shut again.
      // (Synthetic test events skip native activation, which is why only real
      // keyboards ever saw the diff flash open and close.)
      ev.preventDefault();
      activateSelection();
      return;
    }
    case " ": {
      // Space pages the open detail pane (Shift+Space pages up) — the reader
      // idiom: j/k steps items, Space reads more of the current one. A native
      // control target keeps its native Space; a closed pane keeps the
      // browser's default page scroll.
      const t = ev.target;
      if (t instanceof Element && t.closest("a[href], button")) return;
      if (!getState().open) return;
      const pane = $("#detail");
      if (!pane) return;
      ev.preventDefault();
      const page = pane.clientHeight > 0 ? pane.clientHeight * 0.85 : 400;
      pane.scrollTop += ev.shiftKey ? -page : page;
      return;
    }
    case "?":
      ev.preventDefault();
      toggleHelp();
      return;
    default:
      return;
  }
}
