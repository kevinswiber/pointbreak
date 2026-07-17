import { ProgressLocation, type QuickPickItem, window } from "vscode";
import type { AttentionItemNode } from "../attentionView";
import type {
  AttentionItem,
  PointbreakCli,
  RevisionListDoc,
  ValidationStatus,
} from "../cli";
import type {
  HumanWriteContext,
  HumanWriteCoordinator,
} from "../humanWriteCoordinator";
import { revisionDiscoveryDisplay } from "../idDisplay";
import { newestRevisionEntries } from "../revisionOrder";
import {
  pickFolder,
  type ResolvedTargetResolution,
  type TargetResolution,
} from "../targetResolver";
import {
  pickRootTask,
  type TaskCandidate,
  type TaskExecutionSummary,
  type TaskRunner,
} from "../taskRunner";

const RECORD_ACTION = "Record validation";
const STATUSES: readonly ValidationStatus[] = [
  "passed",
  "failed",
  "errored",
  "skipped",
];
type RevisionListEntry = RevisionListDoc["entries"][number];

export interface ValidationStatusPrompt {
  readonly proposedStatus: ValidationStatus;
  readonly taskLabel: string;
  readonly checkName: string;
  readonly execution: TaskExecutionSummary;
}

export interface ValidationWritePreview extends HumanWriteContext {
  readonly revisionId: string;
  readonly taskLabel: string;
  readonly checkName: string;
  readonly execution: TaskExecutionSummary;
  readonly proposedStatus: ValidationStatus;
  readonly status: ValidationStatus;
  readonly startedAt: string;
  readonly completedAt: string;
}

export interface RunTaskAndRecordValidationDependencies {
  readonly humanWrites?: HumanWriteCoordinator;
  readonly taskRunner?: Pick<TaskRunner, "run">;
  readonly pickFolder: typeof pickFolder;
  readonly pickRevision: (
    entries: readonly RevisionListEntry[],
  ) => Promise<RevisionListEntry | undefined>;
  readonly pickTask: (
    folder: ResolvedTargetResolution["folder"],
    requiredCheckName?: string,
  ) => Promise<TaskCandidate | undefined>;
  readonly pickStatus: (
    prompt: ValidationStatusPrompt,
  ) => Promise<ValidationStatus | undefined>;
  readonly now: () => string;
  readonly confirmWrite: (preview: ValidationWritePreview) => Promise<boolean>;
  readonly findAttentionItem: (
    targetKey: string,
    attentionId: string,
  ) => AttentionItem | undefined;
  readonly showInformationMessage: (message: string) => Promise<unknown>;
  readonly showErrorMessage: (message: string) => Promise<unknown>;
}

export async function runTaskAndRecordValidationCommand(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  node?: AttentionItemNode,
  overrides: Partial<RunTaskAndRecordValidationDependencies> = {},
): Promise<void> {
  const dependencies = { ...defaultDependencies(), ...overrides };
  const location = await resolveLocation(cli, resolutions, node, dependencies);
  if (!location) return;

  const candidate = await dependencies.pickTask(
    location.resolution.folder,
    location.requiredCheckName,
  );
  if (!candidate) return;

  const taskRunner = dependencies.taskRunner;
  if (!taskRunner) {
    await dependencies.showErrorMessage(
      "Pointbreak could not prepare the VS Code task runner.",
    );
    return;
  }
  const startedAt = dependencies.now();
  let execution: TaskExecutionSummary;
  try {
    execution = await runTaskWithCancellation(
      taskRunner,
      candidate,
      taskLabelFor(candidate),
    );
  } catch (error) {
    await dependencies.showErrorMessage(
      `Pointbreak could not run the selected VS Code task: ${errorMessage(error)}`,
    );
    return;
  }
  const completedAt = dependencies.now();
  const taskLabel = taskLabelFor(candidate);
  const proposedStatus = proposeValidationStatus(execution);
  const status = await dependencies.pickStatus({
    proposedStatus,
    taskLabel,
    checkName: candidate.checkName,
    execution,
  });
  if (!status) return;

  const humanWrites = dependencies.humanWrites;
  if (!humanWrites) {
    await dependencies.showErrorMessage(
      "Pointbreak could not prepare the human write.",
    );
    return;
  }

  const repo = location.resolution.folder.uri.fsPath;
  try {
    const result = await humanWrites.run({
      repo,
      resource: location.resolution.folder.uri,
      trackOverride: location.trackOverride,
      confirm: (context) =>
        dependencies.confirmWrite({
          ...context,
          revisionId: location.revisionId,
          taskLabel,
          checkName: candidate.checkName,
          execution,
          proposedStatus,
          status,
          startedAt,
          completedAt,
        }),
      write: async ({ track }) => {
        const added = await cli.addValidation(repo, {
          revisionId: location.revisionId,
          track,
          checkName: candidate.checkName,
          status,
          command: taskLabel,
          ...(execution.exitCode === undefined
            ? {}
            : { exitCode: execution.exitCode }),
          startedAt,
          completedAt,
          trigger: "manual",
          summary: validationSummary(execution),
        });
        if (
          added.revisionId !== location.revisionId ||
          added.trackId !== track ||
          added.status !== status
        ) {
          throw new Error(
            "recorded validation identity changed during the write",
          );
        }
        return added;
      },
    });
    if (!result) return;

    await reportResult(location, status, result.refreshed, dependencies);
  } catch (error) {
    await dependencies.showErrorMessage(
      `Pointbreak could not record the validation: ${errorMessage(error)}`,
    );
  }
}

