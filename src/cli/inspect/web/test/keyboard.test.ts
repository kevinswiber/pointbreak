import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Thread } from "../src/model";
import type { HistoryDoc, RevisionsDoc, ThreadsDoc } from "../src/store";
import type { HistoryEntry } from "../src/types";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetHistoryResponse,
  setHistoryResponse,
  uninstallFetchMock,
} from "./support/fetch";

// `keyboard.ts` is the global keydown layer: selection stepping, activation, search
// focus, two-key chords, the layered Escape, and the diff-local jump keys. It is
// top-of-graph — nothing imports it — and it routes every state change through
// `router.navigate` (commit → the subscriber repaints), never render. `pendingChord`
// / `chordTimer` stay module-local. The store, the keyboard module, the overlay
// manager, and the diff controller are singletons, so reset + re-import before each
// test, and wire `onKey` to `document` the way the composition root will.
type Store = typeof import("../src/store");
type Overlay = typeof import("../src/overlay");
type Controller = typeof import("../src/diff/controller");
type Keyboard = typeof import("../src/keyboard");
type Model = typeof import("../src/model");
let store: Store;
let overlay: Overlay;
let controller: Controller;
let keyboard: Keyboard;
let model: Model;

const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

function key(init: KeyboardEventInit, target: EventTarget = document): void {
  target.dispatchEvent(
    new KeyboardEvent("keydown", { bubbles: true, ...init }),
  );
}

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
  overlay = await import("../src/overlay");
  controller = await import("../src/diff/controller");
  keyboard = await import("../src/keyboard");
  model = await import("../src/model");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
  });
  document.addEventListener("keydown", keyboard.onKey);
});

afterEach(() => {
  document.removeEventListener("keydown", keyboard.onKey);
  resetHistoryResponse();
  uninstallFetchMock();
  resetDom();
});

describe("typing targets suppress shortcuts", () => {
  it("does not step the selection while a text field is focused", () => {
    const box = document.querySelector<HTMLInputElement>("#filter-text");
    box?.focus();
    key({ key: "j" }, box ?? document);
    expect(store.getState().selected.id).toBeNull();
  });
});

describe("selection stepping / activation / search", () => {
  it("j selects the first timeline entry, k steps back", () => {
    key({ key: "j" });
    const first = store.getState().selected;
    expect(first.kind).toBe("event");
    expect(first.id).not.toBeNull();
    key({ key: "ArrowDown" });
    expect(store.getState().selected.id).not.toBe(first.id);
    key({ key: "ArrowUp" });
    expect(store.getState().selected.id).toBe(first.id);
  });

  it("Enter activates the selected revision's diff", () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    key({ key: "Enter" });
    expect(store.getState().diff).toBe(OBJ);
  });

  it("/ focuses the search box and switches to the timeline lens", () => {
    store.commit({ lens: "list" });
    key({ key: "/" });
    expect(store.getState().lens).toBe("timeline");
    expect(document.activeElement).toBe(document.querySelector("#filter-text"));
  });
});

describe("two-key chords", () => {
  it("g then l switches to the list lens", () => {
    key({ key: "g" });
    key({ key: "l" });
    expect(store.getState().lens).toBe("list");
  });

  it("g then r switches to the threads lens", () => {
    key({ key: "g" });
    key({ key: "r" });
    expect(store.getState().lens).toBe("threads");
  });
});

describe("overlays via the keyboard", () => {
  it("Cmd-K opens the command palette", () => {
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
  });

  it("? toggles the keyboard help overlay", () => {
    key({ key: "?" });
    expect(overlay.activeName()).toBe("help");
    key({ key: "?" });
    expect(overlay.activeName()).toBeNull();
  });

  it("Escape closes the active overlay first", () => {
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
    key({ key: "Escape" });
    expect(overlay.activeName()).toBeNull();
  });

  it("Escape clears the query when nothing else is open", () => {
    store.commit({ filterText: "obs" });
    key({ key: "Escape" });
    expect(store.getState().filterText).toBe("");
  });
});

