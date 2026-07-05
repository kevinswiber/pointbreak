// The revision-centric master lenses: the flat revision list (`renderRevisionList`,
// the `#units` body) and the supersession threads + laid-out DAG (`renderRevisions`,
// the `#revisions` body). Ported from the served app.js `renderUnits` /
// `renderRevisions` / `threadLabel` / `renderThreadCard` / `renderThreadSvg` /
// `wireDagInteractions`, in the revision vocabulary.
//
// State-reading + DOM-writing. Two fidelity-preserving shape changes from app.js:
// the per-card click listener and the per-card overview-cue listener are dropped —
// cards carry the `data-revision-id` / `[data-open-diff]` / `[data-attention-query]`
// delegation datasets and the `#master` delegate (wired once by the composition
// root) handles selection, open-diff, and cue filtering. The one per-render wiring
// kept is `wireDagInteractions`: the DAG node hover/focus tracing toggles `.traced`
// on a node and its incident edges, which does not delegate, so it stays imperative.

import { CLASS } from "../classNames";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fmtDateTime } from "../format";
import {
  currentThreads,
  matchesRevisionFilters,
  orderedRevisionEntries,
  orderedThreads,
  overviewForRevision,
  renderThreadRevisionOverview,
  supersessionBadge,
  type Thread,
  type ThreadLayout,
  threadMatchesRevisionFilters,
} from "../model";
import { renderRevisionOverview } from "../projection";
import { linkify, shortId, targetDisplayLabel, targetHeadBadge } from "../refs";
import { navigate } from "../router";
import { getState } from "../store";
import { renderSupersessionSvg } from "../supersession";

// ---------------------------------------------------------------------------
// The flat revision list lens (#units)
// ---------------------------------------------------------------------------

