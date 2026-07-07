import { afterEach, describe, expect, it } from "vitest";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetFreshnessResponse,
  setFreshnessResponse,
  uninstallFetchMock,
} from "./support/fetch";

afterEach(() => {
  resetDom();
  uninstallFetchMock();
  resetFreshnessResponse();
});

// The fixed ids the harness must mount (mirror of assets/index.html). This list
// grows as new controls are wired; a missing id is a harness gap, not a module bug.
const FIXED_IDS = [
  "topbar",
  "store-identity",
  "store-chip",
  "store-chip-repo",
  "store-identity-rows",
  "lens-switcher",
  "stat-events",
  "stat-units",
  "stat-threads",
  "stat-hash",
  "stat-live",
  "refresh",
  "refresh-word",
  "theme-toggle",
  "density-toggle",
  "diagnostics",
  "route-diagnostic",
  "toolbar",
  "filter-text",
  "filter-types",
  "order-toggle",
  "filter-clear",
  "master",
  "detail",
  "error",
  "diff-modal",
  "diff-title",
  "diff-close",
  "diff-nav",
  "diff-body",
  "cmd-palette",
  "cmd-input",
  "cmd-results",
  "key-help",
  "key-help-close",
];

describe("mountInspectorDom", () => {
  it("mounts every fixed id from index.html", () => {
    mountInspectorDom();
    for (const id of FIXED_IDS) {
      expect(document.getElementById(id), id).not.toBeNull();
    }
    expect(document.querySelectorAll(".lens-tab")).toHaveLength(3);
  });

  it("does not inject the render-created lens bodies (renderMaster owns those)", () => {
    mountInspectorDom();
    // The master pane is the empty shell; render fills it with the lens body.
    expect(document.getElementById("master")?.childElementCount).toBe(0);
    expect(document.getElementById("timeline")).toBeNull();
    expect(document.getElementById("revisions")).toBeNull();
  });

  it("resets cleanly on a second mount — no duplicated ids", () => {
    mountInspectorDom();
    mountInspectorDom();
    expect(document.querySelectorAll("#master")).toHaveLength(1);
    expect(document.querySelectorAll(".lens-tab")).toHaveLength(3);
  });

  it("resetDom clears the body and the prefs-applied root attributes", () => {
    mountInspectorDom();
    document.documentElement.setAttribute("data-theme", "light");
    document.documentElement.classList.add("compact");
    resetDom();
    expect(document.body.innerHTML).toBe("");
    expect(document.documentElement.getAttribute("data-theme")).toBeNull();
    expect(document.documentElement.classList.contains("compact")).toBe(false);
  });
});

describe("the fetch mock", () => {
  it("returns the committed fixture for each /api route", async () => {
    installFetchMock();
    const history: { entries: unknown[] } = await (
      await fetch("/api/history")
    ).json();
    expect(Array.isArray(history.entries)).toBe(true);

    const revisions: { entries: unknown[] } = await (
      await fetch("/api/revisions")
    ).json();
    expect(Array.isArray(revisions.entries)).toBe(true);

    const threads: { threads: unknown[] } = await (
      await fetch("/api/threads")
    ).json();
    expect(Array.isArray(threads.threads)).toBe(true);

    const snapshot: Record<string, unknown> = await (
      await fetch("/api/snapshots/snap:sha256:abc")
    ).json();
    expect(snapshot).toBeTypeOf("object");

    const revision: Record<string, unknown> = await (
      await fetch("/api/revisions/rev:sha256:abc")
    ).json();
    expect(revision.revision).toBeTypeOf("object");
  });

  it("mirrors the server's member matcher (INV-2): list vs member, encoded, empty, deeper", async () => {
    installFetchMock();

    // Exact list route resolves to the list fixture (has `entries`), not the member.
    const list: { entries?: unknown[] } = await (
      await fetch("/api/revisions")
    ).json();
    expect(Array.isArray(list.entries)).toBe(true);

    // A percent-encoded member id resolves to the single committed fixture.
    const encoded = await fetch("/api/snapshots/snap%3Agit%3Asha256%3Aabc");
    expect(encoded.status).toBe(200);

    // A member with a query string still resolves (the query is stripped).
    const withHash = await fetch(
      "/api/snapshots/snap:sha256:abc?contentHash=h",
    );
    expect(withHash.status).toBe(200);

    // An empty member (trailing slash) is a 400, like the server's decode_member None.
    expect((await fetch("/api/revisions/")).status).toBe(400);
    expect((await fetch("/api/snapshots/")).status).toBe(400);

    // A deeper / multi-segment path is a 404.
    expect((await fetch("/api/revisions/a/b")).status).toBe(404);
  });

  it("serves a 404 for a route with no committed fixture", async () => {
    installFetchMock();
    const res = await fetch("/api/nonexistent");
    expect(res.ok).toBe(false);
    expect(res.status).toBe(404);
  });

  it("serves a default freshness probe and honors an override", async () => {
    installFetchMock();
    const def: Record<string, unknown> = await (
      await fetch("/api/freshness")
    ).json();
    // Defaults to history.json's eventCount marker (so a poll right after load
    // reports unchanged).
    expect(def.eventCount).toBe(8);

    setFreshnessResponse({ eventCount: 9 });
    const overridden: Record<string, unknown> = await (
      await fetch("/api/freshness")
    ).json();
    expect(overridden.eventCount).toBe(9);
  });

  it("uninstall restores the prior global fetch", () => {
    const before = globalThis.fetch;
    installFetchMock();
    expect(globalThis.fetch).not.toBe(before);
    uninstallFetchMock();
    expect(globalThis.fetch).toBe(before);
  });
});
