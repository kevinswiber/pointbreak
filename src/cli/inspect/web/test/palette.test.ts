import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryDoc, RevisionsDoc } from "../src/store";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import { installFetchMock, uninstallFetchMock } from "./support/fetch";

// `palette.ts` is the command palette: it builds the candidate commands over the
// loaded state (actions + contextual revision/object/track/event jumps), opens and
// tears down through the overlay manager (registering its own teardown, importing
// no sibling overlay — the cycle cut), and runs the active command via
// `router.navigate` / `diff/controller` — never render (the subscriber repaints).
// The `cmd*` view state stays module-local. The store, the palette module, and the
// overlay manager are module singletons, so reset and re-import them before each test.
type Store = typeof import("../src/store");
type Overlay = typeof import("../src/overlay");
type Palette = typeof import("../src/palette");
let store: Store;
let overlay: Overlay;
let palette: Palette;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
  overlay = await import("../src/overlay");
  palette = await import("../src/palette");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
  });
  palette.initControls();
});

afterEach(() => {
  uninstallFetchMock();
  resetDom();
});

function paletteEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>("#cmd-palette");
}
function results(): HTMLElement | null {
  return document.querySelector<HTMLElement>("#cmd-results");
}

describe("open / close through the overlay manager", () => {
  it("open shows the palette, builds commands, and focuses the input", () => {
    palette.open();
    expect(paletteEl()?.classList.contains("hidden")).toBe(false);
    expect(overlay.activeName()).toBe("palette");
    expect((results()?.querySelectorAll(".cmd-item").length ?? 0) > 0).toBe(
      true,
    );
    expect(document.activeElement).toBe(document.querySelector("#cmd-input"));
  });

  it("close hides the palette through the manager", () => {
    palette.open();
    palette.close();
    expect(paletteEl()?.classList.contains("hidden")).toBe(true);
    expect(overlay.activeName()).toBeNull();
  });

  it("toggle opens then closes", () => {
    palette.toggle();
    expect(overlay.activeName()).toBe("palette");
    palette.toggle();
    expect(overlay.activeName()).toBeNull();
  });

  it("opening the palette tears down a prior overlay through the manager", () => {
    const help = document.querySelector<HTMLElement>("#key-help");
    const onClose = vi.fn();
    if (help) overlay.register("help", { node: help, onClose });
    overlay.open("help");
    palette.open();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(overlay.activeName()).toBe("palette");
  });
});

describe("buildCommands (actions + contextual jumps over the loaded state)", () => {
  it("lists the action commands and a contextual revision/event jump", () => {
    palette.open();
    const text = results()?.textContent ?? "";
    expect(text).toContain("Copy current view link");
    expect(text).toContain("Switch to list lens");
    expect(text).toContain("Switch to attention lens");
    // The threads lens is dissolved; its palette command must not resurface.
    expect(text).not.toContain("Switch to threads lens");
    // Group headers for the contextual jumps built from state.revisions / history.
    expect(text).toContain("Revisions");
    expect(text).toContain("Events");
  });

  it("surfaces split-resize actions that nudge the divider", () => {
    // The split only exists with a detail open, which the store gates on a
    // selection (store invariant: no selection ⇒ open is forced false).
    store.commit({
      selected: {
        kind: "revision",
        id: "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd",
      },
      open: true,
    });
    palette.open();
    const text = results()?.textContent ?? "";
    expect(text).toContain("Grow timeline pane");
    expect(text).toContain("Shrink timeline pane");
    palette.filterPalette("Grow timeline pane");
    palette.run();
    expect(
      document.documentElement.style.getPropertyValue("--split-master"),
    ).toBe("53%");
  });
});

describe("filter / move / run", () => {
  it("filterPalette narrows to matching commands", () => {
    palette.open();
    palette.filterPalette("list lens");
    const items = results()?.querySelectorAll(".cmd-item") ?? [];
    expect(items.length).toBe(1);
    expect(items[0].textContent).toContain("Switch to list lens");
  });

  it("move steps the active option", () => {
    palette.open();
    palette.filterPalette("");
    palette.move(1);
    const active = results()?.querySelector(".cmd-item.active");
    expect(active?.getAttribute("aria-selected")).toBe("true");
    expect(active?.textContent).toContain("Clear filters");
  });

  it("run executes the active command and closes the palette", () => {
    palette.open();
    palette.filterPalette("Switch to list lens");
    palette.run();
    expect(store.getState().lens).toBe("list");
    expect(overlay.activeName()).toBeNull();
  });

  it("a lens command exits the diff page onto the lens", () => {
    store.commit({ diffPage: true, diffRevision: "rev:sha256:page" });
    palette.open();
    palette.filterPalette("Switch to list lens");
    palette.run();
    expect(store.getState().lens).toBe("list");
    expect(store.getState().diffPage).toBe(false);
    expect(store.getState().diffRevision).toBeNull();
  });

  it("a track command exits the diff page and scopes the timeline", () => {
    store.commit({ diffPage: true, diffRevision: "rev:sha256:page" });
    palette.open();
    palette.filterPalette("human:kevin");
    palette.run();
    expect(store.getState().filterTrack).toBe("human:kevin");
    expect(store.getState().diffPage).toBe(false);
  });

  it("an empty filter shows the no-matches row", () => {
    palette.open();
    palette.filterPalette("zzz-no-such-command");
    expect(results()?.textContent).toContain("No matches");
  });
});

describe("copyCurrentViewLink", () => {
  it("copies the absolute canonical route to the clipboard", () => {
    const writeText = vi.fn();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    palette.copyCurrentViewLink();
    expect(writeText).toHaveBeenCalledTimes(1);
    const link = writeText.mock.calls[0][0] as string;
    expect(link.startsWith(location.origin + location.pathname)).toBe(true);
    expect(link).toContain("#/");
  });
});

describe("the wired #cmd-input / #cmd-palette controls", () => {
  it("the input drives filtering and arrow/enter drive move + run", () => {
    palette.open();
    const input = document.querySelector<HTMLInputElement>("#cmd-input");
    if (input) {
      input.value = "Switch to attention lens";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
    expect(results()?.querySelectorAll(".cmd-item").length).toBe(1);
    input?.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    expect(store.getState().lens).toBe("attention");
  });

  it("a backdrop click closes the palette", () => {
    palette.open();
    paletteEl()?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(overlay.activeName()).toBeNull();
  });
});
