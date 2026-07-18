import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import attentionJson from "./fixtures/attention.json";
import historyJson from "./fixtures/history.json";
import revisionJson from "./fixtures/revision.json";
import revisionsJson from "./fixtures/revisions.json";
import threadsJson from "./fixtures/threads.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetAttentionResponse,
  resetCompositeResponse,
  resetFreshnessResponse,
  resetHistoryResponse,
  resetNewCountResponse,
  resetRevisionsResponse,
  setAttentionResponse,
  setCompositeResponse,
  setFreshnessResponse,
  setHistoryError,
  setHistoryResponse,
  setNewCountResponse,
  setRevisionsResponse,
  uninstallFetchMock,
} from "./support/fetch";

// The data layer loads the `/api/*` documents, builds each timeline entry's
// search index, and commits the payloads to the store — it never calls render
// (the store subscriber repaints). These tests drive it against the fixture fetch
// mock and read the resulting store state. Store and data are module singletons
// sharing one `state`, so reset the registry and remount the DOM each test.
type Store = typeof import("../src/store");
type Data = typeof import("../src/data");
type Follow = typeof import("../src/follow");
type Router = typeof import("../src/router");
let store: Store;
let data: Data;
let follow: Follow;
let router: Router;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
  data = await import("../src/data");
  follow = await import("../src/follow");
  router = await import("../src/router");
  mountInspectorDom();
  installFetchMock();
});

afterEach(() => {
  uninstallFetchMock();
  resetFreshnessResponse();
  resetAttentionResponse();
  resetCompositeResponse();
  resetHistoryResponse();
  resetNewCountResponse();
  resetRevisionsResponse();
  resetDom();
});

// history.json's eventCount, the marker the freshness baseline seeds from.
const HISTORY_EVENT_COUNT = 8;

// Capture the most recent `/api/history` request URL so a test can assert the
// query the loader sent. Wraps the fixture mock (already installed in beforeEach).
let lastHistoryUrl = "";
function captureHistoryUrls(): () => void {
  const inner = globalThis.fetch;
  lastHistoryUrl = "";
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    if (new URL(url, "http://inspector.test").pathname === "/api/history")
      lastHistoryUrl = url;
    return inner(input as RequestInfo, init);
  }) as typeof fetch;
  return () => {
    globalThis.fetch = inner;
  };
}

// Capture request paths during a focused interaction. The fixture mock still
// serves responses; this just records the network shape the data layer chose.
function captureRequestPaths(): { paths: string[]; restore: () => void } {
  const inner = globalThis.fetch;
  const paths: string[] = [];
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    paths.push(new URL(url, "http://inspector.test").pathname);
    return inner(input as RequestInfo, init);
  }) as typeof fetch;
  return {
    paths,
    restore: () => {
      globalThis.fetch = inner;
    },
  };
}

