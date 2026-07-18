import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Assessment,
  InputRequest,
  Observation,
  ValidationCheck,
} from "../src/cards";
import type { RevisionPageDoc } from "../src/detail";
import type {
  AttentionDoc,
  AttentionItem,
  HistoryDoc,
  RevisionsDoc,
  ThreadsDoc,
} from "../src/store";
import historyJson from "./fixtures/history.json";
import revisionJson from "./fixtures/revision.json";
import revisionsJson from "./fixtures/revisions.json";
import threadsJson from "./fixtures/threads.json";
import { mountInspectorDom, resetDom } from "./support/dom";
import {
  attentionRequests,
  installFetchMock,
  resetCompositeResponse,
  resetScopedAttention,
  resetSnapshotResponse,
  setCompositeResponse,
  setScopedAttentionError,
  setScopedAttentionResponse,
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
const REF_EVENT =
  "evt:sha256:fdcfefd1251ddb5fcf0740317c46a2f3197ae8908e6760a625800fd5167db8aa";
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

function kvValue(label: string): string {
  const dt = Array.from(
    document.querySelectorAll<HTMLElement>("#detail dl.kv dt"),
  ).find((node) => node.textContent === label);
  return dt?.nextElementSibling?.textContent ?? "";
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
    threads: threadsJson as unknown as ThreadsDoc,
  });
  detail.initControls();
});

afterEach(() => {
  uninstallFetchMock();
  resetSnapshotResponse();
  resetCompositeResponse();
  resetScopedAttention();
  resetDom();
});

describe("renderDetail (event detail / empty prompt)", () => {
  it("prompts to select when nothing is selected", () => {
    store.commit({ selected: { kind: null, id: null } });
    detail.renderDetail();
    expect(detailEl().textContent).toContain("Select an event or revision");
  });

  it("paints the event detail into #detail-body under a persistent header", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    expect(document.querySelector("#detail-body h2")).not.toBeNull();
    expect(
      document.querySelector("#detail .detail-head #detail-close"),
    ).not.toBeNull();
  });

  it("the detail title and entity kv rows are real anchors without ref-chip datasets", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    const title =
      document.querySelector<HTMLAnchorElement>("#detail-body h2 a");
    expect(title?.getAttribute("href")).toBe(
      `#/event/${encodeURIComponent(OBS_EVENT)}`,
    );
    expect(title?.hasAttribute("data-ref-kind")).toBe(false);
    const revLink = document.querySelector(
      `#detail-body dl.kv a[href="#/revision/${encodeURIComponent(REV)}"]`,
    );
    expect(revLink).not.toBeNull();
    expect(revLink?.hasAttribute("data-ref-kind")).toBe(false);
  });

  it("entity anchors display the short ref form with the full id in href and title", async () => {
    const refs = await import("../src/refs");
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    const evLink = document.querySelector<HTMLAnchorElement>(
      `#detail-body dl.kv a[href="#/event/${encodeURIComponent(OBS_EVENT)}"]`,
    );
    expect(evLink?.textContent).toBe(refs.shortRef(OBS_EVENT));
    expect(evLink?.getAttribute("title")).toBe(OBS_EVENT);
    const revLink = document.querySelector<HTMLAnchorElement>(
      `#detail-body dl.kv a[href="#/revision/${encodeURIComponent(REV)}"]`,
    );
    expect(revLink?.textContent).toBe(refs.shortRef(REV));
  });

  it("the track kv row stays a ref chip (no entity route exists for tracks)", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    const dts = [...document.querySelectorAll("#detail-body dl.kv dt")];
    const trackDt = dts.find((d) => d.textContent === "track");
    const dd = trackDt?.nextElementSibling;
    expect(dd).not.toBeNull();
    expect(dd?.querySelector("a[href]")).toBeNull();
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
    // The raw event JSON is available behind the collapsed debugging disclosure.
    const raw = el.querySelector<HTMLDetailsElement>("details.raw-event");
    expect(raw).not.toBeNull();
    expect(raw?.open).toBe(false);
    expect(raw?.querySelector("pre")?.textContent).toContain(
      "review_observation",
    );
    // Embedded ids linkify into navigable ref chips (the navigation delegate resolves them).
    expect(el.querySelector("[data-ref-kind]")).not.toBeNull();
  });

  it("renders type-specific summary rows for revision ref association events", () => {
    store.commit({ selected: { kind: "event", id: REF_EVENT } });
    detail.renderDetail();
    expect(kvValue("refAssociationId")).toContain("assoc-ref:8cf5b7c2");
    expect(kvValue("refName")).toBe("refs/heads/main");
    expect(kvValue("headOid")).toBe("ffc93defe1");
  });

  it("renders observation metadata as HTML instead of relying on raw JSON", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT } });
    detail.renderDetail();
    expect(kvValue("observationId")).toContain("obs:752a5b0a");
    expect(kvValue("target")).toContain("src/lib.rs:2-2");
    expect(kvValue("bodyHash")).toContain("sha256:24c2131b");
    expect(kvValue("bodyBytes")).toBe("24");
  });

  it("copies raw event JSON from the collapsed debug block", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    store.commit({ selected: { kind: "event", id: OBS_EVENT } });
    detail.renderDetail();
    document
      .querySelector<HTMLElement>("[data-copy-raw-event]")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining(OBS_EVENT));
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

  it("keeps the exact writer actor separate from role and resolved principal", () => {
    const eventId = "evt:sha256:writerrolecharacterization";
    store.commit({
      history: {
        entries: [
          {
            eventType: "review_observation_recorded",
            eventId,
            occurredAt: "unix-ms:1782699185488",
            // An envelope that also carries a role — the readback must ignore it.
            writer: { actorId: "actor:agent:codex", role: "admin" },
            principal: {
              status: "resolved",
              actorId: "actor:git-email:kevin@example.com",
            },
            subject: { revisionId: REV },
            summary: { observationId: "obs:x", title: "obs" },
          },
        ],
        diagnostics: [],
      } as unknown as HistoryDoc,
      selected: { kind: "event", id: eventId },
    });
    detail.renderDetail();
    // The actor identity line renders the writer actor id; the role is never
    // surfaced in the readback (the raw payload dump is a separate, faithful echo).
    const actorDt = Array.from(
      document.querySelectorAll<HTMLElement>("#detail dl.kv dt"),
    ).find((dt) => dt.textContent === "actor");
    const actorReadback = actorDt?.nextElementSibling?.textContent ?? "";
    expect(actorReadback).toBe("actor:agent:codex");
    expect(actorReadback).not.toContain("admin");
    expect(actorReadback).not.toContain("kevin@example.com");
    const principalDt = Array.from(
      document.querySelectorAll<HTMLElement>("#detail dl.kv dt"),
    ).find((dt) => dt.textContent === "principal");
    expect(principalDt?.nextElementSibling?.textContent).toContain(
      "kevin@example.com",
    );
  });
});

