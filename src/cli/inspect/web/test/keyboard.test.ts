import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryDoc, RevisionsDoc } from "../src/store";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import { installFetchMock, uninstallFetchMock } from "./support/fetch";

// `keyboard.ts` is the global keydown layer: selection stepping, activation, search
// focus, two-key chords, the layered Escape, and the diff-local jump keys. It is
// top-of-graph — nothing imports it — and it routes every state change through
// `router.navigate` (commit → the subscriber repaints), never render. `pendingChord`
// / `chordTimer` stay module-local. The store, the keyboard module, the overlay
// manager, and the diff controller are singletons, so reset + re-import before each
// test, and wire `onKey` to `document` the way the composition root will.
type Store = typeof import("../src/store");
type Overlay = typeof import("../src/overlay");
type Controller = typeof import("../src/diff/controller");
type Keyboard = typeof import("../src/keyboard");
let store: Store;
let overlay: Overlay;
let controller: Controller;
let keyboard: Keyboard;

const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

function key(init: KeyboardEventInit, target: EventTarget = document): void {
  target.dispatchEvent(
    new KeyboardEvent("keydown", { bubbles: true, ...init }),
  );
}

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
  overlay = await import("../src/overlay");
  controller = await import("../src/diff/controller");
  keyboard = await import("../src/keyboard");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
  });
  document.addEventListener("keydown", keyboard.onKey);
});

afterEach(() => {
  document.removeEventListener("keydown", keyboard.onKey);
  uninstallFetchMock();
  resetDom();
});

describe("typing targets suppress shortcuts", () => {
  it("does not step the selection while a text field is focused", () => {
    const box = document.querySelector<HTMLInputElement>("#filter-text");
    box?.focus();
    key({ key: "j" }, box ?? document);
    expect(store.getState().selected.id).toBeNull();
  });
});

describe("selection stepping / activation / search", () => {
  it("j selects the first timeline entry, k steps back", () => {
    key({ key: "j" });
    const first = store.getState().selected;
    expect(first.kind).toBe("event");
    expect(first.id).not.toBeNull();
    key({ key: "ArrowDown" });
    expect(store.getState().selected.id).not.toBe(first.id);
    key({ key: "ArrowUp" });
    expect(store.getState().selected.id).toBe(first.id);
  });

  it("Enter activates the selected revision's diff", () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    key({ key: "Enter" });
    expect(store.getState().diff).toBe(OBJ);
  });

  it("/ focuses the search box and switches to the timeline lens", () => {
    store.commit({ lens: "list" });
    key({ key: "/" });
    expect(store.getState().lens).toBe("timeline");
    expect(document.activeElement).toBe(document.querySelector("#filter-text"));
  });
});

describe("two-key chords", () => {
  it("g then l switches to the list lens", () => {
    key({ key: "g" });
    key({ key: "l" });
    expect(store.getState().lens).toBe("list");
  });

  it("g then r switches to the threads lens", () => {
    key({ key: "g" });
    key({ key: "r" });
    expect(store.getState().lens).toBe("threads");
  });
});

describe("overlays via the keyboard", () => {
  it("Cmd-K opens the command palette", () => {
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
  });

  it("? toggles the keyboard help overlay", () => {
    key({ key: "?" });
    expect(overlay.activeName()).toBe("help");
    key({ key: "?" });
    expect(overlay.activeName()).toBeNull();
  });

  it("Escape closes the active overlay first", () => {
    key({ key: "k", metaKey: true });
    expect(overlay.activeName()).toBe("palette");
    key({ key: "Escape" });
    expect(overlay.activeName()).toBeNull();
  });

  it("Escape clears the query when nothing else is open", () => {
    store.commit({ filterText: "obs" });
    key({ key: "Escape" });
    expect(store.getState().filterText).toBe("");
  });
});

describe("a focused ref chip activates on Enter", () => {
  it("resolves the chip reference", () => {
    const detail = document.querySelector("#detail");
    if (detail)
      detail.innerHTML = `<span class="ref" role="link" tabindex="0" data-ref-kind="rev" data-ref-id="${REV}">chip</span>`;
    const chip = document.querySelector<HTMLElement>("[data-ref-kind]");
    chip?.focus();
    key({ key: "Enter" }, chip ?? document);
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });
});

describe("diff-local jump keys (only while the diff overlay is open)", () => {
  it("n jumps to the next review fact, syncing the focus route", async () => {
    controller.initControls();
    store.commit({ diff: OBJ, diffHash: ARTIFACT, focus: null });
    await controller.renderDiffOverlay();
    key({ key: "n" });
    const firstAnno = document.querySelector<HTMLElement>(
      "#diff-body .anno[data-anno]",
    );
    expect(store.getState().focus).toBe(firstAnno?.dataset.anno);
  });
});