export function proposeValidationStatus(
  execution: TaskExecutionSummary,
): ValidationStatus {
  if (execution.terminationSource === "cancelled") return "skipped";
  if (
    execution.terminationSource === "launch-error" ||
    !execution.processStarted ||
    execution.exitCode === undefined
  ) {
    return "errored";
  }
  return execution.exitCode === 0 ? "passed" : "failed";
}

export function validationConfirmation(
  preview: ValidationWritePreview,
): string {
  const exitCode =
    preview.execution.exitCode === undefined
      ? "Exit code: unavailable"
      : `Exit code: ${preview.execution.exitCode}`;
  return [
    "Record this validation evidence?",
    "",
    `Actor: ${preview.actorId}`,
    `Track: ${preview.track}`,
    `Revision: ${preview.revisionId}`,
    `Task: ${preview.taskLabel}`,
    `Check: ${preview.checkName}`,
    `Duration: ${preview.execution.durationMs.toLocaleString("en-US")} ms`,
    exitCode,
    `Completion source: ${preview.execution.terminationSource}`,
    `Proposed status: ${preview.proposedStatus}`,
    `Selected status: ${preview.status}`,
    "",
    "Terminal cancellation can still report exit code zero; confirm the selected status from the observed result.",
  ].join("\n");
}

interface ValidationLocation {
  readonly resolution: ResolvedTargetResolution;
  readonly revisionId: string;
  readonly trackOverride?: string;
  readonly requiredCheckName?: string;
  readonly failedAttention?: {
    readonly targetKey: string;
    readonly attentionId: string;
  };
}

async function resolveLocation(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  node: AttentionItemNode | undefined,
  dependencies: Pick<
    RunTaskAndRecordValidationDependencies,
    "pickFolder" | "pickRevision" | "showErrorMessage"
  >,
): Promise<ValidationLocation | undefined> {
  if (node) {
    const resolution = resolutions.find(
      (candidate): candidate is ResolvedTargetResolution =>
        candidate.kind === "resolved" &&
        candidate.target.key === node.targetKey,
    );
    const item = node.item;
    if (
      !resolution ||
      item.kind !== "failed_validation" ||
      !item.revisionId?.trim() ||
      !item.trackId.trim() ||
      !item.checkName.trim()
    ) {
      await dependencies.showErrorMessage(
        "Use this command from a failed-validation Attention row with an exact revision, track, and check name.",
      );
      return undefined;
    }
    return {
      resolution,
      revisionId: item.revisionId,
      trackOverride: item.trackId,
      requiredCheckName: item.checkName,
      failedAttention: {
        targetKey: node.targetKey,
        attentionId: node.attentionId,
      },
    };
  }

  const resolution = await dependencies.pickFolder(resolutions);
  if (!resolution) return undefined;
  try {
    const revisions = await cli.revisionList(resolution.folder.uri.fsPath, {
      filter: "-is:superseded",
    });
    if (revisions.entries.length === 0) {
      await dependencies.showErrorMessage(
        "Pointbreak has no current revision. Capture current work before recording validation.",
      );
      return undefined;
    }
    const selected =
      revisions.entries.length === 1
        ? revisions.entries[0]
        : await dependencies.pickRevision(revisions.entries);
    return selected
      ? { resolution, revisionId: selected.revisionId }
      : undefined;
  } catch {
    await dependencies.showErrorMessage(
      "Pointbreak could not resolve a current revision for validation.",
    );
    return undefined;
  }
}

