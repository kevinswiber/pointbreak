import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { HistoryDoc, RevisionsDoc, ThreadsDoc } from "../src/store";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import threadsJson from "./fixtures/threads.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetSnapshotResponse,
  uninstallFetchMock,
} from "./support/fetch";

// `render.ts` is the single store-subscriber: a `() => void` that paints one frame
// from `getState()` — stats, diagnostics, the type toggles (facet counts), the lens
// switcher, the master pane (delegating to the lenses), the detail pane (delegating
// to detail), scroll-into-view, and the diff overlay reconciler. It never calls
// `navigate`/`commit`; the once-installed `#master`/`#filter-types` delegates own the
// commits. The store, the render module's `lastMasterLens`, detail's
// `shownCompositeId`, and the overlay manager are module singletons, so reset and
// re-import them before each test.
type Store = typeof import("../src/store");
type Render = typeof import("../src/render");
let store: Store;
let render: Render;

const OBS_EVENT =
  "evt:sha256:8ac34bc85b48ed6623660a174b024bd9099edd09877180bfa87101cc76ac6058";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";

function $<T extends Element = Element>(sel: string): T | null {
  return document.querySelector<T>(sel);
}

beforeEach(async () => {
  const vitest = await import("vitest");
  vitest.vi.resetModules();
  store = await import("../src/store");
  render = await import("../src/render");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
    threads: threadsJson as unknown as ThreadsDoc,
  });
  render.initControls();
});

afterEach(() => {
  uninstallFetchMock();
  resetSnapshotResponse();
  resetDom();
});

describe("render is a no-arg projection of getState()", () => {
  it("takes no arguments (the subscribe(render) signature)", () => {
    expect(render.render.length).toBe(0);
  });

  it("paints the stat row from the loaded document counts", () => {
    render.render();
    expect($("#stat-events")?.textContent).toBe("8 events");
    expect($("#stat-units")?.textContent).toBe("1 units");
    expect($("#stat-threads")?.textContent).toBe("1 threads");
    // The freshness hash is the short form of the event-set hash.
    expect($("#stat-hash")?.textContent).toBe("e81f297a301a");
  });

  it("hides diagnostics when empty and surfaces them when present", () => {
    render.render();
    expect($("#diagnostics")?.classList.contains("hidden")).toBe(true);

    store.commit({
      history: {
        ...(historyJson as unknown as HistoryDoc),
        diagnostics: [{ code: "stale-store", message: "reload to refresh" }],
      },
    });
    render.render();
    const diag = $("#diagnostics");
    expect(diag?.classList.contains("hidden")).toBe(false);
    expect(diag?.textContent).toContain("stale-store");
    expect(diag?.textContent).toContain("reload to refresh");
  });
});

describe("renderTypeToggles (facet distribution + aria-pressed)", () => {
  it("renders one toggle per present type with its facet count and pressed state", () => {
    render.render();
    const container = $("#filter-types");
    expect((container?.querySelectorAll(".type-toggle").length ?? 0) > 0).toBe(
      true,
    );
    const obs = $<HTMLElement>('[data-type="review_observation_recorded"]');
    expect(obs).not.toBeNull();
    expect(obs?.getAttribute("aria-pressed")).toBe("true");
    expect(obs?.querySelector(".type-count")?.textContent).toBe("1");
    const assess = $<HTMLElement>('[data-type="review_assessment_recorded"]');
    expect(assess?.querySelector(".type-count")?.textContent).toBe("2");
  });

  it("the #filter-types delegate toggles a type and navigates (replace)", () => {
    render.render();
    const obs = $<HTMLElement>('[data-type="review_observation_recorded"]');
    expect(
      store.getState().enabledTypes.has("review_observation_recorded"),
    ).toBe(true);
    obs?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(
      store.getState().enabledTypes.has("review_observation_recorded"),
    ).toBe(false);
  });
});

