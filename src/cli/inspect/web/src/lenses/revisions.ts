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

import { CLASS, dagNodeClass } from "../classNames";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fmtDateTime } from "../format";
import {
  matchesRevisionFilters,
  objectThreads,
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

// ---------------------------------------------------------------------------
// The flat revision list lens (#units)
// ---------------------------------------------------------------------------

/** Paint the filtered revision list into the `#units` body, one card per revision. */
export function renderRevisionList(): void {
  const el = $("#units");
  if (!el) return;
  const state = getState();
  const entries = (state.revisions?.entries ?? []).filter(
    matchesRevisionFilters,
  );
  if (!entries.length) {
    el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${
      state.filterText || state.filterObject
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
      const tail: [string, string][] = [["snapshot", shortId(u.objectId)]];
      // The target cell carries pre-escaped derived HTML (label + head badge), so
      // it bypasses the generic escaping cell renderer rather than double-escaping.
      const targetCell = `<span>target</span><b>${targetDisplayLabel(u.targetDisplay)}${targetHeadBadge(u.targetDisplay)}</b>`;
      // The diff button and the card both carry delegation datasets; the #master
      // delegate (wired by the composition root) opens the diff and selects the
      // card. The data-open-diff value is the captured object id, paired with its
      // artifact content hash for rebased-recapture disambiguation.
      return `<div class="${CLASS.unitCard}" data-revision-id="${escapeHtml(revisionId)}"${
        isSelected ? ' aria-selected="true"' : ""
      } title="${escapeHtml(revisionId)}\nclick to open the revision page">
      <h3>${escapeHtml(shortId(revisionId))}</h3>
      ${badge ? `<div class="${CLASS.supersessionBadges}">${badge}</div>` : ""}
      ${renderRevisionOverview(u, overview)}
      <div class="${CLASS.kv}">${rows.map(kv).join("")}${targetCell}${tail.map(kv).join("")}</div>
      <div class="${CLASS.actions}"><button class="${CLASS.ghost} ${CLASS.diffBtn}" data-open-diff="${escapeHtml(u.objectId ?? "")}" data-diff-hash="${escapeHtml(u.objectArtifactContentHash ?? "")}">view snapshot diff</button></div>
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
  const threads = objectThreads().filter(threadMatchesRevisionFilters);
  if (!threads.length) {
    el.innerHTML = `<p class="${CLASS.empty}" style="color:var(--fg-dim)">${
      state.filterText || state.filterObject
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

// Pure painter of the server-laid geometry: nodes are <rect>+<text> groups keyed
// by revision id, edges are routed polylines. No client-side layout — every
// coordinate comes from `laid`, already normalized to a (0,0) origin, so the
// viewBox contains the whole graph with no clipping. Heads carry no
// centering/bold/sort-first (peer-equal); the head-vs-superseded shape cue lives in
// the CSS, not in colour alone.
/** Render the laid-out supersession DAG as an SVG, or "" when there is no layout. */
export function renderThreadSvg(laid: ThreadLayout | null | undefined): string {
  const nodes = laid?.nodes ?? [];
  if (!laid || !nodes.length) return "";
  const w = laid.bounds?.w ?? 0;
  const h = laid.bounds?.h ?? 0;
  // Node centres, so each edge's arrowhead is oriented by node identity: the arrow
  // points at the superseding `from` head, never by raw points order (a
  // reversed/cycle edge still renders correctly).
  const center = new Map<string, [number, number]>(
    nodes.map((n): [string, [number, number]] => [
      n.id ?? "",
      [n.x ?? 0, n.y ?? 0],
    ]),
  );
  // Two shared arrowhead markers: a default (border-coloured) and a traced
  // (accent-coloured) one. The DAG wiring swaps a traced edge to the accent marker
  // — a cross-browser alternative to `context-stroke` (which Safari does not paint).
  const marker = (id: string, cls: string): string =>
    `<marker id="${id}" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto" markerUnits="userSpaceOnUse"><path class="${cls}" d="M0,0 L7,4 L0,8 z" /></marker>`;
  const defs = `<defs>${marker("dag-arrow", CLASS.dagArrowHead)}${marker("dag-arrow-traced", CLASS.dagArrowHeadTraced)}</defs>`;
  const edges = (laid.edges ?? [])
    .map((e) => {
      // Draw so the LAST point is nearest the `from` (superseding head) node;
      // marker-end then points the arrowhead at that head, so succession reads
      // bottom-up (a fork diverges upward into its competing heads).
      let path = e.path ?? [];
      const from = e.from != null ? center.get(e.from) : undefined;
      if (from && path.length > 1) {
        const dist2 = (p: number[]): number =>
          (p[0] - from[0]) ** 2 + (p[1] - from[1]) ** 2;
        if (dist2(path[0]) < dist2(path[path.length - 1]))
          path = [...path].reverse();
      }
      const pts = path.map(([x, y]) => `${x},${y}`).join(" ");
      return `<polyline class="${CLASS.dagEdge}" data-from="${escapeHtml(e.from ?? "")}" data-to="${escapeHtml(e.to ?? "")}" points="${pts}" marker-end="url(#dag-arrow)" />`;
    })
    .join("");
  const selected = getState().selected;
  const nodesHtml = nodes
    .map((n) => {
      const sel = selected.kind === "revision" && selected.id === n.id;
      const nodeW = n.w ?? 0;
      const nodeH = n.h ?? 0;
      const nx = n.x ?? 0;
      const ny = n.y ?? 0;
      const cls = dagNodeClass({
        isHead: !!n.isHead,
        isSuperseded: !!n.isSuperseded,
      });
      return `<g class="${cls}" data-revision-id="${escapeHtml(n.id ?? "")}" tabindex="0" role="link"${sel ? ' aria-selected="true"' : ""} aria-label="revision ${escapeHtml(shortId(n.id))}">
        <rect x="${nx - nodeW / 2}" y="${ny - nodeH / 2}" width="${nodeW}" height="${nodeH}" rx="6" />
        <text x="${nx}" y="${ny}" text-anchor="middle" dominant-baseline="middle">${escapeHtml(shortId(n.id))}</text>
      </g>`;
    })
    .join("");
  // Render at natural pixel size (1 user unit = 1px) so the node text is not scaled
  // to illegibility; CSS `max-width:100%` shrinks an oversized graph proportionally.
  return `<svg class="${CLASS.revisionDag}" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" preserveAspectRatio="xMinYMin meet" role="group" aria-label="supersession graph">${defs}${edges}${nodesHtml}</svg>`;
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
