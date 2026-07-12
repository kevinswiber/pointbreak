import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AttentionDoc, HistoryDoc, RevisionsDoc } from "../src/store";
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
// focus, lens switching, the layered Escape, and the diff-local jump keys. It is
// top-of-graph — nothing imports it — and it routes every state change through
// `router.navigate` (commit → the subscriber repaints), never render. The store,
// the keyboard module, the overlay manager, and the diff controller are singletons,
// so reset + re-import before each test, and wire `onKey` to `document` the way the
// composition root will.
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

function key(init: KeyboardEventInit, target: EventTarget = document): void {
  target.dispatchEvent(
    new KeyboardEvent("keydown", { bubbles: true, ...init }),
  );
}

function settleKeyboard(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function mountTimelineViewport(visibleRows: number): void {
  const master = document.querySelector<HTMLElement>("#master");
  if (!master) throw new Error("#master not mounted");
  master.innerHTML = `<ol id="timeline" class="timeline" aria-label="event timeline" tabindex="0"></ol>`;
  const timeline = document.querySelector<HTMLElement>("#timeline");
  if (!timeline) throw new Error("#timeline not mounted");
  Object.defineProperty(timeline, "clientHeight", {
    configurable: true,
    value: visibleRows * 52,
  });
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

  it("j focuses the timeline tab stop and Enter keeps Tab origin there", async () => {
    const render = await import("../src/render");
    store.subscribe(render.render);
    render.render();
    key({ key: "j" });
    const timeline = document.querySelector("#timeline");
    expect(document.activeElement).toBe(timeline);
    key({ key: "Enter" });
    expect(store.getState().open).toBe(true);
    expect(document.activeElement).toBe(timeline);
  });

  it("Enter activates the selected revision's diff once the detail is open", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    key({ key: "Enter" });
    expect(store.getState().diffPage).toBe(true);
    expect(store.getState().diffRevision).toBe(REV);
  });

  it("Enter on a parked cursor opens the detail; a second Enter opens the diff", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: false });
    key({ key: "Enter" });
    expect(store.getState().open).toBe(true);
    expect(store.getState().diffPage).toBe(false);
    key({ key: "Enter" });
    expect(store.getState().diffPage).toBe(true);
    expect(store.getState().diffRevision).toBe(REV);
  });

  it("Enter twice from a parked EVENT cursor descends into the diff", async () => {
    key({ key: "j" }); // park the cursor on the first timeline event
    await new Promise((r) => setTimeout(r, 0));
    expect(store.getState().selected.kind).toBe("event");
    expect(store.getState().open).toBe(false);
    key({ key: "Enter" });
    expect(store.getState().open).toBe(true);
    key({ key: "Enter" });
    expect(store.getState().diffPage).toBe(true);
    expect(store.getState().diffRevision).not.toBeNull();
  });

  it("Enter on a focused native control stays native (no ladder)", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: false });
    const btn = document.querySelector<HTMLElement>("#theme-toggle");
    btn?.focus();
    key({ key: "Enter" }, btn ?? document);
    expect(store.getState().open).toBe(false);
    expect(store.getState().diff).toBeNull();
  });

  it("/ focuses the search box and switches to the timeline lens", () => {
    store.commit({ lens: "list" });
    key({ key: "/" });
    expect(store.getState().lens).toBe("timeline");
    expect(document.activeElement).toBe(document.querySelector("#filter-text"));
  });

  it("Enter in search focuses the timeline without activating the selection", async () => {
    const render = await import("../src/render");
    store.subscribe(render.render);
    render.render();
    store.commit({
      selected: { kind: "revision", id: REV },
      open: false,
      reading: true,
    });
    const box = document.querySelector<HTMLInputElement>("#filter-text");
    box?.focus();
    key({ key: "Enter" }, box ?? document);
    expect(store.getState().open).toBe(false);
    expect(store.getState().reading).toBe(false);
    expect(document.activeElement).toBe(document.querySelector("#timeline"));
    key({ key: "j" });
    await settleKeyboard();
    expect(store.getState().selected.kind).toBe("event");
  });
});

