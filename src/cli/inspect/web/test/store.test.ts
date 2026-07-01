import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryDoc, RevisionsDoc, ThreadsDoc } from "../src/store";
import { TYPES } from "../src/types";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import threadsJson from "./fixtures/threads.json";

// The store is a module singleton (one `state` object, one subscriber set), just
// like the served app.js. Reset the module registry before each test so every
// case starts from the initial state with no leftover subscribers.
type Store = typeof import("../src/store");
let store: Store;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
});

describe("getState defaults", () => {
  it("returns the initial state ported from the app.js state object", () => {
    const s = store.getState();
    expect(s.history).toBeNull();
    expect(s.revisions).toBeNull();
    expect(s.threads).toBeNull();
    expect(s.lens).toBe("timeline");
    expect(s.selected).toEqual({ kind: null, id: null });
    expect(s.filterText).toBe("");
    expect(s.filterTrack).toBe("");
    expect(s.filterObject).toBe("");
    expect(s.order).toBe("desc");
    expect(s.diff).toBeNull();
    expect(s.diffHash).toBeNull();
    expect(s.focus).toBeNull();
    expect(s.lastEventCount).toBeNull();
  });

  it("seeds enabledTypes and seenTypes from every known event type", () => {
    const s = store.getState();
    const ids = TYPES.map((t) => t.id).sort();
    expect([...s.enabledTypes].sort()).toEqual(ids);
    expect([...s.seenTypes].sort()).toEqual(ids);
  });

  it("returns the live singleton — the same reference reflects later commits", () => {
    const s = store.getState();
    store.commit({ filterText: "abc" });
    expect(s.filterText).toBe("abc");
    expect(store.getState()).toBe(s);
  });
});

describe("commit applies patches", () => {
  it("applies a partial patch, observable via getState, leaving others at default", () => {
    store.commit({ lens: "threads", order: "asc", filterTrack: "agent:codex" });
    const s = store.getState();
    expect(s.lens).toBe("threads");
    expect(s.order).toBe("asc");
    expect(s.filterTrack).toBe("agent:codex");
    expect(s.filterText).toBe("");
  });

  it("holds the loaded /api docs the container forwards to render", () => {
    store.commit({
      history: historyJson as unknown as HistoryDoc,
      revisions: revisionsJson as unknown as RevisionsDoc,
      threads: threadsJson as unknown as ThreadsDoc,
    });
    const s = store.getState();
    expect(s.history?.entries.length ?? 0).toBeGreaterThan(0);
    expect(s.revisions?.entries.length ?? 0).toBeGreaterThan(0);
    expect(Array.isArray(s.threads?.threads)).toBe(true);
  });

  it("holds the loaded page window, facets, and matchCount on the history doc", () => {
    store.commit({
      history: {
        entries: [],
        diagnostics: [],
        facets: { review_observation_recorded: 3 },
        matchCount: 42,
        offset: 20,
        queryKey: "q=pinned&limit=100",
      } as unknown as HistoryDoc,
    });
    const s = store.getState();
    expect(s.history?.facets).toEqual({ review_observation_recorded: 3 });
    expect(s.history?.matchCount).toBe(42);
    expect(s.history?.offset).toBe(20);
    expect(s.history?.queryKey).toBe("q=pinned&limit=100");
  });
});

describe("commit restores the navigate() invariants", () => {
  it("resets a cleared selection back to { kind: null, id: null }", () => {
    store.commit({ selected: { kind: "event", id: "evt:1" } });
    expect(store.getState().selected).toEqual({ kind: "event", id: "evt:1" });
    // A patch that nulls the selection is reconciled back to the empty selection,
    // exactly as navigate() does: `if (!state.selected) state.selected = {…}`.
    store.commit({ selected: undefined });
    expect(store.getState().selected).toEqual({ kind: null, id: null });
  });

  it("nulls diffHash whenever the diff is closed", () => {
    store.commit({ diff: "obj:1", diffHash: "sha256:abc" });
    expect(store.getState().diffHash).toBe("sha256:abc");
    store.commit({ diff: null });
    expect(store.getState().diff).toBeNull();
    expect(store.getState().diffHash).toBeNull();
  });

  it("forces diffHash null even when the same patch supplies a hash without a diff", () => {
    store.commit({ diff: null, diffHash: "sha256:orphan" });
    expect(store.getState().diffHash).toBeNull();
  });

  it("keeps diffHash while a diff is open", () => {
    store.commit({ diff: "obj:2", diffHash: "sha256:keep" });
    expect(store.getState().diffHash).toBe("sha256:keep");
  });

  it("does not normalize focus (navigate() enforces no focus invariant)", () => {
    store.commit({ focus: "evt:9", diff: null });
    expect(store.getState().focus).toBe("evt:9");
  });
});

describe("subscribe", () => {
  it("fires a registered callback once per commit", () => {
    const fn = vi.fn();
    store.subscribe(fn);
    store.commit({ filterText: "a" });
    store.commit({ filterText: "b" });
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("fires every registered subscriber", () => {
    const a = vi.fn();
    const b = vi.fn();
    store.subscribe(a);
    store.subscribe(b);
    store.commit({ order: "asc" });
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("stops notifying after the returned unsubscribe handle is called", () => {
    const fn = vi.fn();
    const off = store.subscribe(fn);
    store.commit({ filterText: "x" });
    off();
    store.commit({ filterText: "y" });
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("notifies subscribers after the patch is applied (they observe the new state)", () => {
    let seen: string | undefined;
    store.subscribe(() => {
      seen = store.getState().filterText;
    });
    store.commit({ filterText: "applied-before-notify" });
    expect(seen).toBe("applied-before-notify");
  });
});
