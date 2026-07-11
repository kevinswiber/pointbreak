import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryDoc, RevisionsDoc } from "../../src/store";
import historyJson from "../fixtures/history.json";
import revisionsJson from "../fixtures/revisions.json";
import { mountInspectorDom, resetDom } from "../support/dom";
import {
  installFetchMock,
  resetSnapshotResponse,
  setSnapshotResponse,
  uninstallFetchMock,
} from "../support/fetch";

// `diff/controller.ts` owns the diff overlay lifecycle. It opens through the
// overlay teardown manager (registering its own `onClose`, importing no sibling
// overlay — the import-cycle cut), fetches the artifact through the `http` leaf,
// paints it via the pure `diff/render.renderDiff` (consuming the returned
// `{ html, ctx }`), and clears the route through `router.navigate` — never calling
// render (INV-6; the store subscriber repaints). The diff cursors / `diffCtx` /
// nav filter stay module-local. The store, the controller, and the overlay manager
// are module singletons, so reset the registry and re-import them before each test.
type Store = typeof import("../../src/store");
type Overlay = typeof import("../../src/overlay");
type Controller = typeof import("../../src/diff/controller");
let store: Store;
let overlay: Overlay;
let controller: Controller;

const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

beforeEach(async () => {
  vi.resetModules();
  store = await import("../../src/store");
  overlay = await import("../../src/overlay");
  controller = await import("../../src/diff/controller");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
  });
  controller.initControls();
});

afterEach(() => {
  uninstallFetchMock();
  resetSnapshotResponse();
  resetDom();
});

/** A synthetic artifact with `n` content-bearing files (more than the open budget). */
function syntheticArtifact(n: number): unknown {
  const files = [];
  for (let i = 0; i < n; i++) {
    files.push({
      status: "modified",
      old_path: `src/f${i}.rs`,
      new_path: `src/f${i}.rs`,
      metadata_rows: [],
      hunks: [
        {
          header: `@@ file ${i} @@`,
          rows: [
            { kind: "added", old_line: null, new_line: 1, text: `line ${i}` },
          ],
        },
      ],
    });
  }
  return { snapshot: { files } };
}

function modal(): HTMLElement | null {
  return document.querySelector<HTMLElement>("#diff-modal");
}

async function openCommitted(): Promise<void> {
  store.commit({ diff: OBJ, diffHash: ARTIFACT, focus: null });
  await controller.renderDiffOverlay();
}

describe("openDiff / openRevisionDiff (route-only, the open is the reconciler's job)", () => {
  it("openDiff commits the diff route without directly opening the modal", () => {
    controller.openDiff(OBJ, null, ARTIFACT);
    expect(store.getState().diff).toBe(OBJ);
    expect(store.getState().diffHash).toBe(ARTIFACT);
    // openDiff only changes the route; the overlay opens when the reconciler runs.
    expect(modal()?.classList.contains("hidden")).toBe(true);
  });

  it("openRevisionDiff resolves the captured object and its artifact hash", () => {
    controller.openRevisionDiff(REV, "obs:focus");
    expect(store.getState().diff).toBe(OBJ);
    expect(store.getState().diffHash).toBe(ARTIFACT);
    expect(store.getState().focus).toBe("obs:focus");
  });
});

describe("renderDiffOverlay (open via the overlay manager + paint the fetched artifact)", () => {
  it("opens #diff-modal through the manager and paints the diff body and navigator", async () => {
    await openCommitted();
    expect(modal()?.classList.contains("hidden")).toBe(false);
    expect(overlay.activeName()).toBe("diff");
    expect(document.querySelector("#diff-body")?.innerHTML).toContain("dfile");
    expect(document.querySelector("#diff-nav")?.innerHTML).toContain("files");
    expect(document.querySelector("#diff-title")?.textContent).toContain(
      "snapshot",
    );
  });

  it("focuses the close button as the initial overlay focus target", async () => {
    await openCommitted();
    expect(document.activeElement).toBe(document.querySelector("#diff-close"));
  });

  it("tears down a prior overlay through the manager when the diff opens (no sibling import)", async () => {
    const paletteNode = document.querySelector<HTMLElement>("#cmd-palette");
    const paletteClose = vi.fn();
    if (paletteNode)
      overlay.register("palette", { node: paletteNode, onClose: paletteClose });
    overlay.open("palette");
    expect(overlay.activeName()).toBe("palette");

    await openCommitted();
    expect(paletteClose).toHaveBeenCalledTimes(1);
    expect(overlay.activeName()).toBe("diff");
  });
});