/** Paint the filtered revision list into the `#units` body, one card per revision. */
export function renderRevisionList(): void {
  const el = $("#units");
  if (!el) return;
  const state = getState();
  const entries = orderedRevisionEntries(
    (state.revisions?.entries ?? []).filter(matchesRevisionFilters),
    state.order,
  );
  if (!entries.length) {
    el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${
      state.filterText || state.filterSnapshot
        ? "No revisions match the current filters."
        : "No captured revisions in this store."
    }</p>`;
    return;
  }
  const selected = state.selected;
  const kv = ([k, v]: [string, string]): string =>
    `<span>${escapeHtml(k)}</span><b>${escapeHtml(v)}</b>`;
  el.innerHTML = entries
    .map((u) => {
      const base = u.base ?? {};
      const overview = u.overview ?? overviewForRevision(u.revisionId ?? "");
      const revisionId = u.revisionId ?? "";
      const isSelected =
        selected.kind === "revision" && selected.id === revisionId;
      const badge = supersessionBadge(revisionId);
      const rows: [string, string][] = [
        ["captured", fmtDateTime(u.capturedAt ?? "")],
        [
          "base",
          base.commitOid
            ? `${shortId(base.commitOid)} (${base.kind ?? ""})`
            : (base.kind ?? "—"),
        ],
      ];
      const tail: [string, string][] = [["snapshot", shortId(u.snapshotId)]];
      // The target cell carries pre-escaped derived HTML (label + head badge), so
      // it bypasses the generic escaping cell renderer rather than double-escaping.
      const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
      // The diff button and the card both carry delegation datasets; the #master
      // delegate (wired by the composition root) opens the diff and selects the
      // card. The data-open-diff value is the captured snapshot id, paired with
      // its content hash for rebased-recapture disambiguation.
      return `<div class="${CLASS.unitCard}" data-revision-id="${escapeHtml(revisionId)}"${
        isSelected ? ' aria-selected="true"' : ""
      } title="${escapeHtml(revisionId)}\nclick to open the revision page">
      <h3>${escapeHtml(shortId(revisionId))}</h3>
      ${badge ? `<div class="${CLASS.supersessionBadges}">${badge}</div>` : ""}
      ${renderRevisionOverview(u, overview)}
      <div class="${CLASS.kv}">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>
      <div class="${CLASS.actions}"><button class="${CLASS.ghost} ${CLASS.diffBtn}" data-open-diff="${escapeHtml(u.snapshotId ?? "")}" data-diff-hash="${escapeHtml(u.snapshotContentHash ?? "")}">view snapshot diff</button></div>
    </div>`;
    })
    .join("");
}

// ---------------------------------------------------------------------------
// The supersession threads lens (#revisions): one card per thread, each rendering
// the revision DAG. Every revision is marked head/superseded and carries its
// forward/reverse edges, so a fork shows as multiple competing heads rather than a
// single linear stack.
// ---------------------------------------------------------------------------

/** Paint the filtered supersession threads into the `#revisions` body. */
export function renderRevisions(): void {
  const el = $("#revisions");
  if (!el) return;
  const state = getState();
  const threads = orderedThreads(
    currentThreads().filter(threadMatchesRevisionFilters),
    state.order,
  );
  if (!threads.length) {
    el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${
      state.filterText || state.filterSnapshot
        ? "No revision threads match the current filters."
        : "No captured revisions in this store."
    }</p>`;
    return;
  }
  el.innerHTML = "";
  for (const thread of threads) el.appendChild(renderThreadCard(thread));
}

/** A thread's label, distinguishing a single current head from competing forks. */
export function threadLabel(thread: Thread): string {
  const heads = thread.heads ?? [];
  if (thread.competing)
    return `revision thread · ${heads.length} competing heads`;
  if (heads.length === 1)
    return `revision thread · current in thread ${shortId(heads[0])}`;
  return "revision thread";
}

/** One thread card: its label, competing badge, head overviews, counts, and DAG. */
export function renderThreadCard(thread: Thread): HTMLElement {
  const revisions = thread.revisions ?? [];
  const heads = thread.heads ?? [];
  const superseded = thread.superseded ?? [];
  const card = document.createElement("div");
  card.className = `unit-card thread-card${thread.competing ? " competing" : ""}`;
  // A fork surfaces every competing head as a navigable chip — never a null head.
  const competingBadge = thread.competing
    ? `<div class="${CLASS.threadCompeting}"><span class="${CLASS.factStatus} ${CLASS.competing}">competing revisions (${heads.length})</span> ${heads.map((h) => linkify(h)).join(" ")}</div>`
    : "";
  const overviewBlocks = heads
    .map((h) => renderThreadRevisionOverview(h))
    .filter(Boolean)
    .join("");
  card.innerHTML = `
    <h3>${escapeHtml(threadLabel(thread))}</h3>
    ${competingBadge}
    ${overviewBlocks ? `<div class="${CLASS.threadOverviews}">${overviewBlocks}</div>` : ""}
    <div class="${CLASS.kv}">
      <span>revisions</span><b>${escapeHtml(String(revisions.length))}</b>
      <span>heads</span><b>${escapeHtml(String(heads.length))}</b>
      <span>superseded</span><b>${escapeHtml(String(superseded.length))}</b>
    </div>
    ${renderThreadSvg(thread.laidOut)}`;
  wireDagInteractions(card);
  return card;
}

// The revision defaults over the shared, node-id-agnostic painter core: revision
// nodes are keyed on `data-revision-id`, interactive (focusable links), and their
// selection is read from the store. `wireDagInteractions` handles the hover/focus
// tracing and click→navigate imperatively per render.
/** Render the laid-out supersession DAG as an SVG, or "" when there is no layout. */
export function renderThreadSvg(laid: ThreadLayout | null | undefined): string {
  return renderSupersessionSvg(laid, {
    idAttr: "data-revision-id",
    ariaNoun: "revision",
    interactive: true,
    isSelected: (id) => {
      const s = getState().selected;
      return s.kind === "revision" && s.id === id;
    },
  });
}

// Wire the DAG nodes into the IA: click / Enter / Space navigate to the revision
// via the router; hover/focus traces the node and its incident edges by class
// toggle (no re-render). This is the one justified per-render wiring — the tracing
// does not delegate — so it stays imperative.
/** Wire each DAG node's navigation and hover/focus edge tracing on a thread card. */
export function wireDagInteractions(card: HTMLElement): void {
  const nav = (node: Element): void => {
    const id = node.getAttribute("data-revision-id");
    if (id)
      navigate({
        selected: { kind: "revision", id },
        diff: null,
        diffHash: null,
        focus: null,
      });
  };
  for (const node of Array.from(
    card.querySelectorAll<SVGGElement>(".dag-node"),
  )) {
    node.addEventListener("click", () => nav(node));
    node.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        nav(node);
      }
    });
    const trace = (on: boolean): void => {
      const id = node.getAttribute("data-revision-id");
      node.classList.toggle("traced", on);
      for (const edge of Array.from(
        card.querySelectorAll<SVGPolylineElement>(
          `.dag-edge[data-from="${id}"], .dag-edge[data-to="${id}"]`,
        ),
      )) {
        edge.classList.toggle("traced", on);
        // Swap the arrowhead to the accent marker via the marker-end attribute
        // (cross-browser; not CSS context paint) so it tracks the edge highlight.
        edge.setAttribute(
          "marker-end",
          on ? "url(#dag-arrow-traced)" : "url(#dag-arrow)",
        );
      }
    };
    node.addEventListener("mouseenter", () => trace(true));
    node.addEventListener("mouseleave", () => trace(false));
    node.addEventListener("focus", () => trace(true));
    node.addEventListener("blur", () => trace(false));
  }
}
