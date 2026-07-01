import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryDoc } from "../../src/store";
import type { HistoryEntry } from "../../src/types";
import historyJson from "../fixtures/history.json";
import { mountInspectorDom, resetDom } from "../support/dom";
import {
  installFetchMock,
  resetHistoryResponse,
  setHistoryResponse,
  uninstallFetchMock,
} from "../support/fetch";

// `lenses/timeline.ts` paints the event timeline into the master pane. The server
// now filters, searches, orders, and facets the history, so the lens paints the
// loaded page window as-is: it is sized by the server `matchCount`, placed at its
// `offset`, and fetches the next page when the viewport nears the loaded edge. It
// is state-reading + DOM-writing: seed the store, inject the timeline body the way
// `renderMaster` does at runtime, then assert the painted rows/spacers. The store
// and the lens are module singletons sharing one `state`, so reset the registry
// and re-import both before each test.
type Store = typeof import("../../src/store");
type Timeline = typeof import("../../src/lenses/timeline");
let store: Store;
let timeline: Timeline;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../../src/store");
  timeline = await import("../../src/lenses/timeline");
  mountInspectorDom();
  // A partial window near its loaded edge fetches the next page during a render;
  // the fixture mock absorbs those fetch-on-scroll requests so they never reach
  // the real network. A paging test overrides the response via setHistoryResponse.
  installFetchMock();
  // renderMaster (the render orchestrator) injects the timeline body inside
  // #master; mirror it.
  const master = document.querySelector("#master");
  if (master) master.innerHTML = `<ol id="timeline" class="timeline"></ol>`;
  history.replaceState(null, "", "/");
});

afterEach(() => {
  resetHistoryResponse();
  uninstallFetchMock();
  resetDom();
});

/** Commit a history window: loaded `entries` plus the server window fields. */
function seedHistory(
  entries: HistoryEntry[],
  over: Partial<HistoryDoc> = {},
): void {
  store.commit({
    history: { entries, diagnostics: [], ...over } as unknown as HistoryDoc,
  });
}

/** A minimal timeline entry with a canonical (non-retired) event type. */
function entry(
  eventId: string,
  over: Partial<HistoryEntry> = {},
): HistoryEntry {
  return {
    eventId,
    eventType: "review_observation_recorded",
    occurredAt: "unix-ms:1782699185391",
    ...over,
  };
}

function rowIds(): string[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>("#timeline li.event"),
  )
    .map((li) => li.dataset.eventId ?? "")
    .filter(Boolean);
}

function spacerHeights(): number[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>("#timeline li[data-spacer]"),
  ).map((li) => Number.parseInt(li.style.height || "0", 10));
}

describe("renderTimeline", () => {
  it("paints one row per loaded event with the data-event-id delegation dataset", () => {
    store.commit({ history: historyJson as unknown as HistoryDoc });
    const entries = (historyJson as unknown as HistoryDoc).entries;
    timeline.renderTimeline();
    const rows = document.querySelectorAll<HTMLElement>("#timeline li.event");
    expect(rows.length).toBe(entries.length);
    for (const li of rows) expect(li.dataset.eventId).toBeTruthy();
  });

  it("paints the loaded page in server order, never reversing client-side", () => {
    // The server applies `order`; the lens paints the window verbatim, so the
    // rows follow the loaded array regardless of the order toggle.
    seedHistory([entry("e1"), entry("e2"), entry("e3")]);
    timeline.renderTimeline();
    expect(rowIds()).toEqual(["e1", "e2", "e3"]);

    store.commit({ order: "asc" });
    timeline.renderTimeline();
    expect(rowIds()).toEqual(["e1", "e2", "e3"]);
  });

  it("renders an advisory verification chip from the entry status", () => {
    seedHistory([entry("e1", { verificationStatus: "unsigned" })]);
    timeline.renderTimeline();
    const chip = document.querySelector("#timeline li.event .verify");
    expect(chip).not.toBeNull();
    expect(chip?.textContent).toContain("unsigned");
  });

  it("linkifies embedded reference ids into navigable chips, not plain text", () => {
    const ref =
      "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
    seedHistory([entry("e1", { summary: { title: `supersedes ${ref}` } })]);
    timeline.renderTimeline();
    const chip = document.querySelector<HTMLElement>(
      "#timeline li.event [data-ref-kind]",
    );
    expect(chip).not.toBeNull();
    expect(chip?.dataset.refKind).toBe("rev");
    expect(chip?.dataset.refId).toBe(ref);
  });

  it("attaches no per-row click listener — selection is left to the #master delegate", () => {
    seedHistory([entry("e1")]);
    timeline.renderTimeline();
    const row = document.querySelector<HTMLElement>("#timeline li.event");
    row?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: null, id: null });
  });

  it("shows a muted empty-state row when the query matches nothing", () => {
    seedHistory([], { matchCount: 0 });
    timeline.renderTimeline();
    expect(rowIds()).toEqual([]);
    expect(document.querySelector("#timeline")?.textContent).toContain(
      "no events match",
    );
  });
});

// happy-dom has no layout engine, so `clientHeight` is 0 and `scrollTop` may
// clamp; mock both so the windowed render is deterministic. When no viewport
// height is mocked, virtualization falls back to a full render of the loaded
// window.
function mockViewport(height: number, scrollTop = 0): HTMLElement {
  const list = document.querySelector("#timeline") as HTMLElement;
  Object.defineProperty(list, "clientHeight", {
    configurable: true,
    value: height,
  });
  Object.defineProperty(list, "scrollTop", {
    configurable: true,
    writable: true,
    value: scrollTop,
  });
  return list;
}

