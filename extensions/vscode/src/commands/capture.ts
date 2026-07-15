import { window } from "vscode";
import {
  type CaptureChoice,
  type CaptureDoc,
  type CaptureOptions,
  type PointbreakCli,
  PointbreakCliError,
} from "../cli";
import type {
  HumanWriteContext,
  HumanWriteCoordinator,
  HumanWriteResult,
} from "../humanWriteCoordinator";
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
  humanWrites?: HumanWriteCoordinator;
}

export interface CoordinatedCaptureRequest {
  repo: string;
  resource: unknown;
  options: CaptureOptions;
  confirm(context: HumanWriteContext): Promise<boolean>;
  afterWrite?(document: CaptureDoc): void;
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
const CONFIRM_CAPTURE_ACTION = "Capture";

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
  const humanWrites = dependencies.humanWrites;
  if (!humanWrites) {
    await window.showErrorMessage(
      "Pointbreak could not prepare the human write.",
    );
    return;
  }

  try {
    const capture = (captureOptions: CaptureOptions) =>
      runCoordinatedCapture(cli, humanWrites, {
        repo: resolution.folder.uri.fsPath,
        resource: resolution.folder.uri,
        options: captureOptions,
        confirm: async ({ actorId }) =>
          (await window.showWarningMessage(
            `${captureDescription(captureOptions)} as ${actorId}?`,
            { modal: true },
            CONFIRM_CAPTURE_ACTION,
          )) === CONFIRM_CAPTURE_ACTION,
        afterWrite: () => {
          markTargetPopulated(resolutions, resolution.target.key);
        },
      });
    const result = await captureWithEmptyRetry(capture, options);
    if (!result) {
      return;
    }
    void window.showInformationMessage(
      `Captured revision ${shortRevisionId(result.document.revision.id)}`,
    );
  } catch (error) {
    await window.showErrorMessage(captureErrorMessage(error));
  }
}

export function runCoordinatedCapture(
  cli: PointbreakCli,
  humanWrites: HumanWriteCoordinator,
  request: CoordinatedCaptureRequest,
): Promise<HumanWriteResult<CaptureDoc> | undefined> {
  return humanWrites.run({
    repo: request.repo,
    resource: request.resource,
    confirm: request.confirm,
    write: async () => {
      const document = await cli.capture(request.repo, request.options);
      request.afterWrite?.(document);
      return document;
    },
  });
}

async function captureWithEmptyRetry(
  capture: (
    options: CaptureOptions,
  ) => Promise<HumanWriteResult<CaptureDoc> | undefined>,
  options: CaptureOptions,
) {
  try {
    return await capture(options);
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
    return capture({ ...options, allowEmpty: true });
  }
}

function captureDescription(options: CaptureOptions): string {
  let source = "Capture current work";
  if (options.choice === "staged") {
    source = "Capture staged work";
  } else if (options.choice === "unstaged") {
    source = "Capture unstaged work";
  }
  return options.allowEmpty ? `${source} as an empty revision` : source;
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
