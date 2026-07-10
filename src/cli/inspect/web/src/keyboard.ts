// The global keydown layer: selection stepping, activation, search focus,
// lens switching, the layered Escape, and the diff-local jump keys. Ported from the served
// app.js keyboard cluster (`onKey` / `handleEscape` / `stepSelection` /
// `activateSelection` / `focusSearch` / `isTypingTarget`).
//
// `keyboard` is top-of-graph — nothing imports it; the composition root wires
// `onKey` to `document.keydown`. Every state change routes through `router.navigate`
// (commit → the store subscriber repaints); it never calls render. Overlay handling
// goes through the manager: `handleEscape` closes whichever overlay is active
// (palette / diff / help are mutually exclusive), and the help toggle opens/closes
// `help` through the manager — so keyboard imports no sibling overlay module.

import { fetchHistoryPage, HISTORY_PAGE } from "./data";
import { jumpChange, jumpFact, openRevisionDiff } from "./diff/controller";
import { $ } from "./dom";
import { loadedWindow, timelineRowHeight } from "./lenses/timeline";
import { attentionEntryKeys, lensEntryIds } from "./model";
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
import { stepSplit } from "./split";
import { commit, getState } from "./store";

let lastTimelineViewportRows = 10;
let lastRevisionViewportRows = 10;

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

function focusTimelineTabStop(): void {
  const state = getState();
  if (state.lens !== "timeline" || state.reading) return;
  $<HTMLElement>("#timeline")?.focus({ preventScroll: true });
}

function isTimelineSearchInput(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement && target.id === "filter-text";
}

function focusTimelineAfterSearch(): void {
  const state = getState();
  if (state.lens !== "timeline") navigate({ lens: "timeline" });
  if (state.reading) commit({ reading: false });
  focusTimelineTabStop();
}

function timelineIsActive(): boolean {
  return getState().lens === "timeline";
}

function revisionLensIsActive(): boolean {
  const lens = getState().lens;
  return lens === "list" || lens === "threads";
}

function attentionLensIsActive(): boolean {
  return getState().lens === "attention";
}

// The attention lens keeps a lens-local focus cursor in `state.attentionFocus`
// (a kind-qualified item id), never a typed `Selection` — so it does not go
// through `lensEntryIds`/`LensEntry`. Committing it triggers a render; the lens
// re-applies the `.attention-focus` class from state, so the cursor survives a
// repaint (a freshness reload, an Enter that opens the detail).
function setAttentionFocus(key: string): void {
  commit({ attentionFocus: key });
  // The commit repainted the lens with the focus class; scroll it into view.
  $<HTMLElement>("#master")
    ?.querySelector<HTMLElement>(".attention-card.attention-focus")
    ?.scrollIntoView({ block: "nearest" });
}

// Step the lens-local focus cursor by `delta`, clamped to the loaded item keys.
// Never writes `Selection` (the desync gotcha's answer: j/k here never touches the
// timeline cursor).
function stepAttention(delta: number): void {
  const keys = attentionEntryKeys(getState());
  if (!keys.length) return;
  const current = getState().attentionFocus;
  let idx = current ? keys.indexOf(current) : -1;
  if (idx < 0) idx = delta > 0 ? -1 : 0;
  const next = Math.max(0, Math.min(keys.length - 1, idx + delta));
  setAttentionFocus(keys[next]);
}

function jumpAttentionBoundary(target: "first" | "last"): void {
  const keys = attentionEntryKeys(getState());
  if (!keys.length) return;
  setAttentionFocus(target === "first" ? keys[0] : keys[keys.length - 1]);
}

// Measure the attention container's viewport in cards, for half/full-page paging.
function attentionViewportRows(): number {
  const el = $<HTMLElement>("#attention");
  const viewportH = el?.clientHeight ?? 0;
  const card = el?.querySelector<HTMLElement>(".attention-card");
  const itemH = card?.getBoundingClientRect().height ?? 0;
  const measured =
    viewportH > 0 && itemH > 0 ? Math.floor(viewportH / itemH) : 0;
  return Math.max(1, measured);
}

// Enter/click activation resolves to the anchored revision via the delegated
// click path's target: the focused card's `data-revision-id`. This writes a
// revision `Selection` (an existing kind), never a new attention selection kind.
function activateAttentionFocus(): void {
  const revisionId = $<HTMLElement>("#master")
    ?.querySelector<HTMLElement>(".attention-card.attention-focus")
    ?.getAttribute("data-revision-id");
  if (revisionId)
    navigate({ selected: { kind: "revision", id: revisionId }, open: true });
}