describe("a focused ref chip activates on Enter", () => {
  it("resolves the chip reference", () => {
    const detail = document.querySelector("#detail");
    if (detail)
      detail.innerHTML = `<span class="ref" role="link" tabindex="0" data-ref-kind="rev" data-ref-id="${REV}">chip</span>`;
    const chip = document.querySelector<HTMLElement>("[data-ref-kind]");
    chip?.focus();
    key({ key: "Enter" }, chip ?? document);
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });
});

describe("diff-local jump keys (only while the diff overlay is open)", () => {
  it("n jumps to the next review fact, syncing the focus route", async () => {
    controller.initControls();
    store.commit({ diff: OBJ, diffHash: ARTIFACT, focus: null });
    await controller.renderDiffOverlay();
    key({ key: "n" });
    const firstAnno = document.querySelector<HTMLElement>(
      "#diff-body .anno[data-anno]",
    );
    expect(store.getState().focus).toBe(firstAnno?.dataset.anno);
  });
});

// A forked thread whose laid-out DAG order (by node y, then x) differs from its
// insertion order, so a regression that fell back to insertion order would be
// caught. B and C each supersede the root A; B/C sit at y=50 (B left of C), A at
// y=150 → DAG order [B, C, A], insertion order [A, B, C].
const FA =
  "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FB =
  "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FC =
  "rev:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FORK: Thread = {
  revisions: [FA, FB, FC],
  heads: [FB, FC],
  superseded: [FA],
  competing: true,
  laidOut: {
    bounds: { w: 300, h: 200 },
    nodes: [
      {
        id: FA,
        x: 150,
        y: 150,
        w: 120,
        h: 40,
        isHead: false,
        isSuperseded: true,
      },
      {
        id: FB,
        x: 80,
        y: 50,
        w: 120,
        h: 40,
        isHead: true,
        isSuperseded: false,
      },
      {
        id: FC,
        x: 220,
        y: 50,
        w: 120,
        h: 40,
        isHead: true,
        isSuperseded: false,
      },
    ],
    edges: [],
  },
};

// Two revisions present in the loaded list, with distinct captured objects, plus a
// thread that contains both. An object filter on the first excludes the second from
// every lens's keyboard-stepping set.
const KR1 =
  "rev:sha256:1111111111111111111111111111111111111111111111111111111111111111";
const KR2 =
  "rev:sha256:2222222222222222222222222222222222222222222222222222222222222222";
const KO1 =
  "obj:sha256:1111111111111111111111111111111111111111111111111111111111111111";
const KO2 =
  "obj:sha256:2222222222222222222222222222222222222222222222222222222222222222";
const FILTERED_REVISIONS = {
  entries: [
    { revisionId: KR1, snapshotId: KO1 },
    { revisionId: KR2, snapshotId: KO2 },
  ],
};
const FILTERED_THREAD: Thread = {
  revisions: [KR1, KR2],
  heads: [KR2],
  superseded: [KR1],
  laidOut: {
    bounds: { w: 200, h: 160 },
    nodes: [
      { id: KR1, x: 100, y: 50, w: 120, h: 40 },
      { id: KR2, x: 100, y: 110, w: 120, h: 40 },
    ],
    edges: [],
  },
};

// Drive `j` (next-selection) `steps` times, collecting the selection id after each.
function stepDown(steps: number): (string | null)[] {
  const visited: (string | null)[] = [];
  for (let i = 0; i < steps; i++) {
    key({ key: "j" });
    visited.push(store.getState().selected.id);
  }
  return visited;
}

describe("keyboard stepping order follows the rendered DAG order", () => {
  it("threads-lens stepping visits revisions in rendered DAG order, not insertion order", () => {
    store.commit({
      revisions: revisionsJson as unknown as RevisionsDoc,
      threads: { threads: [FORK] } as unknown as ThreadsDoc,
      lens: "threads",
      selected: { kind: null, id: null },
    });
    const dagOrder = model.threadRevisionOrder(FORK);
    expect(stepDown(3)).toEqual(dagOrder);
    // The DAG order must genuinely differ from insertion order, or the assertion
    // above would pass for an insertion-order regression too.
    expect(dagOrder).not.toEqual(FORK.revisions);
  });
});

