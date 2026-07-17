import { beforeEach, expect, it, vi } from "vitest";
import type { PointbreakCli } from "../src/cli";
import { runOpenAnnotatedDiffCommand } from "../src/commands/openAnnotatedDiff";
import type { ReviewPanelManager } from "../src/reviewPanel";
import type { ResolvedTargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => ({
  showErrorMessage: vi.fn(),
  showInformationMessage: vi.fn(),
  showQuickPick: vi.fn(),
}));

vi.mock("vscode", () => ({ window: vscodeMocks }));

beforeEach(() => {
  vi.clearAllMocks();
});

it("preserves tree focus and carries the existing attention identity", async () => {
  const open = vi.fn();
  const target = resolution();

  await runOpenAnnotatedDiffCommand(
    {} as PointbreakCli,
    [target],
    { open } as unknown as ReviewPanelManager,
    {
      kind: "attention-item",
      label: "Choose",
      description: "primary",
      targetKey: target.target.key,
      folder: target.folder,
      revisionId: "rev:sha256:one",
      attentionId: "open_input_request:request:sha256:one",
      item: {
        id: "open_input_request:request:sha256:one",
        kind: "open_input_request",
        tier: "primary",
        revisionId: "rev:sha256:one",
        freshness: { state: "current" },
        observedAt: "2026-07-15T00:00:00Z",
        inputRequestId: "request:sha256:one",
        mode: "operative",
        reasonCode: "manual_decision_required",
        title: "Choose",
        trackId: "agent:review",
        openedBy: "actor:agent:reviewer",
      },
      lens: "attention",
      command: "pointbreak.openAnnotatedDiff",
    },
  );

  expect(open).toHaveBeenCalledWith(
    {
      resolution: target,
      revisionId: "rev:sha256:one",
      focus: {
        kind: "attention",
        id: "open_input_request:request:sha256:one",
      },
    },
    { preserveFocus: true },
  );
});

it("lets the explicit command choose a revision and focus the same panel", async () => {
  const open = vi.fn();
  const target = resolution();
  const cli = {
    revisionList: vi.fn(async () => ({
      entries: [
        {
          revisionId: "rev:sha256:chosen",
          mergeStatus: "unmerged",
          capturedAt: "2026-07-13T00:00:00Z",
        },
      ],
    })),
  } as unknown as PointbreakCli;
  vscodeMocks.showQuickPick.mockResolvedValue({
    revisionId: "rev:sha256:chosen",
  });

  await runOpenAnnotatedDiffCommand(
    cli,
    [target],
    { open } as unknown as ReviewPanelManager,
    undefined,
    { pick: vi.fn(async () => target) },
  );

  expect(open).toHaveBeenCalledWith(
    { resolution: target, revisionId: "rev:sha256:chosen" },
    { preserveFocus: false },
  );
});

it("offers every revision newest-first in the explicit picker", async () => {
  const open = vi.fn();
  const target = resolution();
  const entries = Array.from({ length: 25 }, (_, index) => ({
    revisionId: `rev:sha256:${String(index + 1).padStart(2, "0")}`,
    summary: index === 24 ? "Newest readable revision" : undefined,
    mergeStatus: "open",
    capturedAt: `2026-07-${String(index + 1).padStart(2, "0")}T00:00:00Z`,
  }));
  const cli = {
    revisionList: vi.fn(async () => ({ entries })),
  } as unknown as PointbreakCli;
  vscodeMocks.showQuickPick.mockImplementation(
    async (items: readonly { revisionId: string }[]) => items[0],
  );

  await runOpenAnnotatedDiffCommand(
    cli,
    [target],
    { open } as unknown as ReviewPanelManager,
    undefined,
    { pick: vi.fn(async () => target) },
  );

  const items = vscodeMocks.showQuickPick.mock.calls[0][0];
  expect(items).toHaveLength(25);
  expect(items[0]).toMatchObject({
    revisionId: "rev:sha256:25",
    label: "Newest readable revision",
    description: "25 · open",
  });
  expect(open).toHaveBeenCalledWith(
    { resolution: target, revisionId: "rev:sha256:25" },
    { preserveFocus: false },
  );
});

function resolution(): ResolvedTargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder("/private/repo", "repo") as never,
    target: {
      key: "store/context",
      label: "repo",
      storeIdentity: "store",
      contextIdentity: "context",
    },
    emptyInventory: false,
  };
}