describe("detail-pane scroll memory (reset on change, restore on revisit)", () => {
  function pane(): HTMLElement {
    const el = document.querySelector<HTMLElement>("#detail");
    if (!el) throw new Error("#detail not mounted");
    return el;
  }
  function secondEvent(): string {
    const entries = (historyJson as unknown as HistoryDoc).entries;
    const other = entries.find((e) => e.eventId && e.eventId !== OBS_EVENT);
    if (!other?.eventId) throw new Error("fixture needs a second event");
    return other.eventId;
  }

  it("resets the pane scroll to the top when the selection changes", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    pane().scrollTop = 300;
    store.commit({
      selected: { kind: "event", id: secondEvent() },
      open: true,
    });
    detail.renderDetail();
    expect(pane().scrollTop).toBe(0);
  });

  it("restores the reading position when returning to an entry", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    pane().scrollTop = 300;
    store.commit({
      selected: { kind: "event", id: secondEvent() },
      open: true,
    });
    detail.renderDetail();
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    expect(pane().scrollTop).toBe(300);
  });

  it("a same-selection repaint leaves the reader's scroll alone", () => {
    store.commit({ selected: { kind: "event", id: OBS_EVENT }, open: true });
    detail.renderDetail();
    pane().scrollTop = 120;
    detail.renderDetail();
    expect(pane().scrollTop).toBe(120);
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

describe("compositeAnnotations", () => {
  it("normalizes every fact class with exact state, attribution, and continuity", () => {
    const response = structuredClone(
      revisionJson,
    ) as unknown as RevisionPageDoc & {
      observations: Observation[];
      inputRequests: InputRequest[];
      assessments: Assessment[];
      validationChecks: ValidationCheck[];
    };
    const responder = "actor:agent:pointbreak-example-reviewer";
    response.inputRequests[0].status = "responded";
    response.inputRequests[0].responses = [
      {
        id: "input-request-response:sha256:response",
        eventId: "evt:sha256:response",
        outcome: "approved",
        reason: "the reviewer accepts the evidence",
        reasonContentType: "text/plain",
        createdAt: "unix-ms:1782699185900",
        verificationStatus: "unsigned",
        writer: {
          actorId: responder,
          producer: { name: "pointbreak", version: "0.7.0" },
        },
      },
    ];
    response.validationChecks[0].completedAt = "unix-ms:1782699185950";

    const annotations = detail.compositeAnnotations(response);

    expect(annotations.map((annotation) => annotation.kind)).toEqual([
      "observation",
      "input-request",
      "assessment",
      "assessment",
      "validation",
      "validation",
    ]);
    expect(new Set(annotations.map((annotation) => annotation.id)).size).toBe(
      annotations.length,
    );
    expect(annotations[0]).toMatchObject({
      id: response.observations[0].id,
      status: "active",
      writer: response.observations[0].writer,
      target: response.observations[0].target,
    });
    expect(annotations[1]).toMatchObject({
      id: response.inputRequests[0].id,
      status: "responded",
      mode: "operative",
      reasonCode: "manual_decision_required",
      writer: response.inputRequests[0].writer,
      responses: [
        {
          id: "input-request-response:sha256:response",
          outcome: "approved",
          reason: "the reviewer accepts the evidence",
          createdAt: "unix-ms:1782699185900",
          writer: { actorId: responder },
        },
      ],
    });
    expect(annotations.slice(2, 4)).toEqual([
      expect.objectContaining({
        id: response.assessments[0].id,
        status: "replaced",
        replaces: [],
        writer: response.assessments[0].writer,
      }),
      expect.objectContaining({
        id: response.assessments[1].id,
        status: "current",
        replaces: [response.assessments[0].id],
        writer: response.assessments[1].writer,
      }),
    ]);
    expect(annotations.slice(4)).toEqual([
      expect.objectContaining({
        id: response.validationChecks[0].id,
        status: "passed",
        trigger: "manual",
        createdAt: response.validationChecks[0].createdAt,
        completedAt: "unix-ms:1782699185950",
        continuity: "current",
        writer: response.validationChecks[0].writer,
      }),
      expect.objectContaining({
        id: response.validationChecks[1].id,
        status: "failed",
        command: "cargo clippy -- -D warnings",
        exitCode: 1,
        continuity: "outstanding",
        writer: response.validationChecks[1].writer,
      }),
    ]);
    expect(
      annotations.filter((annotation) => annotation.kind === "validation"),
    ).toHaveLength(response.validationChecks.length);
  });

  it("keeps ambiguous assessment candidates as unranked current facts", () => {
    const response = structuredClone(
      revisionJson,
    ) as unknown as RevisionPageDoc & {
      assessments: Assessment[];
    };
    response.currentAssessment = {
      status: "ambiguous",
      candidates: response.assessments.map((assessment) => assessment.id),
    };
    for (const assessment of response.assessments) {
      assessment.status = "current";
    }

    expect(
      detail
        .compositeAnnotations(response)
        .filter((annotation) => annotation.kind === "assessment")
        .map((annotation) => annotation.status),
    ).toEqual(["current", "current"]);
  });
});

describe("openRevision / renderRevisionPage (the composite page, fetched via http)", () => {
  it("fetches the revision and paints the composite page sections", async () => {
    const response = structuredClone(revisionJson) as unknown as {
      revision: { summary?: string };
    };
    response.revision.summary = "Make revision discovery readable";
    setCompositeResponse(response);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const el = detailEl();
    expect(el.querySelector(".unit-page")).not.toBeNull();
    expect(el.querySelector(".unit-page-title")?.textContent).toBe(
      "Make revision discovery readable",
    );
    const summaryLabel = Array.from(el.querySelectorAll("dt")).find(
      (node) => node.textContent === "summary",
    );
    expect(summaryLabel?.nextElementSibling?.textContent).toBe(
      "Make revision discovery readable",
    );
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

  it("renders exact fact, opener, and response writers from the composite", async () => {
    const author = "actor:agent:pointbreak-example-author";
    const reviewer = "actor:agent:pointbreak-example-reviewer";
    const response = structuredClone(revisionJson) as unknown as {
      observations: Array<{ writer: { actorId: string } }>;
      inputRequests: Array<{
        writer: { actorId: string };
        status: string;
        responses: Array<{
          id: string;
          eventId: string;
          outcome: string;
          reason: string;
          createdAt: string;
          verificationStatus: string;
          writer: {
            actorId: string;
            producer: { name: string; version: string };
          };
        }>;
      }>;
      assessments: Array<{ writer: { actorId: string } }>;
      validationChecks: Array<{ writer: { actorId: string } }>;
    };
    response.observations[0].writer.actorId = author;
    response.inputRequests[0].writer.actorId = author;
    response.inputRequests[0].status = "ambiguous";
    response.inputRequests[0].responses = [
      {
        id: "input-request-response:sha256:aaaaaaaaaaaaaaaa",
        eventId: "evt:sha256:aaaaaaaaaaaaaaaa",
        outcome: "approved",
        reason: "reviewer approves",
        createdAt: "unix-ms:1782699185700",
        verificationStatus: "unsigned",
        writer: {
          actorId: reviewer,
          producer: { name: "pointbreak", version: "0.6.0" },
        },
      },
      {
        id: "input-request-response:sha256:bbbbbbbbbbbbbbbb",
        eventId: "evt:sha256:bbbbbbbbbbbbbbbb",
        outcome: "rejected",
        reason: "author disagrees",
        createdAt: "unix-ms:1782699185800",
        verificationStatus: "unsigned",
        writer: {
          actorId: author,
          producer: { name: "pointbreak", version: "0.6.0" },
        },
      },
    ];
    response.assessments[0].writer.actorId = reviewer;
    response.validationChecks[0].writer.actorId = reviewer;
    setCompositeResponse(response);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);

    const page = detailEl();
    const cards = [
      [".anno-observation", author],
      [".anno-input-request", author],
      [".anno-assessment", reviewer],
      [".anno-validation", reviewer],
    ] as const;
    for (const [selector, actorId] of cards) {
      const chip = page.querySelector<HTMLElement>(
        `${selector} > .anno-head [data-ref-kind="actor"]`,
      );
      expect(chip?.textContent).toBe(actorId);
      expect(chip?.dataset.refId).toBe(actorId);
    }
    const responseChips = Array.from(
      page.querySelectorAll<HTMLElement>(
        '.fact-response [data-ref-kind="actor"]',
      ),
    );
    expect(responseChips.map((chip) => chip.dataset.refId)).toEqual([
      reviewer,
      author,
    ]);
    expect(page.querySelectorAll(".fact-response")).toHaveLength(2);
    expect(page.textContent).toContain("reviewer approves");
    expect(page.textContent).toContain("author disagrees");
  });

  it("joins validation dispositions by immutable id while preserving every raw fact", async () => {
    const response = structuredClone(revisionJson) as unknown as {
      validationChecks: Array<{ id: string; status: string }>;
      validationContinuity: {
        checks: Record<string, string>;
      };
    };
    const seed = response.validationChecks[0];
    response.validationChecks = [
      { ...seed, id: "validation:passed", status: "passed" },
      { ...seed, id: "validation:failed", status: "failed" },
      { ...seed, id: "validation:errored", status: "errored" },
      { ...seed, id: "validation:skipped", status: "skipped" },
      { ...seed, id: "validation:recovered", status: "failed" },
      { ...seed, id: "validation:skipped-after-fail", status: "skipped" },
    ];
    response.validationContinuity.checks = {
      "validation:passed": "current",
      "validation:failed": "outstanding",
      "validation:errored": "historical",
      "validation:skipped": "skipped",
      "validation:recovered": "resolved_by_later_pass",
      "validation:skipped-after-fail": "outstanding",
    };
    setCompositeResponse(response);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);

    const cards = Array.from(
      detailEl().querySelectorAll<HTMLElement>(".anno-validation"),
    );
    expect(cards).toHaveLength(6);
    expect(
      cards.map((card) => card.querySelector(".fact-status")?.textContent),
    ).toEqual(["passed", "failed", "errored", "skipped", "failed", "skipped"]);
    expect(
      cards.map(
        (card) => card.querySelector(".validation-continuity")?.textContent,
      ),
    ).toEqual([
      "current result",
      "outstanding",
      "historical",
      "skipped",
      "resolved by strictly later pass",
      "outstanding",
    ]);
    expect(detailEl().textContent).toContain(
      "context only — does not affect the current assessment",
    );
  });

  it("renders raw validation evidence without guessing when private continuity is absent", async () => {
    const response = structuredClone(revisionJson) as unknown as {
      validationContinuity?: unknown;
    };
    delete response.validationContinuity;
    setCompositeResponse(response);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);

    expect(detailEl().querySelectorAll(".anno-validation")).toHaveLength(2);
    expect(detailEl().querySelector(".fact-status.passed")).not.toBeNull();
    expect(detailEl().querySelector(".fact-status.failed")).not.toBeNull();
    expect(detailEl().querySelector(".validation-continuity")).toBeNull();
  });

  it("renders floating, capture-target, landing, and unknown wording from existing authority", async () => {
    const cases = [
      {
        commitRange: {
          anchored: false,
          currentCommits: [],
          currentRefs: [],
          withdrawnCommits: [],
          withdrawnRefs: [],
          liveness: { perCommit: [] },
        },
        expected: "floating revision — no landing commit association recorded",
      },
      {
        commitRange: {
          anchored: true,
          currentCommits: [
            { commitOid: "a".repeat(40), source: "capture_target" },
          ],
          currentRefs: [],
          withdrawnCommits: [],
          withdrawnRefs: [],
          liveness: {
            perCommit: [{ commitOid: "a".repeat(40), condition: "live" }],
            headline: { condition: "live" },
          },
        },
        expected: "anchored capture target live",
      },
      {
        commitRange: {
          anchored: true,
          currentCommits: [
            {
              commitOid: "b".repeat(40),
              source: "association",
              commitAssociationId: "assoc-commit:sha256:12345678",
            },
          ],
          currentRefs: [],
          withdrawnCommits: [],
          withdrawnRefs: [],
          liveness: {
            perCommit: [{ commitOid: "b".repeat(40), condition: "merged" }],
            headline: { condition: "merged" },
          },
        },
        expected: "landing merged",
      },
      {
        commitRange: {
          anchored: true,
          currentCommits: [
            { commitOid: "c".repeat(40), source: "association" },
          ],
          currentRefs: [],
          withdrawnCommits: [],
          withdrawnRefs: [],
        },
        expected: "landing unknown — Git reachability unavailable",
      },
    ];

    for (const [index, { commitRange, expected }] of cases.entries()) {
      const response = {
        ...structuredClone(revisionJson),
        commitRange,
      };
      setCompositeResponse(response);
      store.commit({
        revisions: {
          ...(store.getState().revisions as RevisionsDoc),
          eventSetHash: `case-${index}`,
        },
        selected: { kind: "revision", id: REV },
      });
      await detail.openRevision(REV);
      expect(detailEl().textContent).toContain(expected);
    }
  });

  it("renders full-OID tooltips plus current and withdrawn association history", async () => {
    const oid = "d".repeat(40);
    const response = {
      ...structuredClone(revisionJson),
      commitRange: {
        anchored: true,
        currentCommits: [
          {
            commitOid: oid,
            source: "association",
            commitAssociationId: "assoc-commit:sha256:current",
          },
        ],
        currentRefs: [
          {
            refName: "refs/heads/feature/landing",
            headOid: oid,
            refAssociationId: "assoc-ref:sha256:current",
          },
        ],
        withdrawnCommits: [
          {
            commitOid: "e".repeat(40),
            commitAssociationId: "assoc-commit:sha256:old",
            commitWithdrawalId: "withdraw-commit:sha256:old",
          },
        ],
        withdrawnRefs: [
          {
            refName: "refs/heads/old",
            headOid: "e".repeat(40),
            refAssociationId: "assoc-ref:sha256:old",
            refWithdrawalId: "withdraw-ref:sha256:old",
          },
        ],
        liveness: {
          perCommit: [{ commitOid: oid, condition: "live" }],
          headline: { condition: "live" },
        },
      },
    };
    setCompositeResponse(response);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);

    const section = [...detailEl().querySelectorAll("section")].find(
      (item) =>
        item.querySelector("h2")?.textContent === "Association and landing",
    );
    expect(section).toBeDefined();
    expect(section?.querySelector(`[title="${oid}"]`)).not.toBeNull();
    expect(section?.textContent).toContain("feature/landing");
    expect(section?.textContent).toContain("withdrawn commits");
    expect(section?.textContent).toContain("withdrawn refs");
  });

  it("uses the semantic work label as the title and keeps immutable ids visible", async () => {
    const response = {
      ...structuredClone(revisionJson),
      revision: {
        ...structuredClone(revisionJson.revision),
        targetDisplay: {
          ...structuredClone(revisionJson.revision.targetDisplay),
          workLabel: {
            text: "Review <landing> truth",
            source: "commit_subject",
          },
        },
      },
    };
    setCompositeResponse(response);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);

    expect(detailEl().querySelector(".unit-page-title")?.textContent).toBe(
      "Review <landing> truth",
    );
    expect(detailEl().querySelector(`span[title="${REV}"]`)).not.toBeNull();
    expect(detailEl().querySelector(`span[title="${OBJ}"]`)).not.toBeNull();
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

  it("renders an incomplete revision detail and disables its snapshot action", async () => {
    setCompositeResponse({
      ...(revisionJson as Record<string, unknown>),
      diagnostics: [
        {
          code: "snapshot_content_unavailable",
          message: "captured snapshot artifact is missing",
        },
      ],
    });
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);

    expect(detailEl().querySelector(".unit-page")).not.toBeNull();
    expect(
      detailEl().querySelector(".revision-diagnostic")?.textContent,
    ).toContain("captured snapshot artifact is missing");
    const button = detailEl().querySelector<HTMLButtonElement>("#up-diff-btn");
    expect(button?.disabled).toBe(true);
    expect(button?.hasAttribute("data-open-diff")).toBe(false);
    expect(button?.textContent).toBe("snapshot unavailable");
  });
});

