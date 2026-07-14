// The composition root: wire the whole inspector port into a runnable (but still
// unserved) app. Ported from the served app.js `wireControls` + the bottom-of-file
// bootstrap tail (`wireControls()` / `popstate` / `hashchange` / `load().then(…)`).
//
// This is the ONLY place `subscribe(render)` is called — the single store
// subscriber is registered here, once — and the only place the two `document`
// delegates (`keydown→onKey`, `click→resolveRef`) and the bootstrap tail live. Every
// other module exposes an `initControls()` for its own fixed-id / delegated wiring;
// `main` calls them in order, wires the toolbar, then runs the load tail. `main`
// returns the load chain so a test can await first paint; the served entry (a later
// emit flip) invokes `main()` and ignores the return — the port stays parallel and
// unserved here, so nothing calls `main()` automatically yet.

import {
  AuthCoordinator,
  bootstrapCapability,
  installAuthCoordinator,
  promptForCredential,
  requestReconnect,
} from "./auth";
import { initControls as initAutocomplete } from "./autocomplete";
import {
  configureConnectionActions,
  initConnectionControls,
  renderConnectionChrome,
  setRefreshState,
} from "./connection";
import { load, loadIdentity, maybeReloadForQuery, pollFreshness } from "./data";
import { initControls as initDetail } from "./detail";
import {
  DIFF_ROUTE_CLEARED,
  initControls as initDiff,
} from "./diff/controller";
import { createDisclosure } from "./disclosure";
import { $ } from "./dom";
import { toggleTimelineFollow } from "./follow";
import { initControls as initHelp } from "./help-overlay";
import { jumpLensBoundary, onKey } from "./keyboard";
import { presentTypes } from "./model";
import { onDocumentClick } from "./navigation";
import { initControls as initPalette } from "./palette";
import { applyPrefs, initControls as initPrefs } from "./prefs";
import { initControls as initRender, render } from "./render";
import { applyHash, navigate } from "./router";
import { initControls as initSplit } from "./split";
import { getState, subscribe } from "./store";
import { DEFAULT_LENS, LENSES } from "./types";

let pollTimer: ReturnType<typeof setInterval> | null = null;
let unsubscribers: Array<() => void> = [];

export function stopPolling(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  for (const unsubscribe of unsubscribers) unsubscribe();
  unsubscribers = [];
}

function startPolling(): void {
  setRefreshState("watching");
  if (pollTimer !== null) return;
  pollTimer = setInterval(() => {
    void pollFreshness();
  }, 3000);
}

function boundaryTarget(kind: "latest" | "oldest"): "first" | "last" {
  const state = getState();
  if (state.lens === "attention") return kind === "latest" ? "last" : "first";
  const latestIsFirst = state.order === "desc";
  return (kind === "latest") === latestIsFirst ? "first" : "last";
}

// The toolbar controls that aren't owned by a module's initControls: the lens tabs
// and the timeline filter/order controls. All navigate through the router (commit →
// the subscriber repaints).
function wireToolbar(): void {
  const viewDisclosure = createDisclosure({
    container: "#view-controls",
    trigger: "#view-toggle",
    panel: "#view-panel",
  });
  createDisclosure({
    container: "#filter-controls",
    trigger: "#filters-toggle",
    panel: "#filters-panel",
  });
  for (const tab of document.querySelectorAll<HTMLElement>(".lens-tab")) {
    tab.addEventListener("click", () => {
      const lens = tab.dataset.lens;
      // A lens tab names a record destination: from the diff page it exits the
      // page onto that lens instead of changing hidden state underneath.
      navigate({
        lens: lens && LENSES.includes(lens) ? lens : DEFAULT_LENS,
        ...DIFF_ROUTE_CLEARED,
      });
    });
  }
  const filterText = $<HTMLInputElement>("#filter-text");
  filterText?.addEventListener("input", () => {
    navigate({ filterText: filterText.value }, { replace: true });
  });
  $("#filter-clear")?.addEventListener("click", () => {
    navigate(
      {
        filterText: "",
        filterTrack: "",
        filterSnapshot: "",
        enabledTypes: new Set(presentTypes()),
      },
      { replace: true },
    );
  });
  $("#view-panel")?.addEventListener("change", (event) => {
    const input = event.target;
    if (!(input instanceof HTMLInputElement) || !input.checked) return;
    if (input.name === "view-order") {
      navigate(
        { order: input.value === "asc" ? "asc" : "desc" },
        { replace: true },
      );
    }
  });
  $<HTMLSelectElement>("#sort-picker")?.addEventListener("change", (e) => {
    const value = (e.target as HTMLSelectElement).value;
    navigate(
      { sortKey: value === "activity" ? "activity" : "captured" },
      { replace: true },
    );
  });
  $("#jump-latest")?.addEventListener("click", () => {
    jumpLensBoundary(boundaryTarget("latest"));
    viewDisclosure.close(true);
  });
  $("#jump-oldest")?.addEventListener("click", () => {
    jumpLensBoundary(boundaryTarget("oldest"));
    viewDisclosure.close(true);
  });
  $("#follow-toggle")?.addEventListener("click", () => {
    void toggleTimelineFollow();
  });
}

/**
 * Bootstrap the inspector: apply prefs before first paint, register the single
 * render subscriber, wire every module's controls + the toolbar + the two document
 * delegates, then run the load tail (apply the route, flip the freshness status, and
 * start the poll). Returns the load chain for deterministic test await.
 */
export function main(
  options: { readonly reload?: () => void } = {},
): Promise<void> {
  stopPolling();
  const capability = bootstrapCapability();
  if (capability.token !== null) {
    (options.reload ?? (() => location.reload()))();
    return Promise.resolve();
  }
  applyPrefs();
  unsubscribers.push(subscribe(render));
  // Subscribed after render so the query watcher observes render's type-toggle
  // seeding: a query change re-fetches page 1, and an unchanged query is a no-op.
  unsubscribers.push(subscribe(maybeReloadForQuery));
  initPrefs();
  initDiff();
  initPalette();
  initHelp();
  initRender();
  initDetail();
  initSplit();
  initAutocomplete();
  initConnectionControls();
  wireToolbar();
  document.addEventListener("keydown", onKey);
  document.addEventListener("click", onDocumentClick);
  window.addEventListener("popstate", applyHash);
  window.addEventListener("hashchange", applyHash);
  const coordinator = new AuthCoordinator({
    prompt: promptForCredential,
    navigate: (url) => location.replace(url),
    currentOrigin: () => location.origin,
    currentRoute: () => location.hash,
  });
  installAuthCoordinator(coordinator);
  const retry = async () => {
    const [loaded] = await Promise.all([load(), loadIdentity()]);
    if (loaded) {
      applyHash();
      startPolling();
    }
  };
  configureConnectionActions({
    retry,
    reconnect: async () => {
      if (await requestReconnect()) await retry();
    },
  });
  render();
  renderConnectionChrome();
  // Identity is static per session — fetch it once here, in parallel with the first
  // data load, never on the freshness reload path.
  return Promise.all([load(), loadIdentity()]).then(([loaded]) => {
    if (!loaded) return;
    applyHash();
    startPolling();
  });
}
