import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import {
  AttentionTreeProvider,
  attentionCount,
  deriveTree,
} from "../src/attentionView";
import type {
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
            attentionItem("with-title", "needs judgment", "Review this"),
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
  });

  it("counts attention items across targets for the badge", () => {
    const docs = new Map([
      ["a", attentionItems([attentionItem("a", "first")])],
      [
        "b",
        attentionItems([
          attentionItem("b", "second"),
          attentionItem("c", "third"),
        ]),
      ],
    ]);

    expect(attentionCount(docs)).toBe(3);
  });
});

it("polls only while the view is visible", async () => {
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
  expect(cli.attentionList).toHaveBeenCalledTimes(3);

  provider.setVisible(false);
  await vi.advanceTimersByTimeAsync(30_000);
  expect(cli.attentionList).toHaveBeenCalledTimes(3);
  provider.dispose();
});

it("does not install a stale poller across rapid visibility changes", async () => {
  vi.useFakeTimers();
  const provider = new AttentionTreeProvider({} as PointbreakCli, [
    resolved("/a", "a"),
  ]);
  const firstRefresh = deferred<void>();
  const secondRefresh = deferred<void>();
  const refresh = vi
    .spyOn(provider, "refresh")
    .mockImplementationOnce(() => firstRefresh.promise)
    .mockImplementationOnce(() => secondRefresh.promise);

  const firstVisible = provider.setVisible(true);
  await provider.setVisible(false);
  const secondVisible = provider.setVisible(true);

  firstRefresh.resolve();
  await firstVisible;
  expect(vi.getTimerCount()).toBe(0);

  secondRefresh.resolve();
  await secondVisible;
  expect(refresh).toHaveBeenCalledTimes(2);
  expect(vi.getTimerCount()).toBe(1);

  provider.dispose();
  expect(vi.getTimerCount()).toBe(0);
});

function resolved(
  path: string,
  key: string,
  emptyInventory = false,
): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder(path, key) as WorkspaceFolder,
    target: { key, label: key.toUpperCase() },
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
  return attentionItems([attentionItem(title, "needs_attention", title)]);
}

function attentionItems(items: AttentionListDoc["items"]): AttentionListDoc {
  return {
    schema: "pointbreak.attention-list",
    version: 1,
    items,
    diagnostics: [],
  };
}

function attentionItem(id: string, kind: string, title?: string) {
  return {
    id,
    tier: "primary",
    kind,
    title,
    revisionId: `rev:sha256:${id}`,
  };
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
