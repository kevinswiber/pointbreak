import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mountInspectorDom, resetDom } from "./support/dom";

// The single-overlay manager is a teardown registry: each overlay registers its
// root node plus an opaque `onClose` callback, and the manager enforces mutual
// exclusion (opening one tears down the previously-active one) and traps Tab
// focus. It imports none of the overlay-content modules — that opaque-callback
// indirection is what severs the palette <-> help import cycle. The
// module keeps its `activeOverlay` and registry module-local, so reset the module
// registry before each test and remount the fixed-id DOM the overlays live in.
type Overlay = typeof import("../src/overlay");
let overlay: Overlay;

beforeEach(async () => {
  vi.resetModules();
  overlay = await import("../src/overlay");
  mountInspectorDom();
});

afterEach(() => {
  resetDom();
});

/** The element matching `sel`, or throw — a test-setup guard, never a module path. */
function need(sel: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(sel);
  if (!el) throw new Error(`missing fixture element ${sel}`);
  return el;
}

describe("register + open", () => {
  it("shows a registered overlay's node", () => {
    const help = need("#key-help");
    overlay.register("help", { node: help, onClose: () => {} });
    expect(help.classList.contains("hidden")).toBe(true);
    overlay.open("help");
    expect(help.classList.contains("hidden")).toBe(false);
  });

  it("focuses the requested initial element when opening", () => {
    const help = need("#key-help");
    overlay.register("help", { node: help, onClose: () => {} });
    overlay.open("help", "#key-help-close");
    expect(document.activeElement).toBe(need("#key-help-close"));
  });

  it("ignores an open for an unregistered, unselectable name", () => {
    // No registration and no OVERLAY_SELECTORS entry: nothing to show, no throw.
    expect(() => overlay.open("nope")).not.toThrow();
  });
});

describe("mutual exclusion (the teardown cut)", () => {
  it("hides the previously-active overlay and invokes its onClose when another opens", () => {
    const help = need("#key-help");
    const palette = need("#cmd-palette");
    const helpClose = vi.fn();
    const paletteClose = vi.fn();
    overlay.register("help", { node: help, onClose: helpClose });
    overlay.register("palette", { node: palette, onClose: paletteClose });

    overlay.open("help");
    overlay.open("palette");

    expect(palette.classList.contains("hidden")).toBe(false);
    expect(help.classList.contains("hidden")).toBe(true);
    // The replaced overlay's teardown ran exactly once; the opener's did not.
    expect(helpClose).toHaveBeenCalledTimes(1);
    expect(paletteClose).not.toHaveBeenCalled();
  });

  it("does not tear down or re-run onClose when the same overlay re-opens", () => {
    const help = need("#key-help");
    const helpClose = vi.fn();
    overlay.register("help", { node: help, onClose: helpClose });
    overlay.open("help");
    overlay.open("help");
    expect(help.classList.contains("hidden")).toBe(false);
    expect(helpClose).not.toHaveBeenCalled();
  });
});

describe("closeActive / close", () => {
  it("closeActive hides the active overlay and invokes its onClose", () => {
    const help = need("#key-help");
    const helpClose = vi.fn();
    overlay.register("help", { node: help, onClose: helpClose });
    overlay.open("help");
    overlay.closeActive();
    expect(help.classList.contains("hidden")).toBe(true);
    expect(helpClose).toHaveBeenCalledTimes(1);
  });

  it("closeActive is a no-op when nothing is open", () => {
    const helpClose = vi.fn();
    overlay.register("help", { node: need("#key-help"), onClose: helpClose });
    overlay.closeActive();
    expect(helpClose).not.toHaveBeenCalled();
  });

  it("close(name) tears down the named overlay when it is the active one", () => {
    const help = need("#key-help");
    const helpClose = vi.fn();
    overlay.register("help", { node: help, onClose: helpClose });
    overlay.open("help");
    overlay.close("help");
    expect(help.classList.contains("hidden")).toBe(true);
    expect(helpClose).toHaveBeenCalledTimes(1);
  });

  it("close(name) only hides a non-active overlay, without running its onClose", () => {
    const help = need("#key-help");
    const palette = need("#cmd-palette");
    const helpClose = vi.fn();
    overlay.register("help", { node: help, onClose: helpClose });
    overlay.register("palette", { node: palette, onClose: () => {} });
    overlay.open("palette");
    overlay.close("help");
    expect(help.classList.contains("hidden")).toBe(true);
    expect(palette.classList.contains("hidden")).toBe(false);
    expect(helpClose).not.toHaveBeenCalled();
  });
});

describe("focus restoration", () => {
  it("restores focus to the element active before the overlay opened", () => {
    const opener = need("#theme-toggle");
    opener.focus();
    expect(document.activeElement).toBe(opener);

    overlay.register("help", { node: need("#key-help"), onClose: () => {} });
    overlay.open("help", "#key-help-close");
    expect(document.activeElement).toBe(need("#key-help-close"));

    overlay.closeActive();
    expect(document.activeElement).toBe(opener);
  });

  it("does not restore focus when closeActive is told not to", () => {
    const opener = need("#theme-toggle");
    opener.focus();
    overlay.register("help", { node: need("#key-help"), onClose: () => {} });
    overlay.open("help", "#key-help-close");
    overlay.closeActive({ restoreFocus: false });
    expect(document.activeElement).not.toBe(opener);
  });
});