describe("keyboard stepping visits only the filtered revision set", () => {
  it("skips a revision excluded by the active object filter (list and threads lenses)", () => {
    store.commit({
      revisions: FILTERED_REVISIONS as unknown as RevisionsDoc,
      threads: { threads: [FILTERED_THREAD] } as unknown as ThreadsDoc,
      filterSnapshot: KO1,
      lens: "list",
      selected: { kind: null, id: null },
    });
    // The list lens steps the filtered revision set — KR2 (a different object) is skipped.
    const listVisited = stepDown(3);
    expect(listVisited).toContain(KR1);
    expect(listVisited).not.toContain(KR2);

    // The threads lens steps the filtered set in rendered DAG order.
    store.commit({ lens: "threads", selected: { kind: null, id: null } });
    const threadsVisited = stepDown(3);
    expect(threadsVisited).not.toContain(KR2);
    expect([...new Set(threadsVisited)]).toEqual(
      model.filteredThreadRevisionIds(
        FILTERED_THREAD,
        model.threadRevisionOrder(FILTERED_THREAD),
      ),
    );
  });
});

/** A `[from, to)` run of loaded timeline entries with ids `e<from>`..`e<to-1>`. */
function pageEntries(from: number, to: number): HistoryEntry[] {
  return Array.from({ length: to - from }, (_, i) => ({
    eventId: `e${from + i}`,
    eventType: "review_observation_recorded",
    occurredAt: `unix-ms:${1782699185391 + from + i}`,
  }));
}

/** Seed a timeline window: loaded entries at `offset` within a `matchCount` set. */
function seedTimelineWindow(
  entries: HistoryEntry[],
  offset: number,
  matchCount: number,
): void {
  store.commit({
    history: {
      entries,
      diagnostics: [],
      offset,
      matchCount,
      queryKey: "order=desc&limit=100",
    } as unknown as HistoryDoc,
    lens: "timeline",
  });
}

describe("keyboard stepping pages past the loaded timeline window", () => {
  it("stepping down past the loaded edge fetches the next page then selects", async () => {
    seedTimelineWindow(pageEntries(0, 20), 0, 100);
    store.commit({ selected: { kind: "event", id: "e19" } }); // last loaded row
    setHistoryResponse({
      entries: pageEntries(20, 40),
      diagnostics: [],
      offset: 20,
      matchCount: 100,
      facets: {},
    });
    await keyboard.stepSelectionAsync(1);
    // The next page was fetched, merged, and the selection advanced to global 20.
    expect(store.getState().history?.entries.length).toBe(40);
    expect(store.getState().selected.id).toBe("e20");
  });

  it("stepping up past the loaded start fetches the previous page then selects", async () => {
    seedTimelineWindow(pageEntries(20, 40), 20, 100);
    store.commit({ selected: { kind: "event", id: "e20" } }); // first loaded (global 20)
    setHistoryResponse({
      entries: pageEntries(0, 20),
      diagnostics: [],
      offset: 0,
      matchCount: 100,
      facets: {},
    });
    await keyboard.stepSelectionAsync(-1);
    expect(store.getState().selected.id).toBe("e19");
  });

  it("stepping within the loaded window does not fetch", async () => {
    let fetched = false;
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/history")
        fetched = true;
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      seedTimelineWindow(pageEntries(0, 20), 0, 100);
      store.commit({ selected: { kind: "event", id: "e5" } });
      await keyboard.stepSelectionAsync(1);
    } finally {
      globalThis.fetch = inner;
    }
    expect(fetched).toBe(false);
    expect(store.getState().selected.id).toBe("e6");
  });
});
