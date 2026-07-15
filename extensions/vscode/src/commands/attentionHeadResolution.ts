import { window } from "vscode";
import type { AttentionItemNode } from "../attentionView";
import {
  type AttentionItem,
  type PointbreakCli,
  PointbreakCliError,
} from "../cli";
import type {
  HumanWriteContext,
  HumanWriteCoordinator,
} from "../humanWriteCoordinator";
import type { TargetResolution } from "../targetResolver";
import { runCoordinatedCapture } from "./capture";

interface ResolutionConfirmation extends HumanWriteContext {
  targetLabel: string;
  headRevisionIds: string[];
}

interface AttentionHeadResolutionDependencies {
  humanWrites?: HumanWriteCoordinator;
  confirmResolution(context: ResolutionConfirmation): Promise<boolean>;
  findAttentionItem(
    targetKey: string,
    attentionId: string,
  ): AttentionItem | undefined;
  routeAssessment(node: AttentionItemNode): Promise<unknown>;
  showInformationMessage(message: string): Promise<unknown>;
  showWarningMessage(message: string): Promise<unknown>;
  showErrorMessage(message: string): Promise<unknown>;
}

const CONFIRM_RESOLUTION_ACTION = "Capture resolution";

export async function runAttentionHeadResolutionCommand(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  node: AttentionItemNode | undefined,
  overrides: Partial<AttentionHeadResolutionDependencies> = {},
): Promise<void> {
  const dependencies = { ...defaultDependencies(), ...overrides };
  if (!node) {
    await dependencies.showErrorMessage(
      "Use this command from a matching Pointbreak Attention row.",
    );
    return;
  }
  const heads = resolutionHeads(node.item);
  if (!heads?.length) {
    await dependencies.showErrorMessage(
      "This Pointbreak attention item has no complete head set to resolve.",
    );
    return;
  }
  const resolution = resolutions.find(
    (candidate) =>
      candidate.kind === "resolved" && candidate.target.key === node.targetKey,
  );
  if (resolution?.kind !== "resolved") {
    await dependencies.showErrorMessage(
      "Pointbreak could not resolve the target for these competing heads.",
    );
    return;
  }
  const humanWrites = dependencies.humanWrites;
  if (!humanWrites) {
    await dependencies.showErrorMessage(
      "Pointbreak could not prepare the human write.",
    );
    return;
  }

  try {
    const result = await runCoordinatedCapture(cli, humanWrites, {
      repo: resolution.folder.uri.fsPath,
      resource: resolution.folder.uri,
      options: {
        choice: "worktree",
        includeUntracked: false,
        allowEmpty: false,
        supersedes: heads,
      },
      confirm: (context) =>
        dependencies.confirmResolution({
          ...context,
          targetLabel: resolution.target.label,
          headRevisionIds: heads,
        }),
    });
    if (!result) return;

    if (!result.refreshed) {
      await dependencies.showWarningMessage(
        "Pointbreak recorded the resolution, but refresh failed, so it could not confirm whether the attention item cleared.",
      );
      return;
    }

    const refreshed = dependencies.findAttentionItem(
      node.targetKey,
      node.attentionId,
    );
    if (node.item.kind === "competing_heads" && refreshed) {
      await dependencies.showWarningMessage(
        "Pointbreak recorded the capture, but there are still competing heads after refresh.",
      );
      return;
    }
    if (node.item.kind === "stale_assessment") {
      if (
        refreshed?.kind === "stale_assessment" &&
        refreshed.headRevisionIds?.length === 1
      ) {
        await dependencies.routeAssessment({
          ...node,
          item: refreshed,
          revisionId: refreshed.revisionId,
        });
        return;
      }
      await dependencies.showWarningMessage(
        "Pointbreak recorded the resolution, but could not route the stale assessment to one exact current head.",
      );
      return;
    }
    await dependencies.showInformationMessage(
      `Captured resolution ${result.document.revision.id}; competing-head attention cleared.`,
    );
  } catch (error) {
    await dependencies.showErrorMessage(resolutionErrorMessage(error));
  }
}

function resolutionHeads(item: AttentionItem): string[] | undefined {
  if (item.kind !== "competing_heads" && item.kind !== "stale_assessment") {
    return undefined;
  }
  return [...new Set(item.headRevisionIds ?? [])];
}

function defaultDependencies(): AttentionHeadResolutionDependencies {
  return {
    confirmResolution: async ({ actorId, targetLabel, headRevisionIds }) =>
      (await window.showWarningMessage(
        `Capture current work for ${targetLabel} as ${actorId}, superseding ${headRevisionIds.join(", ")}? Only a genuinely new content state can resolve this complete head set.`,
        { modal: true },
        CONFIRM_RESOLUTION_ACTION,
      )) === CONFIRM_RESOLUTION_ACTION,
    findAttentionItem: () => undefined,
    routeAssessment: async () => undefined,
    showInformationMessage: async (message) =>
      window.showInformationMessage(message),
    showWarningMessage: async (message) => window.showWarningMessage(message),
    showErrorMessage: async (message) => window.showErrorMessage(message),
  };
}

function resolutionErrorMessage(error: unknown): string {
  if (
    error instanceof PointbreakCliError &&
    error.stderr.includes("capture proposal for revision") &&
    error.stderr.includes("genuinely new content state")
  ) {
    return "Pointbreak already captured this content with different resolution metadata. Edit the worktree to create a genuinely new content state, then retry the complete head resolution.";
  }
  if (error instanceof PointbreakCliError && error.stderr.trim()) {
    return `Pointbreak could not capture the head resolution: ${error.stderr.trim()}`;
  }
  const detail = error instanceof Error ? error.message : String(error);
  return `Pointbreak could not capture the head resolution: ${detail}`;
}