describe("trapFocus", () => {
  /** A two-button overlay panel registered + opened under `name`. */
  function openPanel(name: string): { first: HTMLElement; last: HTMLElement } {
    const panel = document.createElement("div");
    panel.innerHTML = `<button id="${name}-a">a</button><button id="${name}-b">b</button>`;
    document.body.appendChild(panel);
    overlay.register(name, { node: panel, onClose: () => {} });
    overlay.open(name);
    return { first: need(`#${name}-a`), last: need(`#${name}-b`) };
  }

  function tab(opts: { shiftKey?: boolean; key?: string } = {}): KeyboardEvent {
    return new KeyboardEvent("keydown", {
      key: opts.key ?? "Tab",
      shiftKey: opts.shiftKey ?? false,
      cancelable: true,
    });
  }

  it("ignores a non-Tab key", () => {
    openPanel("p");
    const ev = tab({ key: "Enter" });
    expect(overlay.trapFocus(ev)).toBe(false);
    expect(ev.defaultPrevented).toBe(false);
  });

  it("is inert when no overlay is open", () => {
    expect(overlay.trapFocus(tab())).toBe(false);
  });

  it("wraps Tab from the last focusable back to the first", () => {
    const { first, last } = openPanel("p");
    last.focus();
    const ev = tab();
    expect(overlay.trapFocus(ev)).toBe(true);
    expect(ev.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);
  });

  it("wraps Shift+Tab from the first focusable to the last", () => {
    const { first, last } = openPanel("p");
    first.focus();
    const ev = tab({ shiftKey: true });
    expect(overlay.trapFocus(ev)).toBe(true);
    expect(document.activeElement).toBe(last);
  });

  it("pulls focus back inside when it has escaped the overlay", () => {
    const { first } = openPanel("p");
    const outside = need("#theme-toggle");
    outside.focus();
    const ev = tab();
    expect(overlay.trapFocus(ev)).toBe(true);
    expect(document.activeElement).toBe(first);
  });
});

// The manager's whole keyboard contract: one entry point owns Tab (the trap),
// Escape (close), delegation to the active overlay's registered keys, and
// deliberate inertness for everything else — inert keys are NOT preventDefaulted,
// so typing into overlay-internal inputs (the palette's #cmd-input) keeps working.
describe("handleOverlayKey", () => {
  function kbd(init: KeyboardEventInit): KeyboardEvent {
    return new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      ...init,
    });
  }

  it("returns false when no overlay is active", () => {
    expect(overlay.handleOverlayKey(kbd({ key: "j" }))).toBe(false);
  });

  it("delegates a registered key to the active overlay's onKey", () => {
    const help = need("#key-help");
    const seen: string[] = [];
    overlay.register("help", {
      node: help,
      onClose: () => {},
      onKey: (ev) => {
        seen.push(ev.key);
        return true;
      },
    });
    overlay.open("help");
    expect(overlay.handleOverlayKey(kbd({ key: "n" }))).toBe(true);
    expect(seen).toEqual(["n"]);
  });

  it("closes the active overlay on Escape", () => {
    const help = need("#key-help");
    let closed = false;
    overlay.register("help", {
      node: help,
      onClose: () => {
        closed = true;
      },
    });
    overlay.open("help");
    expect(overlay.handleOverlayKey(kbd({ key: "Escape" }))).toBe(true);
    expect(closed).toBe(true);
    expect(overlay.activeName()).toBe(null);
  });

  it("owns Escape even when the active overlay registered its own onKey", () => {
    const help = need("#key-help");
    const seen: string[] = [];
    let closed = false;
    overlay.register("help", {
      node: help,
      onClose: () => {
        closed = true;
      },
      onKey: (ev) => {
        seen.push(ev.key);
        return true;
      },
    });
    overlay.open("help");
    expect(overlay.handleOverlayKey(kbd({ key: "Escape" }))).toBe(true);
    // Escape never reaches the overlay's own map — the manager owns it.
    expect(seen).toEqual([]);
    expect(closed).toBe(true);
  });

  it("reports an unregistered key as handled (inert) without preventDefault", () => {
    const help = need("#key-help");
    overlay.register("help", { node: help, onClose: () => {} });
    overlay.open("help");
    const ev = kbd({ key: "j" });
    expect(overlay.handleOverlayKey(ev)).toBe(true);
    // Typing into overlay inputs must keep working.
    expect(ev.defaultPrevented).toBe(false);
  });

  it("treats a key the overlay's onKey declines as inert without preventDefault", () => {
    const help = need("#key-help");
    overlay.register("help", {
      node: help,
      onClose: () => {},
      onKey: () => false,
    });
    overlay.open("help");
    const ev = kbd({ key: "x" });
    expect(overlay.handleOverlayKey(ev)).toBe(true);
    expect(ev.defaultPrevented).toBe(false);
  });

  it("still traps Tab within the active overlay", () => {
    const panel = document.createElement("div");
    panel.innerHTML = `<button id="hk-a">a</button><button id="hk-b">b</button>`;
    document.body.appendChild(panel);
    overlay.register("hk", { node: panel, onClose: () => {} });
    overlay.open("hk");
    need("#hk-b").focus();
    const ev = kbd({ key: "Tab" });
    expect(overlay.handleOverlayKey(ev)).toBe(true);
    expect(ev.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(need("#hk-a"));
  });
});
