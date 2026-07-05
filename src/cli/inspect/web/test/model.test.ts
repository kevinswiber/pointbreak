import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Revision } from "../src/projection";
import type { HistoryDoc, RevisionsDoc, ThreadsDoc } from "../src/store";
import type { HistoryEntry, SearchIndex } from "../src/types";
import historyJson from "./fixtures/history.json";
import revisionsJson from "./fixtures/revisions.json";
import threadsJson from "./fixtures/threads.json";

// `model.ts` is state-reading but DOM-free, so each derivation and filter
// predicate is exercised over a constructed store: seed the committed `/api/*`
// fixtures (the wire-parity path) or hand-built synthetic docs (the supersession
// DAG / multi-revision / filter scenarios the single-revision fixture cannot
// cover). The store and the model are module singletons sharing one `state`, so
// reset the module registry before each test and re-import both.
type Store = typeof import("../src/store");
type Model = typeof import("../src/model");
let store: Store;
let model: Model;

beforeEach(async () => {
  vi.resetModules();
  store = await import("../src/store");
  model = await import("../src/model");
});

// The single revision/object the committed fixtures describe.
const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";
const OBJ =
  "obj:sha256:38a493d2f09d6fde9d1dcac61a12c4ccc4de42a0b9c6829752d34cc648a9f9d7";
const ARTIFACT =
  "sha256:32161336d3627d277a7a5917abe2e2694edec4f3621dbf939bf22091b40e0871";

/** Seed the three committed `/api/*` documents the model reads. */
function seedFixtures(): void {
  store.commit({
    history: historyJson as unknown as HistoryDoc,
    revisions: revisionsJson as unknown as RevisionsDoc,
    threads: threadsJson as unknown as ThreadsDoc,
  });
}

/** Commit a synthetic objects doc (threads + classification) for DAG scenarios. */
function seedObjects(doc: {
  threads?: unknown[];
  revisionClassification?: Record<string, unknown>;
}): void {
  store.commit({ threads: doc as unknown as ThreadsDoc });
}

describe("presentTypes", () => {
  it("lists the known event types present in canonical order, then unknowns", () => {
    seedFixtures();
    // The fixture carries capture/observation/assessment/request/validation in a
    // shuffled timeline plus the unknown `revision_ref_associated`. Known types
    // come back in TYPES order; the unknown is appended after.
    expect(model.presentTypes()).toEqual([
      "work_object_proposed",
      "review_observation_recorded",
      "review_assessment_recorded",
      "input_request_opened",
      "validation_check_recorded",
      "revision_ref_associated",
    ]);
  });

  it("returns an empty list when no history is loaded", () => {
    expect(model.presentTypes()).toEqual([]);
  });
});

describe("currentThreads", () => {
  it("returns the threads from the loaded objects doc", () => {
    seedFixtures();
    const threads = model.currentThreads();
    expect(threads).toHaveLength(1);
    expect(threads[0]?.revisions).toEqual([REV]);
  });

  it("returns an empty list when no objects doc is loaded", () => {
    expect(model.currentThreads()).toEqual([]);
  });
});

describe("threadRevisionOrder", () => {
  it("orders revisions by their laid-out node position (y, then x)", () => {
    const thread = {
      revisions: ["rev:a", "rev:b"],
      laidOut: {
        nodes: [
          { id: "rev:b", x: 0, y: 0 },
          { id: "rev:a", x: 0, y: 10 },
        ],
      },
    };
    expect(model.threadRevisionOrder(thread)).toEqual(["rev:b", "rev:a"]);
  });

  it("falls back to the declared revision list when there are no laid-out nodes", () => {
    expect(
      model.threadRevisionOrder({ revisions: ["rev:a", "rev:b"] }),
    ).toEqual(["rev:a", "rev:b"]);
  });

  it("appends revisions missing from the layout after the laid-out order", () => {
    const thread = {
      revisions: ["rev:a", "rev:b", "rev:c"],
      laidOut: {
        nodes: [
          { id: "rev:b", x: 0, y: 0 },
          { id: "rev:a", x: 0, y: 5 },
        ],
      },
    };
    expect(model.threadRevisionOrder(thread)).toEqual([
      "rev:b",
      "rev:a",
      "rev:c",
    ]);
  });
});

