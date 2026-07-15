import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  window: {
    showErrorMessage: vi.fn(),
    showInformationMessage: vi.fn(),
    showInputBox: vi.fn(),
    showQuickPick: vi.fn(),
    showWarningMessage: vi.fn(),
  },
}));

import type { AttentionItemNode } from "../src/attentionView";
import type { InputRequestOutcome, PointbreakCli } from "../src/cli";
import { runRespondInputRequestCommand } from "../src/commands/respondInputRequest";
import { HumanWriteCoordinator } from "../src/humanWriteCoordinator";
import { workspaceFolder } from "./helpers/vscodeMock";

describe("runRespondInputRequestCommand", () => {
  it("fails closed when invoked without an Attention row", async () => {
    const harness = commandHarness([]);

    await runRespondInputRequestCommand(
      harness.cli,
      undefined,
      harness.dependencies,
    );

    expect(harness.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/matching.*attention row/i),
    );
    expect(harness.identityWhoami).not.toHaveBeenCalled();
  });

  it("responds to one exact request after showing its context and actor", async () => {
    const harness = commandHarness([
      "actor:git-email:human@example.com",
      "actor:git-email:human@example.com",
    ]);
    harness.pickOutcome.mockResolvedValue("rejected");
    harness.promptReason.mockResolvedValue("Use the narrower release.");

    await runRespondInputRequestCommand(
      harness.cli,
      openRequestNode(),
      harness.dependencies,
    );

    expect(harness.pickOutcome).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "open_input_request",
        title: "Choose the release boundary",
        mode: "operative",
        reasonCode: "manual_decision_required",
        trackId: "agent:review",
        openedBy: "actor:agent:reviewer",
      }),
    );
    expect(harness.confirmResponse).toHaveBeenCalledWith({
      actorId: "actor:git-email:human@example.com",
      track: "agent:review",
      inputRequestId: "input-request:sha256:open",
      outcome: "rejected",
      reason: "Use the narrower release.",
    });
    expect(harness.respondInputRequest).toHaveBeenCalledWith("/repo", {
      inputRequestId: "input-request:sha256:open",
      outcome: "rejected",
      reason: "Use the narrower release.",
    });
    expect(harness.identityWhoami).toHaveBeenCalledTimes(2);
    expect(harness.refresh).toHaveBeenCalledOnce();
  });

  it("processes selected follow-up requests sequentially with fresh confirmation", async () => {
    const harness = commandHarness(Array(4).fill("actor:human:kevin"));
    harness.pickRequestIds.mockResolvedValue([
      "input-request:sha256:one",
      "input-request:sha256:two",
    ]);
    harness.pickOutcome.mockResolvedValue("approved");
    harness.promptReason.mockResolvedValue("");

    await runRespondInputRequestCommand(
      harness.cli,
      followUpNode(),
      harness.dependencies,
    );

    expect(harness.respondInputRequest.mock.calls).toEqual([
      [
        "/repo",
        {
          inputRequestId: "input-request:sha256:one",
          outcome: "approved",
          reason: undefined,
        },
      ],
      [
        "/repo",
        {
          inputRequestId: "input-request:sha256:two",
          outcome: "approved",
          reason: undefined,
        },
      ],
    ]);
    expect(harness.identityWhoami).toHaveBeenCalledTimes(4);
    expect(harness.confirmResponse).toHaveBeenCalledTimes(2);
    expect(harness.refresh).toHaveBeenCalledTimes(2);
  });

  it("stops visibly when identity changes between follow-up entries", async () => {
    const harness = commandHarness([
      "actor:human:first",
      "actor:human:first",
      "actor:human:second",
    ]);
    harness.pickRequestIds.mockResolvedValue([
      "input-request:sha256:one",
      "input-request:sha256:two",
    ]);

    await runRespondInputRequestCommand(
      harness.cli,
      followUpNode(),
      harness.dependencies,
    );

    expect(harness.respondInputRequest).toHaveBeenCalledOnce();
    expect(harness.confirmResponse).toHaveBeenCalledOnce();
    expect(harness.showWarningMessage).toHaveBeenCalledWith(
      expect.stringMatching(/identity changed/i),
    );
    expect(harness.refresh).toHaveBeenCalledOnce();
  });

  it("cancels without responding or refreshing", async () => {
    const harness = commandHarness(Array(2).fill("actor:human:kevin"));
    harness.confirmResponse.mockResolvedValue(false);

    await runRespondInputRequestCommand(
      harness.cli,
      openRequestNode(),
      harness.dependencies,
    );

    expect(harness.respondInputRequest).not.toHaveBeenCalled();
    expect(harness.refresh).not.toHaveBeenCalled();
  });

  it("stops after a partial failure without attempting later requests", async () => {
    const harness = commandHarness(Array(6).fill("actor:human:kevin"));
    harness.pickRequestIds.mockResolvedValue([
      "input-request:sha256:one",
      "input-request:sha256:two",
      "input-request:sha256:three",
    ]);
    harness.respondInputRequest
      .mockResolvedValueOnce(responseDocument("one"))
      .mockRejectedValueOnce(new Error("request already responded"));

    await runRespondInputRequestCommand(
      harness.cli,
      followUpNode(),
      harness.dependencies,
    );

    expect(harness.respondInputRequest).toHaveBeenCalledTimes(2);
    expect(harness.refresh).toHaveBeenCalledOnce();
    expect(harness.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/input-request:sha256:two/),
    );
  });
});