describe("lens switching shortcuts", () => {
  it("1/2/3 switch lenses in tab order", () => {
    store.commit({ lens: "attention" });
    key({ key: "1" });
    expect(store.getState().lens).toBe("timeline");
    key({ key: "2" });
    expect(store.getState().lens).toBe("list");
    key({ key: "3" });
    expect(store.getState().lens).toBe("attention");
  });

  it("4 no longer maps to a lens", () => {
    store.commit({ lens: "timeline" });
    key({ key: "4" });
    expect(store.getState().lens).toBe("timeline");
  });

  it("g no longer starts a lens chord", () => {
    store.commit({ lens: "timeline" });
    key({ key: "g" });
    key({ key: "2" });
    expect(store.getState().lens).toBe("list");
    store.commit({ lens: "timeline" });
    key({ key: "g" });
    key({ key: "r" });
    expect(store.getState().lens).toBe("timeline");
  });
});

describe("overlays via the keyboard", () => {
  it("Cmd-K opens the command palette", () => {
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
  });

  it("? toggles the keyboard help overlay", async () => {
    // Register help the way the composition root does: the second ? reaches the
    // open sheet through the manager's delegation to help's own key map.
    const help = await import("../src/help-overlay");
    help.initControls();
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
    // No selection seeded: with a cursor present the ladder's cursor rungs
    // would take precedence over the query clear.
    store.commit({ filterText: "obs" });
    key({ key: "Escape" });
    expect(store.getState().filterText).toBe("");
  });

  it("Escape restores the split before closing the pane (reading rung)", () => {
    store.commit({
      selected: { kind: "revision", id: REV },
      open: true,
      reading: true,
    });
    key({ key: "Escape" });
    expect(store.getState().reading).toBe(false);
    expect(store.getState().open).toBe(true);
    key({ key: "Escape" });
    expect(store.getState().open).toBe(false);
    expect(store.getState().selected.id).toBe(REV);
  });

  it("Escape closes an open detail keeping the cursor, then clears the cursor, then the query", () => {
    store.commit({
      selected: { kind: "revision", id: REV },
      open: true,
      filterText: "sig",
    });
    key({ key: "Escape" });
    expect(store.getState().open).toBe(false);
    expect(store.getState().selected.id).toBe(REV);
    expect(store.getState().filterText).toBe("sig");
    key({ key: "Escape" });
    expect(store.getState().selected.id).toBeNull();
    expect(store.getState().filterText).toBe("sig");
    key({ key: "Escape" });
    expect(store.getState().filterText).toBe("");
  });
});

describe("Space scrolls the open detail pane", () => {
  it("Space pages down, Shift+Space pages up; closed pane leaves Space native", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    const pane = document.querySelector<HTMLElement>("#detail");
    if (!pane) throw new Error("#detail not mounted");
    pane.scrollTop = 0;
    key({ key: " " });
    const paged = pane.scrollTop;
    expect(paged).toBeGreaterThan(0);
    key({ key: " ", shiftKey: true });
    expect(pane.scrollTop).toBeLessThan(paged);
    // Closed: Space is not intercepted (native page scroll keeps working).
    store.commit({ open: false });
    const ev = new KeyboardEvent("keydown", {
      key: " ",
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });
});

describe("h / l resize the split from anywhere", () => {
  function splitMaster(): string {
    return document.documentElement.style.getPropertyValue("--split-master");
  }
  function divider(): HTMLElement {
    const el = document.querySelector<HTMLElement>(".divider");
    if (!el) throw new Error(".divider not mounted");
    return el;
  }

  it("l grows the timeline pane without focusing the divider", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    key({ key: "l" });
    expect(splitMaster()).toBe("53%");
    expect(divider().getAttribute("aria-valuenow")).toBe("53");
  });

  it("h shrinks the timeline pane without focusing the divider", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    key({ key: "h" });
    expect(splitMaster()).toBe("47%");
  });

  it("h past the floor snaps into reading mode (the divider ArrowLeft twin)", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    divider().setAttribute("aria-valuenow", "25"); // at the floor
    key({ key: "h" });
    expect(store.getState().reading).toBe(true);
  });

  it("l from reading mode restores the split", () => {
    store.commit({
      selected: { kind: "revision", id: REV },
      open: true,
      reading: true,
    });
    key({ key: "l" });
    expect(store.getState().reading).toBe(false);
  });

  it("bare l resizes and does not switch lenses", () => {
    store.commit({
      selected: { kind: "revision", id: REV },
      open: true,
      lens: "timeline",
    });
    key({ key: "l" });
    expect(store.getState().lens).toBe("timeline");
    expect(splitMaster()).toBe("53%");
  });

  it("Cmd-L is left to the browser location-bar shortcut", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    const ev = new KeyboardEvent("keydown", {
      key: "l",
      metaKey: true,
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    expect(splitMaster()).toBe("");
  });

  it("h / l are inert while a text field is focused", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    const box = document.querySelector<HTMLInputElement>("#filter-text");
    box?.focus();
    key({ key: "l" }, box ?? document);
    key({ key: "h" }, box ?? document);
    expect(splitMaster()).toBe("");
  });

  it("h / l are inert while the detail pane is closed", () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: false });
    key({ key: "l" });
    key({ key: "h" });
    expect(splitMaster()).toBe("");
    expect(store.getState().reading).toBe(false);
  });
});