/** Let all pending microtasks / the load chain settle. */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function deferredResponse(payload: unknown): {
  promise: Promise<Response>;
  resolve: () => void;
} {
  let resolve!: () => void;
  const promise = new Promise<Response>((done) => {
    resolve = () =>
      done(
        new Response(JSON.stringify(payload), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
  });
  return { promise, resolve };
}

describe("load", () => {
  it("commits history, revisions, and objects to the store", async () => {
    await data.load();
    const s = store.getState();
    expect(s.history?.entries.length).toBe(8);
    expect(s.revisions?.entries.length).toBe(1);
    expect(s.threads?.threads.length).toBe(1);
  });

  it("seeds the freshness baseline from the event-count marker", async () => {
    await data.load();
    const s = store.getState();
    expect(s.lastEventCount).toBe(HISTORY_EVENT_COUNT);
    expect(s.lastCommitGraphStamp).toBe("stamp-fixture");
  });

  it("reads the freshness marker before the documents (baseline can't outrun the docs)", async () => {
    // If the marker were fetched in parallel with (or after) the documents, an
    // append landing mid-load could seed a baseline newer than the committed docs,
    // and the next poll would report "unchanged" forever. The marker must be read
    // first, so the baseline is never newer than what was loaded.
    const order: string[] = [];
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      order.push(new URL(url, "http://inspector.test").pathname);
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      await data.load();
    } finally {
      globalThis.fetch = inner;
    }
    const freshnessAt = order.indexOf("/api/freshness");
    const historyAt = order.indexOf("/api/history");
    expect(freshnessAt).toBeGreaterThanOrEqual(0);
    expect(freshnessAt).toBeLessThan(historyAt);
  });

  it("fetches page 1 of /api/history for the current query, never the full set", async () => {
    const restore = captureHistoryUrls();
    try {
      store.commit({ filterText: "pinned", order: "desc" });
      await data.load();
    } finally {
      restore();
    }
    expect(lastHistoryUrl).toMatch(/\/api\/history\?/);
    expect(lastHistoryUrl).toContain("q=pinned");
    expect(lastHistoryUrl).toContain("order=desc");
    expect(lastHistoryUrl).toContain("limit=");
    // The server owns the haystack now — the loader builds no client index.
    expect(
      store.getState().history?.entries.every((e) => e.__search === undefined),
    ).toBe(true);
  });

  it("commits the server facets, matchCount, and offset onto the store", async () => {
    await data.load();
    const s = store.getState();
    expect(s.history?.facets).toEqual(
      (historyJson as unknown as { facets: Record<string, number> }).facets,
    );
    expect(s.history?.matchCount).toBe(8);
    expect(s.history?.offset).toBe(0);
  });

  it("stamps the loaded page with the query key it was fetched under", async () => {
    store.commit({ filterText: "pinned" });
    await data.load();
    expect(store.getState().history?.queryKey).toContain("q=pinned");
  });

  it("commits the history page before revisions and threads finish loading", async () => {
    const revisions = deferredResponse(revisionsJson);
    const threads = deferredResponse(threadsJson);
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = new URL(url, "http://inspector.test").pathname;
      if (path === "/api/revisions") return revisions.promise;
      if (path === "/api/threads") return threads.promise;
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      const loading = data.load();
      await flush();
      expect(store.getState().history?.entries.length).toBe(8);
      expect(store.getState().lastEventCount).toBe(HISTORY_EVENT_COUNT);
      expect(store.getState().revisions).toBeNull();
      expect(store.getState().threads).toBeNull();

      revisions.resolve();
      threads.resolve();
      await loading;
      expect(store.getState().revisions?.entries.length).toBe(1);
      expect(store.getState().threads?.threads.length).toBe(1);
    } finally {
      globalThis.fetch = inner;
    }
  });

  it("recovers when the query changes during the initial history request", async () => {
    const firstHistory = deferredResponse(historyJson);
    let historyRequested!: () => void;
    const requested = new Promise<void>((resolve) => {
      historyRequested = resolve;
    });
    let deferFirstHistory = true;
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (
        deferFirstHistory &&
        new URL(url, "http://inspector.test").pathname === "/api/history"
      ) {
        deferFirstHistory = false;
        historyRequested();
        return firstHistory.promise;
      }
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    store.subscribe(data.maybeReloadForQuery);
    try {
      const loading = data.load();
      await requested;
      store.commit({ filterText: "pinned" });
      firstHistory.resolve();
      await loading;
      await flush();
    } finally {
      globalThis.fetch = inner;
    }

    const state = store.getState();
    expect(state.history).not.toBeNull();
    expect(state.history?.queryKey).toBe(data.historyQueryParams(state));
    expect(state.history?.queryKey).toContain("q=pinned");
  });

  it("a poll reload re-fetches page 1 of the CURRENT query", async () => {
    await data.load();
    const restore = captureHistoryUrls();
    try {
      store.commit({ filterText: "needle" });
      await data.load();
    } finally {
      restore();
    }
    expect(lastHistoryUrl).toContain("q=needle");
  });

  it("a query change triggers a page-1 fetch via the query watcher", async () => {
    await data.load();
    const restore = captureHistoryUrls();
    try {
      store.subscribe(data.maybeReloadForQuery);
      store.commit({ filterText: "pinned" });
      await flush();
    } finally {
      restore();
    }
    expect(lastHistoryUrl).toContain("q=pinned");
  });

  it("a query change does not re-fetch revisions or threads", async () => {
    await data.load();
    const revisions = store.getState().revisions;
    const threads = store.getState().threads;
    const { paths, restore } = captureRequestPaths();
    try {
      store.subscribe(data.maybeReloadForQuery);
      store.commit({ filterText: "pinned" });
      await flush();
    } finally {
      restore();
    }
    expect(paths).toContain("/api/history");
    expect(paths).not.toContain("/api/freshness");
    expect(paths).not.toContain("/api/revisions");
    expect(paths).not.toContain("/api/threads");
    expect(store.getState().revisions).toBe(revisions);
    expect(store.getState().threads).toBe(threads);
  });

  it("a failed query reload does not retry in a tight loop", async () => {
    await data.load();
    const inner = globalThis.fetch;
    let historyRequests = 0;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/history") {
        historyRequests += 1;
        return Promise.reject(new Error("history offline"));
      }
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      store.subscribe(data.maybeReloadForQuery);
      store.commit({ filterText: "pinned" });
      await flush();
      await flush();
    } finally {
      globalThis.fetch = inner;
    }
    expect(historyRequests).toBe(1);
    expect(document.querySelector("#error")?.textContent).toContain(
      "server unavailable",
    );
  });

  it("the query watcher does not loop when the query key already matches", async () => {
    await data.load();
    // Mirror render's type-toggle seeding: every present type enabled, so the
    // active query serializes with no `type=` param and matches the stamped key.
    const present = new Set(
      (store.getState().history?.entries ?? []).map((e) => e.eventType),
    );
    store.commit({ enabledTypes: present });
    const restore = captureHistoryUrls();
    try {
      store.subscribe(data.maybeReloadForQuery);
      // A commit that leaves the query untouched must not re-fetch.
      store.commit({ selected: { kind: "event", id: "evt:x" } });
      await flush();
    } finally {
      restore();
    }
    expect(lastHistoryUrl).toBe("");
  });

  it("does not paint the master pane itself — the store subscriber repaints", async () => {
    const master = document.querySelector("#master");
    await data.load();
    expect(master?.innerHTML).toBe("");
  });

  it("clears any prior error after a successful load", async () => {
    data.showError("stale");
    await data.load();
    const el = document.querySelector("#error");
    expect(el?.classList.contains("hidden")).toBe(true);
  });

  it("surfaces a load failure in #error instead of throwing", async () => {
    const restore = globalThis.fetch;
    globalThis.fetch = () => Promise.reject(new Error("network down"));
    try {
      await expect(data.load()).resolves.toBe(false);
      const el = document.querySelector("#error");
      expect(el?.classList.contains("hidden")).toBe(false);
      expect(el?.textContent).toContain("server unavailable");
    } finally {
      globalThis.fetch = restore;
    }
  });

  it("parking freezes the anchor without changing follow intent", async () => {
    await data.load();
    const head = store.getState().history?.entries?.[0];

    follow.parkTimelineRead();

    const s = store.getState();
    expect(s.followByLens.timeline).toBe(true);
    expect(s.timelineHeadAnchor).toEqual({
      occurredAt: head?.occurredAt,
      eventId: head?.eventId,
    });
  });

  it("catch-up reloads the head and clears stream position without clearing selection", async () => {
    await data.load();
    const selected = store.getState().history?.entries?.[2]?.eventId ?? null;
    store.commit({
      selected: { kind: "event", id: selected },
      open: true,
    });
    follow.parkTimelineRead();
    store.commit({ timelineNewCount: 7 });

    await follow.catchUpTimeline();

    const s = store.getState();
    expect(s.followByLens.timeline).toBe(true);
    expect(s.timelineHeadAnchor).toBeNull();
    expect(s.timelineNewCount).toBe(0);
    expect(s.selected.id).toBe(selected);
  });

  it("catch-up replaces shifted offsets while retaining an off-page selection", async () => {
    await data.load();
    const parked = {
      eventId: "evt:sha256:parked-selection",
      eventType: "review_observation_recorded",
      occurredAt: "2026-07-01T10:00:00.000Z",
    };
    const loaded = store.getState().history;
    if (!loaded) throw new Error("expected loaded history");
    store.commit({
      history: { ...loaded, entries: [parked], offset: 200 },
      selected: { kind: "event", id: parked.eventId },
      open: true,
    });
    follow.parkTimelineRead();
    const newHead = {
      eventId: "evt:sha256:new-live-head",
      eventType: "review_observation_recorded",
      occurredAt: "2026-07-13T21:00:00.000Z",
    };
    setHistoryResponse({
      entries: [newHead],
      diagnostics: [],
      offset: 0,
      matchCount: 1,
      facets: {},
    });

    await follow.catchUpTimeline();

    const state = store.getState();
    expect(state.history?.entries).toEqual([newHead]);
    expect(state.history?.retainedEntry).toEqual(parked);
    expect(state.selected.id).toBe(parked.eventId);
  });

  it("a failed catch-up leaves the reader parked with the count intact", async () => {
    await data.load();
    follow.parkTimelineRead();
    store.commit({ timelineNewCount: 7 });
    setHistoryError(503, "history offline");

    await follow.catchUpTimeline();

    const s = store.getState();
    expect(s.followByLens.timeline).toBe(true);
    expect(s.timelineNewCount).toBe(7);
  });

  it("re-enabling a parked follow probes unseen events without clearing selection", async () => {
    await data.load();
    const selected = store.getState().history?.entries?.[2]?.eventId ?? null;
    store.commit({ selected: { kind: "event", id: selected }, open: true });
    follow.parkTimelineRead();
    await follow.toggleTimelineFollow();
    setNewCountResponse({
      schema: "pointbreak.inspect-history-new-count",
      newCount: 4,
    });

    await follow.toggleTimelineFollow();

    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineNewCount).toBe(4);
    expect(store.getState().selected.id).toBe(selected);
  });

  it("an in-window sel application parks without changing follow intent", async () => {
    await data.load();
    const head = store.getState().history?.entries?.[0];
    const selected = store.getState().history?.entries?.[2]?.eventId;
    history.replaceState(
      null,
      "",
      `#/timeline?sel=${encodeURIComponent(selected ?? "")}`,
    );

    router.applyHash();

    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineHeadAnchor).toEqual({
      occurredAt: head?.occurredAt,
      eventId: head?.eventId,
    });
  });

  it("a query change with no engaged selection preserves disabled follow", async () => {
    await data.load();
    await follow.toggleTimelineFollow();
    store.subscribe(data.maybeReloadForQuery);

    router.navigate({ filterText: "pinned" }, { replace: true });
    await flush();

    const s = store.getState();
    expect(s.followByLens.timeline).toBe(false);
    expect(s.timelineHeadAnchor).toBeNull();
    expect(s.timelineNewCount).toBe(0);
  });

  it("a query change with an engaged selection re-parks without changing follow intent", async () => {
    await data.load();
    const selected = store.getState().history?.entries?.[2]?.eventId ?? null;
    store.commit({ selected: { kind: "event", id: selected }, open: true });
    const detail = await import("../src/detail");
    detail.renderDetail();
    expect(document.querySelector("#detail-body")?.textContent).toContain(
      selected,
    );
    follow.parkTimelineRead();
    store.commit({ timelineNewCount: 7 });
    const newHead = {
      eventId: "evt:sha256:new-query-head",
      eventType: "review_observation_recorded",
      occurredAt: "2026-07-13T20:00:00Z",
    };
    setHistoryResponse({
      ...(historyJson as unknown as Record<string, unknown>),
      entries: [newHead],
      offset: 0,
      matchCount: 1,
    });
    store.subscribe(data.maybeReloadForQuery);

    router.navigate({ filterText: "pinned" }, { replace: true });
    await flush();

    const s = store.getState();
    expect(s.followByLens.timeline).toBe(true);
    expect(s.selected).toEqual({ kind: "event", id: selected });
    expect(s.timelineHeadAnchor).toEqual({
      occurredAt: newHead.occurredAt,
      eventId: newHead.eventId,
    });
    expect(s.timelineNewCount).toBe(0);
    detail.renderDetail();
    expect(document.querySelector("#detail-body")?.textContent).toContain(
      selected,
    );
  });
});

