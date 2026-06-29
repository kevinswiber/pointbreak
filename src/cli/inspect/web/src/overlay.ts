// The single-overlay manager, ported from the served app.js overlay cluster
// (openOverlay / closeActiveOverlay / closeOverlay / trapOverlayFocus +
// overlayNode / overlayFocusable + the `activeOverlay` global), re-shaped as a
// teardown registry. Each overlay registers its root node plus an opaque
// `onClose` callback; opening one overlay tears down the previously-active one by
// invoking that callback. Because the manager holds the teardown callbacks
// opaquely and imports none of the overlay-content modules (diff / palette /
// help), the overlays no longer import each other to coordinate mutual
// exclusion — that indirection is the import-cycle cut.
//
// `activeOverlay` stays module-local (the transient view-cache belongs here, not
// on the store).

import { $ } from "./dom";
import { OVERLAY_SELECTORS } from "./types";

/** An overlay's registration: its root node plus the teardown to run on close. */
export interface OverlayRegistration {
  node: HTMLElement;
  onClose: () => void;
}

/** Options shared by the close paths. */
export interface OverlayCloseOptions {
  // When false, leave focus where it is instead of restoring the pre-open target
  // (used when one overlay immediately replaces another).
  restoreFocus?: boolean;
}

interface ActiveOverlay {
  name: string;
  node: HTMLElement;
  onClose: () => void;
  priorFocus: Element | null;
}

const registry = new Map<string, OverlayRegistration>();
let activeOverlay: ActiveOverlay | null = null;

/**
 * Register (or re-register) an overlay's node and teardown callback under `name`.
 * The manager invokes `onClose` when the overlay is torn down — directly via
 * {@link closeActive}/{@link close}, or when another overlay opens over it.
 */
export function register(
  name: string,
  registration: OverlayRegistration,
): void {
  registry.set(name, registration);
}

// Resolve an overlay's root node. A registered overlay supplies its node; absent
// a registration, fall back to the static selector map (the served app.js
// resolved every overlay node this way) so a close-by-name still works before any
// registration has run.
function overlayNode(name: string): HTMLElement | null {
  const registered = registry.get(name);
  if (registered) return registered.node;
  const selector = OVERLAY_SELECTORS[name];
  return selector ? $<HTMLElement>(selector) : null;
}

// The focusable descendants of an overlay, in document order, skipping disabled
// and laid-out-away elements (mirrors the served app.js focusable filter).
function overlayFocusable(node: HTMLElement): HTMLElement[] {
  return Array.from(
    node.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter(
    (el) => el.getClientRects().length > 0 || el === document.activeElement,
  );
}

/**
 * Show the overlay registered (or selectable) as `name`. If a different overlay
 * is active, tear it down first (no focus restore — the new overlay takes focus).
 * Focus the `initialSelector` element when given, else the first focusable.
 */
export function open(name: string, initialSelector?: string): void {
  const node = overlayNode(name);
  if (!node) return;
  if (activeOverlay && activeOverlay.name !== name) {
    closeActive({ restoreFocus: false });
  }
  const priorFocus =
    activeOverlay?.name === name
      ? activeOverlay.priorFocus
      : document.activeElement;
  const onClose = registry.get(name)?.onClose ?? noop;
  activeOverlay = { name, node, onClose, priorFocus };
  node.classList.remove("hidden");
  const target = initialSelector
    ? node.querySelector<HTMLElement>(initialSelector)
    : overlayFocusable(node)[0];
  target?.focus();
}

/**
 * Tear down the active overlay: hide it, run its `onClose`, and (unless told not
 * to) restore focus to the element that was focused before it opened.
 */
export function closeActive(opts: OverlayCloseOptions = {}): void {
  if (!activeOverlay) return;
  const current = activeOverlay;
  current.node.classList.add("hidden");
  activeOverlay = null;
  current.onClose();
  if (
    opts.restoreFocus !== false &&
    current.priorFocus instanceof HTMLElement &&
    document.contains(current.priorFocus)
  ) {
    current.priorFocus.focus();
  }
}

/**
 * Close the overlay named `name`. If it is the active overlay, run the full
 * teardown; otherwise just hide its node (no teardown, no focus change) — the
 * served app.js close-by-name contract.
 */
export function close(name: string, opts: OverlayCloseOptions = {}): void {
  if (activeOverlay?.name === name) {
    closeActive(opts);
    return;
  }
  const node = overlayNode(name);
  if (node) node.classList.add("hidden");
}

/**
 * Keep Tab focus within the active overlay: wrap from the last focusable to the
 * first (and Shift+Tab the reverse), and pull focus back in if it has escaped.
 * Returns true when the event was handled (the keyboard layer then stops).
 */
export function trapFocus(ev: KeyboardEvent): boolean {
  if (ev.key !== "Tab" || !activeOverlay) return false;
  const focusable = overlayFocusable(activeOverlay.node);
  if (!focusable.length) {
    ev.preventDefault();
    return true;
  }
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!activeOverlay.node.contains(document.activeElement)) {
    ev.preventDefault();
    first.focus();
    return true;
  }
  if (ev.shiftKey && document.activeElement === first) {
    ev.preventDefault();
    last.focus();
    return true;
  }
  if (!ev.shiftKey && document.activeElement === last) {
    ev.preventDefault();
    first.focus();
    return true;
  }
  return false;
}

function noop(): void {}
