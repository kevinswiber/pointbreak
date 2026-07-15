import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  window: {
    showErrorMessage: vi.fn(),
    showInformationMessage: vi.fn(),
    showWarningMessage: vi.fn(),
  },
}));

import type { AttentionItemNode } from "../src/attentionView";
import {
  type AttentionItem,
  type PointbreakCli,
  PointbreakCliError,
  type StaleAssessmentAttentionItem,
} from "../src/cli";
import { runAttentionHeadResolutionCommand } from "../src/commands/attentionHeadResolution";
import { HumanWriteCoordinator } from "../src/humanWriteCoordinator";
import type { TargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

describe("runAttentionHeadResolutionCommand", () => {
  it("fails closed when invoked without an Attention row", async () => {
    const harness = commandHarness();

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      undefined,
      harness.dependencies,
    );

    expect(harness.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/matching.*attention row/i),
    );
    expect(harness.capture).not.toHaveBeenCalled();
  });

  it("confirms and captures every displayed head exactly once", async () => {
    const harness = commandHarness();
    const node = competingNode([
      "rev:sha256:two",
      "rev:sha256:one",
      "rev:sha256:two",
    ]);

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      node,
      harness.dependencies,
    );

    expect(harness.confirmResolution).toHaveBeenCalledWith({
      actorId: "actor:human:kevin",
      track: "human:local",
      targetLabel: "Repo",
      headRevisionIds: ["rev:sha256:two", "rev:sha256:one"],
    });
    expect(harness.capture).toHaveBeenCalledWith("/repo", {
      choice: "worktree",
      includeUntracked: false,
      allowEmpty: false,
      supersedes: ["rev:sha256:two", "rev:sha256:one"],
    });
    expect(harness.refresh).toHaveBeenCalledOnce();
  });

  it("cancels without capturing or refreshing", async () => {
    const harness = commandHarness();
    harness.confirmResolution.mockResolvedValue(false);

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      competingNode(),
      harness.dependencies,
    );

    expect(harness.capture).not.toHaveBeenCalled();
    expect(harness.refresh).not.toHaveBeenCalled();
  });

  it.each([
    "rev:sha256:displayed",
    "rev:sha256:older-non-head",
  ])("turns an existing-content proposal conflict for %s into edit-then-retry guidance", async (revisionId) => {
    const harness = commandHarness();
    harness.capture.mockRejectedValueOnce(
      new PointbreakCliError(
        "capture failed",
        1,
        `capture proposal for revision ${revisionId} conflicts with the proposal already visible to this writer; create a genuinely new content state before retrying with different capture metadata`,
      ),
    );

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      competingNode(),
      harness.dependencies,
    );

    expect(harness.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/edit.*genuinely new content.*retry/i),
    );
    expect(harness.refresh).not.toHaveBeenCalled();
  });

  it.each([
    ["exact-payload retry", "rev:sha256:existing"],
    ["genuinely new content", "rev:sha256:new"],
  ])("accepts an idempotent %s and refreshes", async (_case, revisionId) => {
    const harness = commandHarness();
    harness.capture.mockResolvedValueOnce(captureDocument(revisionId));

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      competingNode(),
      harness.dependencies,
    );

    expect(harness.refresh).toHaveBeenCalledOnce();
    expect(harness.showInformationMessage).toHaveBeenCalledWith(
      expect.stringContaining(revisionId),
    );
  });

  it("reports the exact residual competing-head item after authoritative refresh", async () => {
    const harness = commandHarness();
    const original = competingNode();
    harness.findAttentionItem.mockReturnValue(original.item);

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      original,
      harness.dependencies,
    );

    expect(harness.findAttentionItem).toHaveBeenCalledWith(
      "repo",
      original.attentionId,
    );
    expect(harness.showWarningMessage).toHaveBeenCalledWith(
      expect.stringMatching(/still.*competing head/i),
    );
    expect(harness.showInformationMessage).not.toHaveBeenCalled();
  });

  it("routes a stale multi-successor assessment to its refreshed exact successor", async () => {
    const harness = commandHarness();
    const original = staleNode(
      ["rev:sha256:one", "rev:sha256:two"],
      ["rev:sha256:intermediate"],
    );
    const refreshed = staleNode(["rev:sha256:new"]);
    harness.findAttentionItem.mockReturnValue(refreshed.item);

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      original,
      harness.dependencies,
    );

    expect(harness.capture).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({
        supersedes: ["rev:sha256:one", "rev:sha256:two"],
      }),
    );
    expect(harness.routeAssessment).toHaveBeenCalledWith(
      expect.objectContaining({
        attentionId: original.attentionId,
        item: refreshed.item,
      }),
    );
  });

  it("does not claim a clearing result when the post-write refresh fails", async () => {
    const harness = commandHarness();
    harness.refresh.mockRejectedValueOnce(new Error("refresh failed"));

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      competingNode(),
      harness.dependencies,
    );

    expect(harness.findAttentionItem).not.toHaveBeenCalled();
    expect(harness.showInformationMessage).not.toHaveBeenCalled();
    expect(harness.showWarningMessage).toHaveBeenCalledWith(
      expect.stringMatching(/recorded.*refresh.*could not confirm/i),
    );
  });

  it("fails closed when an older attention v1 item has no complete head set", async () => {
    const harness = commandHarness();

    await runAttentionHeadResolutionCommand(
      harness.cli,
      [resolved()],
      legacyStaleNode(),
      harness.dependencies,
    );

    expect(harness.capture).not.toHaveBeenCalled();
    expect(harness.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/no complete head set/i),
    );
  });
});