describe("renderLensSwitcher + renderMaster (lens dispatch + scaffold)", () => {
  it("marks the active lens tab and paints the timeline lens by default", () => {
    render.render();
    expect(
      $('.lens-tab[data-lens="timeline"]')?.getAttribute("aria-pressed"),
    ).toBe("true");
    expect($('.lens-tab[data-lens="list"]')?.getAttribute("aria-pressed")).toBe(
      "false",
    );
    const master = $("#master");
    expect(master?.querySelector("#timeline")).not.toBeNull();
    expect((master?.querySelectorAll("#timeline .event").length ?? 0) > 0).toBe(
      true,
    );
  });

  it("dispatches the list lens to renderRevisionList (#units)", () => {
    store.commit({ lens: "list" });
    render.render();
    expect($('.lens-tab[data-lens="list"]')?.getAttribute("aria-pressed")).toBe(
      "true",
    );
    const master = $("#master");
    expect(master?.querySelector("#units")).not.toBeNull();
    expect(master?.querySelector("#units .unit-card")).not.toBeNull();
  });

  it("dispatches the threads lens to renderRevisions (#revisions)", () => {
    store.commit({ lens: "threads" });
    render.render();
    const master = $("#master");
    expect(master?.querySelector("#revisions")).not.toBeNull();
    expect(master?.querySelector("#revisions .thread-card")).not.toBeNull();
  });

  it("rebuilds the lens scaffold only on a lens change (idempotent re-render)", () => {
    render.render();
    render.render();
    const master = $("#master");
    // Two renders at the same lens leave exactly one timeline body, repopulated.
    expect(master?.querySelectorAll("#timeline").length).toBe(1);
    expect((master?.querySelectorAll("#timeline .event").length ?? 0) > 0).toBe(
      true,
    );
  });
});

describe("renderSelected (delegates to detail)", () => {
  it("paints the event detail for a selected event", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT } });
    render.render();
    const detail = $("#detail");
    expect(detail?.querySelector("dl.kv")).not.toBeNull();
    expect(detail?.textContent).toContain("the return value changed");
  });

  it("prompts when nothing is selected", () => {
    store.commit({ selected: { kind: null, id: null } });
    render.render();
    expect($("#detail")?.textContent).toContain("Select an event or revision");
  });
});

describe("the #master delegate (selection / open-diff / cue filter, ref-chip guard)", () => {
  it("selects an event on a timeline row click", () => {
    render.render();
    const row = $<HTMLElement>("#master #timeline .event[data-event-id]");
    expect(row).not.toBeNull();
    const id = row?.dataset.eventId;
    row?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: "event", id });
  });

  it("opens the snapshot diff on a list-card diff button click", () => {
    store.commit({ lens: "list" });
    render.render();
    const diffBtn = $<HTMLElement>("#master [data-open-diff]");
    expect(diffBtn?.dataset.openDiff).toBe(OBJ);
    diffBtn?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().diff).toBe(OBJ);
  });

  it("applies an attention-cue filter on click", () => {
    store.commit({ lens: "list" });
    render.render();
    const cue = $<HTMLElement>("#master [data-attention-query]");
    const query = cue?.dataset.attentionQuery;
    expect(query).toBeTruthy();
    cue?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().filterText).toBe(query);
  });

  it("lets ref chips fall through to the navigation delegate (no selection)", () => {
    render.render();
    const row = $<HTMLElement>("#master #timeline .event[data-event-id]");
    // A ref chip inside a selectable row must not trigger row selection — the
    // navigation delegate resolves data-ref-kind.
    const chip = document.createElement("span");
    chip.setAttribute("data-ref-kind", "rev");
    row?.appendChild(chip);
    chip.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected.id).toBeNull();
  });
});

describe("scrollSelectionIntoView materializes an off-screen virtual row", () => {
  // Build enough entries that the timeline virtualizes; only a window is in the
  // DOM at a time, so selecting an off-screen entry must scroll its index into
  // the window before it can be revealed.
  function seedManyAndVirtualize(): HTMLElement {
    const entries = Array.from({ length: 500 }, (_, i) => ({
      eventId: `e${i}`,
      eventType: "review_observation_recorded",
      occurredAt: `unix-ms:${1782699185391 + i}`,
    }));
    store.commit({
      history: { entries, diagnostics: [] } as unknown as HistoryDoc,
      lens: "timeline",
    });
    render.render(); // creates #timeline and paints the top window
    const list = $<HTMLElement>("#timeline") as HTMLElement;
    Object.defineProperty(list, "clientHeight", {
      configurable: true,
      value: 500,
    });
    Object.defineProperty(list, "scrollTop", {
      configurable: true,
      writable: true,
      value: 0,
    });
    return list;
  }

  it("scrolls the selected off-screen event into the rendered window", () => {
    seedManyAndVirtualize();
    // Default desc order puts e0 (oldest) last — far below the top window.
    const targetId = "e0";
    render.render();
    expect($(`#timeline li[data-event-id="${targetId}"]`)).toBeNull();

    store.commit({ selected: { kind: "event", id: targetId } });
    render.render();
    expect($(`#timeline li[data-event-id="${targetId}"]`)).not.toBeNull();
  });
});
