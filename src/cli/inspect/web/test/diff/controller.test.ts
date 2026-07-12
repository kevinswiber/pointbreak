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

async function openCommitted(): Promise<void> {
  store.commit({ diffPage: true, diffRevision: REV, focus: null });
  await controller.renderDiffPage();
}

describe("openDiff / openRevisionDiff (page navigations — route-only, the paint is the reconciler's job)", () => {
  it("openDiff lands on the revision-primary page when the snapshot maps", () => {
    const push = vi.spyOn(history, "pushState");
    try {
      controller.openDiff(OBJ, null, ARTIFACT);
      expect(store.getState().diffPage).toBe(true);
      expect(store.getState().diffRevision).toBe(REV);
      expect(store.getState().diff).toBe(OBJ); // payload pointer retained
      expect(store.getState().diffHash).toBe(ARTIFACT);
      expect(push).toHaveBeenCalledTimes(1); // open = push
    } finally {
      push.mockRestore();
    }
  });

  it("openDiff lands on the snapshot-only page when nothing maps", () => {
    controller.openDiff("obj:sha256:unmapped", null, null);
    expect(store.getState().diffPage).toBe(true);
    expect(store.getState().diffRevision).toBeNull();
    expect(store.getState().diff).toBe("obj:sha256:unmapped");
  });

  it("openRevisionDiff opens the page on the revision's own identity", () => {
    controller.openRevisionDiff(REV, "obs:focus");
    expect(store.getState().diffPage).toBe(true);
    expect(store.getState().diffRevision).toBe(REV);
    expect(store.getState().focus).toBe("obs:focus");
  });

  it("openRevisionDiff needs no list entry (the composite resolves the snapshot)", () => {
    store.commit({ revisions: { entries: [] } as unknown as RevisionsDoc });
    controller.openRevisionDiff(REV);
    expect(store.getState().diffPage).toBe(true);
    expect(store.getState().diffRevision).toBe(REV);
  });
});

describe("closeDiff (a real push back to the record, never a replace)", () => {
  it("pushes the cleared route and never touches the parked cursor", async () => {
    store.commit({ selected: { kind: "revision", id: REV }, open: true });
    controller.openDiff(OBJ, "obs:focus", ARTIFACT);
    store.commit({ diffFile: "src/lib.rs", diffNav: "with-facts" });

    const pushSpy = vi.spyOn(history, "pushState");
    const replaceSpy = vi.spyOn(history, "replaceState");
    try {
      controller.closeDiff();
      expect(store.getState().diffPage).toBe(false);
      expect(store.getState().diffRevision).toBeNull();
      expect(store.getState().diff).toBeNull();
      expect(store.getState().diffHash).toBeNull();
      expect(store.getState().focus).toBeNull();
      expect(store.getState().diffFile).toBeNull();
      expect(store.getState().diffNav).toBe("all");
      expect(pushSpy).toHaveBeenCalledTimes(1); // close = push
      expect(replaceSpy).not.toHaveBeenCalled(); // never {replace: true}
      // The cursor (and its open pane) survives the round trip untouched.
      expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
      expect(store.getState().open).toBe(true);
    } finally {
      pushSpy.mockRestore();
      replaceSpy.mockRestore();
    }
  });

  it("closes through the wired page close control", async () => {
    controller.openRevisionDiff(REV);
    await controller.renderDiffPage();
    document
      .querySelector("#diff-page-close")
      ?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().diffPage).toBe(false);
    expect(store.getState().diffRevision).toBeNull();
  });

  it("is a no-op when no diff surface is addressed (no junk history entry)", () => {
    const pushSpy = vi.spyOn(history, "pushState");
    try {
      controller.closeDiff();
      expect(pushSpy).not.toHaveBeenCalled();
    } finally {
      pushSpy.mockRestore();
    }
  });
});