describe("closeDiff (route-clearing via the router, never a direct render)", () => {
  it("replaces the route and leaves the repaint to the store subscriber", async () => {
    await openCommitted();
    expect(modal()?.classList.contains("hidden")).toBe(false);

    const replaceSpy = vi.spyOn(history, "replaceState");
    controller.closeDiff();
    expect(store.getState().diff).toBeNull();
    expect(store.getState().diffHash).toBeNull();
    expect(store.getState().focus).toBeNull();
    expect(replaceSpy).toHaveBeenCalledTimes(1);
    // closeDiff only cleared the route; the modal is still open until a repaint.
    expect(modal()?.classList.contains("hidden")).toBe(false);

    // The store subscriber's repaint (render → renderDiffOverlay) closes it.
    await controller.renderDiffOverlay();
    expect(modal()?.classList.contains("hidden")).toBe(true);
    replaceSpy.mockRestore();
  });

  it("closes through the wired #diff-close button and the modal backdrop", async () => {
    await openCommitted();
    document
      .querySelector("#diff-close")
      ?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().diff).toBeNull();

    await openCommitted();
    modal()?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().diff).toBeNull();
  });
});

describe("lazy file bodies", () => {
  it("fills a collapsed file body on first expand and toggles its disclosure state", async () => {
    setSnapshotResponse(syntheticArtifact(12));
    await openCommitted();
    const collapsed = document.querySelector<HTMLElement>(
      '#diff-body .dfile[data-dfile="11"]',
    );
    expect(collapsed).not.toBeNull();
    const body = collapsed?.querySelector<HTMLElement>("[data-dfile-body]");
    expect(body?.dataset.rendered).toBeUndefined();
    expect(
      collapsed?.querySelector(".dfile-head")?.getAttribute("aria-expanded"),
    ).toBe("false");

    if (collapsed) controller.toggleDiffFile(collapsed);
    expect(
      collapsed?.querySelector(".dfile-head")?.getAttribute("aria-expanded"),
    ).toBe("true");
    expect(body?.dataset.rendered).toBe("1");
    expect(body?.innerHTML).toContain("dhunk");

    // Toggling again collapses without re-rendering the (already filled) body.
    if (collapsed) controller.toggleDiffFile(collapsed);
    expect(
      collapsed?.querySelector(".dfile-head")?.getAttribute("aria-expanded"),
    ).toBe("false");
    expect(body?.dataset.rendered).toBe("1");
  });
});