function timelineViewportRows(): number {
  const list = $<HTMLElement>("#timeline");
  const viewportH = list?.clientHeight ?? 0;
  const rowH = timelineRowHeight();
  const measured = viewportH > 0 && rowH > 0 ? Math.floor(viewportH / rowH) : 0;
  if (measured > 0) {
    lastTimelineViewportRows = Math.max(1, measured);
    return lastTimelineViewportRows;
  }
  const { count } = loadedWindow(getState());
  return Math.max(
    1,
    Math.min(count || lastTimelineViewportRows, lastTimelineViewportRows),
  );
}

function revisionLensViewportRows(): number {
  const selector = getState().lens === "threads" ? "#revisions" : "#units";
  const list = $<HTMLElement>(selector);
  const item = list?.querySelector<HTMLElement>(".unit-card");
  const viewportH = list?.clientHeight ?? 0;
  const itemH = item?.getBoundingClientRect().height ?? 0;
  const measured =
    viewportH > 0 && itemH > 0 ? Math.floor(viewportH / itemH) : 0;
  if (measured > 0) {
    lastRevisionViewportRows = Math.max(1, measured);
    return lastRevisionViewportRows;
  }
  const count = lensEntryIds().length;
  return Math.max(
    1,
    Math.min(count || lastRevisionViewportRows, lastRevisionViewportRows),
  );
}

function loadedLensIndex(delta: number): number | null {
  const ids = lensEntryIds();
  if (!ids.length) return null;
  let idx = ids.findIndex((x) => x.id === getState().selected.id);
  if (idx < 0) idx = delta > 0 ? -1 : 0;
  return Math.max(0, Math.min(ids.length - 1, idx + delta));
}

function selectLoadedLensIndex(index: number): void {
  const ids = lensEntryIds();
  if (!ids.length) return;
  const target = Math.max(0, Math.min(ids.length - 1, index));
  navigate({ selected: ids[target] }, { replace: true });
}

// Step the fully-loaded revisions/threads lenses over their in-memory entries.
function stepList(delta: number): void {
  const next = loadedLensIndex(delta);
  if (next !== null) selectLoadedLensIndex(next);
}

function jumpLoadedLensBoundary(target: "first" | "last"): void {
  const ids = lensEntryIds();
  if (!ids.length) return;
  selectLoadedLensIndex(target === "first" ? 0 : ids.length - 1);
}

function pageLoadedLens(deltaRows: number): void {
  const next = loadedLensIndex(deltaRows);
  if (next !== null) selectLoadedLensIndex(next);
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
    focusTimelineTabStop();
    return;
  }
  const cur = offset + local;
  const target = Math.max(0, Math.min(matchCount - 1, cur + delta));
  if (target === cur) {
    focusTimelineTabStop();
    return; // clamped at an end of the matched set
  }
  if (target >= offset && target < offset + count) {
    navigate({ selected: ids[target - offset] }, { replace: true });
    focusTimelineTabStop();
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
  if (localAfter >= 0 && localAfter < loaded.length) {
    navigate({ selected: loaded[localAfter] }, { replace: true });
    focusTimelineTabStop();
  }
}

function pageOffsetContaining(target: number): number {
  return Math.floor(target / HISTORY_PAGE) * HISTORY_PAGE;
}

async function selectTimelineIndex(targetIndex: number): Promise<void> {
  const state = getState();
  const { offset, count, matchCount } = loadedWindow(state);
  const ids = lensEntryIds();
  if (!ids.length || matchCount === 0) return;
  const target = Math.max(0, Math.min(matchCount - 1, targetIndex));
  if (target >= offset && target < offset + count) {
    navigate({ selected: ids[target - offset] }, { replace: true });
    focusTimelineTabStop();
    return;
  }
  await fetchHistoryPage({ offset: pageOffsetContaining(target) });
  const w = loadedWindow(getState());
  const loaded = lensEntryIds();
  const localAfter = target - w.offset;
  if (localAfter >= 0 && localAfter < loaded.length) {
    navigate({ selected: loaded[localAfter] }, { replace: true });
    focusTimelineTabStop();
  }
}

async function jumpTimelineBoundary(target: "first" | "last"): Promise<void> {
  const { matchCount } = loadedWindow(getState());
  if (matchCount === 0) return;
  await selectTimelineIndex(target === "first" ? 0 : matchCount - 1);
}

async function pageTimeline(deltaRows: number): Promise<void> {
  const state = getState();
  const { offset, matchCount } = loadedWindow(state);
  if (matchCount === 0) return;
  const ids = lensEntryIds();
  if (!ids.length) return;
  const local = ids.findIndex((x) => x.id === state.selected.id);
  const cur = local < 0 ? offset : offset + local;
  await selectTimelineIndex(cur + deltaRows);
}

