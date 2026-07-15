import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import {
  AttentionTreeProvider,
  attentionCount,
  deriveTree,
} from "../src/attentionView";
import type {
  AttentionItem,
  AttentionListDoc,
  PointbreakCli,
  RevisionListDoc,
} from "../src/cli";
import type { TargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => ({
  EventEmitter: class<T> {
    readonly event = vi.fn();
    fire = vi.fn<(value?: T) => void>();
    dispose = vi.fn();
  },
  ThemeIcon: class {
    constructor(readonly id: string) {}
  },
  TreeItem: class {
    constructor(
      readonly label: string,
      readonly collapsibleState?: number,
    ) {}
  },
}));

vi.mock("vscode", () => ({
  EventEmitter: vscodeMocks.EventEmitter,
  ThemeIcon: vscodeMocks.ThemeIcon,
  TreeItem: vscodeMocks.TreeItem,
  TreeItemCollapsibleState: { None: 0, Collapsed: 1, Expanded: 2 },
}));

beforeEach(() => {
  vi.useRealTimers();
});

describe("deriveTree", () => {
  it("derives one section per resolved target, ordered by folder position", () => {
    const resolutions = [resolved("/a", "a"), resolved("/b", "b")];

    const nodes = deriveTree(
      resolutions,
      new Map([
        ["a", attention("A")],
        ["b", attention("B")],
      ]),
      new Map([
        ["a", revisions("rev:sha256:a")],
        ["b", revisions("rev:sha256:b")],
      ]),
    );

    expect(nodes.map((node) => node.kind)).toEqual(["target", "target"]);
    expect(nodes.map((node) => node.targetKey)).toEqual(["a", "b"]);
  });

  it("collapses to flat sections when exactly one target resolves", () => {
    const nodes = deriveTree(
      [resolved("/a", "a")],
      new Map([["a", attention("A")]]),
      new Map([["a", revisions("rev:sha256:a")]]),
    );

    expect(nodes.map((node) => node.kind)).toEqual([
      "attention-section",
      "revisions-section",
    ]);
  });

  it("maps an empty-inventory target to the capture onboarding affordance", () => {
    const nodes = deriveTree(
      [resolved("/empty", "empty", true)],
      new Map(),
      new Map(),
    );

    expect(nodes).toMatchObject([
      { kind: "onboarding", command: "pointbreak.capture" },
    ]);
  });

  it("renders error resolutions as repair-state nodes", () => {
    const nodes = deriveTree(
      [errorResolution("/outside", "not a Git worktree")],
      new Map(),
      new Map(),
    );

    expect(nodes).toMatchObject([
      { kind: "repair", message: "not a Git worktree" },
    ]);
  });

  it("renders flattened attention titles and falls back to their kind", () => {
    const nodes = deriveTree(
      [resolved("/a", "a")],
      new Map([
        [
          "a",
          attentionItems([
            attentionItem("with-title", "open_input_request", "Review this"),
            attentionItem("without-title", "stale_assessment"),
          ]),
        ],
      ]),
      new Map([["a", revisions()]]),
    );
    const section = nodes.find((node) => node.kind === "attention-section");

    expect(section?.children.map((child) => child.label)).toEqual([
      "Review this",
      "stale_assessment",
    ]);
    expect(section?.children).toMatchObject([
      {
        lens: "attention",
        attentionId: "with-title",
        command: "pointbreak.openAnnotatedDiff",
      },
      {
        lens: "attention",
        attentionId: "without-title",
        command: "pointbreak.openAnnotatedDiff",
      },
    ]);
  });

  it("opens revision and eligible attention nodes in one annotated diff panel", () => {
    const nodes = deriveTree(
      [resolved("/a", "a")],
      new Map([
        [
          "a",
          attentionItems([
            attentionItem("with-revision", "open_input_request"),
            {
              ...attentionItem("without-revision", "competing_heads"),
              revisionId: undefined,
            },
          ]),
        ],
      ]),
      new Map([["a", revisions("rev:sha256:a")]]),
    );
    const attentionSection = nodes.find(
      (node) => node.kind === "attention-section",
    );
    const revisionsSection = nodes.find(
      (node) => node.kind === "revisions-section",
    );

    expect(attentionSection?.children).toMatchObject([
      { command: "pointbreak.openAnnotatedDiff" },
      { command: undefined },
    ]);
    expect(revisionsSection?.children).toMatchObject([
      { command: "pointbreak.openAnnotatedDiff" },
    ]);
  });

  it("retains typed input-request details and exposes only their response action", () => {
    const openRequest = {
      ...attentionItem("open", "open_input_request"),
      inputRequestId: "input-request:sha256:open",
      mode: "operative" as const,
      reasonCode: "manual_decision_required",
      title: "Choose the release boundary",
      trackId: "agent:review",
      openedBy: "actor:agent:reviewer",
    };
    const followUp = {
      ...attentionItem("follow", "follow_up_outstanding"),
      assessmentId: "assess:sha256:follow",
      trackId: "agent:review",
      recordedBy: "actor:agent:reviewer",
      openInputRequestIds: [
        "input-request:sha256:one",
        "input-request:sha256:two",
      ],
    };
    const nodes = deriveTree(
      [resolved("/a", "a")],
      new Map([["a", attentionItems([openRequest, followUp])]]),
      new Map([["a", revisions("rev:sha256:a")]]),
    );
    const section = nodes.find((node) => node.kind === "attention-section");
    const provider = new AttentionTreeProvider({} as PointbreakCli, [
      resolved("/a", "a"),
    ]);

    expect(section?.children).toMatchObject([
      {
        item: {
          kind: "open_input_request",
          inputRequestId: "input-request:sha256:open",
          title: "Choose the release boundary",
          mode: "operative",
          reasonCode: "manual_decision_required",
          trackId: "agent:review",
          openedBy: "actor:agent:reviewer",
        },
      },
      {
        item: {
          kind: "follow_up_outstanding",
          openInputRequestIds: [
            "input-request:sha256:one",
            "input-request:sha256:two",
          ],
        },
      },
    ]);
    expect(
      section?.children.map(
        (child) => provider.getTreeItem(child).contextValue,
      ),
    ).toEqual([
      "pointbreak.attention.inputRequest",
      "pointbreak.attention.inputRequest",
    ]);
    provider.dispose();
  });

  it("routes assessment-bearing attention kinds to the assessment action", () => {
    const nodes = deriveTree(
      [resolved("/a", "a")],
      new Map([
        [
          "a",
          attentionItems([
            attentionItem("ambiguous", "ambiguous_assessment"),
            attentionItem("stale", "stale_assessment"),
            attentionItem("failed", "failed_validation"),
          ]),
        ],
      ]),
      new Map([["a", revisions("rev:sha256:a")]]),
    );
    const section = nodes.find((node) => node.kind === "attention-section");
    const provider = new AttentionTreeProvider({} as PointbreakCli, [
      resolved("/a", "a"),
    ]);

    expect(
      section?.children.map(
        (child) => provider.getTreeItem(child).contextValue,
      ),
    ).toEqual([
      "pointbreak.attention.assessment",
      "pointbreak.attention.assessment",
      "pointbreak.attention.assessment",
    ]);
    provider.dispose();
  });

  it("routes competing heads to resolution and exposes refreshed attention lookup", async () => {
    const competing = attentionItem("heads", "competing_heads");
    const cli = {
      attentionList: vi.fn(async () => attentionItems([competing])),
      revisionList: vi.fn(async () => revisions("rev:sha256:a")),
    } as unknown as PointbreakCli;
    const provider = new AttentionTreeProvider(cli, [resolved("/a", "a")]);

    await provider.refresh();
    const section = provider
      .getChildren()
      .find((node) => node.kind === "attention-section");

    expect(
      section?.children.map(
        (child) => provider.getTreeItem(child).contextValue,
      ),
    ).toEqual(["pointbreak.attention.headResolution"]);
    expect(provider.findAttentionItem("a", competing.id)).toEqual(competing);
    provider.dispose();
  });

  it("counts attention items across targets for the badge", () => {
    const docs = new Map([
      ["a", attentionItems([attentionItem("a", "open_input_request")])],
      [
        "b",
        attentionItems([
          attentionItem("b", "open_input_request"),
          attentionItem("c", "failed_validation"),
        ]),
      ],
    ]);

    expect(attentionCount(docs)).toBe(3);
  });
});

