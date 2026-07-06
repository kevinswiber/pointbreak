import { afterEach, beforeEach, expect, it } from "vitest";
import type { IdentityDoc } from "../src/store";
import { mountInspectorDom, resetDom } from "./support/dom";
import { installFetchMock, uninstallFetchMock } from "./support/fetch";

// `renderIdentity` (inside the single `render()` subscriber) paints a compact
// top-bar store chip — the repository name only — with the full identity (store
// placement, family, worktree) in a hover/focus detail popover, and sets the
// browser tab `<title>` (issue #391). Module singletons (store, render), so reset
// and re-import before each test.
type Store = typeof import("../src/store");
type Render = typeof import("../src/render");
let store: Store;
let render: Render;

const CLONE: IdentityDoc = {
  repository: "shoreline",
  placement: { tier: "clone", label: "clone store" },
};

beforeEach(async () => {
  const vitest = await import("vitest");
  vitest.vi.resetModules();
  store = await import("../src/store");
  render = await import("../src/render");
  mountInspectorDom();
  installFetchMock();
});

afterEach(() => {
  uninstallFetchMock();
  resetDom();
  document.title = "shore inspector";
});

it("shows only the repository name in the chip and sets the tab title", () => {
  store.commit({ identity: CLONE });
  render.render();
  const chip = document.querySelector("#store-identity .store-identity-chip");
  expect(chip?.textContent).toContain("shoreline");
  // The placement label is NOT in the always-visible chip.
  expect(chip?.textContent).not.toContain("clone store");
  expect(document.title).toBe("shoreline · shore inspector");
});

it("puts repository and placement in the detail popover", () => {
  store.commit({ identity: CLONE });
  render.render();
  const detail = document.querySelector(".store-identity-detail");
  expect(detail?.textContent).toContain("shoreline");
  expect(detail?.textContent).toContain("clone store");
});

it("omits family and worktree rows when absent", () => {
  store.commit({ identity: CLONE });
  render.render();
  const detail = document.querySelector(".store-identity-detail");
  expect(detail?.textContent).not.toContain("family");
  expect(detail?.textContent).not.toContain("worktree");
});

it("shows the family row under the user-level tier", () => {
  store.commit({
    identity: {
      repository: "shoreline",
      placement: { tier: "family", label: "family store" },
      family: { id: "acme-web" },
    },
  });
  render.render();
  const detail = document.querySelector(".store-identity-detail");
  expect(detail?.textContent).toContain("family");
  expect(detail?.textContent).toContain("acme-web");
});

it("shows the worktree row when present", () => {
  store.commit({
    identity: {
      repository: "shoreline",
      worktree: "feat-foo",
      placement: { tier: "clone", label: "clone store" },
    },
  });
  render.render();
  const detail = document.querySelector(".store-identity-detail");
  expect(detail?.textContent).toContain("worktree");
  expect(detail?.textContent).toContain("feat-foo");
});

it("exposes the full identity as an accessible label on the chip", () => {
  store.commit({
    identity: {
      repository: "shoreline",
      placement: { tier: "family", label: "family store" },
      family: { id: "acme-web" },
    },
  });
  render.render();
  const label =
    document
      .querySelector(".store-identity-chip")
      ?.getAttribute("aria-label") ?? "";
  expect(label).toContain("shoreline");
  expect(label).toContain("family store");
  expect(label).toContain("acme-web");
});

it("falls back to the default title when identity is null", () => {
  store.commit({ identity: null });
  render.render();
  expect(document.title).toBe("shore inspector");
  expect(document.querySelector("#store-identity")?.textContent).toBe("");
});