function jumpLensBoundary(target: "first" | "last"): void {
  if (timelineIsActive()) void jumpTimelineBoundary(target);
  else if (revisionLensIsActive()) jumpLoadedLensBoundary(target);
  else if (attentionLensIsActive()) jumpAttentionBoundary(target);
}

function pageLensRows(deltaRows: number): void {
  if (timelineIsActive()) {
    void pageTimeline(deltaRows);
    return;
  }
  if (revisionLensIsActive()) pageLoadedLens(deltaRows);
  else if (attentionLensIsActive()) stepAttention(deltaRows);
}

function pageLensFullPage(direction: 1 | -1): void {
  if (timelineIsActive()) {
    pageLensRows(direction * timelineViewportRows());
    return;
  }
  if (revisionLensIsActive()) {
    pageLensRows(direction * revisionLensViewportRows());
    return;
  }
  if (attentionLensIsActive()) {
    pageLensRows(direction * attentionViewportRows());
  }
}

function pageLensHalfPage(direction: 1 | -1): void {
  if (timelineIsActive()) {
    pageLensRows(
      direction * Math.max(1, Math.floor(timelineViewportRows() / 2)),
    );
    return;
  }
  if (attentionLensIsActive()) {
    pageLensRows(
      direction * Math.max(1, Math.floor(attentionViewportRows() / 2)),
    );
    return;
  }
  if (revisionLensIsActive()) {
    pageLensRows(
      direction * Math.max(1, Math.floor(revisionLensViewportRows() / 2)),
    );
  }
}

/** Step the selection by delta, paging the timeline past its loaded edges. */
export async function stepSelectionAsync(delta: number): Promise<void> {
  if (attentionLensIsActive()) {
    stepAttention(delta);
    return;
  }
  if (getState().lens === "timeline") {
    await stepTimeline(delta);
    return;
  }
  stepList(delta);
}

// The Enter descend ladder: a parked cursor opens the detail pane; an open
// selection descends into its snapshot diff — a read affordance, never a gate.
function activateSelection(): void {
  if (attentionLensIsActive()) {
    activateAttentionFocus();
    return;
  }
  const sel = getState().selected;
  if (!getState().open) {
    if (!sel.id) return;
    navigate({ open: true });
    focusTimelineTabStop();
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
  if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
  // Escape is global (it fires even while typing). Search Enter is an explicit
  // "done searching" action that moves focus back to the timeline; everything
  // else yields to a focused text field.
  if (ev.key === "Escape") {
    handleEscape();
    return;
  }
  if (ev.key === "Enter" && isTimelineSearchInput(ev.target)) {
    ev.preventDefault();
    focusTimelineAfterSearch();
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

  switch (ev.key) {
    case "1":
      ev.preventDefault();
      navigate({ lens: "timeline" });
      return;
    case "2":
      ev.preventDefault();
      navigate({ lens: "list" });
      return;
    case "3":
      ev.preventDefault();
      navigate({ lens: "threads" });
      return;
    case "4":
      ev.preventDefault();
      navigate({ lens: "attention" });
      return;
    case "g":
      if (
        timelineIsActive() ||
        revisionLensIsActive() ||
        attentionLensIsActive()
      ) {
        ev.preventDefault();
        jumpLensBoundary("first");
      }
      return;
    case "G":
      if (
        timelineIsActive() ||
        revisionLensIsActive() ||
        attentionLensIsActive()
      ) {
        ev.preventDefault();
        jumpLensBoundary("last");
      }
      return;
    case "/":
      ev.preventDefault();
      focusSearch();
      return;
    case "f":
      if (
        timelineIsActive() ||
        revisionLensIsActive() ||
        attentionLensIsActive()
      ) {
        ev.preventDefault();
        pageLensFullPage(1);
      }
      return;
    case "b":
      if (
        timelineIsActive() ||
        revisionLensIsActive() ||
        attentionLensIsActive()
      ) {
        ev.preventDefault();
        pageLensFullPage(-1);
      }
      return;
    case "d":
      if (
        timelineIsActive() ||
        revisionLensIsActive() ||
        attentionLensIsActive()
      ) {
        ev.preventDefault();
        pageLensHalfPage(1);
      }
      return;
    case "u":
      if (
        timelineIsActive() ||
        revisionLensIsActive() ||
        attentionLensIsActive()
      ) {
        ev.preventDefault();
        pageLensHalfPage(-1);
      }
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
    // h/l resize the split from anywhere (the divider's ArrowLeft/Right without
    // focusing it): h shrinks the timeline pane, l grows it. preventDefault only a
    // keystroke stepSplit consumed — so an inert h/l (pane closed, or h already at
    // the reading rail) still lets the browser's own type-ahead find fire.
    case "h":
      if (stepSplit(-1)) ev.preventDefault();
      return;
    case "l":
      if (stepSplit(1)) ev.preventDefault();
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
