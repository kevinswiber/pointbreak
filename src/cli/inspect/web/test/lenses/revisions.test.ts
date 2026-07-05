import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Thread } from "../../src/model";
import type { HistoryDoc, RevisionsDoc, ThreadsDoc } from "../../src/store";
import historyJson from "../fixtures/history.json";
import revisionsJson from "../fixtures/revisions.json";
import threadsJson from "../fixtures/threads.json";
import { mountInspectorDom, resetDom } from "../support/dom";

// `lenses/revisions.ts` paints the two revision-centric master lenses: the flat
// revision list (`renderRevisionList`, the `#units` body) and the supersession
// threads + DAG (`renderRevisions`, the `#revisions` body). Cards carry the
// `data-revision-id`/`[data-open-diff]`/`[data-attention-query]` delegation attrs
// and no per-card listeners — except `wireDagInteractions`, which wires the DAG
// node hover/focus tracing and click→navigate imperatively per render. The store
// and the lens share one `state`, so reset the registry and re-import before each.
type Store = typeof import("../../src/store");
type Revisions = typeof import("../../src/lenses/revisions");
let store: Store;
let revisions: Revisions;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../../src/store");
  revisions = await import("../../src/lenses/revisions");
  mountInspectorDom();
  history.replaceState(null, "", "/");
});

afterEach(() => {
  resetDom();
});

// The single revision/object/artifact the committed fixtures describe.
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

function seedFixtures(): void {
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
    threads: threadsJson as unknown as ThreadsDoc,
  });
}

function mountListBody(): void {
  const master = document.querySelector("#master");
  if (master) master.innerHTML = `<div id="units" class="units"></div>`;
}

function mountThreadsBody(): void {
  const master = document.querySelector("#master");
  if (master) master.innerHTML = `<div id="revisions" class="units"></div>`;
}

// A forked thread with a laid-out competing-heads DAG: B and C each supersede the
// root A, so the layout has two equal-rank heads, one superseded root, and two
// edges pointing at the root.
const A =
  "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B =
  "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C =
  "rev:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FORK: Thread = {
  revisions: [A, B, C],
  heads: [B, C],
  superseded: [A],
  competing: true,
  laidOut: {
    bounds: { w: 300, h: 200 },
    nodes: [
      {
        id: A,
        x: 150,
        y: 150,
        w: 120,
        h: 40,
        isHead: false,
        isSuperseded: true,
      },
      { id: B, x: 80, y: 50, w: 120, h: 40, isHead: true, isSuperseded: false },
      {
        id: C,
        x: 220,
        y: 50,
        w: 120,
        h: 40,
        isHead: true,
        isSuperseded: false,
      },
    ],
    edges: [
      {
        from: B,
        to: A,
        path: [
          [80, 70],
          [150, 130],
        ],
      },
      {
        from: C,
        to: A,
        path: [
          [220, 70],
          [150, 130],
        ],
      },
    ],
  },
};

function seedThread(thread: Thread): void {
  store.commit({
    revisions: revisionsJson as unknown as RevisionsDoc,
    threads: { threads: [thread] } as unknown as ThreadsDoc,
  });
}

describe("renderRevisionList (the flat revision list lens)", () => {
  it("paints one card per revision with the delegation datasets and the open-diff control", () => {
    seedFixtures();
    mountListBody();
    revisions.renderRevisionList();
    const cards = document.querySelectorAll<HTMLElement>("#units .unit-card");
    expect(cards.length).toBe(
      (revisionsJson as unknown as RevisionsDoc).entries.length,
    );
    const card = cards[0];
    expect(card.dataset.revisionId).toBe(REV);
    const diffBtn = card.querySelector<HTMLElement>("[data-open-diff]");
    expect(diffBtn?.dataset.openDiff).toBe(OBJ);
    expect(diffBtn?.dataset.diffHash).toBe(ARTIFACT);
  });

  it("surfaces the supersession badge and the advisory attention cues", () => {
    seedFixtures();
    mountListBody();
    revisions.renderRevisionList();
    const card = document.querySelector<HTMLElement>("#units .unit-card");
    // The lone isolated revision is a current head ("current in thread").
    expect(card?.querySelector(".badge.head")?.textContent).toContain(
      "current in thread",
    );
    // Attention cues render as filter buttons carrying the data-attention-query
    // attr; the #master delegate (a later PR) handles the click, not the lens.
    const cue = card?.querySelector<HTMLElement>("[data-attention-query]");
    if (cue) expect(cue.dataset.attentionQuery).toMatch(/^attention:/);
  });

  it("attaches no per-card click listener — selection is left to the #master delegate", () => {
    seedFixtures();
    mountListBody();
    revisions.renderRevisionList();
    const card = document.querySelector<HTMLElement>("#units .unit-card");
    card?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: null, id: null });
  });

  it("marks the selected revision card with aria-selected", () => {
    seedFixtures();
    store.commit({ selected: { kind: "revision", id: REV } });
    mountListBody();
    revisions.renderRevisionList();
    const card = document.querySelector<HTMLElement>("#units .unit-card");
    expect(card?.getAttribute("aria-selected")).toBe("true");
  });

  it("shows the empty message when no revisions are loaded", () => {
    mountListBody();
    revisions.renderRevisionList();
    expect(document.querySelector("#units")?.textContent).toContain(
      "No captured revisions",
    );
  });
});

