import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import historyJson from "./fixtures/history.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetFreshnessResponse,
  setFreshnessResponse,
  uninstallFetchMock,
} from "./support/fetch";

// The data layer loads the `/api/*` documents, builds each timeline entry's
// search index, and commits the payloads to the store — it never calls render
// (the store subscriber repaints). These tests drive it against the fixture fetch
// mock and read the resulting store state. Store and data are module singletons
// sharing one `state`, so reset the registry and remount the DOM each test.
type Store = typeof import("../src/store");
type Data = typeof import("../src/data");
let store: Store;
let data: Data;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
  data = await import("../src/data");
  mountInspectorDom();
  installFetchMock();
});

afterEach(() => {
  uninstallFetchMock();
  resetFreshnessResponse();
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

/** Let all pending microtasks / the load chain settle. */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
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
      await expect(data.load()).resolves.toBeUndefined();
      const el = document.querySelector("#error");
      expect(el?.classList.contains("hidden")).toBe(false);
      expect(el?.textContent).toContain("network down");
    } finally {
      globalThis.fetch = restore;
    }
  });
});

describe("pollFreshness", () => {
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
    await data.pollFreshness();
    const refresh = document.querySelector("#refresh");
    expect(refresh?.getAttribute("data-state")).toBe("updated");
    // The reload re-seeded the baseline from the freshness probe (the new marker),
    // so a subsequent poll at the same marker reports unchanged — no reload loop.
    expect(store.getState().lastEventCount).toBe(HISTORY_EVENT_COUNT + 1);
  });

  it("does not reload when a non-marker field changes but the marker is steady", async () => {
    await data.load();
    // The poll keys ONLY on eventCount; an eventSetHash/diagnosticCount that moves
    // while the marker holds must not trigger a reload (the old diagnostic-count
    // key would have looped forever against a store carrying diagnostics).
    setFreshnessResponse({
      eventCount: HISTORY_EVENT_COUNT,
      eventSetHash: "sha256:changed",
      diagnosticCount: 3,
    });
    await data.pollFreshness();
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "watching",
    );
  });

  it("marks the indicator stalled when the freshness probe fails", async () => {
    await data.load();
    const restore = globalThis.fetch;
    globalThis.fetch = () => Promise.reject(new Error("offline"));
    try {
      await data.pollFreshness();
      const refresh = document.querySelector("#refresh");
      expect(refresh?.getAttribute("data-state")).toBe("stalled");
      // Degraded: the word surfaces beside the chip so a stall is noticed.
      expect(document.querySelector("#refresh-word")?.textContent).toBe(
        "stalled",
      );
    } finally {
      globalThis.fetch = restore;
    }
  });
});

describe("setLiveness", () => {
  it("drives the dot state + title and clears the degraded word on recovery", () => {
    data.setLiveness("stalled");
    expect(document.querySelector("#refresh-word")?.textContent).toBe(
      "stalled",
    );
    data.setLiveness("watching");
    const dot = document.querySelector("#refresh");
    expect(dot?.getAttribute("data-state")).toBe("watching");
    expect(dot?.getAttribute("title")).toBe("Auto-refresh: watching");
    expect(document.querySelector("#refresh-word")?.textContent).toBe("");
    // The detail-popover line mirrors the word AND the dot's state (for color).
    const line = document.querySelector("#stat-live");
    expect(line?.textContent).toBe("watching");
    expect(line?.getAttribute("data-state")).toBe("watching");
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