describe("lazy file bodies", () => {
  it("fills a collapsed file body on first expand and toggles its disclosure state", async () => {
    setSnapshotResponse(syntheticArtifact(12));
    await openCommitted();
    const collapsed = document.querySelector<HTMLElement>(
      '#diff-page-body .dfile[data-dfile="11"]',
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
    const nav = document.querySelector("#diff-page-nav");
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
    // On the page the filter is route state; the subscriber repaint (played
    // here by the explicit renderDiffPage) re-renders the navigator.
    controller.setDiffNavFilter("unanchored");
    expect(store.getState().diffNav).toBe("unanchored");
    await controller.renderDiffPage();
    const nav = document.querySelector("#diff-page-nav");
    expect(
      nav
        ?.querySelector('[data-diff-nav-filter="unanchored"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
    expect(nav?.querySelectorAll(".diff-nav-file").length).toBe(0);
    expect(nav?.querySelector(".diff-unanchored")).not.toBeNull();
  });

  it("filters to files carrying facts only, hiding the unanchored panel", async () => {
    await openCommitted();
    controller.setDiffNavFilter("with-facts");
    await controller.renderDiffPage();
    const nav = document.querySelector("#diff-page-nav");
    expect(nav?.querySelectorAll(".diff-nav-file").length).toBe(1);
    expect(nav?.querySelector(".diff-unanchored")).toBeNull();
  });

  it("ignores an unrecognized filter value", async () => {
    await openCommitted();
    controller.setDiffNavFilter("bogus");
    expect(store.getState().diffNav).toBe("all");
    const nav = document.querySelector("#diff-page-nav");
    expect(
      nav
        ?.querySelector('[data-diff-nav-filter="all"]')
        ?.getAttribute("aria-pressed"),
    ).toBe("true");
  });
});

// The page owns its keyboard through the global layer's diff-page block (no
// overlay is involved — activeName() stays null): ]/[/n/p and Escape run;
// every lens, selection, paging, and lens-switch key is inert; with no page
// active the jump keys are dead.
describe("diff page keyboard + history", () => {
  const EVT_ON_REV = (
    historyJson as {
      entries: Array<{ eventId?: string; subject?: { revisionId?: string } }>;
    }
  ).entries.find((e) => e.subject?.revisionId === REV)?.eventId as string;

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

  it("jumps to the next change on ']' while the page is active", async () => {
    await openCommitted();
    expect(overlay.activeName()).toBe(null); // a page, not an overlay
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
      "#diff-page-body .anno[data-anno]",
    );
    expect(firstAnno).not.toBeNull();
    expect(store.getState().focus).toBe(firstAnno?.dataset.anno);
  });

  it("keeps the whole lens key family inert on the page", async () => {
    await openCommitted();
    const before = structuredClone(store.getState());
    const pushSpy = vi.spyOn(history, "pushState");
    const replaceSpy = vi.spyOn(history, "replaceState");
    try {
      for (const k of [
        "j",
        "k",
        "1",
        "2",
        "3",
        "g",
        "G",
        "f",
        "b",
        "d",
        "u",
        "/",
        "Enter",
      ]) {
        press(k);
      }
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(store.getState()).toEqual(before); // no store commit
      expect(pushSpy).not.toHaveBeenCalled(); // no navigate...
      expect(replaceSpy).not.toHaveBeenCalled(); // ...not even a refinement
    } finally {
      pushSpy.mockRestore();
      replaceSpy.mockRestore();
    }
  });

  it("closes the page on Escape with a push", async () => {
    await openCommitted();
    const pushSpy = vi.spyOn(history, "pushState");
    const replaceSpy = vi.spyOn(history, "replaceState");
    try {
      press("Escape");
      expect(store.getState().diffPage).toBe(false);
      expect(store.getState().diffRevision).toBeNull();
      expect(pushSpy).toHaveBeenCalledTimes(1);
      expect(replaceSpy).not.toHaveBeenCalled();
    } finally {
      pushSpy.mockRestore();
      replaceSpy.mockRestore();
    }
  });

  it("keeps ']' inert when no page is active", async () => {
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

  it("Enter descends an open EVENT selection onto the page, cursor untouched", async () => {
    // The event case proves the page's identity lives in diffRevision, never
    // in the selection: the parked cursor survives open and close.
    store.commit({ selected: { kind: "event", id: EVT_ON_REV }, open: true });
    const pushSpy = vi.spyOn(history, "pushState");
    try {
      press("Enter");
      expect(pushSpy).toHaveBeenCalledTimes(1); // open = push
      expect(store.getState().diffPage).toBe(true);
      expect(store.getState().diffRevision).toBe(REV);
      expect(store.getState().selected).toEqual({
        kind: "event",
        id: EVT_ON_REV,
      });
      expect(store.getState().open).toBe(true);
      press("Escape");
      expect(store.getState().diffPage).toBe(false);
      expect(store.getState().selected).toEqual({
        kind: "event",
        id: EVT_ON_REV,
      }); // cursor intact
      expect(store.getState().open).toBe(true);
    } finally {
      pushSpy.mockRestore();
    }
  });

  it("returns to the record with the cursor intact on browser Back", async () => {
    const router = await import("../../src/router");
    router.navigate({
      selected: { kind: "event", id: EVT_ON_REV },
      open: false,
    });
    const recordHash = location.hash; // the parked-cursor record address
    controller.openRevisionDiff(REV);
    expect(location.hash).toContain("/diff");
    // Simulate the browser Back: the prior entry's hash is restored and the
    // popstate listener re-applies the route.
    history.replaceState(null, "", recordHash);
    router.applyHash();
    expect(store.getState().diffPage).toBe(false);
    expect(store.getState().selected).toEqual({
      kind: "event",
      id: EVT_ON_REV,
    });
    expect(store.getState().open).toBe(false);
  });
});

describe("fact / change jump keys", () => {
  it("jumpFact advances to the next fact and replaces the route focus", async () => {
    await openCommitted();
    const replaceSpy = vi.spyOn(history, "replaceState");
    controller.jumpFact(1);
    const first = document.querySelector<HTMLElement>(
      "#diff-page-body .anno[data-anno]",
    );
    expect(store.getState().focus).toBe(first?.dataset.anno);
    expect(replaceSpy).toHaveBeenCalled();
    replaceSpy.mockRestore();
  });

  it("jumpChange cycles change anchors without touching the focus route", async () => {
    await openCommitted();
    expect(
      document.querySelectorAll("#diff-page-body .dhunk").length,
    ).toBeGreaterThan(0);
    const focusBefore = store.getState().focus;
    controller.jumpChange(1);
    expect(store.getState().focus).toBe(focusBefore);
  });

  it("a noted gutter click scrolls to the annotation and syncs the focus route", async () => {
    await openCommitted();
    const noted = document.querySelector<HTMLElement>(
      "#diff-page-body .drow-noted[data-anno]",
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
