import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { HistoryDoc, RevisionsDoc } from "../src/store";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  installFetchMock,
  resetHistoryResponse,
  setHistoryResponse,
  uninstallFetchMock,
} from "./support/fetch";

// `navigation.ts` is the ref-chip resolution layer: `resolveRef` turns a clicked
// `data-ref-kind` chip into a `router.navigate` / diff-open, and the reveal helpers
// make an event/revision visible and selected. It owns the single `document`
// `click→resolveRef` delegate (chips render across timeline/detail/diff/cards) and,
// per the detail layer's deferral, also resolves the `data-reveal-revision`
// "show in timeline" button. All navigation goes through `router.navigate` (commit →
// the subscriber repaints); it never calls render. The store is a module singleton,
// so reset and re-import it before each test.
type Store = typeof import("../src/store");
type Navigation = typeof import("../src/navigation");
let store: Store;
let navigation: Navigation;

const OBS_EVENT =
  "evt:sha256:8ac34bc85b48ed6623660a174b024bd9099edd09877180bfa87101cc76ac6058";
const OBS_ID =
  "obs:sha256:752a5b0ab30cfa3aa062bcf6f11b4c6ee3dcfd055207b6a995b91bf81ffec8d9";
const ASSESS_EVENT =
  "evt:sha256:63d8e6174cc943ee8f049e5f6718ff385e763fd55731bb196de5fb2d3e90d4e0";
const ASSESS_ID =
  "assess:sha256:96faccf9a8ca8174f227dd4d667fbe47a439ad8afa0259e01fb278685eff35da";
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";

beforeEach(async () => {
  const vitest = await import("vitest");
  vitest.vi.resetModules();
  store = await import("../src/store");
  navigation = await import("../src/navigation");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
  });
});

afterEach(() => {
  resetHistoryResponse();
  uninstallFetchMock();
  resetDom();
});

/** A single-entry reveal page whose at=/q= fetches both resolve to `event`. */
function revealPageFor(event: HistoryDoc["entries"][number]): void {
  setHistoryResponse({
    entries: [event],
    diagnostics: [],
    offset: 0,
    matchCount: 1,
    matchIndex: 0,
    facets: { [event.eventType]: 1 },
  });
}

describe("resolveRef routes a chip by kind", () => {
  it("a rev chip selects the revision and dismisses the diff", () => {
    store.commit({ diff: OBJ });
    navigation.resolveRef("rev", REV);
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
    expect(store.getState().diff).toBeNull();
  });

  it("a review-unit chip resolves onto the same revision identity", () => {
    navigation.resolveRef("review-unit", REV);
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });

  it("a track chip scopes the timeline to that track", () => {
    navigation.resolveRef("track", "agent:codex");
    expect(store.getState().lens).toBe("timeline");
    expect(store.getState().filterTrack).toBe("agent:codex");
  });

  it("a snap chip opens the diff overlay route", () => {
    navigation.resolveRef("snap", OBJ);
    expect(store.getState().diff).toBe(OBJ);
  });

  it("an obs chip resolves the id to its event server-side, then reveals it", async () => {
    const obsEntry = {
      eventId: OBS_EVENT,
      eventType: "review_observation_recorded",
      summary: { observationId: OBS_ID },
    };
    revealPageFor(obsEntry as HistoryDoc["entries"][number]);
    await navigation.resolveRefAsync("obs", OBS_ID);
    expect(store.getState().selected).toEqual({ kind: "event", id: OBS_EVENT });
    expect(store.getState().lens).toBe("timeline");
  });

  it("an assess chip resolves the id to its event server-side, then reveals it", async () => {
    const assessEntry = {
      eventId: ASSESS_EVENT,
      eventType: "review_assessment_recorded",
      summary: { assessmentId: ASSESS_ID },
    };
    revealPageFor(assessEntry as HistoryDoc["entries"][number]);
    await navigation.resolveRefAsync("assess", ASSESS_ID);
    expect(store.getState().selected).toEqual({
      kind: "event",
      id: ASSESS_EVENT,
    });
  });

  it("an evt chip reveals and selects the event directly", async () => {
    const evtEntry = {
      eventId: OBS_EVENT,
      eventType: "review_observation_recorded",
    };
    revealPageFor(evtEntry as HistoryDoc["entries"][number]);
    await navigation.resolveRefAsync("evt", OBS_EVENT);
    expect(store.getState().selected).toEqual({ kind: "event", id: OBS_EVENT });
  });

  it("a non-clickable kind (validation) is inert", () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    navigation.resolveRef("validation", "validation:sha256:abc");
    // The default branch routes nowhere — the selection is untouched.
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });
});

