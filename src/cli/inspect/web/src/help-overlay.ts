// The keyboard cheat-sheet overlay, ported from the served app.js
// openKeyHelp / closeKeyHelp / toggleKeyHelp plus the #key-help control wiring.
// It opens and closes purely through the overlay manager: the manager's mutual
// exclusion tears down whatever overlay was active, so this module no longer
// reaches into the diff overlay or the command palette (the served app.js
// openKeyHelp explicitly closed both — that coupling is gone, replaced by the
// registered teardown).

import { $ } from "./dom";
import { close, type OverlayCloseOptions, open, register } from "./overlay";

// The cheat sheet has no module-local state to tear down — the manager hides its
// node and restores focus — but it registers a teardown so the manager treats it
// uniformly in mutual exclusion.
function onClose(): void {}

/** Open the cheat sheet, focusing its close button. */
export function openKeyHelp(): void {
  open("help", "#key-help-close");
}

/** Close the cheat sheet. */
export function closeKeyHelp(opts: OverlayCloseOptions = {}): void {
  close("help", opts);
}

/** Toggle the cheat sheet open or closed. */
export function toggleKeyHelp(): void {
  const help = $("#key-help");
  if (!help) return;
  if (help.classList.contains("hidden")) openKeyHelp();
  else closeKeyHelp();
}

/** Register the overlay with the manager and wire its close button + backdrop. */
export function initControls(): void {
  const node = $<HTMLElement>("#key-help");
  if (!node) return;
  register("help", {
    node,
    onClose,
    // ? toggles the cheat sheet, so the open sheet owns the key: pressing it
    // again closes. Every other key is the manager's business (Tab trap,
    // Escape, deliberate inertness).
    onKey: (ev) => {
      if (ev.key !== "?") return false;
      ev.preventDefault();
      closeKeyHelp();
      return true;
    },
  });
  $("#key-help-close")?.addEventListener("click", () => closeKeyHelp());
  node.addEventListener("click", (ev) => {
    if (ev.target === node) closeKeyHelp();
  });
}
