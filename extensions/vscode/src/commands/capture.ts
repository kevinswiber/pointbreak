import { window } from "vscode";
import { refreshAfterWrite } from "../attentionView";
import {
  type CaptureChoice,
  type CaptureOptions,
  type PointbreakCli,
  PointbreakCliError,
} from "../cli";
import { pickFolder, type TargetResolution } from "../targetResolver";

interface CapturePick {
  label: string;
  description: string;
  choice: CaptureChoice;
}

interface UntrackedPick {
  label: string;
  includeUntracked: boolean;
}

interface CaptureDependencies {
  pick?: typeof pickFolder;
  refresh?: typeof refreshAfterWrite;
}

const CAPTURE_CHOICES: CapturePick[] = [
  {
    label: "My current work",
    description: "Tracked working-tree changes",
    choice: "worktree",
  },
  {
    label: "Staged only",
    description: "Changes currently staged in Git",
    choice: "staged",
  },
  {
    label: "Unstaged only",
    description: "Tracked changes not staged in Git",
    choice: "unstaged",
  },
];

const UNTRACKED_CHOICES: UntrackedPick[] = [
  { label: "Tracked files only", includeUntracked: false },
  { label: "Include untracked files", includeUntracked: true },
];
const EMPTY_CAPTURE_ACTION = "Capture empty revision";

export async function runCaptureCommand(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  dependencies: CaptureDependencies = {},
): Promise<void> {
  const resolution = await (dependencies.pick ?? pickFolder)(resolutions);
  if (!resolution) {
    return;
  }

  const choice = await window.showQuickPick(CAPTURE_CHOICES, {
    placeHolder: "What should Pointbreak capture?",
  });
  if (!choice) {
    return;
  }

  let includeUntracked = false;
  if (choice.choice !== "staged") {
    const untracked = await window.showQuickPick(UNTRACKED_CHOICES, {
      placeHolder: "Include untracked files?",
    });
    if (!untracked) {
      return;
    }
    includeUntracked = untracked.includeUntracked;
  }

  const options: CaptureOptions = {
    choice: choice.choice,
    includeUntracked,
    allowEmpty: false,
  };

  try {
    const result = await captureWithEmptyRetry(
      cli,
      resolution.folder.uri.fsPath,
      options,
    );
    if (!result) {
      return;
    }
    markTargetPopulated(resolutions, resolution.target.key);
    await window.showInformationMessage(
      `Captured revision ${shortRevisionId(result.revision.id)}`,
    );
    await (dependencies.refresh ?? refreshAfterWrite)();
  } catch (error) {
    await window.showErrorMessage(captureErrorMessage(error));
  }
}

async function captureWithEmptyRetry(
  cli: PointbreakCli,
  repo: string,
  options: CaptureOptions,
) {
  try {
    return await cli.capture(repo, options);
  } catch (error) {
    if (!isZeroChangedFilesError(error)) {
      throw error;
    }
    const retry = await window.showInformationMessage(
      "Capture an empty revision?",
      EMPTY_CAPTURE_ACTION,
    );
    if (retry !== EMPTY_CAPTURE_ACTION) {
      return undefined;
    }
    return cli.capture(repo, { ...options, allowEmpty: true });
  }
}

function isZeroChangedFilesError(error: unknown): boolean {
  return (
    error instanceof PointbreakCliError &&
    error.stderr.includes("capture produced no changed files")
  );
}

function captureErrorMessage(error: unknown): string {
  if (error instanceof PointbreakCliError && error.stderr.trim()) {
    return `Pointbreak could not capture this work: ${error.stderr.trim()}`;
  }
  const detail = error instanceof Error ? error.message : String(error);
  return `Pointbreak could not capture this work: ${detail}`;
}

function shortRevisionId(revisionId: string): string {
  return revisionId.split(":").at(-1)?.slice(0, 12) ?? revisionId;
}

function markTargetPopulated(
  resolutions: TargetResolution[],
  targetKey: string,
): void {
  for (const resolution of resolutions) {
    if (resolution.kind === "resolved" && resolution.target.key === targetKey) {
      resolution.emptyInventory = false;
    }
  }
}