describe("reveal helpers fetch the target page and select through the router", () => {
  it("revealEvent fetches the page, clears filters, and selects", async () => {
    store.commit({
      filterText: "something",
      filterTrack: "human:kevin",
      filterSnapshot: OBJ,
      lens: "list",
    });
    const obsEntry = {
      eventId: OBS_EVENT,
      eventType: "review_observation_recorded",
    };
    revealPageFor(obsEntry as HistoryDoc["entries"][number]);
    await navigation.revealEvent(OBS_EVENT);
    const s = store.getState();
    expect(s.selected).toEqual({ kind: "event", id: OBS_EVENT });
    expect(s.lens).toBe("timeline");
    expect(s.filterText).toBe("");
    expect(s.filterTrack).toBe("");
    expect(s.filterSnapshot).toBe("");
    expect(s.enabledTypes.has("review_observation_recorded")).toBe(true);
  });

  it("revealEvent fetches the page containing an off-page event via at=, then selects it", async () => {
    const X =
      "evt:sha256:0000000000000000000000000000000000000000000000000000000000000abc";
    let atUrl = "";
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (new URL(url, "http://inspector.test").pathname === "/api/history")
        atUrl = url;
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    setHistoryResponse({
      entries: [{ eventId: X, eventType: "review_observation_recorded" }],
      diagnostics: [],
      offset: 120,
      matchCount: 500,
      matchIndex: 123,
      facets: {},
    });
    try {
      // The loaded window (the fixture) does NOT contain X.
      await navigation.revealEvent(X);
    } finally {
      globalThis.fetch = inner;
    }
    expect(atUrl).toContain("at=evt%3Asha256%3A");
    expect(store.getState().selected).toEqual({ kind: "event", id: X });
    expect(store.getState().history?.entries.some((e) => e.eventId === X)).toBe(
      true,
    );
    expect(store.getState().history?.offset).toBe(120);
  });

  it("revealEvent leaves the view unchanged for a genuinely absent event", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    setHistoryResponse({
      entries: [],
      diagnostics: [],
      offset: 0,
      matchCount: 0,
      matchIndex: null,
      facets: {},
    });
    await navigation.revealEvent("evt:sha256:absent");
    // No matching page → the selection is not switched to the absent event.
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });

  it("navigateToRevision filters the timeline to that revision", () => {
    navigation.navigateToRevision(REV);
    expect(store.getState().lens).toBe("timeline");
    expect(store.getState().filterText).toBe(`revision:${REV}`);
    expect(store.getState().filterTrack).toBe("");
  });
});

describe("the single document click delegate", () => {
  it("resolves a clicked ref chip anywhere in the document", () => {
    document.addEventListener("click", navigation.onDocumentClick);
    const detail = document.querySelector("#detail");
    if (detail)
      detail.innerHTML = `<span class="ref" role="link" data-ref-kind="rev" data-ref-id="${REV}">chip</span>`;
    document
      .querySelector("[data-ref-kind]")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
    document.removeEventListener("click", navigation.onDocumentClick);
  });

  it("resolves a clicked data-reveal-revision button (the detail show-in-timeline)", () => {
    document.addEventListener("click", navigation.onDocumentClick);
    const detail = document.querySelector("#detail");
    if (detail)
      detail.innerHTML = `<button data-reveal-revision="${REV}">show in timeline</button>`;
    document
      .querySelector("[data-reveal-revision]")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().lens).toBe("timeline");
    expect(store.getState().filterText).toBe(`revision:${REV}`);
    document.removeEventListener("click", navigation.onDocumentClick);
  });
});
