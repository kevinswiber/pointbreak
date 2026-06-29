import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mountInspectorDom, resetDom } from "./support/dom";

// The keyboard cheat-sheet overlay opens and closes purely through the overlay
// manager (register + open/close) and wires its own close button + backdrop in
// initControls() — it imports neither the diff overlay nor the palette, so the
// manager's mutual exclusion is what tears the other overlays down. The module
// state lives behind the manager (module-local), so reset the registry per test.
type HelpOverlay = typeof import("../src/help-overlay");
let help: HelpOverlay;

beforeEach(async () => {
  vi.resetModules();
  help = await import("../src/help-overlay");
  mountInspectorDom();
});

afterEach(() => {
  resetDom();
});

/** The cheat-sheet root, or throw — a test-setup guard. */
function sheet(): HTMLElement {
  const el = document.querySelector<HTMLElement>("#key-help");
  if (!el) throw new Error("missing #key-help");
  return el;
}

const isOpen = () => !sheet().classList.contains("hidden");

describe("open / close / toggle", () => {
  it("openKeyHelp shows the cheat sheet and focuses its close button", () => {
    help.initControls();
    expect(isOpen()).toBe(false);
    help.openKeyHelp();
    expect(isOpen()).toBe(true);
    expect(document.activeElement).toBe(
      document.querySelector("#key-help-close"),
    );
  });

  it("closeKeyHelp hides the cheat sheet", () => {
    help.initControls();
    help.openKeyHelp();
    help.closeKeyHelp();
    expect(isOpen()).toBe(false);
  });

  it("toggleKeyHelp flips the cheat sheet open then closed", () => {
    help.initControls();
    help.toggleKeyHelp();
    expect(isOpen()).toBe(true);
    help.toggleKeyHelp();
    expect(isOpen()).toBe(false);
  });

  it("openKeyHelp works through the manager even before initControls registers it", () => {
    // open() falls back to the static overlay selector, so the cheat sheet still
    // shows; this keeps the bootstrap order forgiving.
    help.openKeyHelp();
    expect(isOpen()).toBe(true);
  });
});

describe("initControls wiring", () => {
  it("closes the cheat sheet when its close button is clicked", () => {
    help.initControls();
    help.openKeyHelp();
    document.querySelector<HTMLElement>("#key-help-close")?.click();
    expect(isOpen()).toBe(false);
  });

  it("closes on a backdrop click but not on a click inside the card", () => {
    help.initControls();
    help.openKeyHelp();

    // A click on the inner card bubbles to the backdrop listener but is not the
    // backdrop itself, so the sheet stays open.
    document.querySelector<HTMLElement>(".key-help-card")?.click();
    expect(isOpen()).toBe(true);

    // A click on the backdrop element itself closes the sheet.
    sheet().click();
    expect(isOpen()).toBe(false);
  });
});
