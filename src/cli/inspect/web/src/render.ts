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

import { CLASS } from "./classNames";
import { renderDetail, showComposite } from "./detail";
import { openDiff, renderDiffOverlay, renderDiffPage } from "./diff/controller";
import { $ } from "./dom";
import { escapeHtml } from "./escape";
import { partitionAttentionTiers, renderAttention } from "./lenses/attention";
import { renderRevisionList } from "./lenses/revisions";
import {
  renderTimeline,
  scrollTimelineSelectionIntoView,
} from "./lenses/timeline";
import { presentTypes } from "./model";
import { shortId } from "./refs";
import { navigate } from "./router";
import { commit, getState } from "./store";
import { typeColor, typeLabel } from "./types";

const INSPECTOR_TITLE = "Pointbreak Review";

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

/**
 * Paint the top-bar repo/store identity and the browser tab `<title>` (issue #391).
 * Static per session; the sole writer of `#store-identity` and `document.title`. Left
 * empty (title reset to the default) until the one-shot identity fetch lands.
 */
function renderIdentity(): void {
  const root = $("#store-identity");
  if (!root) return;
  const id = getState().identity;
  if (!id) {
    // Hide the store chip until the one-shot identity fetch lands.
    root.classList.add("hidden");
    document.title = INSPECTOR_TITLE;
    return;
  }
  root.classList.remove("hidden");

  // The detail popover's identity rows: repository + store always, family/worktree
  // only when set. The stat rows and the trust note below them are static markup
  // (renderStats owns #stat-*); this fills only the identity rows, the chip's repo
  // label + accessible name, and the tab title.
  const rows: Array<[string, string]> = [
    ["repository", id.repository],
    ["store", id.placement.label],
  ];
  if (id.family) rows.push(["family", id.family.id]);
  if (id.worktree) rows.push(["worktree", id.worktree]);

  const rowsEl = $("#store-identity-rows");
  if (rowsEl) {
    rowsEl.innerHTML = rows
      .map(([k, v]) => `<dt>${escapeHtml(k)}</dt><dd>${escapeHtml(v)}</dd>`)
      .join("");
  }
  const repoEl = $("#store-chip-repo");
  if (repoEl) repoEl.textContent = id.repository;
  // The chip's accessible label carries the full identity, so a screen reader gets
  // everything without depending on the visual (aria-hidden) popover.
  $("#store-chip")?.setAttribute(
    "aria-label",
    rows.map(([k, v]) => `${k} ${v}`).join(", "),
  );

  // Plain-text title (no HTML escaping needed for document.title).
  document.title = `${id.repository} · ${INSPECTOR_TITLE}`;
}

/** Paint the topbar stat row from the loaded document counts and the history event-set hash. */
function renderStats(): void {
  const h = getState().history;
  const r = getState().revisions;
  const o = getState().threads;
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
      return `<div><span class="${CLASS.code}">${escapeHtml(d.code || "diagnostic")}</span>${escapeHtml(d.message || "")}</div>`;
    })
    .join("");
}

/** Paint the per-type filter toggles with their live facet counts and pressed state. */
function renderTypeToggles(): void {
  const container = $("#filter-types");
  if (!container) return;
  container.innerHTML = "";
  // Server-computed facet counts: how many events each type contributes under the
  // rest of the current query (the toggle distribution, excluding the type filter).
  const counts = getState().history?.facets ?? {};
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
    btn.innerHTML = `<span class="${CLASS.dot}" style="background:${typeColor(id)}"></span>${escapeHtml(typeLabel(id))}<span class="${CLASS.typeCount}">${count}</span>`;
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
    if (tab.dataset.lens === "attention") renderAttentionBadge(tab);
  }
}