describe("renderAssociationAndLanding (liveness + ref-continuity readout)", () => {
  it("keeps unreachable and missing distinct, with reflog retention, never orphaned", () => {
    const html = detail.renderAssociationAndLanding(
      {
        anchored: true,
        currentCommits: [
          { commitOid: "b".repeat(40), source: "capture_target" },
          { commitOid: "c".repeat(40), source: "association" },
        ],
        liveness: {
          perCommit: [
            {
              commitOid: "b".repeat(40),
              condition: "unreachable",
              retention: "reflog",
            },
            { commitOid: "c".repeat(40), condition: "missing" },
          ],
          headline: { condition: "unreachable" },
        },
      },
      [],
    );
    expect(html).toContain("landing unreachable");
    expect(html).toContain("unreachable (reflog-retained)");
    expect(html).toContain("missing");
    expect(html).not.toContain("orphaned");
  });

  it("labels a rewritten recorded ref with the action and current tip", () => {
    const html = detail.renderAssociationAndLanding(
      {
        anchored: true,
        currentRefs: [
          {
            refName: "refs/heads/feat/amend",
            headOid: "a".repeat(40),
            refAssociationId: "assoc-ref:sha256:test",
          },
        ],
        liveness: {
          perCommit: [],
          refContinuity: [
            {
              refName: "refs/heads/feat/amend",
              recordedHeadOid: "a".repeat(40),
              currentTipOid: "d".repeat(40),
              continuity: "rewritten",
              rewriteAction: "commit (amend)",
              sameTree: false,
            },
          ],
        },
      },
      [],
    );
    expect(html).toContain("rewritten by commit (amend)");
    expect(html).toContain(`→ ${"d".repeat(12)}`);
  });

  it("reads an expired-reflog move without rewrite evidence", () => {
    const html = detail.renderAssociationAndLanding(
      {
        anchored: true,
        currentRefs: [
          {
            refName: "refs/heads/feat/amend",
            headOid: "a".repeat(40),
            refAssociationId: "assoc-ref:sha256:test",
          },
        ],
        liveness: {
          perCommit: [],
          refContinuity: [
            {
              refName: "refs/heads/feat/amend",
              recordedHeadOid: "a".repeat(40),
              currentTipOid: "d".repeat(40),
              continuity: "moved",
            },
          ],
        },
      },
      [],
    );
    expect(html).toContain("moved");
    expect(html).not.toContain(" by ");
  });
});