function manyEntries(n: number, from = 0): HistoryEntry[] {
  return Array.from({ length: n }, (_, i) =>
    entry(`e${from + i}`, {
      occurredAt: `unix-ms:${1782699185391 + from + i}`,
    }),
  );
}

describe("renderTimeline virtualization (a fully-loaded window)", () => {
  it("renders only the visible window of rows, not every entry", () => {
    seedHistory(manyEntries(500));
    mockViewport(500);
    timeline.renderTimeline();
    const rendered = rowIds().length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(500);
  });

  it("preserves total scroll geometry via spacers summing to the full list", () => {
    expect(timeline.ROW_H).toBeGreaterThan(0);
    seedHistory(manyEntries(500));
    mockViewport(500);
    timeline.renderTimeline();
    const spacerHeight = spacerHeights().reduce((sum, h) => sum + h, 0);
    const rowHeight = rowIds().length * timeline.ROW_H;
    expect(spacerHeight + rowHeight).toBe(500 * timeline.ROW_H);
  });

  it("re-renders a different window after scrolling down", () => {
    seedHistory(manyEntries(500));
    const list = mockViewport(500, 0);
    timeline.renderTimeline();
    const firstBefore = rowIds()[0];
    (list as unknown as { scrollTop: number }).scrollTop = 450 * timeline.ROW_H;
    list.dispatchEvent(new Event("scroll"));
    const firstAfter = rowIds()[0];
    expect(firstAfter).not.toBe(firstBefore);
  });

  it("renders every loaded row when no viewport height is known (full fallback)", () => {
    seedHistory(manyEntries(30));
    timeline.renderTimeline();
    expect(rowIds().length).toBe(30);
  });
});

describe("paged virtual timeline (server matchCount + offset window)", () => {
  it("sizes the virtual scroll height by matchCount, not the loaded count", () => {
    // 20 loaded rows at offset 0, but the matched set is 500.
    seedHistory(manyEntries(20), { offset: 0, matchCount: 500 });
    timeline.renderTimeline();
    const total = spacerHeights().reduce((sum, h) => sum + h, 0);
    const painted = rowIds().length * timeline.ROW_H;
    expect(Math.round((total + painted) / timeline.ROW_H)).toBe(500);
  });

  it("places the loaded window at its offset with a leading spacer", () => {
    seedHistory(manyEntries(20, 100), { offset: 100, matchCount: 500 });
    timeline.renderTimeline();
    // The first spacer covers the 100 rows above the loaded window.
    expect(Math.round(spacerHeights()[0] / timeline.ROW_H)).toBe(100);
  });

  it("requests the next page when the viewport nears the loaded edge", async () => {
    let lastUrl = "";
    const innerFetch = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/history")
        lastUrl = url;
      return innerFetch(input as RequestInfo, init);
    }) as typeof fetch;
    setHistoryResponse({
      entries: manyEntries(20, 20),
      diagnostics: [],
      offset: 20,
      matchCount: 100,
      facets: {},
    });
    try {
      seedHistory(manyEntries(20, 0), {
        offset: 0,
        matchCount: 100,
        // The default state (order desc) serializes to this key; a next-page fetch
        // under the same query merges rather than replacing the window.
        queryKey: "order=desc&limit=100",
      });
      mockViewport(500, 0);
      timeline.renderTimeline();
      await new Promise((resolve) => setTimeout(resolve, 0));
    } finally {
      globalThis.fetch = innerFetch;
    }
    // A next-page fetch for the rows past the loaded edge was issued and appended.
    expect(lastUrl).toContain("offset=20");
    expect(store.getState().history?.entries.length).toBe(40);
  });

  it("does not page past the end when the whole matched set is loaded", async () => {
    let fetched = false;
    const innerFetch = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/history")
        fetched = true;
      return innerFetch(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      seedHistory(manyEntries(20, 0), { offset: 0, matchCount: 20 });
      mockViewport(500, 0);
      timeline.renderTimeline();
      await new Promise((resolve) => setTimeout(resolve, 0));
    } finally {
      globalThis.fetch = innerFetch;
    }
    expect(fetched).toBe(false);
  });
});

describe("scrollTimelineSelectionIntoView (global-index scroller)", () => {
  it("scrolls a loaded off-screen row into the window using its global index", () => {
    seedHistory(manyEntries(40, 100), { offset: 100, matchCount: 500 });
    const list = mockViewport(500, 0);
    timeline.renderTimeline();
    const target = "e139"; // the last loaded row (global index 139)
    expect(
      document.querySelector(`#timeline li[data-event-id="${target}"]`),
    ).toBeNull();
    timeline.scrollTimelineSelectionIntoView(target);
    expect(list.scrollTop).toBeGreaterThan(0);
    expect(
      document.querySelector(`#timeline li[data-event-id="${target}"]`),
    ).not.toBeNull();
  });

  it("does nothing for an off-page id not in the loaded window", () => {
    seedHistory(manyEntries(20, 0), { offset: 0, matchCount: 500 });
    const list = mockViewport(500, 0);
    timeline.renderTimeline();
    timeline.scrollTimelineSelectionIntoView("e400");
    expect(list.scrollTop).toBe(0);
  });
});