it("refreshes immediately on visibility without owning a timer", async () => {
  vi.useFakeTimers();
  const cli = {
    attentionList: vi.fn(async () => attention("A")),
    revisionList: vi.fn(async () => revisions("rev:sha256:a")),
  } as unknown as PointbreakCli;
  const provider = new AttentionTreeProvider(cli, [resolved("/a", "a")]);

  await vi.advanceTimersByTimeAsync(30_000);
  expect(cli.attentionList).not.toHaveBeenCalled();

  await provider.setVisible(true);
  expect(cli.attentionList).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(30_000);
  expect(cli.attentionList).toHaveBeenCalledTimes(1);

  provider.setVisible(false);
  await vi.advanceTimersByTimeAsync(30_000);
  expect(cli.attentionList).toHaveBeenCalledTimes(1);
  expect(vi.getTimerCount()).toBe(0);
  provider.dispose();
});

it("refreshes one target without replaying another workspace target", async () => {
  const cli = {
    attentionList: vi.fn(async (path: string) => attention(path)),
    revisionList: vi.fn(async (path: string) => revisions(`rev:${path}`)),
  } as unknown as PointbreakCli;
  const provider = new AttentionTreeProvider(cli, [
    resolved("/a", "a"),
    resolved("/b", "b"),
  ]);

  await provider.refreshTarget("b");

  expect(cli.attentionList).toHaveBeenCalledOnce();
  expect(cli.attentionList).toHaveBeenCalledWith("/b");
  expect(cli.revisionList).toHaveBeenCalledWith("/b");
  provider.dispose();
});