function commandHarness() {
  const identityWhoami = vi.fn(async () => ({
    schema: "pointbreak.identity-whoami" as const,
    version: 1 as const,
    actorId: "actor:human:kevin",
    diagnostics: [],
  }));
  const capture = vi.fn(async () => captureDocument("rev:sha256:new"));
  const cli = { identityWhoami, capture } as unknown as PointbreakCli;
  const refresh = vi.fn(async () => undefined);
  const showWarningMessage = vi.fn(async () => undefined);
  const humanWrites = new HumanWriteCoordinator(cli, {
    resolveTrack: vi.fn(() => "human:local"),
    showDiagnostic: showWarningMessage,
    refresh,
    showRefreshError: showWarningMessage,
  });
  const confirmResolution = vi.fn(async () => true);
  const findAttentionItem = vi.fn(() => undefined as AttentionItem | undefined);
  const routeAssessment = vi.fn(async () => undefined);
  const showInformationMessage = vi.fn(async () => undefined);
  const showErrorMessage = vi.fn(async () => undefined);
  return {
    cli,
    capture,
    refresh,
    confirmResolution,
    findAttentionItem,
    routeAssessment,
    showWarningMessage,
    showInformationMessage,
    showErrorMessage,
    dependencies: {
      humanWrites,
      confirmResolution,
      findAttentionItem,
      routeAssessment,
      showWarningMessage,
      showInformationMessage,
      showErrorMessage,
    },
  };
}

function resolved(): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder("/repo", "Repo") as never,
    target: {
      key: "repo",
      label: "Repo",
      storeIdentity: "store:repo",
      contextIdentity: "context:repo",
    },
    emptyInventory: false,
  };
}

function competingNode(
  headRevisionIds = ["rev:sha256:one", "rev:sha256:two"],
): AttentionItemNode {
  return node({
    id: "competing_heads:thread:sha256:one",
    kind: "competing_heads",
    tier: "primary",
    freshness: { state: "current" },
    observedAt: "2026-07-15T00:00:00Z",
    headRevisionIds,
    threadRevisionCount: headRevisionIds.length,
  });
}

function staleNode(
  headRevisionIds: string[],
  supersededBy = headRevisionIds,
): AttentionItemNode {
  return node({
    id: "stale_assessment:assess:sha256:stale",
    kind: "stale_assessment",
    tier: "primary",
    revisionId: "rev:sha256:stale",
    freshness: { state: "superseded", supersededBy },
    observedAt: "2026-07-15T00:00:00Z",
    assessmentId: "assess:sha256:stale",
    assessment: "accepted",
    trackId: "agent:review",
    recordedBy: "actor:agent:reviewer",
    headRevisionIds,
  });
}

function legacyStaleNode(): AttentionItemNode {
  const item: StaleAssessmentAttentionItem = {
    id: "stale_assessment:assess:sha256:legacy",
    kind: "stale_assessment",
    tier: "primary",
    revisionId: "rev:sha256:legacy",
    freshness: {
      state: "superseded",
      supersededBy: ["rev:sha256:intermediate"],
    },
    observedAt: "2026-07-15T00:00:00Z",
    assessmentId: "assess:sha256:legacy",
    assessment: "accepted",
    trackId: "agent:review",
    recordedBy: "actor:agent:reviewer",
  };
  return node(item);
}

function node(item: AttentionItemNode["item"]): AttentionItemNode {
  return {
    kind: "attention-item",
    label: item.kind,
    targetKey: "repo",
    folder: workspaceFolder("/repo", "Repo") as never,
    description: item.tier,
    revisionId: item.revisionId,
    attentionId: item.id,
    item,
    lens: "attention",
    command: item.revisionId ? "pointbreak.openAnnotatedDiff" : undefined,
  };
}

function captureDocument(revisionId: string) {
  return {
    schema: "pointbreak.review-capture" as const,
    version: 1 as const,
    revision: { id: revisionId },
    diagnostics: [],
  };
}