// The attention tab's count badge: the store-wide judgment queue, glanceable
// from every lens (the per-revision view lives on the detail page). The
// needs-input count is the number; the advisory count rides muted beside it.
// Both zero removes the element — an honest empty state, never a "0" chip. The
// counts are derived from the served items, so they fall (or rise) on their own
// and the badge follows on the same repaint; there is no dismissal affordance.
/** Reconcile the judgment-queue count badge inside the attention tab. */
function renderAttentionBadge(tab: HTMLElement): void {
  const { primary, secondary } = partitionAttentionTiers(
    getState().attention?.items ?? [],
  );
  tab.querySelector(`.${CLASS.attentionBadge}`)?.remove();
  if (!primary.length && !secondary.length) return;
  const badge = document.createElement("span");
  badge.className = CLASS.attentionBadge;
  // The aria-label replaces the badge's text in the tab's accessible name, so
  // the counts are announced with their meaning, not read as bare digits.
  const needsInput =
    primary.length === 1
      ? "1 item needs input"
      : `${primary.length} items need input`;
  badge.setAttribute(
    "aria-label",
    [
      primary.length ? needsInput : "",
      secondary.length ? `${secondary.length} advisory` : "",
    ]
      .filter(Boolean)
      .join(", "),
  );
  badge.innerHTML = `${primary.length ? primary.length : ""}${secondary.length ? `<span class="${CLASS.attentionBadgeSecondary}">+${secondary.length}</span>` : ""}`;
  tab.appendChild(badge);
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
// reflects the current filters/selection.
/** Mount the active lens body scaffold (on a lens change) and populate it. */
function renderMaster(): void {
  const master = $("#master");
  if (!master) return;
  const lens = getState().lens;
  if (lens !== lastMasterLens) {
    lastMasterLens = lens;
    if (lens === "list") {
      master.innerHTML = `<div id="units" class="${CLASS.units}"></div>`;
    } else if (lens === "attention") {
      master.innerHTML = `<div id="attention" class="${CLASS.units}" aria-label="attention"></div>`;
    } else {
      master.innerHTML = `<ol id="timeline" class="${CLASS.timeline}" aria-label="event timeline" tabindex="0"></ol>`;
    }
  }
  if (lens === "list") renderRevisionList();
  else if (lens === "attention") renderAttention();
  else renderTimeline();
}

// Project the open state onto the split grid: `split-closed` is a string-literal
// state class (the `hidden` precedent, not a CLASS entry) the stylesheet keys the
// single-column collapse — and, at narrow widths, the slide-over sheet — off.
/** Toggle the split grid's mode classes from the open/reading state. */
function applySplitMode(): void {
  const split = $(".split");
  if (!split) return;
  const s = getState();
  split.classList.toggle("split-closed", !s.open);
  const reading = s.reading && s.open;
  split.classList.toggle("reading", reading);
  // The reading toggle presents its own state: ⤢ enters, ⤡ restores.
  const readBtn = $("#detail-read");
  if (readBtn) {
    readBtn.textContent = reading ? "⤡" : "⤢";
    const label = reading ? "Restore split" : "Reading mode";
    readBtn.setAttribute("aria-label", label);
    readBtn.setAttribute("title", label);
  }
}

// Detail pane: a pure projection of the single selection — the event detail or
// the revision composite (fetched on demand by detail). A closed pane paints
// nothing and, for a revision cursor, must not fetch the composite.
/** Paint the detail pane from the open selection (delegating to detail). */
function renderSelected(): void {
  if (!getState().open) return;
  const sel = getState().selected;
  if (sel.kind === "revision" && sel.id) void showComposite(sel.id);
  else renderDetail();
}

// Scroll the selected entry into view within the master pane (the timeline row,
// the list card, or the DAG node), so cursor stepping keeps the selection visible.
// The timeline is virtualized, so an off-screen event row is not in the DOM; event
// selection routes through the timeline's index-scroller, which scrolls the row's
// index into the virtual window and repaints before scrolling it into view. This is
// the single materialization point — route-state, deep-link, and keyboard selection
// all reach off-screen rows through here.
/** Scroll the selected master entry into view, if any. */
function scrollSelectionIntoView(): void {
  const sel = getState().selected;
  if (!sel.id) return;
  if (sel.kind === "event") {
    scrollTimelineSelectionIntoView(sel.id);
    return;
  }
  const master = $("#master");
  if (!master) return;
  const el = master.querySelector(`[data-revision-id="${sel.id}"]`);
  if (el) el.scrollIntoView({ block: "center" });
}

// ---------------------------------------------------------------------------
// The single render entry (the store subscriber)
// ---------------------------------------------------------------------------

// Project the routed diff page's frame ownership: while `diffPage` is set the
// page shows and the lens layout (toolbar + master-detail split) hides — no
// lens renders underneath. Off the page every piece is restored (syncControls
// then re-owns the toolbar's lens-conditional visibility). Returns whether the
// page owns this frame.
function applyDiffPageMode(): boolean {
  const onPage = getState().diffPage;
  $("#diff-page")?.classList.toggle("hidden", !onPage);
  for (const sel of [
    "#toolbar",
    "#master",
    "#detail",
    "#master-rail",
    ".divider",
  ]) {
    $(sel)?.classList.toggle("hidden", onPage);
  }
  return onPage;
}

/**
 * The single render entry: project the whole view from state. Registered once in
 * the composition root as the only `subscribe()` callback, so every `commit`
 * (navigate, load, freshness poll) repaints through here. While the diff page
 * owns the frame only the topbar surfaces repaint above it; otherwise it calls
 * the overlay reconciler and ignores its returned promise (app.js parity).
 */
export function render(): void {
  renderIdentity();
  renderStats();
  renderDiagnostics();
  renderLensSwitcher();
  if (applyDiffPageMode()) {
    void renderDiffPage();
    return;
  }
  syncControls();
  renderTypeToggles();
  applySplitMode();
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
// selection (replacing the served stopPropagation). Revision selection is scoped
// to the list `.unit-card` so other `data-revision-id` carriers (badges, chips)
// never double-navigate.
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
    const snapshotId = diffBtn.dataset.openDiff;
    if (snapshotId)
      openDiff(snapshotId, null, diffBtn.dataset.diffHash || null);
    return;
  }
  // A mouse click both parks the cursor and opens the detail (today's feel);
  // keyboard stepping, by contrast, leaves `open` unchanged.
  const eventEl = t.closest<HTMLElement>("[data-event-id]");
  if (eventEl) {
    const id = eventEl.dataset.eventId;
    if (id) navigate({ selected: { kind: "event", id }, open: true });
    return;
  }
  const revEl = t.closest<HTMLElement>(".unit-card[data-revision-id]");
  if (revEl) {
    const id = revEl.dataset.revisionId;
    if (id) navigate({ selected: { kind: "revision", id }, open: true });
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
  // The pane's close control: closing is deselection of the *pane*, not the
  // cursor — the selection survives so Enter/j/k resume from it.
  $<HTMLElement>("#detail-close")?.addEventListener("click", () =>
    navigate({ open: false }),
  );
  // The narrow-viewport sheet's back affordance — the same close rung with a
  // different face; CSS hides it at wide widths (no TS knows the breakpoint).
  $<HTMLElement>("#detail-back")?.addEventListener("click", () =>
    navigate({ open: false }),
  );
  // Reading mode toggles through commit, never navigate — it is session-only
  // state (not URL), and a navigate would push a junk history entry.
  $<HTMLElement>("#detail-read")?.addEventListener("click", () =>
    commit({ reading: !getState().reading }),
  );
  $<HTMLElement>("#master-rail")?.addEventListener("click", () =>
    commit({ reading: false }),
  );
}