// The design's behavioral invariants as standing guards: a red here is a bug in
// the production code, never a reason to weaken a guard.
describe("split-view invariants (plan 0122, I4)", () => {
  function flush(): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, 0));
  }

  it("j/k repaints the detail with the master pane hidden (reading mode)", async () => {
    const render = await import("../src/render");
    store.subscribe(render.render);
    key({ key: "j" });
    await flush();
    const first = store.getState().selected.id;
    expect(first).not.toBeNull();
    store.commit({ open: true, reading: true });
    const before = document.querySelector("#detail-body")?.innerHTML;
    key({ key: "j" });
    await flush();
    expect(store.getState().selected.id).not.toBe(first);
    expect(
      document.querySelector(".split")?.classList.contains("reading"),
    ).toBe(true);
    expect(document.querySelector("#detail-body")?.innerHTML).not.toBe(before);
  });

  it("a step that pages the timeline still works while reading", async () => {
    const render = await import("../src/render");
    store.subscribe(render.render);
    const doc = historyJson as unknown as HistoryDoc;
    const entries = doc.entries;
    const last = entries[entries.length - 1];
    const nextPageEntry = {
      ...entries[0],
      eventId: "evt:sha256:next-page-entry",
    };
    store.commit({
      history: { ...doc, offset: 0, matchCount: entries.length + 1 },
      selected: { kind: "event", id: last.eventId ?? null },
      open: true,
      reading: true,
    });
    setHistoryResponse({
      entries: [nextPageEntry],
      diagnostics: [],
      offset: entries.length,
      matchCount: entries.length + 1,
      facets: {},
    });
    const before = document.querySelector("#detail-body")?.innerHTML;
    key({ key: "j" });
    await flush();
    await flush();
    expect(store.getState().selected.id).toBe("evt:sha256:next-page-entry");
    expect(store.getState().open).toBe(true); // the form survives the page fetch
    expect(document.querySelector("#detail-body")?.innerHTML).not.toBe(before);
  });

  it("closing the detail never moves the cursor", async () => {
    const router = await import("../src/router");
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    router.navigate({ open: false });
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });

  it("keyboard stepping preserves the URL form", async () => {
    key({ key: "j" });
    await flush();
    expect(store.getState().open).toBe(false); // parked stays parked
    key({ key: "j" });
    await flush();
    expect(store.getState().open).toBe(false);
    store.commit({ open: true });
    key({ key: "j" });
    await flush();
    expect(store.getState().open).toBe(true); // open stays open
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

// While an overlay owns focus, the global keyboard layer runs only that
// overlay's registered keys, Escape, and the palette chord; every lens,
// selection, paging, and lens-switch key is inert (issue #455 — the diff
// overlay leaked j/k to the record underneath, and the help overlay had the
// identical leak).
describe("overlay keyboard scope (#455)", () => {
  const LEAKY_KEYS = [
    "j",
    "k",
    "1",
    "2",
    "3",
    "4",
    "g",
    "G",
    "Enter",
    " ",
    "n",
    "p",
    "]",
    "[",
  ];

  // Register + open the help overlay the way the composition root does: the
  // overlay focuses its close <button> — the historically leaking case.
  async function openHelp(): Promise<void> {
    const help = await import("../src/help-overlay");
    help.initControls();
    key({ key: "?" });
    expect(overlay.activeName()).toBe("help");
    expect(document.activeElement).toBe(
      document.querySelector("#key-help-close"),
    );
  }

  async function assertKeysInert(keys: string[]): Promise<void> {
    const before = structuredClone(store.getState());
    const pushSpy = vi.spyOn(history, "pushState");
    const replaceSpy = vi.spyOn(history, "replaceState");
    try {
      for (const k of keys) {
        key({ key: k });
        await settleKeyboard();
      }
      expect(store.getState()).toEqual(before); // no store commit
      expect(pushSpy).not.toHaveBeenCalled(); // no navigate...
      expect(replaceSpy).not.toHaveBeenCalled(); // ...not even a refinement
    } finally {
      pushSpy.mockRestore();
      replaceSpy.mockRestore();
    }
  }

  it("runs no store commit and no navigate for non-owned keys while help is active", async () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: false });
    await openHelp();
    await assertKeysInert(LEAKY_KEYS);
  });

  it("runs no store commit and no navigate for lens keys while the diff page is active", async () => {
    controller.initControls();
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    store.commit({ diffPage: true, diffRevision: REV });
    await controller.renderDiffPage();
    // A route surface, not an overlay: the diff-page block owns the keyboard.
    expect(overlay.activeName()).toBe(null);
    // The page's own jump keys (]/[/n/p) act through the diff-page block —
    // asserted below — so this table is the lens family only.
    await assertKeysInert(["j", "k", "1", "2", "3", "4", "g", "G", "Enter"]);
  });

  it("runs the page's own jump keys through the diff-page block", async () => {
    controller.initControls();
    store.commit({ diffPage: true, diffRevision: REV });
    await controller.renderDiffPage();
    key({ key: "n" });
    const firstAnno = document.querySelector<HTMLElement>(
      "#diff-page-body .anno[data-anno]",
    );
    expect(firstAnno).not.toBeNull();
    expect(store.getState().focus).toBe(firstAnno?.dataset.anno);
  });

  it("keeps Escape closing the active overlay", async () => {
    await openHelp();
    key({ key: "Escape" });
    expect(overlay.activeName()).toBe(null);
  });

  it("keeps the palette chord global while another overlay is active", async () => {
    await openHelp();
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
  });

  it("still types into the palette input (no swallowed text keys)", async () => {
    const palette = await import("../src/palette");
    palette.initControls();
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
    const input = document.querySelector<HTMLInputElement>("#cmd-input");
    expect(document.activeElement).toBe(input);
    // The character key's default action (inserting the character) must
    // survive the keyboard layer — inert keys are never preventDefault-ed.
    const ev = new KeyboardEvent("keydown", {
      key: "a",
      bubbles: true,
      cancelable: true,
    });
    input?.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });
});

