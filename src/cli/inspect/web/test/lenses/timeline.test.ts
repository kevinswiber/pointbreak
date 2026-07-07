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

  it("renders the event date above the clock in the time cell", () => {
    seedHistory([entry("e1")]);
    timeline.renderTimeline();
    const date = document.querySelector("#timeline li.event .event-date");
    expect(date?.textContent?.trim()).toBeTruthy();
    // the date line is distinct from the clock (HH:MM:SS.mmm)
    expect(date?.textContent).not.toContain(":");
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

  it("pills a fact superseded by a loaded sibling, and leaves the superseder unpilled", () => {
    seedHistory([
      entry("e-super", {
        eventType: "review_observation_recorded",
        summary: { observationId: "obs:new", supersedes: ["obs:old"] },
      }),
      entry("e-old", {
        eventType: "review_observation_recorded",
        summary: { observationId: "obs:old" },
      }),
    ]);
    timeline.renderTimeline();
    const oldPill = document.querySelector<HTMLElement>(
      '#timeline li[data-event-id="e-old"] .badge.superseded',
    );
    expect(oldPill?.textContent).toBe("superseded");
    // The still-current superseding fact carries no pill (advisory, additive).
    expect(
      document.querySelector(
        '#timeline li[data-event-id="e-super"] .badge.superseded',
      ),
    ).toBeNull();
  });

  it("labels a replaced assessment pill `replaced`", () => {
    seedHistory([
      entry("e-super", {
        eventType: "review_assessment_recorded",
        summary: { assessmentId: "assess:new", replaces: ["assess:old"] },
      }),
      entry("e-old", {
        eventType: "review_assessment_recorded",
        summary: { assessmentId: "assess:old" },
      }),
    ]);
    timeline.renderTimeline();
    const pill = document.querySelector<HTMLElement>(
      '#timeline li[data-event-id="e-old"] .badge.superseded',
    );
    expect(pill?.textContent).toBe("replaced");
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

  it("requests the previous page when the viewport nears the loaded start (post-reveal, offset > 0)", async () => {
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
      entries: manyEntries(20, 0),
      diagnostics: [],
      offset: 0,
      matchCount: 100,
      facets: {},
    });
    try {
      // A post-reveal window loaded at a non-zero offset; the same default query
      // key so the fetched previous page merges rather than replacing the window.
      seedHistory(manyEntries(20, 20), {
        offset: 20,
        matchCount: 100,
        queryKey: "order=desc&limit=100",
      });
      // scrollTop 0 puts the visible start at the leading edge (into the spacer).
      mockViewport(500, 0);
      timeline.renderTimeline();
      await new Promise((resolve) => setTimeout(resolve, 0));
    } finally {
      globalThis.fetch = innerFetch;
    }
    // A previous-page fetch back-filled the leading gap: the window grew and its
    // offset dropped, with the total match count unchanged.
    expect(lastUrl).toContain("offset=0");
    expect(store.getState().history?.entries.length).toBe(40);
    expect(store.getState().history?.offset).toBe(0);
    expect(store.getState().history?.matchCount).toBe(100);
  });

  it("does not page before the start when the window already begins at offset 0", async () => {
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
      // A window at offset 0 large enough that the trailing edge is far from the
      // viewport (so the forward branch stays quiet). Scrolled to the very top,
      // the leading-edge check must NOT fire a previous-page fetch (loadStart 0).
      seedHistory(manyEntries(60, 0), { offset: 0, matchCount: 100 });
      mockViewport(500, 0);
      timeline.renderTimeline();
      await new Promise((resolve) => setTimeout(resolve, 0));
    } finally {
      globalThis.fetch = innerFetch;
    }
    expect(fetched).toBe(false);
  });
});

// Real rows render taller than the built-in estimate when the master pane is
// narrow (the meta line wraps) or shorter under compact density; the lens
// re-derives its estimate from the painted rows' live layout. happy-dom has no
// layout engine, so real measurements come back 0 — stub the painted rows'
// rects to simulate a laid-out pane.
function stubPaintedRowHeights(height: number): void {
  for (const li of document.querySelectorAll<HTMLElement>(
    "#timeline li.event[data-event-id]",
  )) {
    vi.spyOn(li, "getBoundingClientRect").mockReturnValue({
      height,
    } as DOMRect);
  }
}