describe("the file/fact navigator", () => {
  it("renders a summary, filters, a file list, and the unanchored-facts panel", async () => {
    await openCommitted();
    const nav = document.querySelector("#diff-nav");
    expect(
      nav
        ?.querySelector('[data-diff-nav-filter="all"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
    expect(nav?.querySelectorAll(".diff-nav-file").length).toBe(1);
    // The three revision-level facts (an input request + two assessments) are
    // unanchored and reachable in the navigator panel.
    expect(nav?.querySelector(".diff-unanchored")).not.toBeNull();
  });

  it("filters to unanchored facts only, hiding the file list", async () => {
    await openCommitted();
    controller.setDiffNavFilter("unanchored");
    const nav = document.querySelector("#diff-nav");
    expect(
      nav
        ?.querySelector('[data-diff-nav-filter="unanchored"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
    expect(nav?.querySelectorAll(".diff-nav-file").length).toBe(0);
    expect(nav?.querySelector(".diff-unanchored")).not.toBeNull();
    // Re-rendering the navigator adds no route state.
    expect(store.getState().diff).toBe(OBJ);
  });

  it("filters to files carrying facts only, hiding the unanchored panel", async () => {
    await openCommitted();
    controller.setDiffNavFilter("with-facts");
    const nav = document.querySelector("#diff-nav");
    expect(nav?.querySelectorAll(".diff-nav-file").length).toBe(1);
    expect(nav?.querySelector(".diff-unanchored")).toBeNull();
  });

  it("ignores an unrecognized filter value", async () => {
    await openCommitted();
    controller.setDiffNavFilter("bogus");
    const nav = document.querySelector("#diff-nav");
    expect(
      nav
        ?.querySelector('[data-diff-nav-filter="all"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
  });
});

// The jump keys are owned by the diff's overlay registration: while the diff is
// the active overlay, the global keydown layer hands ]/[/n/p to the diff's own
// key map through the manager's delegation; with no overlay active they are dead.
describe("diff overlay key ownership", () => {
  let onKey: (ev: KeyboardEvent) => void;

  beforeEach(async () => {
    const keyboard = await import("../../src/keyboard");
    onKey = keyboard.onKey;
    document.addEventListener("keydown", onKey);
  });

  afterEach(() => {
    document.removeEventListener("keydown", onKey);
  });

  function press(key: string): KeyboardEvent {
    const ev = new KeyboardEvent("keydown", {
      key,
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(ev);
    return ev;
  }

  it("jumps to the next change on ']' while the diff overlay is active", async () => {
    await openCommitted();
    expect(overlay.activeName()).toBe("diff");
    const scrollSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {});
    try {
      const ev = press("]");
      expect(ev.defaultPrevented).toBe(true);
      const jumped = scrollSpy.mock.instances.at(-1);
      expect(
        jumped instanceof HTMLElement && jumped.classList.contains("dhunk"),
      ).toBe(true);
    } finally {
      scrollSpy.mockRestore();
    }
  });

  it("jumps to the next review fact on 'n', syncing the focus route", async () => {
    await openCommitted();
    press("n");
    const firstAnno = document.querySelector<HTMLElement>(
      "#diff-body .anno[data-anno]",
    );
    expect(firstAnno).not.toBeNull();
    expect(store.getState().focus).toBe(firstAnno?.dataset.anno);
  });

  it("keeps ']' inert when no overlay is active", async () => {
    const before = structuredClone(store.getState());
    const scrollSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {});
    try {
      press("]");
      expect(store.getState()).toEqual(before);
      expect(scrollSpy).not.toHaveBeenCalled();
    } finally {
      scrollSpy.mockRestore();
    }
  });
});

describe("fact / change jump keys", () => {
  it("jumpFact advances to the next fact and replaces the route focus", async () => {
    await openCommitted();
    const replaceSpy = vi.spyOn(history, "replaceState");
    controller.jumpFact(1);
    const first = document.querySelector<HTMLElement>(
      "#diff-body .anno[data-anno]",
    );
    expect(store.getState().focus).toBe(first?.dataset.anno);
    expect(replaceSpy).toHaveBeenCalled();
    replaceSpy.mockRestore();
  });

  it("jumpChange cycles change anchors without touching the focus route", async () => {
    await openCommitted();
    expect(
      document.querySelectorAll("#diff-body .dhunk").length,
    ).toBeGreaterThan(0);
    const focusBefore = store.getState().focus;
    controller.jumpChange(1);
    expect(store.getState().focus).toBe(focusBefore);
  });

  it("a noted gutter click scrolls to the annotation and syncs the focus route", async () => {
    await openCommitted();
    const noted = document.querySelector<HTMLElement>(
      "#diff-body .drow-noted[data-anno]",
    );
    expect(noted).not.toBeNull();
    noted?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().focus).toBe(noted?.dataset.anno);
  });
});

// The routed diff page: a route surface (never an overlay — activeName() stays
// null) painted from `state.diffPage`/`diffRevision`. Facts AND snapshot identity
// come from the composite revision document, so cold and grouped-away deep links
// paint annotated; an unmappable snapshot-only link paints bytes with blank facts.
describe("renderDiffPage (the routed page surface)", () => {
  function pageBody(): HTMLElement | null {
    return document.querySelector<HTMLElement>("#diff-page-body");
  }

  it("paints facts on a cold revision-primary deep link (no history, no list)", async () => {
    // Cold: nothing loaded — only the composite + snapshot endpoints answer.
    store.commit({
      history: { entries: [], diagnostics: [] } as unknown as HistoryDoc,
      revisions: { entries: [] } as unknown as RevisionsDoc,
      diffPage: true,
      diffRevision: REV,
    });
    await controller.renderDiffPage();
    expect(pageBody()?.innerHTML).toContain("dfile");
    // The annotated half of "annotated diff": fact markers render from the
    // composite document, never the paged history or the list document.
    expect(
      pageBody()?.querySelectorAll(".anno[data-anno]").length,
    ).toBeGreaterThan(0);
    expect(document.querySelector("#diff-page-title")?.textContent).toContain(
      "snapshot",
    );
    // A route surface, not an overlay: the manager never owns the page.
    expect(overlay.activeName()).toBe(null);
  });

  it("paints facts for a grouped-away revision absent from the loaded list", async () => {
    store.commit({
      revisions: { entries: [] } as unknown as RevisionsDoc, // grouped away
      diffPage: true,
      diffRevision: REV,
    });
    await controller.renderDiffPage();
    expect(pageBody()?.innerHTML).toContain("dfile");
    expect(
      pageBody()?.querySelectorAll(".anno[data-anno]").length,
    ).toBeGreaterThan(0);
  });

  it("paints best-effort blank for an unmappable snapshot-only link", async () => {
    store.commit({
      diffPage: true,
      diffRevision: null,
      diff: OBJ,
      diffHash: ARTIFACT,
    });
    await controller.renderDiffPage();
    expect(pageBody()?.innerHTML).toContain("dfile"); // the bytes render
    expect(pageBody()?.querySelectorAll(".anno[data-anno]").length).toBe(0);
    expect(pageBody()?.textContent).toContain("no review facts");
  });

  it("applies ?nav= from route state and expands the ?file= target", async () => {
    setSnapshotResponse(syntheticArtifact(12));
    store.commit({
      diffPage: true,
      diffRevision: REV,
      diffNav: "with-facts",
      diffFile: "src/f11.rs",
    });
    await controller.renderDiffPage();
    const nav = document.querySelector("#diff-page-nav");
    expect(
      nav
        ?.querySelector('[data-diff-nav-filter="with-facts"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
    // The ?file= target's body is ensured and its section expanded.
    const section = pageBody()?.querySelector('.dfile[data-dfile="11"]');
    expect(
      section?.querySelector(".dfile-head")?.getAttribute("aria-expanded"),
    ).toBe("true");
    expect(
      section?.querySelector<HTMLElement>("[data-dfile-body]")?.innerHTML,
    ).toContain("dhunk");
  });

  it("promotes the navigator filter to route state on the page", async () => {
    store.commit({ diffPage: true, diffRevision: REV });
    await controller.renderDiffPage();
    controller.setDiffNavFilter("unanchored");
    expect(store.getState().diffNav).toBe("unanchored");
    // The repaint (the store subscriber in production) re-renders the navigator.
    await controller.renderDiffPage();
    expect(
      document
        .querySelector('#diff-page-nav [data-diff-nav-filter="unanchored"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("re-paints idempotently on a freshness-poll repaint", async () => {
    store.commit({ diffPage: true, diffRevision: REV });
    await controller.renderDiffPage();
    expect(pageBody()?.innerHTML).toContain("dfile");
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const scrollSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {});
    try {
      await controller.renderDiffPage(); // unchanged state — the poll repaint
      expect(fetchSpy).not.toHaveBeenCalled(); // no duplicate fetch
      expect(scrollSpy).not.toHaveBeenCalled(); // no scroll reset
      expect(pageBody()?.innerHTML).toContain("dfile"); // still painted
    } finally {
      fetchSpy.mockRestore();
      scrollSpy.mockRestore();
    }
  });
});