// Two revisions present in the loaded list, with distinct captured objects. An
// object filter on the first excludes the second from the keyboard-stepping set.
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

// Drive `j` (next-selection) `steps` times, collecting the selection id after each.
function stepDown(steps: number): (string | null)[] {
  const visited: (string | null)[] = [];
  for (let i = 0; i < steps; i++) {
    key({ key: "j" });
    visited.push(store.getState().selected.id);
  }
  return visited;
}

describe("keyboard stepping visits only the filtered revision set", () => {
  it("skips a revision excluded by the active object filter", () => {
    store.commit({
      revisions: FILTERED_REVISIONS as unknown as RevisionsDoc,
      filterSnapshot: KO1,
      lens: "list",
      selected: { kind: null, id: null },
    });
    // The list lens steps the filtered revision set — KR2 (a different object) is skipped.
    const listVisited = stepDown(3);
    expect(listVisited).toContain(KR1);
    expect(listVisited).not.toContain(KR2);
  });
});

function revisionIds(count: number): string[] {
  return Array.from({ length: count }, (_, i) => `rev:${i}`);
}

function revisionDoc(ids: string[]): RevisionsDoc {
  return {
    entries: ids.map((id, i) => ({
      revisionId: id,
      snapshotId: `obj:${i}`,
      capturedAt: `unix-ms:${1000 + i}`,
    })),
  };
}

function seedRevisionNavigationLens(): string[] {
  const ids = revisionIds(8);
  store.commit({
    revisions: revisionDoc(ids),
    lens: "list",
    order: "asc",
    selected: { kind: "revision", id: ids[1] },
  });
  return ids;
}