function commandHarness(actorIds: string[]) {
  const identityWhoami = vi.fn(async () => {
    const actorId = actorIds.shift();
    if (!actorId) throw new Error("unexpected whoami call");
    return {
      schema: "pointbreak.identity-whoami" as const,
      version: 1 as const,
      actorId,
      diagnostics: [],
    };
  });
  const respondInputRequest = vi.fn(async (_repo, options) =>
    responseDocument(options.inputRequestId),
  );
  const cli = {
    identityWhoami,
    respondInputRequest,
  } as unknown as PointbreakCli;
  const refresh = vi.fn(async () => undefined);
  const showWarningMessage = vi.fn(async () => undefined);
  const humanWrites = new HumanWriteCoordinator(cli, {
    resolveTrack: vi.fn(() => "human:local"),
    showDiagnostic: showWarningMessage,
    refresh,
    showRefreshError: showWarningMessage,
  });
  const pickRequestIds = vi.fn(async () => undefined as string[] | undefined);
  const pickOutcome = vi.fn(
    async () => "approved" as InputRequestOutcome | undefined,
  );
  const promptReason = vi.fn(async () => "" as string | undefined);
  const confirmResponse = vi.fn(async () => true);
  const showErrorMessage = vi.fn(async () => undefined);
  return {
    cli,
    identityWhoami,
    respondInputRequest,
    refresh,
    pickRequestIds,
    pickOutcome,
    promptReason,
    confirmResponse,
    showWarningMessage,
    showErrorMessage,
    dependencies: {
      humanWrites,
      pickRequestIds,
      pickOutcome,
      promptReason,
      confirmResponse,
      showInformationMessage: vi.fn(async () => undefined),
      showWarningMessage,
      showErrorMessage,
    },
  };
}

function openRequestNode(): AttentionItemNode {
  return {
    kind: "attention-item",
    label: "Choose the release boundary",
    targetKey: "repo",
    folder: workspaceFolder("/repo") as never,
    description: "primary",
    revisionId: "rev:sha256:one",
    attentionId: "open_input_request:input-request:sha256:open",
    lens: "attention",
    command: "pointbreak.openAnnotatedDiff",
    item: {
      id: "open_input_request:input-request:sha256:open",
      kind: "open_input_request",
      tier: "primary",
      revisionId: "rev:sha256:one",
      freshness: { state: "current" },
      observedAt: "2026-07-15T00:00:00Z",
      inputRequestId: "input-request:sha256:open",
      mode: "operative",
      reasonCode: "manual_decision_required",
      title: "Choose the release boundary",
      trackId: "agent:review",
      openedBy: "actor:agent:reviewer",
    },
  };
}

function followUpNode(): AttentionItemNode {
  return {
    ...openRequestNode(),
    label: "Follow up",
    attentionId: "follow_up_outstanding:assess:sha256:follow",
    item: {
      id: "follow_up_outstanding:assess:sha256:follow",
      kind: "follow_up_outstanding",
      tier: "primary",
      revisionId: "rev:sha256:one",
      freshness: { state: "current" },
      observedAt: "2026-07-15T00:00:00Z",
      assessmentId: "assess:sha256:follow",
      trackId: "agent:review",
      recordedBy: "actor:agent:reviewer",
      openInputRequestIds: [
        "input-request:sha256:one",
        "input-request:sha256:two",
      ],
    },
  };
}

function responseDocument(inputRequestId: string) {
  return {
    schema: "pointbreak.review-input-request-respond" as const,
    version: 1 as const,
    inputRequestId,
    inputRequestResponseId: `input-request-response:${inputRequestId}`,
    eventId: `evt:${inputRequestId}`,
    outcome: "approved",
    diagnostics: [],
  };
}