describe("revisionClassification family", () => {
  beforeEach(() => {
    seedObjects({
      revisionClassification: {
        "rev:head": {
          state: "head",
          supersededBy: [],
          supersedes: ["rev:old"],
        },
        "rev:old": {
          state: "superseded",
          supersededBy: ["rev:head"],
          supersedes: [],
        },
        "rev:iso": { state: "isolated", supersededBy: [], supersedes: [] },
      },
    });
  });

  it("treats head and isolated revisions as current heads", () => {
    expect(model.revisionIsHead("rev:head")).toBe(true);
    expect(model.revisionIsHead("rev:iso")).toBe(true);
    expect(model.revisionIsHead("rev:old")).toBe(false);
    expect(model.revisionIsHead("rev:unknown")).toBe(false);
  });

  it("reads the direct superseders of a revision", () => {
    expect(model.supersededByRevision("rev:old")).toEqual(["rev:head"]);
    expect(model.supersededByRevision("rev:head")).toEqual([]);
    expect(model.supersededByRevision("rev:unknown")).toEqual([]);
  });

  it("reads the predecessors a revision supersedes", () => {
    expect(model.supersedesRevision("rev:head")).toEqual(["rev:old"]);
    expect(model.supersedesRevision("rev:old")).toEqual([]);
  });

  it("isolated revisions in the fixture classify as heads", () => {
    seedFixtures();
    expect(model.revisionIsHead(REV)).toBe(true);
  });
});

describe("snapshot accessors over the revisions list", () => {
  beforeEach(seedFixtures);

  it("resolves a revision to its captured snapshot id", () => {
    expect(model.snapshotIdForRevision(REV)).toBe(OBJ);
    expect(model.snapshotIdForRevision("rev:missing")).toBe("");
  });

  it("resolves a revision to its captured snapshot content hash", () => {
    expect(model.snapshotContentHashForRevision(REV)).toBe(ARTIFACT);
    expect(model.snapshotContentHashForRevision("rev:missing")).toBe("");
  });

  it("finds the revision that captured a snapshot, disambiguating by content hash", () => {
    expect(model.revisionIdForSnapshot(OBJ)).toBe(REV);
    expect(model.revisionIdForSnapshot(OBJ, ARTIFACT)).toBe(REV);
    // A mismatched content hash falls back to the first snapshot-id match.
    expect(model.revisionIdForSnapshot(OBJ, "sha256:nomatch")).toBe(REV);
    expect(model.revisionIdForSnapshot("obj:missing")).toBeNull();
  });

  it("no longer exports the object-vocabulary accessors", () => {
    const m = model as unknown as Record<string, unknown>;
    expect(m.objectIdForRevision).toBeUndefined();
    expect(m.objectArtifactHashForRevision).toBeUndefined();
    expect(m.revisionIdForObject).toBeUndefined();
  });
});

describe("revisionForId / overviewForRevision", () => {
  beforeEach(seedFixtures);

  it("finds a revision entry by id", () => {
    expect(model.revisionForId(REV)?.snapshotId).toBe(OBJ);
    expect(model.revisionForId("rev:missing")).toBeNull();
  });

  it("reads the server overview for a revision", () => {
    expect(model.overviewForRevision(REV)?.currentAssessment?.assessment).toBe(
      "accepted",
    );
    expect(model.overviewForRevision("rev:missing")).toBeNull();
  });
});

describe("isSupersedableFact", () => {
  it("classifies the review-fact event types as supersedable", () => {
    for (const type of [
      "review_observation_recorded",
      "review_assessment_recorded",
      "input_request_opened",
      "validation_check_recorded",
    ]) {
      expect(model.isSupersedableFact({ eventType: type })).toBe(true);
    }
  });

  it("classifies capture and note-import events as not supersedable", () => {
    expect(
      model.isSupersedableFact({ eventType: "work_object_proposed" }),
    ).toBe(false);
    expect(
      model.isSupersedableFact({ eventType: "review_note_imported" }),
    ).toBe(false);
  });
});