async function reportResult(
  location: ValidationLocation,
  status: ValidationStatus,
  refreshed: boolean,
  dependencies: Pick<
    RunTaskAndRecordValidationDependencies,
    "findAttentionItem" | "showInformationMessage"
  >,
): Promise<void> {
  const attention = location.failedAttention;
  if (!attention) {
    await dependencies.showInformationMessage("Validation recorded.");
    return;
  }
  if (!refreshed) {
    await dependencies.showInformationMessage(
      "Validation recorded, but Pointbreak could not confirm the failed-validation item after refresh.",
    );
    return;
  }
  if (status !== "passed") {
    await dependencies.showInformationMessage(
      `Validation recorded with status ${status}. Only a confirmed passed result can clear the failed-validation attention item.`,
    );
    return;
  }
  const remaining = dependencies.findAttentionItem(
    attention.targetKey,
    attention.attentionId,
  );
  await dependencies.showInformationMessage(
    remaining
      ? "Validation recorded. The failed-validation attention item remains after refresh."
      : "Validation recorded. The failed-validation attention item cleared after refresh.",
  );
}

function validationSummary(execution: TaskExecutionSummary): string {
  const exitCode =
    execution.exitCode === undefined
      ? "Root exit code was unavailable."
      : `Root exit code: ${execution.exitCode}.`;
  return `VS Code reported the selected root task in ${execution.durationMs} ms. ${exitCode} Completion source: ${execution.terminationSource}. The validation status was explicitly selected and confirmed by the user.`;
}

async function runTaskWithCancellation(
  taskRunner: Pick<TaskRunner, "run">,
  candidate: TaskCandidate,
  taskLabel: string,
): Promise<TaskExecutionSummary> {
  return await window.withProgress(
    {
      location: ProgressLocation.Notification,
      title: `Running ${taskLabel}`,
      cancellable: true,
    },
    async (_progress, token) => {
      const cancellation = new AbortController();
      const subscription = token.onCancellationRequested(() =>
        cancellation.abort(),
      );
      try {
        return await taskRunner.run(candidate.task, cancellation.signal);
      } finally {
        subscription.dispose();
      }
    },
  );
}

function taskLabelFor(candidate: TaskCandidate): string {
  return `${candidate.task.source}: ${candidate.task.name}`;
}

interface RevisionPickItem extends QuickPickItem {
  readonly entry: RevisionListEntry;
}

function revisionPickItems(
  entries: readonly RevisionListEntry[],
): RevisionPickItem[] {
  const openCount = entries.filter(
    ({ mergeStatus }) => mergeStatus === "open",
  ).length;
  return newestRevisionEntries(entries)
    .map((entry) => ({
      ...revisionDiscoveryDisplay(entry),
      detail: `${entry.revisionId} · captured ${entry.capturedAt}`,
      picked: openCount === 1 && entry.mergeStatus === "open",
      entry,
    }))
    .sort(compareRevisionPickItems);
}

function compareRevisionPickItems(
  left: RevisionPickItem,
  right: RevisionPickItem,
): number {
  return revisionPriority(left.entry) - revisionPriority(right.entry);
}

function revisionPriority(entry: RevisionListEntry): number {
  return entry.mergeStatus === "open" ? 0 : 1;
}

function defaultDependencies(): RunTaskAndRecordValidationDependencies {
  return {
    pickFolder,
    pickRevision: async (entries) => {
      const picked = await window.showQuickPick(revisionPickItems(entries), {
        placeHolder: "Choose the exact revision for validation",
        matchOnDescription: true,
        matchOnDetail: true,
      });
      return picked?.entry;
    },
    pickTask: (folder, requiredCheckName) =>
      pickRootTask(folder, undefined, requiredCheckName),
    pickStatus: async ({ proposedStatus, taskLabel, checkName }) => {
      const items = STATUSES.map((status) => ({
        label: status,
        description: status === proposedStatus ? "Proposed" : undefined,
        picked: status === proposedStatus,
        status,
      }));
      const picked = await window.showQuickPick(items, {
        placeHolder: `Confirm validation status for ${taskLabel} (${checkName})`,
      });
      return picked?.status;
    },
    now: () => new Date().toISOString(),
    confirmWrite: async (preview) =>
      (await window.showWarningMessage(
        validationConfirmation(preview),
        { modal: true },
        RECORD_ACTION,
      )) === RECORD_ACTION,
    findAttentionItem: () => undefined,
    showInformationMessage: async (message) =>
      window.showInformationMessage(message),
    showErrorMessage: async (message) => window.showErrorMessage(message),
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