describe("renderRevisions (the supersession threads lens)", () => {
  it("paints one thread card per laid-out thread", () => {
    seedFixtures();
    mountThreadsBody();
    revisions.renderRevisions();
    const cards = document.querySelectorAll("#revisions .thread-card");
    expect(cards.length).toBe(1);
  });

  it("uses current-in-thread copy for a single-head thread (no bare head badge)", () => {
    seedFixtures();
    mountThreadsBody();
    revisions.renderRevisions();
    const heading = document.querySelector("#revisions .thread-card h3");
    expect(heading?.textContent).toContain(
      "revision thread · current in thread",
    );
  });

  it("shows a competing-revisions badge for a forked thread, never a null head", () => {
    seedThread(FORK);
    mountThreadsBody();
    revisions.renderRevisions();
    const card = document.querySelector("#revisions .thread-card");
    expect(card?.querySelector("h3")?.textContent).toContain(
      "2 competing heads",
    );
    expect(card?.querySelector(".thread-competing")?.textContent).toContain(
      "competing revisions (2)",
    );
  });
});

describe("threadLabel", () => {
  it("names a single head as current-in-thread", () => {
    expect(revisions.threadLabel({ heads: [B], competing: false })).toContain(
      "revision thread · current in thread",
    );
  });

  it("names competing heads by count", () => {
    expect(revisions.threadLabel({ heads: [B, C], competing: true })).toBe(
      "revision thread · 2 competing heads",
    );
  });

  it("falls back to a bare thread label with no heads", () => {
    expect(revisions.threadLabel({ heads: [], competing: false })).toBe(
      "revision thread",
    );
  });
});

