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

import { load, loadIdentity, maybeReloadForQuery, pollFreshness } from "./data";
import { initControls as initDetail } from "./detail";
import { initControls as initDiff } from "./diff/controller";
import { $ } from "./dom";
import { initControls as initHelp } from "./help-overlay";
import { onKey } from "./keyboard";
import { scheduleTimelineRemeasure } from "./lenses/timeline";
import { presentTypes } from "./model";
import { onDocumentClick } from "./navigation";
import { initControls as initPalette } from "./palette";
import { applyPrefs, initControls as initPrefs } from "./prefs";
import { initControls as initRender, render } from "./render";
import { applyHash, navigate } from "./router";
import { initControls as initSplit } from "./split";
import { getState, subscribe } from "./store";
import { DEFAULT_LENS, LENSES } from "./types";

// The toolbar controls that aren't owned by a module's initControls: the lens tabs
// and the timeline filter/order controls. All navigate through the router (commit →
// the subscriber repaints).
function wireToolbar(): void {
  for (const tab of document.querySelectorAll<HTMLElement>(".lens-tab")) {
    tab.addEventListener("click", () => {
      const lens = tab.dataset.lens;
      navigate({ lens: lens && LENSES.includes(lens) ? lens : DEFAULT_LENS });
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
  $("#order-toggle")?.addEventListener("click", () => {
    navigate(
      { order: getState().order === "desc" ? "asc" : "desc" },
      { replace: true },
    );
  });
  // Cross-module trigger the composition root owns: a density flip changes row
  // heights without resizing the #timeline box, so the timeline's own size
  // observer can never see it. The toggle itself stays prefs-owned; the settle
  // delay means listener order against prefs' handler doesn't matter.
  $("#density-toggle")?.addEventListener("click", scheduleTimelineRemeasure);
}

/**
 * Bootstrap the inspector: apply prefs before first paint, register the single
 * render subscriber, wire every module's controls + the toolbar + the two document
 * delegates, then run the load tail (apply the route, flip the freshness status, and
 * start the poll). Returns the load chain for deterministic test await.
 */
export function main(): Promise<void> {
  applyPrefs();
  subscribe(render);
  // Subscribed after render so the query watcher observes render's type-toggle
  // seeding: a query change re-fetches page 1, and an unchanged query is a no-op.
  subscribe(maybeReloadForQuery);
  initPrefs();
  initDiff();
  initPalette();
  initHelp();
  initRender();
  initDetail();
  initSplit();
  wireToolbar();
  document.addEventListener("keydown", onKey);
  document.addEventListener("click", onDocumentClick);
  window.addEventListener("popstate", applyHash);
  window.addEventListener("hashchange", applyHash);
  // Identity is static per session — fetch it once here, in parallel with the first
  // data load, never on the freshness reload path.
  return Promise.all([load(), loadIdentity()]).then(() => {
    applyHash();
    const refresh = $("#refresh");
    if (refresh) refresh.textContent = "watching";
    setInterval(() => {
      void pollFreshness();
    }, 3000);
  });
}