describe("supersession badges", () => {
  beforeEach(() => {
    seedObjects({
      revisionClassification: {
        "rev:head": {
          state: "head",
          supersededBy: [],
          supersedes: ["rev:old"],
        },
        "rev:old": {
          state: "superseded",
          supersededBy: ["rev:head"],
          supersedes: [],
        },
      },
    });
  });

  it("stale-badges a supersedable fact on a superseded revision, naming successors", () => {
    const badge = model.supersessionStaleBadge({
      eventType: "review_observation_recorded",
      subject: { revisionId: "rev:old" },
    });
    expect(badge).toContain("superseded by");
    expect(badge).toContain("rev:old".replace("rev:old", "rev:head"));
    expect(badge).toContain("stale");
  });

  it("does not stale-badge a fact on a current head", () => {
    expect(
      model.supersessionStaleBadge({
        eventType: "review_observation_recorded",
        subject: { revisionId: "rev:head" },
      }),
    ).toBe("");
  });

  it("does not stale-badge a non-supersedable event", () => {
    expect(
      model.supersessionStaleBadge({
        eventType: "work_object_proposed",
        subject: { revisionId: "rev:old" },
      }),
    ).toBe("");
  });

  it("badges a capture event with the predecessors it supersedes", () => {
    const badge = model.captureSupersedesBadge({
      eventType: "work_object_proposed",
      subject: { revisionId: "rev:head" },
    });
    expect(badge).toContain("supersedes");
    expect(badge).toContain("rev:old");
  });

  it("does not supersedes-badge a non-capture event or a capture with no predecessors", () => {
    expect(
      model.captureSupersedesBadge({
        eventType: "review_observation_recorded",
        subject: { revisionId: "rev:head" },
      }),
    ).toBe("");
    expect(
      model.captureSupersedesBadge({
        eventType: "work_object_proposed",
        subject: { revisionId: "rev:old" },
      }),
    ).toBe("");
  });

  it("renders the per-revision supersession status badge", () => {
    expect(model.supersessionBadge("rev:head")).toContain("current in thread");
    const superseded = model.supersessionBadge("rev:old");
    expect(superseded).toContain("superseded by");
    expect(superseded).toContain("rev:head");
    expect(model.supersessionBadge("")).toBe("");
  });
});

describe("fact-level supersession (client-side reverse index over the loaded window)", () => {
  /** Seed a history window whose loaded siblings carry forward pointers. */
  function seedFactSupersession(): void {
    store.commit({
      history: {
        entries: [
          {
            eventType: "review_assessment_recorded",
            summary: { assessmentId: "assess:new", replaces: ["assess:old"] },
          },
          {
            eventType: "review_assessment_recorded",
            summary: { assessmentId: "assess:old" },
          },
          {
            eventType: "review_observation_recorded",
            summary: { observationId: "obs:new", supersedes: ["obs:old"] },
          },
          {
            eventType: "review_observation_recorded",
            summary: { observationId: "obs:old" },
          },
        ],
        diagnostics: [],
      } as unknown as HistoryDoc,
    });
  }

  it("reverses the loaded forward pointers into a superseded-by index", () => {
    seedFactSupersession();
    expect(model.factSupersededBy("assess:old")).toEqual(["assess:new"]);
    expect(model.factSupersededBy("obs:old")).toEqual(["obs:new"]);
    // A superseder is not itself superseded; an unknown id has no superseders.
    expect(model.factSupersededBy("assess:new")).toEqual([]);
    expect(model.factSupersededBy("obs:unknown")).toEqual([]);
  });

  it("pills a superseded observation and a replaced assessment; leaves current facts unpilled", () => {
    seedFactSupersession();
    const obsBadge = model.factSupersessionBadge({
      eventType: "review_observation_recorded",
      summary: { observationId: "obs:old" },
    });
    expect(obsBadge).toContain("superseded");
    expect(obsBadge).toContain("badge superseded");

    const assessBadge = model.factSupersessionBadge({
      eventType: "review_assessment_recorded",
      summary: { assessmentId: "assess:old" },
    });
    expect(assessBadge).toContain("replaced");

    expect(
      model.factSupersessionBadge({
        eventType: "review_observation_recorded",
        summary: { observationId: "obs:new" },
      }),
    ).toBe("");
  });
});