describe("pollFreshness", () => {
  it("computes a signed attention delta across a poll reload", async () => {
    await data.load();
    const fewer = {
      ...(attentionJson as unknown as Record<string, unknown>),
      items: attentionJson.items.slice(0, -2),
    };
    setAttentionResponse(fewer);
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });

    await data.pollFreshness();

    expect(store.getState().attentionDelta).toBe(-2);
  });

  it("clears the attention delta on the liveness settle", async () => {
    await data.load();
    setAttentionResponse({
      ...(attentionJson as unknown as Record<string, unknown>),
      items: attentionJson.items.slice(0, -1),
    });
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    vi.useFakeTimers();
    try {
      await data.pollFreshness();
      expect(store.getState().attentionDelta).toBe(-1);
      vi.advanceTimersByTime(1200);
      expect(store.getState().attentionDelta).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("an unchanged tick leaves the attention delta null", async () => {
    await data.load();
    await data.pollFreshness();
    expect(store.getState().attentionDelta).toBeNull();
  });

  it("preserves the revisions scroll position across a poll reload", async () => {
    await data.load();
    const units = document.createElement("div");
    units.id = "units";
    units.scrollTop = 120;
    document.querySelector("#master")?.appendChild(units);
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });

    await data.pollFreshness();

    // happy-dom has no layout/repaint scroll reset; this remains a contract pin
    // while the live browser gate verifies the observable behavior.
    expect(document.querySelector<HTMLElement>("#units")?.scrollTop).toBe(120);
  });

  it("a followed parked tick probes new-count instead of refetching page 1", async () => {
    await data.load();
    follow.parkTimelineRead();
    setNewCountResponse({
      schema: "pointbreak.inspect-history-new-count",
      newCount: 4,
    });
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(paths).toContain("/api/history/new-count");
    expect(paths).not.toContain("/api/history");
    expect(store.getState().timelineNewCount).toBe(4);
  });

  it("the parked probe sends the frozen anchor and active filter params", async () => {
    store.commit({ filterText: "pinned" });
    await data.load();
    follow.parkTimelineRead();
    const anchor = store.getState().timelineHeadAnchor;
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    let newCountUrl = "";
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (
        new URL(url, "http://inspector.test").pathname ===
        "/api/history/new-count"
      )
        newCountUrl = url;
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      await data.pollFreshness();
    } finally {
      globalThis.fetch = inner;
    }
    const target = new URL(newCountUrl, "http://inspector.test");
    expect(target.searchParams.get("sinceOccurredAt")).toBe(anchor?.occurredAt);
    expect(target.searchParams.get("sinceEventId")).toBe(anchor?.eventId);
    expect(target.searchParams.get("q")).toBe("pinned");
    expect(target.searchParams.has("limit")).toBe(false);
  });

  it("a following changed tick replaces the head without creating an anchor", async () => {
    await data.load();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(paths).toContain("/api/history");
    expect(paths).not.toContain("/api/history/new-count");
    expect(store.getState().timelineHeadAnchor).toBeNull();
  });

  it("an in-flight following reload cannot replace history after the reader parks", async () => {
    await data.load();
    const selected = store.getState().history?.entries?.[2]?.eventId ?? null;
    const deferred = deferredResponse({
      ...(historyJson as unknown as Record<string, unknown>),
      entries: [
        {
          eventId: "evt:sha256:late-poll-head",
          eventType: "review_observation_recorded",
          occurredAt: "2026-07-13T21:00:00Z",
        },
      ],
    });
    let historyRequested!: () => void;
    const requested = new Promise<void>((resolve) => {
      historyRequested = resolve;
    });
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/history") {
        historyRequested();
        return deferred.promise;
      }
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    try {
      const poll = data.pollFreshness();
      await requested;
      follow.parkTimelineRead();
      store.commit({ selected: { kind: "event", id: selected }, open: true });
      deferred.resolve();
      await poll;
    } finally {
      globalThis.fetch = inner;
    }

    expect(store.getState().followByLens.timeline).toBe(true);
    expect(
      store
        .getState()
        .history?.entries.some((entry) => entry.eventId === selected),
    ).toBe(true);
    expect(store.getState().selected.id).toBe(selected);
  });

  it("a stale parked count response cannot repopulate the count after catch-up", async () => {
    await data.load();
    follow.parkTimelineRead();
    const deferred = deferredResponse({
      schema: "pointbreak.inspect-history-new-count",
      newCount: 7,
    });
    let probeRequested!: () => void;
    const requested = new Promise<void>((resolve) => {
      probeRequested = resolve;
    });
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (
        new URL(url, "http://inspector.test").pathname ===
        "/api/history/new-count"
      ) {
        probeRequested();
        return deferred.promise;
      }
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    try {
      const poll = data.pollFreshness();
      await requested;
      await follow.catchUpTimeline();
      deferred.resolve();
      await poll;
    } finally {
      globalThis.fetch = inner;
    }

    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineNewCount).toBe(0);
  });

  it("does not probe or replace while parked under ascending order", async () => {
    await data.load();
    follow.parkTimelineRead();
    store.commit({ order: "asc", timelineNewCount: 0 });
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(paths).not.toContain("/api/history");
    expect(paths).not.toContain("/api/history/new-count");
    expect(store.getState().timelineNewCount).toBe(0);
  });

  it("does not probe or replace while follow is explicitly disabled", async () => {
    await data.load();
    await follow.toggleTimelineFollow();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(paths).not.toContain("/api/history");
    expect(paths).not.toContain("/api/history/new-count");
    expect(store.getState().followByLens.timeline).toBe(false);
    expect(store.getState().timelineNewCount).toBe(0);
  });

  it("preserves a parked-away window and its selection on a changed tick", async () => {
    await data.load();
    const history = store.getState().history;
    if (!history) throw new Error("expected load to commit history");
    store.commit({
      history: {
        ...history,
        offset: 200,
        entries: [
          {
            eventId: "evt:sha256:parked",
            eventType: "review_observation_recorded",
            occurredAt: "2026-07-01T10:00:00.000Z",
          },
        ],
      },
      selected: { kind: "event", id: "evt:sha256:parked" },
      open: true,
    });
    follow.parkTimelineRead();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });

    await data.pollFreshness();

    const s = store.getState();
    expect(s.history?.offset).toBe(200);
    expect(
      s.history?.entries?.some(
        (entry) => entry.eventId === "evt:sha256:parked",
      ),
    ).toBe(true);
    expect(s.selected).toEqual({ kind: "event", id: "evt:sha256:parked" });
    expect(s.open).toBe(true);
  });

  it("still replaces page 1 when the window is at the head", async () => {
    await data.load();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(paths).toContain("/api/history");
  });

  it("does not advance the baseline past whole documents during a head reload", async () => {
    await data.load();
    const inner = globalThis.fetch;
    let freshnessRequests = 0;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/freshness") {
        freshnessRequests += 1;
        return Promise.resolve(
          new Response(
            JSON.stringify({
              schema: "pointbreak.inspect-freshness",
              version: 1,
              eventCount: HISTORY_EVENT_COUNT + freshnessRequests,
              commitGraphStamp: "stamp-fixture",
            }),
            { status: 200, headers: { "content-type": "application/json" } },
          ),
        );
      }
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      await data.pollFreshness();
    } finally {
      globalThis.fetch = inner;
    }
    expect(freshnessRequests).toBe(2);
    expect(store.getState().lastEventCount).toBe(HISTORY_EVENT_COUNT + 1);
  });

  it("refreshes the whole documents and re-seeds the baseline even when parked", async () => {
    await data.load();
    const history = store.getState().history;
    if (!history) throw new Error("expected load to commit history");
    store.commit({
      history: { ...history, offset: 200 },
    });
    follow.parkTimelineRead();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(paths).toContain("/api/revisions");
    expect(paths).toContain("/api/attention");
    expect(paths).not.toContain("/api/history");
    expect(store.getState().lastEventCount).toBe(HISTORY_EVENT_COUNT + 1);
  });

  it("repaints an open revision composite from a parked whole-document refresh", async () => {
    const before = structuredClone(revisionJson);
    before.observations[0].title = "Before parked poll";
    setCompositeResponse(before);
    await data.load();

    const revisionId = revisionsJson.entries[0].revisionId;
    const render = await import("../src/render");
    store.commit({
      lens: "list",
      selected: { kind: "revision", id: revisionId },
      open: true,
    });
    store.subscribe(render.render);
    render.render();
    await flush();
    expect(document.querySelector("#detail-body")?.textContent).toContain(
      "Before parked poll",
    );
    const detailPane = document.querySelector<HTMLElement>("#detail");
    if (!detailPane) throw new Error("expected detail pane");
    detailPane.scrollTop = 120;

    const after = structuredClone(revisionJson);
    after.observations[0].title = "After parked poll";
    setCompositeResponse(after);
    setRevisionsResponse({
      ...revisionsJson,
      eventSetHash: "sha256:after-parked-poll",
    });
    follow.parkTimelineRead();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
      await flush();
    } finally {
      restore();
    }

    expect(paths).toContain("/api/revisions");
    expect(paths).not.toContain("/api/history");
    expect(document.querySelector("#detail-body")?.textContent).toContain(
      "After parked poll",
    );
    expect(detailPane.scrollTop).toBe(120);
  });

  it("repaints an open revision composite after a commit-graph-only refresh", async () => {
    const before = structuredClone(revisionJson);
    before.observations[0].title = "Before commit-graph poll";
    setCompositeResponse(before);
    await data.load();

    const revisionId = revisionsJson.entries[0].revisionId;
    const render = await import("../src/render");
    store.commit({
      lens: "list",
      selected: { kind: "revision", id: revisionId },
      open: true,
    });
    store.subscribe(render.render);
    render.render();
    await flush();
    expect(document.querySelector("#detail-body")?.textContent).toContain(
      "Before commit-graph poll",
    );

    const after = structuredClone(revisionJson);
    after.observations[0].title = "After commit-graph poll";
    setCompositeResponse(after);
    follow.parkTimelineRead();
    setFreshnessResponse({
      eventCount: HISTORY_EVENT_COUNT,
      commitGraphStamp: "stamp-after-landing",
    });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
      await flush();
    } finally {
      restore();
    }

    expect(paths).toContain("/api/revisions");
    expect(paths).not.toContain("/api/history");
    expect(store.getState().lastCommitGraphStamp).toBe("stamp-after-landing");
    expect(document.querySelector("#detail-body")?.textContent).toContain(
      "After commit-graph poll",
    );
  });

  it("marks the refresh indicator watching when nothing changed", async () => {
    await data.load();
    await data.pollFreshness();
    const refresh = document.querySelector("#refresh");
    expect(refresh?.getAttribute("data-state")).toBe("watching");
    // Healthy: the degraded word stays empty and the detail line reads watching.
    expect(document.querySelector("#refresh-word")?.textContent).toBe("");
    expect(document.querySelector("#stat-live")?.textContent).toBe("watching");
  });

  it("reloads and flags the indicator when the event-count marker changed", async () => {
    await data.load();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    const refresh = document.querySelector("#refresh");
    expect(refresh?.getAttribute("data-state")).toBe("updated");
    expect(paths).toContain("/api/revisions");
    expect(paths).toContain("/api/threads");
    // The reload re-seeded the baseline from the freshness probe (the new marker),
    // so a subsequent poll at the same marker reports unchanged — no reload loop.
    expect(store.getState().lastEventCount).toBe(HISTORY_EVENT_COUNT + 1);
  });

  it("reloads when the commit-graph stamp moved while the marker held", async () => {
    await data.load();
    // A pure-git landing (a fast-forward) flips revision merge statuses without
    // appending an event: the marker holds but the stamp moves, and the poll
    // must refetch (#467).
    setFreshnessResponse({
      eventCount: HISTORY_EVENT_COUNT,
      commitGraphStamp: "stamp-moved",
    });
    const { paths, restore } = captureRequestPaths();
    try {
      await data.pollFreshness();
    } finally {
      restore();
    }
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "updated",
    );
    expect(paths).toContain("/api/revisions");
    // The reload re-seeded the stamp baseline, so the next poll at the same
    // stamp reports unchanged — no reload loop.
    expect(store.getState().lastCommitGraphStamp).toBe("stamp-moved");
  });

  it("reloads when a stamp first appears after a degraded load, then tracks it", async () => {
    // A degraded load (server could not derive the stamp) seeds a null
    // baseline, and the documents were fetched under an UNKNOWN git state —
    // git may have moved during the outage. The first stamped poll must
    // therefore RELOAD (re-seeding the baseline through load()), not silently
    // adopt a stamp the displayed data may predate; a steady stamp afterwards
    // reports unchanged.
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT });
    await data.load();
    expect(store.getState().lastCommitGraphStamp).toBeNull();

    setFreshnessResponse({
      eventCount: HISTORY_EVENT_COUNT,
      commitGraphStamp: "stamp-recovered",
    });
    await data.pollFreshness();
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "updated",
    );
    // The reload's own freshness fetch re-seeded the baseline.
    expect(store.getState().lastCommitGraphStamp).toBe("stamp-recovered");

    await data.pollFreshness();
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "watching",
    );
  });

  it("does not reload when the stamp is omitted while the marker is steady", async () => {
    await data.load();
    // A transient server-side stamp failure omits the field. Absence is not a
    // signal — flapping omit↔value must not fire reloads.
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT });
    await data.pollFreshness();
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "watching",
    );
  });

  it("does not reload when a non-key field changes but marker and stamp are steady", async () => {
    await data.load();
    // The poll keys ONLY on eventCount + commitGraphStamp; an eventSetHash/
    // diagnosticCount that moves while both hold must not trigger a reload (the
    // old diagnostic-count key would have looped forever against a store
    // carrying diagnostics).
    setFreshnessResponse({
      eventCount: HISTORY_EVENT_COUNT,
      commitGraphStamp: "stamp-fixture",
      eventSetHash: "sha256:changed",
      diagnosticCount: 3,
    });
    await data.pollFreshness();
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "watching",
    );
  });

  it("marks refresh degraded and connection unreachable when the probe fails", async () => {
    await data.load();
    const restore = globalThis.fetch;
    globalThis.fetch = () => Promise.reject(new Error("offline"));
    try {
      await data.pollFreshness();
      const refresh = document.querySelector("#refresh");
      expect(refresh?.getAttribute("data-state")).toBe("degraded");
      expect(refresh?.getAttribute("data-connection")).toBe("unreachable");
      expect(document.querySelector("#refresh-word")?.textContent).toBe(
        "server unavailable",
      );
    } finally {
      globalThis.fetch = restore;
    }
  });
});

describe("showError", () => {
  it("shows a prefixed error message in #error", () => {
    data.showError("disk on fire");
    const el = document.querySelector("#error");
    expect(el?.classList.contains("hidden")).toBe(false);
    expect(el?.textContent).toBe("error: disk on fire");
  });

  it("hides and clears #error when given no message", () => {
    data.showError("x");
    data.showError(null);
    const el = document.querySelector("#error");
    expect(el?.classList.contains("hidden")).toBe(true);
    expect(el?.textContent).toBe("");
  });
});
