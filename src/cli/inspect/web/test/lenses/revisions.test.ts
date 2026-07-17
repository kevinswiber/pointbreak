import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryDoc, RevisionsDoc, ThreadsDoc } from "../../src/store";
import historyJson from "../fixtures/history.json";
import revisionsJson from "../fixtures/revisions.json";
import threadsJson from "../fixtures/threads.json";
import { mountInspectorDom, resetDom } from "../support/dom";

// `lenses/revisions.ts` paints the flat revision list (`renderRevisionList`, the
// `#units` body). Cards carry the `data-revision-id`/`[data-open-diff]`/
// `[data-attention-query]` delegation attrs and no per-card listeners — the once-
// installed `#master` delegate owns selection, open-diff, and cue filtering. The
// store and the lens share one `state`, so reset the registry and re-import
// before each.
type Store = typeof import("../../src/store");
type Revisions = typeof import("../../src/lenses/revisions");
let store: Store;
let revisions: Revisions;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../../src/store");
  revisions = await import("../../src/lenses/revisions");
  mountInspectorDom();
  history.replaceState(null, "", "/");
});

afterEach(() => {
  resetDom();
});

// The single revision/object/artifact the committed fixtures describe.
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

function seedFixtures(): void {
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
    threads: threadsJson as unknown as ThreadsDoc,
  });
}

function mountListBody(): void {
  const master = document.querySelector("#master");
  if (master) master.innerHTML = `<div id="units" class="units"></div>`;
}

describe("renderRevisionList (the flat revision list lens)", () => {
  it("paints one card per revision with the delegation datasets and the open-diff control", () => {
    seedFixtures();
    mountListBody();
    revisions.renderRevisionList();
    const cards = document.querySelectorAll<HTMLElement>("#units .unit-card");
    expect(cards.length).toBe(
      (revisionsJson as unknown as RevisionsDoc).entries.length,
    );
    const card = cards[0];
    expect(card.dataset.revisionId).toBe(REV);
    const diffBtn = card.querySelector<HTMLElement>("[data-open-diff]");
    expect(diffBtn?.dataset.openDiff).toBe(OBJ);
    expect(diffBtn?.dataset.diffHash).toBe(ARTIFACT);
  });

  it("uses the work label as the heading while retaining revision and snapshot ids", () => {
    seedFixtures();
    const current = store.getState().revisions as RevisionsDoc;
    store.commit({
      revisions: {
        ...current,
        entries: current.entries.map((entry) => ({
          ...entry,
          targetDisplay: {
            ...entry.targetDisplay,
            workLabel: {
              text: "Review landing truth",
              source: "commit_subject",
            },
          },
        })),
      },
    });
    mountListBody();
    revisions.renderRevisionList();

    const card = document.querySelector<HTMLElement>("#units .unit-card");
    expect(card?.querySelector("h3")?.textContent).toBe("Review landing truth");
    expect(card?.textContent).toContain(REV.split(":").at(-1)?.slice(0, 12));
    expect(card?.textContent).toContain(OBJ.split(":").at(-1)?.slice(0, 12));
  });

  it("uses the capture summary as the primary revision heading", () => {
    seedFixtures();
    const current = store.getState().revisions as RevisionsDoc;
    store.commit({
      revisions: {
        ...current,
        entries: current.entries.map((entry) => ({
          ...entry,
          summary: "Make revision discovery readable",
        })),
      },
    });
    mountListBody();
    revisions.renderRevisionList();

    expect(
      document.querySelector<HTMLElement>("#units .unit-card h3")?.textContent,
    ).toBe("Make revision discovery readable");
  });

  it("keeps an incomplete revision visible and disables its unavailable snapshot", () => {
    seedFixtures();
    const current = store.getState().revisions as RevisionsDoc;
    store.commit({
      revisions: {
        ...current,
        entries: current.entries.map((entry) => ({
          ...entry,
          diagnostics: [
            {
              code: "snapshot_content_unavailable",
              message: "captured snapshot artifact is missing",
            },
          ],
        })),
      },
    });
    mountListBody();
    revisions.renderRevisionList();

    const card = document.querySelector<HTMLElement>("#units .unit-card");
    expect(card?.dataset.revisionId).toBe(REV);
    expect(card?.querySelector(".revision-diagnostic")?.textContent).toContain(
      "captured snapshot artifact is missing",
    );
    const button = card?.querySelector<HTMLButtonElement>(".diff-btn");
    expect(button?.disabled).toBe(true);
    expect(button?.hasAttribute("data-open-diff")).toBe(false);
    expect(button?.textContent).toBe("snapshot unavailable");
  });

  it("labels each card's landing with the merge-status vocabulary, never orphaned", () => {
    seedFixtures();
    const current = store.getState().revisions as RevisionsDoc;
    store.commit({
      revisions: {
        ...current,
        entries: current.entries.map((entry) => ({
          ...entry,
          mergeStatus: "unreachable",
        })),
      },
    });
    mountListBody();
    revisions.renderRevisionList();

    const labeled = document.querySelector<HTMLElement>("#units .unit-card");
    expect(labeled?.textContent).toContain("landing");
    expect(labeled?.textContent).toContain("unreachable");
    expect(labeled?.textContent).not.toContain("orphaned");
  });

  it("surfaces the supersession badge and the advisory attention cues", () => {
    seedFixtures();
    mountListBody();
    revisions.renderRevisionList();
    const card = document.querySelector<HTMLElement>("#units .unit-card");
    // The lone isolated revision is a current head ("current in thread").
    expect(card?.querySelector(".badge.head")?.textContent).toContain(
      "current in thread",
    );
    // Attention cues render as filter buttons carrying the data-attention-query
    // attr; the #master delegate (a later PR) handles the click, not the lens.
    const cue = card?.querySelector<HTMLElement>("[data-attention-query]");
    if (cue) expect(cue.dataset.attentionQuery).toMatch(/^attention:/);
  });

  it("attaches no per-card click listener — selection is left to the #master delegate", () => {
    seedFixtures();
    mountListBody();
    revisions.renderRevisionList();
    const card = document.querySelector<HTMLElement>("#units .unit-card");
    card?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: null, id: null });
  });

  it("marks the selected revision card with aria-selected", () => {
    seedFixtures();
    store.commit({ selected: { kind: "revision", id: REV } });
    mountListBody();
    revisions.renderRevisionList();
    const card = document.querySelector<HTMLElement>("#units .unit-card");
    expect(card?.getAttribute("aria-selected")).toBe("true");
  });

  it("shows the empty message when no revisions are loaded", () => {
    mountListBody();
    revisions.renderRevisionList();
    expect(document.querySelector("#units")?.textContent).toContain(
      "No captured revisions",
    );
  });
});

describe("newest-first list ordering (#256)", () => {
  const rev = (id: string, ms: number) => ({
    revisionId: id,
    capturedAt: `unix-ms:${ms}`,
    snapshotId: `snap:${id}`,
  });

  it("renders the flat revision list newest-first by default", () => {
    store.commit({
      revisions: {
        entries: [rev("a", 100), rev("c", 300), rev("b", 200)],
      } as unknown as RevisionsDoc,
    });
    mountListBody();
    revisions.renderRevisionList();
    const ids = [...document.querySelectorAll("#units .unit-card")].map((c) =>
      c.getAttribute("data-revision-id"),
    );
    expect(ids).toEqual(["c", "b", "a"]);
  });
});