describe("renderThreadSvg + wireDagInteractions (the no-trunk competing-heads DAG)", () => {
  it("emits the laid-out SVG with directional arrowhead markers and per-node datasets", () => {
    seedThread(FORK);
    const card = revisions.renderThreadCard(FORK);
    const svg = card.querySelector("svg.revision-dag");
    expect(svg).not.toBeNull();
    // Two shared arrowhead markers (default + traced) so a traced edge can swap.
    expect(svg?.querySelector("marker#dag-arrow")).not.toBeNull();
    expect(svg?.querySelector("marker#dag-arrow-traced")).not.toBeNull();
    // Three nodes; the two heads carry the head class, the root the superseded one.
    const nodes = card.querySelectorAll<SVGGElement>("g.dag-node");
    expect(nodes.length).toBe(3);
    expect(card.querySelectorAll("g.dag-node.head").length).toBe(2);
    expect(card.querySelectorAll("g.dag-node.superseded").length).toBe(1);
    // Two edges, each pointing at the superseded root via the default marker.
    const edges =
      card.querySelectorAll<SVGPolylineElement>("polyline.dag-edge");
    expect(edges.length).toBe(2);
    for (const e of edges) {
      expect(e.getAttribute("data-to")).toBe(A);
      expect(e.getAttribute("marker-end")).toBe("url(#dag-arrow)");
    }
  });

  it("traces a node and its incident edges on hover, swapping the arrowhead marker", () => {
    seedThread(FORK);
    mountThreadsBody();
    revisions.renderRevisions();
    const root = document.querySelector("#revisions");
    const headB = root?.querySelector<SVGGElement>(
      `g.dag-node[data-revision-id="${B}"]`,
    );
    expect(headB).not.toBeNull();
    headB?.dispatchEvent(new Event("mouseenter"));
    expect(headB?.classList.contains("traced")).toBe(true);
    const tracedEdges =
      root?.querySelectorAll("polyline.dag-edge.traced") ?? [];
    expect(tracedEdges.length).toBe(1); // only the B→A edge is incident to B
    const incident = root?.querySelector(`polyline.dag-edge[data-from="${B}"]`);
    expect(incident?.getAttribute("marker-end")).toBe("url(#dag-arrow-traced)");

    headB?.dispatchEvent(new Event("mouseleave"));
    expect(headB?.classList.contains("traced")).toBe(false);
    expect(
      (root?.querySelectorAll("polyline.dag-edge.traced") ?? []).length,
    ).toBe(0);
    expect(incident?.getAttribute("marker-end")).toBe("url(#dag-arrow)");
  });

  it("navigates to a revision when its DAG node is clicked (imperative wiring)", () => {
    seedThread(FORK);
    mountThreadsBody();
    revisions.renderRevisions();
    const node = document
      .querySelector("#revisions")
      ?.querySelector<SVGGElement>(`g.dag-node[data-revision-id="${C}"]`);
    node?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: "revision", id: C });
    // Opening a revision from the DAG clears any open diff overlay route.
    expect(store.getState().diff).toBeNull();
  });

  it("keeps revision DAG nodes interactive and keyed on data-revision-id", () => {
    seedThread(FORK);
    const card = revisions.renderThreadCard(FORK);
    const node = card.querySelector("g.dag-node[data-revision-id]");
    expect(node).not.toBeNull();
    expect(node?.getAttribute("tabindex")).toBe("0");
    expect(node?.getAttribute("role")).toBe("link");
    expect(node?.getAttribute("aria-label")?.startsWith("revision ")).toBe(
      true,
    );
  });

  it("renderThreadSvg emits byte-identical revision markup (characterization guard)", () => {
    seedThread(FORK);
    // Paint the SVG directly from the fixture's server-laid geometry. The FORK
    // fixture's coordinates are fixed, so the output string is deterministic; any
    // diff after the painter-core extraction means the wrapper regressed.
    expect(revisions.renderThreadSvg(FORK.laidOut)).toMatchInlineSnapshot(`
      "<svg class="revision-dag" width="300" height="200" viewBox="0 0 300 200" preserveAspectRatio="xMinYMin meet" role="group" aria-label="supersession graph"><defs><marker id="dag-arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto" markerUnits="userSpaceOnUse"><path class="dag-arrow-head" d="M0,0 L7,4 L0,8 z" /></marker><marker id="dag-arrow-traced" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto" markerUnits="userSpaceOnUse"><path class="dag-arrow-head-traced" d="M0,0 L7,4 L0,8 z" /></marker></defs><polyline class="dag-edge" data-from="rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" data-to="rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" points="150,130 80,70" marker-end="url(#dag-arrow)" /><polyline class="dag-edge" data-from="rev:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" data-to="rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" points="150,130 220,70" marker-end="url(#dag-arrow)" /><g class="dag-node superseded" data-revision-id="rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" tabindex="0" role="link" aria-label="revision aaaaaaaaaaaa">
              <rect x="90" y="130" width="120" height="40" rx="6" />
              <text x="150" y="150" text-anchor="middle" dominant-baseline="middle">aaaaaaaaaaaa</text>
            </g><g class="dag-node head" data-revision-id="rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" tabindex="0" role="link" aria-label="revision bbbbbbbbbbbb">
              <rect x="20" y="30" width="120" height="40" rx="6" />
              <text x="80" y="50" text-anchor="middle" dominant-baseline="middle">bbbbbbbbbbbb</text>
            </g><g class="dag-node head" data-revision-id="rev:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" tabindex="0" role="link" aria-label="revision cccccccccccc">
              <rect x="160" y="30" width="120" height="40" rx="6" />
              <text x="220" y="50" text-anchor="middle" dominant-baseline="middle">cccccccccccc</text>
            </g></svg>"
    `);
  });
});

describe("newest-first list ordering (#256)", () => {
  const rev = (id: string, ms: number) => ({
    revisionId: id,
    capturedAt: `unix-ms:${ms}`,
    snapshotId: `snap:${id}`,
  });

  it("renders the flat revision list newest-first by default", () => {
    store.commit({
      revisions: {
        entries: [rev("a", 100), rev("c", 300), rev("b", 200)],
      } as unknown as RevisionsDoc,
    });
    mountListBody();
    revisions.renderRevisionList();
    const ids = [...document.querySelectorAll("#units .unit-card")].map((c) =>
      c.getAttribute("data-revision-id"),
    );
    expect(ids).toEqual(["c", "b", "a"]);
  });

  it("orders thread cards newest-first by member recency", () => {
    store.commit({
      revisions: {
        entries: [rev("a", 100), rev("b", 500), rev("c", 300)],
      } as unknown as RevisionsDoc,
      threads: {
        threads: [{ revisions: ["a"] }, { revisions: ["b", "c"] }],
      } as unknown as ThreadsDoc,
    });
    mountThreadsBody();
    revisions.renderRevisions();
    const cards = document.querySelectorAll("#revisions .thread-card");
    expect(cards.length).toBe(2);
    // The 2-revision thread (member "b" at ms 500) sorts before the 1-revision
    // thread (member "a" at ms 100); its "revisions" count cell (first <b>) is "2".
    const firstCount = cards[0].querySelector(".kv b")?.textContent;
    expect(firstCount).toBe("2");
  });
});