describe("annotationsForRevision", () => {
  beforeEach(seedFixtures);

  it("gathers observations, input requests, and assessments into one list", () => {
    const annos = model.annotationsForRevision(REV);
    expect(annos.map((a) => a.kind)).toEqual([
      "observation",
      "input-request",
      "assessment",
      "assessment",
    ]);
  });

  it("carries the fact identity, title, body, and track for each annotation", () => {
    const annos = model.annotationsForRevision(REV);
    const observation = annos[0];
    expect(observation?.id).toBe(
      "obs:sha256:752a5b0ab30cfa3aa062bcf6f11b4c6ee3dcfd055207b6a995b91bf81ffec8d9",
    );
    expect(observation?.title).toBe("Observed change");
    expect(observation?.body).toBe("the return value changed");
    expect(observation?.track).toBe("agent:codex");

    const request = annos[1];
    expect(request?.title).toBe("Need a decision");
    expect(request?.tags).toEqual(["operative · manual_decision_required"]);

    const needsChanges = annos[2];
    expect(needsChanges?.title).toBe("assessment: needs-changes");
    expect(needsChanges?.body).toBe("not yet");

    const accepted = annos[3];
    expect(accepted?.title).toBe("assessment: accepted");
    expect(accepted?.track).toBe("human:kevin");
  });

  it("returns an empty list for a revision with no facts", () => {
    expect(model.annotationsForRevision("rev:missing")).toEqual([]);
  });
});

describe("renderThreadRevisionOverview", () => {
  beforeEach(seedFixtures);

  it("renders the thread overview from the revision target and overview", () => {
    const html = model.renderThreadRevisionOverview(REV);
    expect(html).toContain("thread-overview");
    expect(html).toContain(".tmplPi8eZ");
    expect(html).toContain("current assessment");
    expect(html).toContain("review cues");
  });

  it("renders nothing for a revision with no entry or overview", () => {
    expect(model.renderThreadRevisionOverview("rev:missing")).toBe("");
  });
});

describe("existence predicates", () => {
  beforeEach(seedFixtures);

  it("reports whether a revision exists in the revisions list", () => {
    expect(model.revisionExists(REV)).toBe(true);
    expect(model.revisionExists("rev:missing")).toBe(false);
  });

  it("reports whether an event exists in the history", () => {
    const existing = (historyJson as unknown as HistoryDoc).entries[0]
      .eventId as string;
    expect(model.eventExists(existing)).toBe(true);
    expect(model.eventExists("evt:missing")).toBe(false);
  });

  it("reports whether a revision appears in any laid-out thread", () => {
    seedObjects({
      threads: [{ revisions: ["rev:a", "rev:b"] }],
    });
    expect(model.revisionInAnyThread("rev:a")).toBe(true);
    expect(model.revisionInAnyThread("rev:z")).toBe(false);
  });
});

