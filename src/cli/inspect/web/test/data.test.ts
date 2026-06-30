import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

// The single revision/object the committed fixtures describe.
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
// history.json's eventCount, the marker the freshness baseline seeds from.
const HISTORY_EVENT_COUNT = 8;

describe("load", () => {
  it("commits history, revisions, and objects to the store", async () => {
    await data.load();
    const s = store.getState();
    expect(s.history?.entries.length).toBe(8);
    expect(s.revisions?.entries.length).toBe(1);
    expect(s.objects?.threads.length).toBe(1);
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

  it("indexes every entry before committing — a subscriber never sees an un-indexed entry", async () => {
    const indexedAtEachNotification: boolean[] = [];
    store.subscribe(() => {
      const entries = store.getState().history?.entries ?? [];
      indexedAtEachNotification.push(
        entries.every((e) => e.__search !== undefined),
      );
    });

    await data.load();

    expect(indexedAtEachNotification.length).toBeGreaterThan(0);
    expect(indexedAtEachNotification.every(Boolean)).toBe(true);
  });

  it("builds a structured search index (text + type + cross-doc object id) per entry", async () => {
    await data.load();
    const entries = store.getState().history?.entries ?? [];
    expect(entries.length).toBe(8);
    for (const e of entries) {
      const idx = e.__search;
      expect(idx).toBeDefined();
      expect(typeof idx?.text).toBe("string");
      expect(idx?.type).toBe(e.eventType);
      expect(idx?.revision).toBe(REV);
      // The object id is resolved against the revisions payload (cross-document).
      expect(idx?.object).toBe(OBJ);
    }
    // A validation entry carries its status into the index.
    const failed = entries.find(
      (e) =>
        e.eventType === "validation_check_recorded" &&
        e.trackId === "human:kevin",
    );
    expect(failed?.__search?.status).toBe("failed");
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
    expect(refresh?.textContent).toBe("watching");
    expect(refresh?.classList.contains("live")).toBe(false);
  });

  it("reloads and flags the indicator when the event-count marker changed", async () => {
    await data.load();
    setFreshnessResponse({ eventCount: HISTORY_EVENT_COUNT + 1 });
    await data.pollFreshness();
    const refresh = document.querySelector("#refresh");
    expect(refresh?.textContent).toBe("updated");
    expect(refresh?.classList.contains("live")).toBe(true);
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
    expect(document.querySelector("#refresh")?.textContent).toBe("watching");
  });

  it("marks the indicator stalled when the freshness probe fails", async () => {
    await data.load();
    const restore = globalThis.fetch;
    globalThis.fetch = () => Promise.reject(new Error("offline"));
    try {
      await data.pollFreshness();
      expect(document.querySelector("#refresh")?.textContent).toBe("stalled");
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
