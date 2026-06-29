// The render orchestrator: the single store subscriber. `render` is a plain
// `() => void` that paints one frame from `getState()` — the stat row, the load
// diagnostics, the type-toggle facets, the lens switcher, the master lens body
// (delegating to the lenses), the detail pane (delegating to detail), the
// selection scroll, and the diff overlay reconciler. Ported from the served app.js
// render cluster (`renderAll`/`render`/`renderMaster`/`renderSelected`/
// `renderStats`/`renderDiagnostics`/`renderTypeToggles`/`renderLensSwitcher`/
// `syncControls`/`scrollSelectionIntoView`).
//
// Two invariants shape it. `render` is the ONLY `subscribe()` target (wired once in
// the composition root); it reads `getState()` and never calls `navigate`/`commit`
// — the once-installed `#master` / `#filter-types` delegates own the commits. The
// served per-render row / card / type-toggle listeners collapse into those two
// delegates: `initControls` installs them once, never per render.

import { renderDetail, showComposite } from "./detail";
import { openDiff, renderDiffOverlay } from "./diff/controller";
import { $ } from "./dom";
import { escapeHtml } from "./escape";
import { renderRevisionList, renderRevisions } from "./lenses/revisions";
import { renderTimeline } from "./lenses/timeline";
import { facetCounts, presentTypes } from "./model";
import { shortId } from "./refs";
import { navigate } from "./router";
import { getState } from "./store";
import { typeColor, typeLabel } from "./types";

// One load diagnostic (the narrowed shape the diagnostics list reads off the
// `unknown[]` the store holds).
interface Diagnostic {
  code?: string;
  message?: string;
}

// The master lens whose body scaffold is currently mounted, so the scaffold is
// rebuilt only on a lens change (the populate runs every render). Transient
// view-cache — never on the store.
let lastMasterLens: string | null = null;

// ---------------------------------------------------------------------------
// Data-driven surfaces (stats, diagnostics, type facets)
// ---------------------------------------------------------------------------

/** Paint the topbar stat row from the loaded document counts and the freshness hash. */
function renderStats(): void {
  const h = getState().history;
  const r = getState().revisions;
  const o = getState().objects;
  const events = $("#stat-events");
  if (events) events.textContent = `${h?.eventCount ?? "—"} events`;
  const units = $("#stat-units");
  if (units) units.textContent = `${r?.revisionCount ?? "—"} units`;
  const threads = $("#stat-threads");
  if (threads) threads.textContent = `${o?.threadCount ?? "—"} threads`;
  const hash = $("#stat-hash");
  if (hash) hash.textContent = shortId(h?.eventSetHash);
}

/** Show the load diagnostics banner when there are any, else hide it. */
function renderDiagnostics(): void {
  const el = $("#diagnostics");
  if (!el) return;
  const diags = getState().history?.diagnostics ?? [];
  if (!diags.length) {
    el.classList.add("hidden");
    el.innerHTML = "";
    return;
  }
  el.classList.remove("hidden");
  el.innerHTML = diags
    .map((raw) => {
      const d = (raw ?? {}) as Diagnostic;
      return `<div><span class="code">${escapeHtml(d.code || "diagnostic")}</span>${escapeHtml(d.message || "")}</div>`;
    })
    .join("");
}

/** Paint the per-type filter toggles with their live facet counts and pressed state. */
function renderTypeToggles(): void {
  const container = $("#filter-types");
  if (!container) return;
  container.innerHTML = "";
  // Live facet counts: how many events each type would contribute under the rest
  // of the current query (distribution first, then narrow).
  const counts = facetCounts();
  const state = getState();
  for (const id of presentTypes()) {
    // Default a newly-seen type (e.g. an unknown event type) to enabled once;
    // after that the user's toggle sticks instead of being re-enabled here. This
    // is the served default-seeding (a transient set mutation, not a commit, so it
    // triggers no repaint).
    if (!state.seenTypes.has(id)) {
      state.seenTypes.add(id);
      state.enabledTypes.add(id);
    }
    const enabled = state.enabledTypes.has(id);
    const count = counts[id] ?? 0;
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `type-toggle${enabled ? "" : " off"}`;
    // The delegated #filter-types handler reads the toggled type off this dataset.
    btn.dataset.type = id;
    btn.setAttribute("aria-pressed", String(enabled));
    btn.setAttribute(
      "aria-label",
      `${enabled ? "Hide" : "Show"} ${typeLabel(id)} events (${count})`,
    );
    btn.innerHTML = `<span class="dot" style="background:${typeColor(id)}"></span>${escapeHtml(typeLabel(id))}<span class="type-count">${count}</span>`;
    btn.title = id;
    container.appendChild(btn);
  }
}

// ---------------------------------------------------------------------------
// Lens switcher + master pane + selection
// ---------------------------------------------------------------------------

/** Reflect the active lens onto the lens-switcher tabs' pressed state. */
function renderLensSwitcher(): void {
  const lens = getState().lens;
  for (const tab of document.querySelectorAll<HTMLElement>(".lens-tab")) {
    tab.setAttribute("aria-pressed", String(tab.dataset.lens === lens));
  }
}

// Reflect the current filter/order state into the toolbar controls (the option
// lists are rebuilt only on load, so a navigation that changes a filter syncs the
// displayed value here). The toolbar filters the event timeline, so it is shown
// only for that lens.
/** Sync the toolbar's search text, order label, and timeline-only visibility. */
function syncControls(): void {
  const state = getState();
  const text = $<HTMLInputElement>("#filter-text");
  if (text && text.value !== state.filterText) text.value = state.filterText;
  const order = $("#order-toggle");
  if (order)
    order.textContent =
      state.order === "desc" ? "newest first" : "oldest first";
  const toolbar = $("#toolbar");
  if (toolbar) toolbar.classList.toggle("hidden", state.lens !== "timeline");
}