describe("selectedEventId", () => {
  it("returns the selected id only when the selection is an event", () => {
    expect(model.selectedEventId()).toBeNull();
    store.commit({ selected: { kind: "event", id: "evt:1" } });
    expect(model.selectedEventId()).toBe("evt:1");
    store.commit({ selected: { kind: "revision", id: "rev:1" } });
    expect(model.selectedEventId()).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// State-bound filter predicates (currentClauses / matchesFilters / facetCounts)
// over a synthetic two-object timeline, so the track / object / query slices can
// each be isolated.
// ---------------------------------------------------------------------------

/** A history entry with a precomputed search index, as `load()` will build it. */
function indexedEntry(
  eventType: string,
  eventId: string,
  revisionId: string,
  track: string,
  search: SearchIndex,
): HistoryEntry {
  return {
    eventType,
    eventId,
    trackId: track,
    subject: { revisionId },
    __search: search,
  };
}

/** Seed a two-object timeline plus the revisions that map each event to an object. */
function seedTimeline(): void {
  const entries = [
    indexedEntry(
      "review_observation_recorded",
      "evt:1",
      "rev:a",
      "agent:codex",
      {
        text: "alpha observation",
        type: "review_observation_recorded",
        track: "agent:codex",
        revision: "rev:a",
        snapshot: "obj:a",
        status: "",
      },
    ),
    indexedEntry(
      "review_assessment_recorded",
      "evt:2",
      "rev:b",
      "human:kevin",
      {
        text: "beta assessment",
        type: "review_assessment_recorded",
        track: "human:kevin",
        revision: "rev:b",
        snapshot: "obj:b",
        status: "accepted",
      },
    ),
    indexedEntry(
      "review_observation_recorded",
      "evt:3",
      "rev:a",
      "agent:codex",
      {
        text: "gamma observation",
        type: "review_observation_recorded",
        track: "agent:codex",
        revision: "rev:a",
        snapshot: "obj:a",
        status: "",
      },
    ),
  ];
  store.commit({
    history: { entries, diagnostics: [] } as unknown as HistoryDoc,
    revisions: {
      entries: [
        { revisionId: "rev:a", snapshotId: "obj:a" },
        { revisionId: "rev:b", snapshotId: "obj:b" },
      ],
    } as unknown as RevisionsDoc,
  });
}

describe("the history filter predicates are retired (server-owned now)", () => {
  it("no longer exports the timeline query predicates", () => {
    // The server filters/searches/facets the history now; the client keeps only
    // the revisions/threads-lens predicates.
    const m = model as unknown as Record<string, unknown>;
    expect(m.matchesFilters).toBeUndefined();
    expect(m.facetCounts).toBeUndefined();
    expect(m.currentClauses).toBeUndefined();
    expect(m.eventMatchesObject).toBeUndefined();
    expect(typeof model.matchesRevisionFilters).toBe("function");
    expect(typeof model.threadMatchesRevisionFilters).toBe("function");
  });
});

describe("revision filter predicates", () => {
  beforeEach(seedTimeline);

  it("matchesRevisionFilters honors the object filter and the query clauses", () => {
    const revA = { revisionId: "rev:a", snapshotId: "obj:a" };
    const revB = { revisionId: "rev:b", snapshotId: "obj:b" };
    store.commit({ filterSnapshot: "obj:a" });
    expect(model.matchesRevisionFilters(revA)).toBe(true);
    expect(model.matchesRevisionFilters(revB)).toBe(false);
  });

  it("threadMatchesRevisionFilters keeps a thread with any matching revision", () => {
    store.commit({ filterSnapshot: "obj:a" });
    expect(
      model.threadMatchesRevisionFilters({ revisions: ["rev:a", "rev:b"] }),
    ).toBe(true);
    expect(model.threadMatchesRevisionFilters({ revisions: ["rev:b"] })).toBe(
      false,
    );
  });

  it("threadMatchesRevisionFilters passes everything when no filter is set", () => {
    expect(model.threadMatchesRevisionFilters({ revisions: ["rev:b"] })).toBe(
      true,
    );
  });

  it("filteredThreadRevisionIds keeps only the matching revision ids in order", () => {
    store.commit({ filterSnapshot: "obj:a" });
    expect(
      model.filteredThreadRevisionIds({ revisions: ["rev:a", "rev:b"] }),
    ).toEqual(["rev:a"]);
  });
});

describe("lensEntryIds", () => {
  it("lists the filtered revisions in order for the list lens", () => {
    seedTimeline();
    store.commit({ lens: "list" });
    expect(model.lensEntryIds()).toEqual([
      { kind: "revision", id: "rev:a" },
      { kind: "revision", id: "rev:b" },
    ]);
  });

  it("walks each thread's laid-out revisions for the threads lens", () => {
    seedTimeline();
    seedObjects({
      threads: [
        {
          revisions: ["rev:a", "rev:b"],
          laidOut: {
            nodes: [
              { id: "rev:b", x: 0, y: 0 },
              { id: "rev:a", x: 0, y: 10 },
            ],
          },
        },
      ],
    });
    store.commit({ lens: "threads" });
    expect(model.lensEntryIds()).toEqual([
      { kind: "revision", id: "rev:b" },
      { kind: "revision", id: "rev:a" },
    ]);
  });

  it("orders list-lens cursor entries newest-first, matching the rendered cards", () => {
    // The cursor order must track the rendered card order (orderedRevisionEntries),
    // or keyboard stepping selects a different revision than the top card.
    store.commit({
      lens: "list",
      revisions: {
        entries: [
          { revisionId: "rev:a", capturedAt: "unix-ms:100" },
          { revisionId: "rev:c", capturedAt: "unix-ms:300" },
          { revisionId: "rev:b", capturedAt: "unix-ms:200" },
        ],
      } as unknown as RevisionsDoc,
    });
    expect(model.lensEntryIds().map((e) => e.id)).toEqual([
      "rev:c",
      "rev:b",
      "rev:a",
    ]);
  });

  it("orders threads-lens cursor cards newest-first by member recency", () => {
    store.commit({
      lens: "threads",
      revisions: {
        entries: [
          { revisionId: "rev:a", capturedAt: "unix-ms:100" },
          { revisionId: "rev:b", capturedAt: "unix-ms:500" },
        ],
      } as unknown as RevisionsDoc,
    });
    seedObjects({
      threads: [{ revisions: ["rev:a"] }, { revisions: ["rev:b"] }],
    });
    expect(model.lensEntryIds().map((e) => e.id)).toEqual(["rev:b", "rev:a"]);
  });

  it("lists the loaded timeline events in server order for the timeline lens", () => {
    // The server pre-filters and pre-orders the page; the lens paints the loaded
    // window as-is, so the order toggle no longer reorders it client-side.
    seedTimeline();
    store.commit({ lens: "timeline", order: "desc" });
    expect(model.lensEntryIds().map((s) => s.id)).toEqual([
      "evt:1",
      "evt:2",
      "evt:3",
    ]);
    store.commit({ order: "asc" });
    expect(model.lensEntryIds().map((s) => s.id)).toEqual([
      "evt:1",
      "evt:2",
      "evt:3",
    ]);
  });
});

describe("newest-first ordering", () => {
  const rev = (id: string, ms: number) =>
    ({ revisionId: id, capturedAt: `unix-ms:${ms}` }) as unknown as Revision;

  it("orders revision entries newest-first for desc and oldest-first for asc", () => {
    const entries = [rev("a", 100), rev("c", 300), rev("b", 200)];
    expect(
      model.orderedRevisionEntries(entries, "desc").map((r) => r.revisionId),
    ).toEqual(["c", "b", "a"]);
    expect(
      model.orderedRevisionEntries(entries, "asc").map((r) => r.revisionId),
    ).toEqual(["a", "b", "c"]);
  });

  it("sorts numerically, not lexicographically, and puts undated entries last (desc)", () => {
    const entries = [
      rev("big", 1000000000000),
      rev("small", 900000000000),
      { revisionId: "none" } as Revision,
    ];
    expect(
      model.orderedRevisionEntries(entries, "desc").map((r) => r.revisionId),
    ).toEqual(["big", "small", "none"]);
  });

  it("derives a thread's recency from its most-recent member revision", () => {
    store.commit({
      revisions: {
        entries: [rev("a", 100), rev("b", 500)],
      } as unknown as RevisionsDoc,
    });
    expect(model.threadRecencyMs({ revisions: ["a", "b"] })).toBe(500);
    expect(model.threadRecencyMs({ revisions: [] })).toBe(
      Number.NEGATIVE_INFINITY,
    );
  });

  it("orders threads newest-first by member recency", () => {
    store.commit({
      revisions: {
        entries: [rev("a", 100), rev("b", 500), rev("c", 300)],
      } as unknown as RevisionsDoc,
    });
    const older = { revisions: ["a"] };
    const newer = { revisions: ["b", "c"] };
    expect(model.orderedThreads([older, newer], "desc")).toEqual([
      newer,
      older,
    ]);
    expect(model.orderedThreads([older, newer], "asc")).toEqual([older, newer]);
  });
});
