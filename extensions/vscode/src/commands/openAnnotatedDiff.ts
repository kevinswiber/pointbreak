import { window } from "vscode";
import type { AttentionItemNode, RevisionItemNode } from "../attentionView";
import type { PointbreakCli } from "../cli";
import { revisionDiscoveryDisplay } from "../idDisplay";
import type { ReviewPanelManager } from "../reviewPanel";
import { newestRevisionEntries } from "../revisionOrder";
import {
  pickFolder,
  type ResolvedTargetResolution,
  type TargetResolution,
} from "../targetResolver";

type AnnotatedDiffNode = AttentionItemNode | RevisionItemNode;

interface OpenAnnotatedDiffDependencies {
  readonly pick?: typeof pickFolder;
}

export async function runOpenAnnotatedDiffCommand(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  panel: ReviewPanelManager,
  node?: AnnotatedDiffNode,
  dependencies: OpenAnnotatedDiffDependencies = {},
): Promise<void> {
  const selection = node
    ? await locationFromNode(resolutions, node)
    : await pickLocation(cli, resolutions, dependencies.pick ?? pickFolder);
  if (!selection) return;

  await panel.open(
    {
      resolution: selection.resolution,
      revisionId: selection.revisionId,
      focus:
        selection.attentionId === undefined
          ? undefined
          : { kind: "attention", id: selection.attentionId },
    },
    { preserveFocus: !!node },
  );
}

interface Selection {
  readonly resolution: ResolvedTargetResolution;
  readonly revisionId: string;
  readonly attentionId?: string;
}

async function locationFromNode(
  resolutions: TargetResolution[],
  node: AnnotatedDiffNode,
): Promise<Selection | undefined> {
  const resolution = resolutions.find(
    (candidate): candidate is ResolvedTargetResolution =>
      candidate.kind === "resolved" && candidate.target.key === node.targetKey,
  );
  if (!resolution) {
    await window.showErrorMessage(
      "Pointbreak could not identify this review target. Refresh the extension and try again.",
    );
    return undefined;
  }
  if (!node.revisionId) {
    await window.showInformationMessage(
      "This attention item does not identify one revision to open.",
    );
    return undefined;
  }
  return {
    resolution,
    revisionId: node.revisionId,
    attentionId: node.kind === "attention-item" ? node.attentionId : undefined,
  };
}

async function pickLocation(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  pick: typeof pickFolder,
): Promise<Selection | undefined> {
  const resolution = await pick(resolutions);
  if (!resolution) return undefined;
  try {
    const revisions = await cli.revisionList(resolution.folder.uri.fsPath);
    const items = newestRevisionEntries(revisions.entries).map((entry) => ({
      ...revisionDiscoveryDisplay(entry),
      detail: entry.capturedAt,
      revisionId: entry.revisionId,
    }));
    if (!items.length) {
      await window.showInformationMessage(
        "Pointbreak has no captured revisions in this target yet.",
      );
      return undefined;
    }
    const chosen = await window.showQuickPick(items, {
      placeHolder: "Choose a revision to open as an annotated diff",
    });
    return chosen ? { resolution, revisionId: chosen.revisionId } : undefined;
  } catch {
    await window.showErrorMessage(
      "Pointbreak could not list revisions for this review target.",
    );
    return undefined;
  }
}
