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
  "lens-switcher",
  "stat-events",
  "stat-units",
  "stat-threads",
  "stat-hash",
  "refresh",
  "advisory-mode",
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

    const objects: { threads: unknown[] } = await (
      await fetch("/api/objects")
    ).json();
    expect(Array.isArray(objects.threads)).toBe(true);

    const object: Record<string, unknown> = await (
      await fetch("/api/object?id=rev:sha256:abc")
    ).json();
    expect(object).toBeTypeOf("object");

    const revision: Record<string, unknown> = await (
      await fetch("/api/revision?id=rev:sha256:abc")
    ).json();
    expect(revision).toBeTypeOf("object");
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
    // Defaults to history.json's hash + diagnostic count (so a poll right after
    // load reports unchanged).
    expect(def.eventSetHash).toBeTypeOf("string");
    expect(def.diagnosticCount).toBe(0);

    setFreshnessResponse({
      eventSetHash: "sha256:changed",
      diagnosticCount: 2,
    });
    const overridden: Record<string, unknown> = await (
      await fetch("/api/freshness")
    ).json();
    expect(overridden.eventSetHash).toBe("sha256:changed");
    expect(overridden.diagnosticCount).toBe(2);
  });

  it("uninstall restores the prior global fetch", () => {
    const before = globalThis.fetch;
    installFetchMock();
    expect(globalThis.fetch).not.toBe(before);
    uninstallFetchMock();
    expect(globalThis.fetch).toBe(before);
  });
});