describe("staleFactSectionContext (state-bound, fed into the pure factSection)", () => {
  it("repeats the superseded-by context near each fact section of a stale revision", async () => {
    // Mark the captured revision superseded so its facts carry the stale context.
    store.commit({
      threads: {
        threads: [],
        revisionClassification: {
          [REV]: {
            state: "superseded",
            supersededBy: [SUCCESSOR],
            supersedes: [],
          },
        },
      } as unknown as ThreadsDoc,
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
      threads: {
        threads: [],
        revisionClassification: {
          [REV]: {
            state: "superseded",
            supersededBy: [SUCCESSOR],
            supersedes: [],
          },
        },
      } as unknown as ThreadsDoc,
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

  it("re-fetches an open composite when the loaded event set moves", async () => {
    const initial = structuredClone(revisionJson) as unknown as RevisionPageDoc;
    initial.observations = [];
    initial.summary = { ...initial.summary, observationCount: 0 };
    setCompositeResponse(initial);
    store.commit({
      revisions: {
        ...(store.getState().revisions as RevisionsDoc),
        eventSetHash: "sha256:before-poll",
      },
      selected: { kind: "revision", id: REV },
      open: true,
    });
    await detail.showComposite(REV);
    expect(detailEl().textContent).not.toContain("Freshly polled observation");

    const updated = structuredClone(revisionJson) as unknown as RevisionPageDoc;
    const updatedObservation = updated.observations?.[0];
    if (!updatedObservation) throw new Error("expected observation fixture");
    updatedObservation.title = "Freshly polled observation";
    setCompositeResponse(updated);
    store.commit({
      revisions: {
        ...(store.getState().revisions as RevisionsDoc),
        eventSetHash: "sha256:after-poll",
      },
    });

    // The render subscriber calls showComposite again while the same revision
    // remains open. A moved event set must invalidate both the fetch cache and
    // the shown-revision guard so new facts appear without a page reload.
    await detail.showComposite(REV);

    expect(detailEl().textContent).toContain("Freshly polled observation");
  });

  it("keeps the current revision mounted while its fresh composite loads", async () => {
    const initial = structuredClone(revisionJson) as unknown as RevisionPageDoc;
    const initialObservation = initial.observations?.[0];
    if (!initialObservation) throw new Error("expected observation fixture");
    initialObservation.title = "Existing composite stays mounted";
    setCompositeResponse(initial);
    store.commit({
      revisions: {
        ...(store.getState().revisions as RevisionsDoc),
        eventSetHash: "sha256:before-mounted-refresh",
      },
      selected: { kind: "revision", id: REV },
      open: true,
    });
    await detail.showComposite(REV);
    detailEl().scrollTop = 120;

    const updated = structuredClone(revisionJson) as unknown as RevisionPageDoc;
    const updatedObservation = updated.observations?.[0];
    if (!updatedObservation) throw new Error("expected observation fixture");
    updatedObservation.title = "Fresh composite replaces existing";

    let resolveRefresh!: (response: Response) => void;
    const refreshResponse = new Promise<Response>((resolve) => {
      resolveRefresh = resolve;
    });
    const baseFetch = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (
        new URL(url, "http://inspector.test").pathname.startsWith(
          "/api/revisions/",
        )
      )
        return refreshResponse;
      return baseFetch(input, init);
    }) as typeof fetch;

    try {
      store.commit({
        revisions: {
          ...(store.getState().revisions as RevisionsDoc),
          eventSetHash: "sha256:after-mounted-refresh",
        },
      });
      const repaint = detail.showComposite(REV);

      expect(detailEl().textContent).toContain(
        "Existing composite stays mounted",
      );
      expect(detailEl().innerHTML).not.toContain("loading…");

      resolveRefresh(
        new Response(JSON.stringify(updated), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
      await repaint;

      expect(detailEl().textContent).toContain(
        "Fresh composite replaces existing",
      );
      expect(detailEl().scrollTop).toBe(120);
    } finally {
      globalThis.fetch = baseFetch;
    }
  });

  it("does not reuse or repaint an older composite read after the event set moves", async () => {
    const initial = structuredClone(revisionJson) as unknown as RevisionPageDoc;
    initial.observations = [];
    initial.summary = { ...initial.summary, observationCount: 0 };

    let resolveInitial!: (response: Response) => void;
    const initialResponse = new Promise<Response>((resolve) => {
      resolveInitial = resolve;
    });
    let compositeRequests = 0;
    const baseFetch = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (
        new URL(url, "http://inspector.test").pathname.startsWith(
          "/api/revisions/",
        )
      ) {
        compositeRequests += 1;
        if (compositeRequests === 1) return initialResponse;
      }
      return baseFetch(input, init);
    }) as typeof fetch;

    try {
      store.commit({
        revisions: {
          ...(store.getState().revisions as RevisionsDoc),
          eventSetHash: "sha256:before-in-flight-poll",
        },
        selected: { kind: "revision", id: REV },
        open: true,
      });
      const firstPaint = detail.showComposite(REV);

      const updated = structuredClone(
        revisionJson,
      ) as unknown as RevisionPageDoc;
      const updatedObservation = updated.observations?.[0];
      if (!updatedObservation) throw new Error("expected observation fixture");
      updatedObservation.title = "Newest composite wins";
      setCompositeResponse(updated);
      store.commit({
        revisions: {
          ...(store.getState().revisions as RevisionsDoc),
          eventSetHash: "sha256:after-in-flight-poll",
        },
      });
      const secondPaint = detail.showComposite(REV);

      resolveInitial(
        new Response(JSON.stringify(initial), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
      await Promise.all([firstPaint, secondPaint]);

      expect(compositeRequests).toBe(2);
      expect(detailEl().textContent).toContain("Newest composite wins");
    } finally {
      globalThis.fetch = baseFetch;
    }
  });
});

describe("the per-revision outstanding block (scoped attention on the detail page)", () => {
  // The block answers "what's outstanding on THIS revision?" from the scoped
  // /api/attention?revision= read — never a client-side filter of the global
  // document (a naive filter cannot reproduce competing-heads component
  // coverage). One row per item, kind + ask; navigation is the only interaction.
  const OTHER_HEAD =
    "rev:sha256:3333333333333333333333333333333333333333333333333333333333333333";
  const scopedItems: AttentionItem[] = [
    {
      id: "open_input_request:input-request:sha256:aaaa",
      kind: "open_input_request",
      tier: "primary",
      revisionId: REV,
      title: "Runtime trace required",
    },
    {
      id: "failed_validation:validation:sha256:bbbb",
      kind: "failed_validation",
      tier: "secondary",
      revisionId: REV,
      checkName: "just check",
      status: "failed",
    },
    {
      id: `competing_heads:${REV}`,
      kind: "competing_heads",
      tier: "primary",
      headRevisionIds: [OTHER_HEAD, REV],
    },
  ];
  const attentionOf = (eventSetHash: string): AttentionDoc => ({
    items: [],
    eventSetHash,
  });

  it("fetches the scoped set with the composite document and renders one row per item", async () => {
    setScopedAttentionResponse({ items: scopedItems });
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.showComposite(REV);

    // The mock saw the scoped form — revision= in the query string.
    const scoped = attentionRequests().filter((u) => u.includes("revision="));
    expect(scoped.length).toBe(1);
    expect(scoped[0]).toContain(`revision=${encodeURIComponent(REV)}`);

    const block = detailEl().querySelector(".outstanding-set");
    expect(block).not.toBeNull();
    const rows = Array.from(block?.querySelectorAll("li") ?? []);
    expect(rows.length).toBe(3); // the SET — one row per item, never collapsed
    const text = block?.textContent ?? "";
    // Each row names the item kind and carries the attention lens's ask wording.
    expect(block?.querySelectorAll(".attention-kind").length).toBe(3);
    expect(text).toContain("Runtime trace required");
    expect(text).toContain("just check failed");
    expect(text).toContain("2 competing heads");
    // A navigation chip to the anchor where one exists.
    expect(block?.querySelector(`[data-ref-id="${REV}"]`)).not.toBeNull();
  });

  it("renders nothing when the scoped set is empty", async () => {
    // The default scoped response is an empty set.
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.showComposite(REV);
    expect(detailEl().querySelector(".unit-page")).not.toBeNull();
    expect(detailEl().querySelector(".outstanding-set")).toBeNull();
  });

  it("offers no dismissal affordance of any kind", async () => {
    setScopedAttentionResponse({ items: scopedItems });
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.showComposite(REV);
    const block = detailEl().querySelector(".outstanding-set");
    expect(block).not.toBeNull();
    // No per-item write control, no read-state, no done/snooze. The one allowed
    // button is the clipboard-only command copy — it mutates nothing.
    expect(
      block?.querySelector(
        "button:not([data-copy-workflow-command]), input, select, textarea",
      ),
    ).toBeNull();
  });

  it("re-fetches the scoped set when the global attention doc moves under an open revision", async () => {
    setScopedAttentionResponse({ items: scopedItems });
    store.commit({
      selected: { kind: "revision", id: REV },
      attention: attentionOf("sha256:hash-one"),
    });
    await detail.showComposite(REV); // scoped fetch #1
    expect(
      attentionRequests().filter((u) => u.includes("revision=")).length,
    ).toBe(1);

    // A repaint with an unchanged event set keeps the cached scoped set.
    await detail.showComposite(REV);
    expect(
      attentionRequests().filter((u) => u.includes("revision=")).length,
    ).toBe(1);

    // The freshness poll delivers a moved global doc while the same revision
    // stays open — the composite revision-id dedupe must not pin the block.
    setScopedAttentionResponse({ items: [scopedItems[0]] });
    store.commit({ attention: attentionOf("sha256:hash-two") });
    await detail.showComposite(REV); // scoped fetch #2
    expect(
      attentionRequests().filter((u) => u.includes("revision=")).length,
    ).toBe(2);
    // The block re-rendered from the new scoped response.
    const rows = detailEl().querySelectorAll(".outstanding-set li");
    expect(rows.length).toBe(1);
  });

  it("drops an out-of-order scoped response instead of overwriting a fresher cache", async () => {
    // Defer the scoped responses so they can settle out of order: a superseded
    // read resolving last must not overwrite the cache the newer read
    // committed, and the pending marker must clear on settlement so the
    // freshness check cannot stay pinned behind a read that lost the race.
    const deferred: Array<(data: unknown) => void> = [];
    const base = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      if (url.includes("revision=")) {
        return new Promise<Response>((resolve) => {
          deferred.push((data) =>
            resolve(
              new Response(
                JSON.stringify({
                  schema: "pointbreak.inspect-attention",
                  ...(data as Record<string, unknown>),
                }),
                {
                  status: 200,
                  headers: { "content-type": "application/json" },
                },
              ),
            ),
          );
        });
      }
      return base(input, init);
    }) as typeof fetch;

    store.commit({
      selected: { kind: "revision", id: REV },
      attention: attentionOf("sha256:hash-one"),
    });
    const first = detail.showComposite(REV); // composite + scoped read #1 (deferred)
    expect(deferred.length).toBe(1);

    // The global doc moves while #1 is still in flight; the repaint starts #2.
    store.commit({ attention: attentionOf("sha256:hash-two") });
    const second = detail.showComposite(REV);
    expect(deferred.length).toBe(2);

    deferred[1]({ items: [scopedItems[0]] }); // the newer read settles first
    await second;
    deferred[0]({ items: [] }); // the superseded read settles last
    await first;

    // The composite paint renders from the cache: the stale empty set was
    // dropped, so the newer one-item set survives.
    expect(detailEl().querySelectorAll(".outstanding-set li").length).toBe(1);

    // And the settled state is fresh — no further scoped fetch on repaint.
    await detail.showComposite(REV);
    expect(deferred.length).toBe(2);
  });

  it("degrades to omission when the scoped fetch fails (the page stays functional)", async () => {
    setScopedAttentionError(500, "attention read failed");
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.showComposite(REV);
    const el = detailEl();
    expect(el.querySelector(".unit-page")).not.toBeNull();
    expect(el.textContent).toContain("Current assessment");
    expect(el.querySelector(".outstanding-set")).toBeNull();
  });
});

describe("renderRevisionPage fact-supersession DAG (fork-gated, additive)", () => {
  it("mounts the assessment fact DAG in the Assessments section when it forks", async () => {
    const forked = {
      ...(revisionJson as Record<string, unknown>),
      factSupersession: {
        assessments: {
          laidOut: {
            nodes: [
              {
                id: "as:a",
                x: 30,
                y: 20,
                w: 50,
                h: 22,
                isHead: false,
                isSuperseded: true,
              },
              {
                id: "as:b",
                x: 20,
                y: 70,
                w: 50,
                h: 22,
                isHead: true,
                isSuperseded: false,
              },
              {
                id: "as:c",
                x: 90,
                y: 70,
                w: 50,
                h: 22,
                isHead: true,
                isSuperseded: false,
              },
            ],
            edges: [
              {
                from: "as:b",
                to: "as:a",
                path: [
                  [20, 58],
                  [30, 32],
                ],
                kind: "replaces",
              },
            ],
            bounds: { w: 150, h: 100 },
          },
        },
      },
    };
    setCompositeResponse(forked);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const el = detailEl();
    // The DAG mounts inside the ASSESSMENTS section specifically, reusing the painter.
    const section = Array.from(el.querySelectorAll("section")).find((s) =>
      s.querySelector("h2")?.textContent?.startsWith("Assessments"),
    );
    const figure = section?.querySelector("figure.fact-dag");
    expect(figure).not.toBeNull();
    expect(figure?.querySelectorAll("g.dag-node[data-fact-id]").length).toBe(3);
    expect(figure?.querySelectorAll("g.dag-node.head").length).toBe(2);
    // The observation section carries no DAG (this fixture only forks assessments).
    const obsSection = Array.from(el.querySelectorAll("section")).find((s) =>
      s.querySelector("h2")?.textContent?.startsWith("Observations"),
    );
    expect(obsSection?.querySelector("figure.fact-dag")).toBeNull();
  });

  it("mounts the observation fact DAG in the Observations section when it forks", async () => {
    const forked = {
      ...(revisionJson as Record<string, unknown>),
      factSupersession: {
        observations: {
          laidOut: {
            nodes: [
              {
                id: "obs:a",
                x: 40,
                y: 20,
                w: 50,
                h: 22,
                isHead: false,
                isSuperseded: true,
              },
              {
                id: "obs:b",
                x: 40,
                y: 70,
                w: 50,
                h: 22,
                isHead: true,
                isSuperseded: false,
              },
            ],
            edges: [
              {
                from: "obs:b",
                to: "obs:a",
                path: [
                  [40, 58],
                  [40, 32],
                ],
                kind: "supersedes",
              },
            ],
            bounds: { w: 100, h: 100 },
          },
        },
      },
    };
    setCompositeResponse(forked);
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const el = detailEl();
    // The DAG mounts inside the Observations section (identify it by the section
    // whose <h2> starts with "Observations").
    const section = Array.from(el.querySelectorAll("section")).find((s) =>
      s.querySelector("h2")?.textContent?.startsWith("Observations"),
    );
    expect(section?.querySelector("figure.fact-dag")).not.toBeNull();
    expect(section?.querySelectorAll("g.dag-node[data-fact-id]").length).toBe(
      2,
    );
    // The assessment DAG is NOT present (this fixture only forks observations).
    const assessmentsSection = Array.from(el.querySelectorAll("section")).find(
      (s) => s.querySelector("h2")?.textContent?.startsWith("Assessments"),
    );
    expect(assessmentsSection?.querySelector("figure.fact-dag")).toBeNull();
  });

  it("omits the fact DAG when the document carries no factSupersession", async () => {
    setCompositeResponse(revisionJson); // the default, non-forked fixture
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    expect(detailEl().querySelector("figure.fact-dag")).toBeNull();
  });
});

// The revision-level supersession block: fork-gated by the server (the field is
// absent for a singleton component) and rendered from component data identically
// under EVERY member's page — never hosted under one head, no primary chrome.
describe("renderRevisionPage revision supersession block (fork-gated)", () => {
  const RS_ROOT =
    "rev:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const RS_HEAD_B =
    "rev:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  const RS_HEAD_C =
    "rev:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
  // A forked component: B and C each supersede the root A, so the layout has two
  // equal-rank heads (id-ordered on the wire) and one superseded root.
  const REVISION_SUPERSESSION = {
    revisions: [RS_ROOT, RS_HEAD_B, RS_HEAD_C],
    heads: [RS_HEAD_B, RS_HEAD_C],
    superseded: [RS_ROOT],
    competing: true,
    laidOut: {
      bounds: { w: 300, h: 200 },
      nodes: [
        {
          id: RS_ROOT,
          x: 150,
          y: 150,
          w: 120,
          h: 40,
          isHead: false,
          isSuperseded: true,
        },
        {
          id: RS_HEAD_B,
          x: 80,
          y: 50,
          w: 120,
          h: 40,
          isHead: true,
          isSuperseded: false,
        },
        {
          id: RS_HEAD_C,
          x: 220,
          y: 50,
          w: 120,
          h: 40,
          isHead: true,
          isSuperseded: false,
        },
      ],
      edges: [
        {
          from: RS_HEAD_B,
          to: RS_ROOT,
          path: [
            [80, 70],
            [150, 130],
          ],
        },
        {
          from: RS_HEAD_C,
          to: RS_ROOT,
          path: [
            [220, 70],
            [150, 130],
          ],
        },
      ],
    },
  };

  /** The forked composite document as served under `selfId`'s page. */
  function forkedDocFor(selfId: string): Record<string, unknown> {
    const base = revisionJson as Record<string, unknown>;
    return {
      ...base,
      revision: { ...(base.revision as Record<string, unknown>), id: selfId },
      revisionSupersession: REVISION_SUPERSESSION,
    };
  }

  async function openForkedPage(selfId: string): Promise<void> {
    setCompositeResponse(forkedDocFor(selfId));
    store.commit({ selected: { kind: "revision", id: selfId } });
    await detail.openRevision(selfId);
  }

  function blockEl(): HTMLElement | null {
    return detailEl().querySelector<HTMLElement>(
      "figure.revision-supersession",
    );
  }

  it("renders the DAG and unranked competing-head chips when the block is present", async () => {
    await openForkedPage(RS_HEAD_B);
    const figure = blockEl();
    expect(figure).not.toBeNull();
    expect(figure?.querySelector("svg.revision-dag")).not.toBeNull();
    expect(
      figure?.querySelectorAll("g.dag-node[data-revision-id]").length,
    ).toBe(3);
    // Every head renders as a navigable peer chip, in the server's id order —
    // no recency sort, no "current first", no primary styling hook.
    const chips = Array.from(
      figure?.querySelectorAll('.revision-heads [data-ref-kind="rev"]') ?? [],
    );
    expect(chips.map((c) => c.getAttribute("data-ref-id"))).toEqual([
      RS_HEAD_B,
      RS_HEAD_C,
    ]);
    expect(figure?.querySelector(".revision-heads")?.textContent).toContain(
      "competing revisions (2)",
    );
  });

  it("marks the reader's own head with only a quiet you-are-here note", async () => {
    await openForkedPage(RS_HEAD_B);
    const notes = Array.from(
      blockEl()?.querySelectorAll(".revision-self") ?? [],
    );
    expect(notes.length).toBe(1);
    expect(notes[0]?.textContent).toBe("you are here");
    // The marker follows the self chip, not the other head's.
    expect(notes[0]?.previousElementSibling?.getAttribute("data-ref-id")).toBe(
      RS_HEAD_B,
    );
  });

  it("renders identical bytes under every member's page (host-independence)", async () => {
    await openForkedPage(RS_HEAD_B);
    const asB = blockEl()?.outerHTML ?? "";
    await openForkedPage(RS_HEAD_C);
    const asC = blockEl()?.outerHTML ?? "";
    expect(asB).not.toBe("");
    expect(asC).not.toBe("");
    // The hosts differ only in the self-node selection state and the quiet
    // you-are-here marker; everything else is byte-identical component data.
    const normalize = (html: string): string =>
      html
        .replaceAll(' aria-selected="true"', "")
        .replace(/<span class="revision-self">[^<]*<\/span>/g, "");
    expect(normalize(asB)).toBe(normalize(asC));
    expect(asB).not.toBe(asC); // the self markers really do move
  });

  it("renders no block when the document carries no revisionSupersession", async () => {
    setCompositeResponse(revisionJson); // the default, non-forked fixture
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    expect(blockEl()).toBeNull();
  });

  it("navigates to a revision when its DAG node is clicked", async () => {
    await openForkedPage(RS_HEAD_B);
    const node = blockEl()?.querySelector<SVGGElement>(
      `g.dag-node[data-revision-id="${RS_ROOT}"]`,
    );
    expect(node).not.toBeNull();
    node?.dispatchEvent(new Event("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({
      kind: "revision",
      id: RS_ROOT,
    });
    // Opening a revision from the DAG clears any open diff overlay route.
    expect(store.getState().diff).toBeNull();
  });

  it("activates a DAG node from the keyboard (Enter)", async () => {
    await openForkedPage(RS_HEAD_B);
    const node = blockEl()?.querySelector<SVGGElement>(
      `g.dag-node[data-revision-id="${RS_HEAD_C}"]`,
    );
    expect(node?.getAttribute("tabindex")).toBe("0");
    expect(node?.getAttribute("role")).toBe("link");
    node?.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    expect(store.getState().selected).toEqual({
      kind: "revision",
      id: RS_HEAD_C,
    });
  });

  it("traces a node and its incident edges on hover, swapping the arrowhead marker", async () => {
    await openForkedPage(RS_HEAD_B);
    const figure = blockEl();
    const headB = figure?.querySelector<SVGGElement>(
      `g.dag-node[data-revision-id="${RS_HEAD_B}"]`,
    );
    headB?.dispatchEvent(new Event("mouseenter"));
    expect(headB?.classList.contains("traced")).toBe(true);
    const traced = figure?.querySelectorAll("polyline.dag-edge.traced") ?? [];
    expect(traced.length).toBe(1); // only the B→root edge is incident to B
    const incident = figure?.querySelector(
      `polyline.dag-edge[data-from="${RS_HEAD_B}"]`,
    );
    expect(incident?.getAttribute("marker-end")).toBe("url(#dag-arrow-traced)");
    headB?.dispatchEvent(new Event("mouseleave"));
    expect(headB?.classList.contains("traced")).toBe(false);
    expect(incident?.getAttribute("marker-end")).toBe("url(#dag-arrow)");
  });
});

describe("revision-page CLI handoffs (exact-revision templates + copy delegate)", () => {
  function stubClipboard(writeText: (text: string) => Promise<void>): void {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
  }

  it("offers the five stage templates filled with the loaded revision id", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const section = detailEl().querySelector(".workflow-handoffs");
    expect(section).not.toBeNull();
    const commands = Array.from(
      section?.querySelectorAll("[data-workflow-command]") ?? [],
    ).map((code) => code.textContent ?? "");
    expect(commands.length).toBe(5);
    expect(commands[0]).toBe(
      `pointbreak observation add --exact-revision ${REV} --track <your-track> --title "<claim title>" --body "<why it matters>"`,
    );
    expect(commands[4]).toBe(
      `pointbreak association record --revision ${REV} --track <your-track> --commit <landed-commit>`,
    );
    // Every template carries its copy control and stays read-only.
    expect(
      section?.querySelectorAll("[data-copy-workflow-command]").length,
    ).toBe(5);
  });

  it("renders kind-specific commands beside complete outstanding items only", async () => {
    setScopedAttentionResponse({
      items: [
        {
          id: "open_input_request:input-request:sha256:full",
          kind: "open_input_request",
          tier: "primary",
          revisionId: REV,
          title: "Answerable",
          inputRequestId: "input-request:sha256:full",
        },
        {
          id: "open_input_request:input-request:sha256:partial",
          kind: "open_input_request",
          tier: "primary",
          revisionId: REV,
          title: "Missing id",
        },
      ],
    });
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.showComposite(REV);
    const block = detailEl().querySelector(".outstanding-set");
    const commands = Array.from(
      block?.querySelectorAll("[data-workflow-command]") ?? [],
    ).map((code) => code.textContent ?? "");
    expect(commands).toEqual([
      'pointbreak input-request respond input-request:sha256:full --outcome <approved|rejected|dismissed|superseded|abandoned> --reason "<answer>"',
    ]);
  });

  it("the detail copy delegate writes only to the clipboard and mutates nothing", async () => {
    store.commit({ selected: { kind: "revision", id: REV } });
    await detail.openRevision(REV);
    const button = detailEl().querySelector<HTMLElement>(
      ".workflow-handoffs [data-copy-workflow-command]",
    );
    expect(button).not.toBeNull();
    const expected =
      detailEl().querySelector(".workflow-handoffs [data-workflow-command]")
        ?.textContent ?? "";
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard(writeText);
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const selectionBefore = { ...store.getState().selected };

    button?.click();
    await Promise.resolve();

    expect(writeText).toHaveBeenCalledWith(expected);
    expect(expected).not.toBe("");
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(store.getState().selected).toEqual(selectionBefore);
    fetchSpy.mockRestore();
  });
});
