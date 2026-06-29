import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { HistoryDoc, ObjectsDoc, RevisionsDoc } from "../src/store";
import historyJson from "./fixtures/history.json";
import objectsJson from "./fixtures/objects.json";
import revisionsJson from "./fixtures/revisions.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetObjectResponse,
  uninstallFetchMock,
} from "./support/fetch";

// `detail.ts` paints the `#detail` pane from the single selection: the event
// detail (composing the pure projection readback), the revision composite page
// (fetched on demand via `http`, mounting the pure `cards` renderers), and the
// state-bound `staleFactSectionContext` fed into the pure `cards.factSection`. It
// mutates `#detail` and reads state / `http`; it never calls render (the store
// subscriber repaints on commit). The "open diff" affordance delegates to
// `diff/controller` through the once-installed `#detail` handler. The store, the
// module's `shownCompositeId`, and the overlay manager are module singletons, so
// reset and re-import them before each test.
type Store = typeof import("../src/store");
type Detail = typeof import("../src/detail");
let store: Store;
let detail: Detail;

const OBS_EVENT =
  "evt:sha256:8ac34bc85b48ed6623660a174b024bd9099edd09877180bfa87101cc76ac6058";
const OBS_ID =
  "obs:sha256:752a5b0ab30cfa3aa062bcf6f11b4c6ee3dcfd055207b6a995b91bf81ffec8d9";
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const SUCCESSOR =
  "rev:sha256:1111111111111111111111111111111111111111111111111111111111111111";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

function detailEl(): HTMLElement {
  const el = document.querySelector<HTMLElement>("#detail");
  if (!el) throw new Error("#detail not mounted");
  return el;
}

beforeEach(async () => {
  const vitest = await import("vitest");
  vitest.vi.resetModules();
  store = await import("../src/store");
  detail = await import("../src/detail");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
    objects: objectsJson as unknown as ObjectsDoc,
  });
  detail.initControls();
});

afterEach(() => {
  uninstallFetchMock();
  resetObjectResponse();
  resetDom();
});

describe("renderDetail (event detail / empty prompt)", () => {
  it("prompts to select when nothing is selected", () => {
    store.commit({ selected: { kind: null, id: null } });
    detail.renderDetail();
    expect(detailEl().textContent).toContain("Select an event or revision");
  });

  it("paints the selected event's identity, body, and raw payload", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT } });
    detail.renderDetail();
    const el = detailEl();
    // The identity table carries the event id and the addressed revision.
    expect(el.querySelector("dl.kv")?.textContent).toContain("eventId");
    expect(el.innerHTML).toContain(OBS_EVENT);
    // The observation body renders.
    expect(el.textContent).toContain("the return value changed");
    // The raw event JSON is dumped for inspection.
    expect(el.querySelector("pre")?.textContent).toContain(
      "review_observation",
    );
    // Embedded ids linkify into navigable ref chips (the navigation delegate resolves them).
    expect(el.querySelector("[data-ref-kind]")).not.toBeNull();
  });

  it("renders the diff affordance with the open-diff / hash / focus datasets", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT } });
    detail.renderDetail();
    const btn = document.querySelector<HTMLElement>("#detail-diff-btn");
    expect(btn).not.toBeNull();
    expect(btn?.dataset.openDiff).toBe(OBJ);
    expect(btn?.dataset.diffHash).toBe(ARTIFACT);
    expect(btn?.dataset.diffFocus).toBe(OBS_ID);
  });
});

describe("the #detail open-diff delegate (installed once, delegates to diff/controller)", () => {
  it("opens the diff overlay route when the detail diff button is clicked", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT } });
    detail.renderDetail();
    document
      .querySelector("#detail-diff-btn")
      ?.dispatchEvent(new Event("click", { bubbles: true }));
    // The affordance delegates to diff/controller.openDiff → router.navigate,
    // committing the diff route (and the observation focus) without detail
    // calling render or importing router itself.
    expect(store.getState().diff).toBe(OBJ);
    expect(store.getState().diffHash).toBe(ARTIFACT);
    expect(store.getState().focus).toBe(OBS_ID);
  });
});

describe("openRevision / renderRevisionPage (the composite page, fetched via http)", () => {
  it("fetches the revision and paints the composite page sections", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const el = detailEl();
    expect(el.querySelector(".unit-page")).not.toBeNull();
    const text = el.textContent ?? "";
    expect(text).toContain("Revision");
    expect(text).toContain("Current assessment");
    expect(text).toContain("Observations (1)");
    expect(text).toContain("Input requests (1)");
    expect(text).toContain("Assessments (2)");
    expect(text).toContain("Validation checks (2)");
    // The revision page carries the annotated-diff and show-in-timeline affordances.
    const diffBtn = document.querySelector<HTMLElement>("#up-diff-btn");
    expect(diffBtn?.dataset.openDiff).toBe(OBJ);
    expect(diffBtn?.dataset.diffHash).toBe(ARTIFACT);
    expect(
      document.querySelector<HTMLElement>("#up-timeline-btn")?.dataset
        .revealRevision,
    ).toBe(REV);
  });

  it("opens the revision page diff via the #detail delegate", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    document
      .querySelector("#up-diff-btn")
      ?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().diff).toBe(OBJ);
    expect(store.getState().diffHash).toBe(ARTIFACT);
  });
});

describe("staleFactSectionContext (state-bound, fed into the pure factSection)", () => {
  it("repeats the superseded-by context near each fact section of a stale revision", async () => {
    // Mark the captured revision superseded so its facts carry the stale context.
    store.commit({
      objects: {
        threads: [],
        revisionClassification: {
          [REV]: {
            state: "superseded",
            supersededBy: [SUCCESSOR],
            supersedes: [],
          },
        },
      } as unknown as ObjectsDoc,
    });
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const stale = detailEl().querySelectorAll(".fact-stale-context");
    // One per fact section (Observations / Input requests / Assessments /
    // Validation checks), each naming the successor.
    expect(stale.length).toBeGreaterThanOrEqual(4);
    expect(stale[0].textContent).toContain("superseded by");
    expect(detailEl().innerHTML).toContain(SUCCESSOR);
  });

  it("omits the stale context for a current (isolated) revision", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    expect(detailEl().querySelector(".fact-stale-context")).toBeNull();
  });

  it("computes the stale context string directly from state", () => {
    expect(detail.staleFactSectionContext(REV)).toBe("");
    store.commit({
      objects: {
        threads: [],
        revisionClassification: {
          [REV]: {
            state: "superseded",
            supersededBy: [SUCCESSOR],
            supersedes: [],
          },
        },
      } as unknown as ObjectsDoc,
    });
    const context = detail.staleFactSectionContext(REV);
    expect(context).toContain("fact-stale-context");
    expect(context).toContain("superseded by");
  });
});

describe("showComposite (shownCompositeId guards re-fetch)", () => {
  it("renders on first selection and is a no-op on an unchanged re-selection", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.showComposite(REV);
    expect(detailEl().querySelector(".unit-page")).not.toBeNull();

    // Re-selecting the same revision returns early — no reload-to-loading flash.
    await detail.showComposite(REV);
    expect(detailEl().innerHTML).not.toContain("loading…");
    expect(detailEl().querySelector(".unit-page")).not.toBeNull();
  });
});