it("does not publish a target refresh after its generation is aborted", async () => {
  const pendingAttention = deferred<AttentionListDoc>();
  const pendingRevisions = deferred<RevisionListDoc>();
  const cli = {
    attentionList: vi.fn(() => pendingAttention.promise),
    revisionList: vi.fn(() => pendingRevisions.promise),
  } as unknown as PointbreakCli;
  const provider = new AttentionTreeProvider(cli, [resolved("/a", "a")]);
  const controller = new AbortController();
  const refresh = provider.refreshTarget("a", controller.signal);

  controller.abort();
  pendingAttention.resolve(attention("stale"));
  pendingRevisions.resolve(revisions("rev:stale"));
  await refresh;

  expect(provider.getChildren()).toMatchObject([
    { kind: "attention-section", children: [] },
    { kind: "revisions-section", children: [] },
  ]);
  provider.dispose();
});

function resolved(
  path: string,
  key: string,
  emptyInventory = false,
): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder(path, key) as WorkspaceFolder,
    target: {
      key,
      label: key.toUpperCase(),
      storeIdentity: `store:${key}`,
      contextIdentity: `context:${key}`,
    },
    emptyInventory,
  };
}

function errorResolution(path: string, message: string): TargetResolution {
  return {
    kind: "error",
    folder: workspaceFolder(path) as WorkspaceFolder,
    message,
  };
}

function attention(title: string): AttentionListDoc {
  return attentionItems([attentionItem(title, "open_input_request", title)]);
}

function attentionItems(items: AttentionListDoc["items"]): AttentionListDoc {
  return {
    schema: "pointbreak.attention-list",
    version: 1,
    items,
    diagnostics: [],
  };
}

function attentionItem(
  id: string,
  kind: AttentionItem["kind"],
  title?: string,
): AttentionItem {
  const base = {
    id,
    tier: "primary" as const,
    revisionId: `rev:sha256:${id}`,
    freshness: { state: "current" as const },
    observedAt: "2026-07-15T00:00:00Z",
  };
  switch (kind) {
    case "open_input_request":
      return {
        ...base,
        kind,
        inputRequestId: `input-request:sha256:${id}`,
        mode: "operative",
        reasonCode: "manual_decision_required",
        title: title ?? "Choose",
        trackId: "agent:review",
        openedBy: "actor:agent:reviewer",
      };
    case "follow_up_outstanding":
      return {
        ...base,
        kind,
        assessmentId: `assess:sha256:${id}`,
        trackId: "agent:review",
        recordedBy: "actor:agent:reviewer",
        openInputRequestIds: [`input-request:sha256:${id}`],
      };
    case "ambiguous_assessment":
      return { ...base, kind, assessments: [] };
    case "competing_heads":
      return {
        ...base,
        kind,
        revisionId: undefined,
        headRevisionIds: [`rev:sha256:${id}`],
        threadRevisionCount: 1,
      };
    case "stale_assessment":
      return {
        ...base,
        kind,
        assessmentId: `assess:sha256:${id}`,
        assessment: "accepted",
        trackId: "agent:review",
        recordedBy: "actor:agent:reviewer",
        headRevisionIds: [`rev:sha256:${id}`],
      };
    case "failed_validation":
      return {
        ...base,
        kind,
        validationCheckId: `validation:sha256:${id}`,
        checkName: "test",
        status: "failed",
        trackId: "agent:review",
        recordedBy: "actor:agent:reviewer",
      };
  }
}

function revisions(...revisionIds: string[]): RevisionListDoc {
  return {
    schema: "pointbreak.review-revision-list",
    version: 1,
    entries: revisionIds.map((revisionId) => ({
      revisionId,
      capturedAt: "2026-07-12T00:00:00Z",
      mergeStatus: "unmerged",
    })),
    revisionCount: revisionIds.length,
    eventCount: revisionIds.length,
    eventSetHash: "sha256:test",
    diagnostics: [],
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}