describe("measured row-height estimate", () => {
  it("derives the estimate from the mean painted-row height and re-derives the spacers", () => {
    seedHistory(manyEntries(500));
    mockViewport(500);
    timeline.renderTimeline();
    stubPaintedRowHeights(72);
    timeline.remeasureTimelineRows();
    expect(timeline.timelineRowHeight()).toBe(72);
    // The change repainted: the whole scroll track re-derives on the new
    // estimate (spacers exactly cover the off-window rows at 72px/row, which
    // also proves spacers and the empty state are never sampled).
    const spacerHeight = spacerHeights().reduce((sum, h) => sum + h, 0);
    expect(spacerHeight + rowIds().length * 72).toBe(500 * 72);
  });

  it("keeps the fallback estimate when layout yields no row heights", () => {
    seedHistory(manyEntries(500));
    mockViewport(500);
    timeline.renderTimeline();
    // No stubs: happy-dom rects measure 0 — the estimate must not collapse.
    timeline.remeasureTimelineRows();
    expect(timeline.timelineRowHeight()).toBe(timeline.ROW_H);
  });

  it("keeps the estimate when nothing is painted (the empty state)", () => {
    seedHistory([], { matchCount: 0 });
    timeline.renderTimeline();
    timeline.remeasureTimelineRows();
    expect(timeline.timelineRowHeight()).toBe(timeline.ROW_H);
  });

  it("reveals with the live estimate, not the built-in fallback", () => {
    seedHistory(manyEntries(40, 100), { offset: 100, matchCount: 500 });
    // Scrolled into the loaded window, so rows are painted and measurable.
    const list = mockViewport(500, 100 * timeline.ROW_H);
    timeline.renderTimeline();
    stubPaintedRowHeights(80);
    timeline.remeasureTimelineRows();
    timeline.scrollTimelineSelectionIntoView("e139");
    // Centered in the 80px-row model: global index 139.
    expect(list.scrollTop).toBe(139 * 80 - (500 - 80) / 2);
    expect(
      document.querySelector('#timeline li[data-event-id="e139"]'),
    ).not.toBeNull();
  });

  it("keeps the revealed row inside the viewport across the post-reveal repaint", () => {
    // The lost-cursor mechanism: reveal's final scrollIntoView fires a scroll
    // event whose repaint re-anchors rows to the spacer model. With a measured
    // estimate the model matches the real rows, so that repaint cannot push
    // the revealed row out of the viewport.
    seedHistory(manyEntries(500));
    const list = mockViewport(500, 0);
    timeline.renderTimeline();
    stubPaintedRowHeights(72);
    timeline.remeasureTimelineRows();
    timeline.scrollTimelineSelectionIntoView("e250");
    // Emulate the browser correction: center the row's painted position, then
    // fire the scroll event scrollIntoView produces.
    const rowTop = () =>
      spacerHeights()[0] +
      rowIds().indexOf("e250") * timeline.timelineRowHeight();
    (list as unknown as { scrollTop: number }).scrollTop =
      rowTop() - (500 - 72) / 2;
    list.dispatchEvent(new Event("scroll"));
    expect(rowIds()).toContain("e250");
    expect(rowTop()).toBeGreaterThanOrEqual(list.scrollTop);
    expect(rowTop() + 72).toBeLessThanOrEqual(list.scrollTop + 500);
  });

  it("preserves the viewport-top index across an estimate change (no layout fallback)", () => {
    // A new estimate re-derives the whole scroll track; without anchoring,
    // a fixed scrollTop lands on entirely different rows (the density toggle
    // teleports the reading position). With no row geometry to anchor to
    // (zero rects), the same global index must stay at the viewport top:
    // scrollTop scales with the estimate.
    seedHistory(manyEntries(500));
    const list = mockViewport(500, 100 * timeline.ROW_H);
    timeline.renderTimeline();
    stubPaintedRowHeights(72);
    timeline.remeasureTimelineRows();
    expect(list.scrollTop).toBe(100 * 72);
  });

  it("anchors to the real row at the viewport top when layout disagrees with the model", () => {
    // After a reveal, scrollIntoView corrects scrollTop against REAL row
    // heights, so scrollTop is no longer `index * estimate` — scaling it
    // would overshoot. The anchor must come from the painted DOM: the row
    // whose rect straddles the viewport top keeps its exact offset.
    seedHistory(manyEntries(500));
    const list = mockViewport(500, 100 * timeline.ROW_H);
    timeline.renderTimeline();
    vi.spyOn(list, "getBoundingClientRect").mockReturnValue({
      top: 0,
      bottom: 500,
      height: 500,
    } as DOMRect);
    // Lay the painted rows out at their real 72px heights, starting where the
    // leading spacer ends (the 52px model), relative to the mocked scrollTop.
    const spacerPx = spacerHeights()[0];
    const rows = Array.from(
      document.querySelectorAll<HTMLElement>(
        "#timeline li.event[data-event-id]",
      ),
    );
    rows.forEach((li, i) => {
      const top = spacerPx + i * 72 - list.scrollTop;
      vi.spyOn(li, "getBoundingClientRect").mockReturnValue({
        top,
        bottom: top + 72,
        height: 72,
      } as DOMRect);
    });
    const paintStart = Math.round(spacerPx / timeline.ROW_H);
    // The first row whose real bottom crosses the viewport top, and its offset.
    const topIdx = rows.findIndex(
      (li) => li.getBoundingClientRect().bottom > 0,
    );
    const topOffset = rows[topIdx].getBoundingClientRect().top;
    timeline.remeasureTimelineRows();
    expect(timeline.timelineRowHeight()).toBe(72);
    expect(list.scrollTop).toBe((paintStart + topIdx) * 72 - topOffset);
  });

  it("measures on the first laid-out paint, before any reveal runs", () => {
    // The load-time deep-link reveal runs right after the first paint; if it
    // computes on the fallback estimate, its own scrollIntoView-correction
    // repaint displaces the revealed row before any observer fires. The first
    // paint with real layout must seed the estimate itself.
    const rect = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockReturnValue({ top: 0, bottom: 72, height: 72 } as DOMRect);
    try {
      seedHistory(manyEntries(500));
      mockViewport(500);
      timeline.renderTimeline();
      expect(timeline.timelineRowHeight()).toBe(72);
    } finally {
      rect.mockRestore();
    }
  });

  it("re-measures at reveal time so a stale estimate cannot lose the target", () => {
    // The estimate can go stale between paints — the load path measures at
    // full width, then the same render pass opens the split and narrows the
    // pane before the deep-link reveal runs. Reveal must measure the (already
    // reflowed) painted rows itself before seeding the scroll position.
    seedHistory(manyEntries(500));
    const list = mockViewport(500, 0);
    timeline.renderTimeline();
    stubPaintedRowHeights(72);
    timeline.scrollTimelineSelectionIntoView("e250");
    expect(timeline.timelineRowHeight()).toBe(72);
    expect(list.scrollTop).toBe(250 * 72 - (500 - 72) / 2);
    expect(rowIds()).toContain("e250");
  });

  it("coalesces scheduled re-measures into one trailing measurement", () => {
    vi.useFakeTimers();
    try {
      seedHistory(manyEntries(500));
      mockViewport(500);
      timeline.renderTimeline();
      stubPaintedRowHeights(72);
      timeline.scheduleTimelineRemeasure();
      timeline.scheduleTimelineRemeasure();
      // Trailing, not immediate: a divider drag's burst must settle first.
      expect(timeline.timelineRowHeight()).toBe(timeline.ROW_H);
      vi.advanceTimersByTime(1000);
      expect(timeline.timelineRowHeight()).toBe(72);
    } finally {
      vi.useRealTimers();
    }
  });

  it("observes the timeline element's size (once) and re-measures on a change", () => {
    const observed: Element[] = [];
    let fireResize = () => {};
    class FakeResizeObserver {
      constructor(cb: () => void) {
        fireResize = cb;
      }
      observe(el: Element): void {
        observed.push(el);
      }
      unobserve(): void {}
      disconnect(): void {}
    }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.useFakeTimers();
    try {
      seedHistory(manyEntries(500));
      mockViewport(500);
      timeline.renderTimeline();
      timeline.renderTimeline();
      // One observer on the (stable) timeline element, same guard as the
      // scroll listener — covers divider release, window resize, the narrow
      // media query, pane open/close, and reading mode via one width signal.
      expect(observed).toEqual([document.querySelector("#timeline")]);
      stubPaintedRowHeights(64);
      fireResize();
      vi.advanceTimersByTime(1000);
      expect(timeline.timelineRowHeight()).toBe(64);
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
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
