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

// The single revision/object the committed fixtures describe, plus the history
// payload's freshness hash.
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const HISTORY_HASH =
  "sha256:e81f297a301ad7d9df6bd90ceeb257511f661b4fc108460e7f2bdc3f76de0164";

describe("load", () => {
  it("commits history, revisions, and objects to the store", async () => {
    await data.load();
    const s = store.getState();
    expect(s.history?.entries.length).toBe(8);
    expect(s.revisions?.entries.length).toBe(1);
    expect(s.objects?.threads.length).toBe(1);
  });

  it("seeds the freshness baselines from the history payload", async () => {
    await data.load();
    const s = store.getState();
    expect(s.lastHash).toBe(HISTORY_HASH);
    expect(s.lastDiagnosticCount).toBe(0);
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

  it("reloads and flags the indicator when the event set changed", async () => {
    await data.load();
    setFreshnessResponse({
      eventSetHash: "sha256:changed",
      diagnosticCount: 0,
    });
    await data.pollFreshness();
    const refresh = document.querySelector("#refresh");
    expect(refresh?.textContent).toBe("updated");
    expect(refresh?.classList.contains("live")).toBe(true);
    // The reload re-fetched and re-seeded the baseline from the history payload.
    expect(store.getState().lastHash).toBe(HISTORY_HASH);
  });

  it("reloads when only the diagnostic count changed", async () => {
    await data.load();
    setFreshnessResponse({ eventSetHash: HISTORY_HASH, diagnosticCount: 3 });
    await data.pollFreshness();
    expect(document.querySelector("#refresh")?.textContent).toBe("updated");
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