// Master pane: swap in the active lens body and populate it. The scaffold is
// rebuilt only on a lens change; the populate runs every render so the lens
// reflects the current filters/selection. The lens function names are inverted vs
// the served app.js (revision vocabulary): the list lens is `renderRevisionList`
// (#units) and the threads lens is `renderRevisions` (#revisions).
/** Mount the active lens body scaffold (on a lens change) and populate it. */
function renderMaster(): void {
  const master = $("#master");
  if (!master) return;
  const lens = getState().lens;
  if (lens !== lastMasterLens) {
    lastMasterLens = lens;
    if (lens === "list") {
      master.innerHTML = `<div id="units" class="units"></div>`;
    } else if (lens === "threads") {
      master.innerHTML = `<div id="revisions" class="units" aria-label="supersession threads"></div>`;
    } else {
      master.innerHTML = `<ol id="timeline" class="timeline" aria-label="event timeline"></ol>`;
    }
  }
  if (lens === "list") renderRevisionList();
  else if (lens === "threads") renderRevisions();
  else renderTimeline();
}

// Detail pane: a pure projection of the single selection — the event detail, the
// revision composite (fetched on demand by detail), or the empty prompt.
/** Paint the detail pane from the single selection (delegating to detail). */
function renderSelected(): void {
  const sel = getState().selected;
  if (sel.kind === "revision" && sel.id) void showComposite(sel.id);
  else renderDetail();
}

// Scroll the selected entry into view within the master pane (the timeline row,
// the list card, or the DAG node), so cursor stepping keeps the selection visible.
/** Scroll the selected master entry into view, if any. */
function scrollSelectionIntoView(): void {
  const sel = getState().selected;
  if (!sel.id) return;
  const master = $("#master");
  if (!master) return;
  const el =
    sel.kind === "event"
      ? master.querySelector('.event[aria-selected="true"]')
      : master.querySelector(`[data-revision-id="${sel.id}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}

// ---------------------------------------------------------------------------
// The single render entry (the store subscriber)
// ---------------------------------------------------------------------------

/**
 * The single render entry: project the whole view from state. Registered once in
 * the composition root as the only `subscribe()` callback, so every `commit`
 * (navigate, load, freshness poll) repaints through here. It calls the diff
 * reconciler and ignores its returned promise (app.js parity).
 */
export function render(): void {
  renderStats();
  renderDiagnostics();
  renderLensSwitcher();
  syncControls();
  renderTypeToggles();
  renderMaster();
  renderSelected();
  scrollSelectionIntoView();
  void renderDiffOverlay();
}

// ---------------------------------------------------------------------------
// Delegated controls (installed once by the composition root)
// ---------------------------------------------------------------------------

// Toggle a present type in/out of the enabled set and navigate (replace), so the
// route is the single source of truth for the filter. The delegate, not render,
// owns this commit.
function onTypeToggleClick(ev: Event): void {
  const t = ev.target;
  if (!(t instanceof Element)) return;
  const btn = t.closest<HTMLElement>("[data-type]");
  const id = btn?.dataset.type;
  if (!id) return;
  const types = new Set(getState().enabledTypes);
  if (types.has(id)) types.delete(id);
  else types.add(id);
  navigate({ enabledTypes: types }, { replace: true });
}

// The single master-pane delegate (replacing the served per-row / per-card
// listeners). Order matters: ref chips fall through to the navigation delegate
// first; the attention-cue and open-diff affordances are handled before card
// selection (replacing the served stopPropagation). Revision selection is scoped to
// the list `.unit-card` so DAG `<g data-revision-id>` nodes stay owned by their
// imperative `wireDagInteractions` handler (no double-navigate).
function onMasterClick(ev: Event): void {
  const t = ev.target;
  if (!(t instanceof Element)) return;
  if (t.closest("[data-ref-kind]")) return;
  const cue = t.closest<HTMLElement>("[data-attention-query]");
  if (cue) {
    const query = cue.dataset.attentionQuery;
    if (query) navigate({ filterText: query });
    return;
  }
  const diffBtn = t.closest<HTMLElement>("[data-open-diff]");
  if (diffBtn) {
    const objectId = diffBtn.dataset.openDiff;
    if (objectId) openDiff(objectId, null, diffBtn.dataset.diffHash || null);
    return;
  }
  const eventEl = t.closest<HTMLElement>("[data-event-id]");
  if (eventEl) {
    const id = eventEl.dataset.eventId;
    if (id) navigate({ selected: { kind: "event", id } });
    return;
  }
  const revEl = t.closest<HTMLElement>(".unit-card[data-revision-id]");
  if (revEl) {
    const id = revEl.dataset.revisionId;
    if (id) navigate({ selected: { kind: "revision", id } });
  }
}

/**
 * Install the delegated master-pane and type-toggle handlers — once, on the stable
 * `#master` / `#filter-types` containers (their innerHTML is repainted per render,
 * but the containers persist). Called by the composition root, never per render.
 */
export function initControls(): void {
  $<HTMLElement>("#master")?.addEventListener("click", onMasterClick);
  $<HTMLElement>("#filter-types")?.addEventListener("click", onTypeToggleClick);
}