function mountRevisionViewport(visibleRows: number): void {
  const master = document.querySelector<HTMLElement>("#master");
  if (!master) throw new Error("#master not mounted");
  master.innerHTML = `<div id="units" class="units"><div class="unit-card"></div></div>`;
  const list = document.querySelector<HTMLElement>("#units");
  const card = list?.querySelector<HTMLElement>(".unit-card");
  if (!list || !card) throw new Error("units viewport not mounted");
  Object.defineProperty(list, "clientHeight", {
    configurable: true,
    value: visibleRows * 52,
  });
  vi.spyOn(card, "getBoundingClientRect").mockReturnValue({
    height: 52,
  } as DOMRect);
}

describe("revision-centric timeline navigation", () => {
  it("g/G jump to the revision-list bounds", () => {
    const ids = seedRevisionNavigationLens();
    key({ key: "g" });
    expect(store.getState().selected.id).toBe(ids[0]);
    key({ key: "G" });
    expect(store.getState().selected.id).toBe(ids[ids.length - 1]);
  });

  it("f/b and u/d page through the revision list", () => {
    const ids = seedRevisionNavigationLens();
    mountRevisionViewport(4);
    key({ key: "f" });
    expect(store.getState().selected.id).toBe(ids[5]);
    key({ key: "b" });
    expect(store.getState().selected.id).toBe(ids[1]);
    key({ key: "d" });
    expect(store.getState().selected.id).toBe(ids[3]);
    key({ key: "u" });
    expect(store.getState().selected.id).toBe(ids[1]);
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

describe("less-style timeline keyboard navigation", () => {
  it("g and G jump to the loaded timeline bounds", async () => {
    seedTimelineWindow(pageEntries(0, 20), 0, 20);
    store.commit({ selected: { kind: "event", id: "e10" } });
    key({ key: "g" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e0");
    key({ key: "G" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e19");
  });

  it("g loads the top page when newer rows are outside the loaded window", async () => {
    seedTimelineWindow(pageEntries(100, 120), 100, 500);
    store.commit({ selected: { kind: "event", id: "e110" } });
    setHistoryResponse({
      entries: pageEntries(0, 100),
      diagnostics: [],
      offset: 0,
      matchCount: 500,
      facets: {},
    });
    key({ key: "g" });
    await settleKeyboard();
    await settleKeyboard();
    expect(store.getState().history?.offset).toBe(0);
    expect(store.getState().selected.id).toBe("e0");
  });

  it("G loads the final page when older rows are outside the loaded window", async () => {
    seedTimelineWindow(pageEntries(0, 100), 0, 250);
    store.commit({ selected: { kind: "event", id: "e10" } });
    setHistoryResponse({
      entries: pageEntries(200, 250),
      diagnostics: [],
      offset: 200,
      matchCount: 250,
      facets: {},
    });
    key({ key: "G" });
    await settleKeyboard();
    await settleKeyboard();
    expect(store.getState().history?.offset).toBe(200);
    expect(store.getState().selected.id).toBe("e249");
  });

  it("f/b page by the visible timeline row count and u/d by half that count", async () => {
    mountTimelineViewport(4);
    seedTimelineWindow(pageEntries(0, 20), 0, 20);
    store.commit({ selected: { kind: "event", id: "e5" } });
    key({ key: "f" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e9");
    key({ key: "b" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e5");
    key({ key: "d" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e7");
    key({ key: "u" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e5");
  });

  it("f can page across the loaded window edge", async () => {
    mountTimelineViewport(4);
    seedTimelineWindow(pageEntries(0, 100), 0, 250);
    store.commit({ selected: { kind: "event", id: "e98" } });
    setHistoryResponse({
      entries: pageEntries(100, 200),
      diagnostics: [],
      offset: 100,
      matchCount: 250,
      facets: {},
    });
    key({ key: "f" });
    await settleKeyboard();
    await settleKeyboard();
    expect(store.getState().history?.entries.length).toBe(200);
    expect(store.getState().selected.id).toBe("e102");
  });

  it("timeline paging continues to work while reading mode hides the master pane", async () => {
    seedTimelineWindow(pageEntries(0, 100), 0, 250);
    store.commit({
      selected: { kind: "event", id: "e10" },
      open: true,
      reading: true,
    });
    key({ key: "f" });
    await settleKeyboard();
    expect(store.getState().selected.id).toBe("e20");
    expect(store.getState().open).toBe(true);
    expect(store.getState().reading).toBe(true);
  });
});

describe("the attention lens has a lens-local cursor", () => {
  const R1 = "rev:sha256:r1";
  const R2 = "rev:sha256:r2";
  // Two items share the R1 anchor (open request + its ambiguity), so they are two
  // distinct cursor stops that both activate to R1.
  const ITEMS = [
    {
      id: "open_input_request:input-request:sha256:aa",
      kind: "open_input_request",
      tier: "primary",
      revisionId: R1,
    },
    {
      id: "ambiguous_assessment:rev:sha256:r1",
      kind: "ambiguous_assessment",
      tier: "primary",
      revisionId: R1,
    },
    {
      id: "failed_validation:validation:sha256:vv",
      kind: "failed_validation",
      tier: "primary",
      revisionId: R2,
    },
  ];

  // Drive the real render pipeline: the lens paints `#attention` from the fixture
  // and applies the `.attention-focus` class from `state.attentionFocus`, so the
  // cursor is renderable state that survives a repaint (the reviewer's fix).
  async function seedAttentionLens(): Promise<void> {
    const render = await import("../src/render");
    store.subscribe(render.render);
    store.commit({
      attention: { items: ITEMS } as unknown as AttentionDoc,
      lens: "attention",
      selected: { kind: null, id: null },
      attentionFocus: null,
    });
    render.render();
  }

  function focusedEntryId(): string | null {
    return (
      document
        .querySelector<HTMLElement>(".attention-card.attention-focus")
        ?.getAttribute("data-entry-id") ?? null
    );
  }

  it("key 3 navigates to the attention lens", () => {
    store.commit({ lens: "list" });
    key({ key: "3" });
    expect(store.getState().lens).toBe("attention");
  });

  it("attentionEntryKeys returns the kind-qualified ids in render order", async () => {
    await seedAttentionLens();
    expect(model.attentionEntryKeys(store.getState())).toEqual(
      ITEMS.map((i) => i.id),
    );
  });

  it("j/k step the lens-local focus without writing a Selection", async () => {
    await seedAttentionLens();
    key({ key: "j" });
    await settleKeyboard();
    expect(focusedEntryId()).toBe(ITEMS[0].id);
    // The timeline cursor is untouched — no Selection written (the desync gotcha).
    expect(store.getState().selected.id).toBeNull();

    key({ key: "j" });
    await settleKeyboard();
    expect(focusedEntryId()).toBe(ITEMS[1].id);
    // Focus is exclusive: only one card carries it.
    expect(
      document.querySelectorAll(".attention-card.attention-focus").length,
    ).toBe(1);

    key({ key: "k" });
    await settleKeyboard();
    expect(focusedEntryId()).toBe(ITEMS[0].id);

    // lensEntryIds is untouched: it never gained an attention kind.
    expect(
      model
        .lensEntryIds()
        .every((e) => e.kind === "event" || e.kind === "revision"),
    ).toBe(true);
  });

  it("the focus cursor survives a repaint (freshness reload / Enter)", async () => {
    await seedAttentionLens();
    key({ key: "j" });
    await settleKeyboard();
    expect(focusedEntryId()).toBe(ITEMS[0].id);
    // A render-triggering commit (a freshness reload re-commits the docs) repaints
    // #attention; the focus class must be re-applied from state, not lost.
    store.commit({ attention: { items: ITEMS } as unknown as AttentionDoc });
    expect(focusedEntryId()).toBe(ITEMS[0].id);
  });

  it("Enter activates the focused card to its anchored revision", async () => {
    await seedAttentionLens();
    key({ key: "j" });
    await settleKeyboard();
    key({ key: "Enter" });
    await settleKeyboard();
    expect(store.getState().selected).toEqual({ kind: "revision", id: R1 });
    expect(store.getState().open).toBe(true);
    // The cursor survives the Enter repaint.
    expect(focusedEntryId()).toBe(ITEMS[0].id);
  });

  it("overlapping-anchor cards are distinct stops that activate to the same revision", async () => {
    await seedAttentionLens();
    key({ key: "j" });
    await settleKeyboard();
    key({ key: "j" });
    await settleKeyboard();
    // The second stop is a different card...
    expect(focusedEntryId()).toBe(ITEMS[1].id);
    key({ key: "Enter" });
    await settleKeyboard();
    // ...that still activates to the shared R1 anchor.
    expect(store.getState().selected).toEqual({ kind: "revision", id: R1 });
  });
});
