import { commands, window } from "vscode";
import type { PointbreakCli } from "../cli";
import type {
  HumanWriteContext,
  HumanWriteCoordinator,
} from "../humanWriteCoordinator";
import {
  liveSelectionToSnapshot,
  repoRelativeFile,
  type ZeroBasedSelection,
} from "../sourceMapping";
import type {
  ResolvedTargetResolution,
  TargetResolution,
} from "../targetResolver";
import type { SourceReviewContextStore } from "./openInSource";

const DRIFT_ACTION = "Add to changed source";
const UNVERIFIED_ACTION = "Add outside captured lines";
const CAPTURE_ACTION = "Capture current work";
const RECORD_ACTION = "Record observation";
const OPEN_SOURCE_GUIDANCE =
  "Open a captured range from Pointbreak Review before adding an observation.";

interface SourceDocument {
  readonly uri: { readonly fsPath: string };
  readonly lineCount: number;
  lineAt(line: number): { readonly text: string };
}

interface SourceEditor {
  readonly document: SourceDocument;
  readonly selection: ZeroBasedSelection;
}

export interface AddObservationDependencies {
  activeEditor(): SourceEditor | undefined;
  humanWrites?: HumanWriteCoordinator;
  isRevisionCurrent(
    resolution: ResolvedTargetResolution,
    revisionId: string,
  ): Promise<boolean>;
  promptTitle(): Promise<string | undefined>;
  confirmDrift(message: string): Promise<boolean>;
  confirmUnverified(message: string): Promise<boolean>;
  offerCapture(message: string): Promise<boolean>;
  confirmWrite(
    context: HumanWriteContext & { revisionId: string; title: string },
  ): Promise<boolean>;
  capture(): Promise<unknown>;
  showInformationMessage(message: string): Promise<unknown>;
  showErrorMessage(message: string): Promise<unknown>;
}

export async function runAddObservationFromSelectionCommand(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  contexts: SourceReviewContextStore,
  overrides: Partial<AddObservationDependencies> = {},
): Promise<void> {
  const dependencies = { ...defaultDependencies(), ...overrides };
  const editor = dependencies.activeEditor();
  if (!editor) {
    await dependencies.showInformationMessage(OPEN_SOURCE_GUIDANCE);
    return;
  }
  const context = contexts.get(editor.document);
  if (!context) {
    await dependencies.showInformationMessage(OPEN_SOURCE_GUIDANCE);
    return;
  }
  if (context.side === "old") {
    await dependencies.showInformationMessage(
      "Observations can only be added from new-side or added source.",
    );
    return;
  }
  const resolution = resolutions.find(
    (candidate) =>
      candidate.kind === "resolved" &&
      candidate.target.key === context.targetKey,
  );
  if (resolution?.kind !== "resolved") {
    await dependencies.showInformationMessage(
      "Pointbreak could not resolve the review target for this source editor.",
    );
    return;
  }

  let filePath: string;
  try {
    filePath = repoRelativeFile(
      resolution.folder.uri.fsPath,
      editor.document.uri.fsPath,
    );
  } catch {
    await dependencies.showInformationMessage(
      "This source editor is outside the reviewed repository.",
    );
    return;
  }
  if (filePath !== context.filePath) {
    await offerCapture(
      dependencies,
      "This source editor no longer matches its captured review context.",
    );
    return;
  }

  const liveLines = Array.from(
    { length: editor.document.lineCount },
    (_, index) => editor.document.lineAt(index).text,
  );
  const verification = liveSelectionToSnapshot(
    context.snapshot,
    filePath,
    editor.selection,
    liveLines,
    context.target.startLine - 1 - context.lastLanding.range.start.line,
  );
  if (verification.kind === "not-in-snapshot") {
    await offerCapture(dependencies, verification.message);
    return;
  }
  if (
    context.lastLanding.kind === "drifted" &&
    verification.kind === "verified" &&
    !(await dependencies.confirmDrift(
      context.lastLanding.message ??
        "The live source has changed since this review was captured.",
    ))
  ) {
    return;
  }
  if (
    verification.kind === "drifted" &&
    !(await dependencies.confirmDrift(verification.message))
  ) {
    return;
  }
  if (
    verification.kind === "unverified" &&
    !(await dependencies.confirmUnverified(verification.message))
  ) {
    return;
  }

  const title = (await dependencies.promptTitle())?.trim();
  if (!title) return;

  try {
    if (
      !(await dependencies.isRevisionCurrent(resolution, context.revisionId))
    ) {
      await dependencies.showErrorMessage(
        "This captured review is no longer current. Reopen it before adding an observation.",
      );
      return;
    }
  } catch {
    await dependencies.showErrorMessage(
      "Pointbreak could not confirm that this captured review is current.",
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
    const result = await humanWrites.run({
      repo: resolution.folder.uri.fsPath,
      resource: editor.document.uri,
      confirm: (writer) =>
        dependencies.confirmWrite({
          ...writer,
          revisionId: context.revisionId,
          title,
        }),
      write: async ({ track }) => {
        const added = await cli.addObservation(resolution.folder.uri.fsPath, {
          revisionId: context.revisionId,
          track,
          title,
          file: verification.target.filePath,
          side: verification.target.side,
          startLine: verification.target.startLine,
          endLine: verification.target.endLine,
        });
        if (added.revisionId !== context.revisionId) {
          throw new Error("observation revision changed during write");
        }
        return added;
      },
    });
    if (!result) return;
  } catch {
    await dependencies.showErrorMessage(
      "Pointbreak could not add the observation.",
    );
    return;
  }
  void dependencies.showInformationMessage("Observation recorded.");
}

function defaultDependencies(): AddObservationDependencies {
  return {
    activeEditor: () =>
      window.activeTextEditor as unknown as SourceEditor | undefined,
    isRevisionCurrent: async () => false,
    promptTitle: async () =>
      window.showInputBox({
        prompt: "Observation title",
        placeHolder: "What should reviewers notice?",
        validateInput: (value) =>
          value.trim() ? undefined : "An observation title is required.",
      }),
    confirmDrift: async (message) =>
      (await window.showWarningMessage(
        message,
        { modal: true },
        DRIFT_ACTION,
      )) === DRIFT_ACTION,
    confirmUnverified: async (message) =>
      (await window.showWarningMessage(
        message,
        { modal: true },
        UNVERIFIED_ACTION,
      )) === UNVERIFIED_ACTION,
    offerCapture: async (message) =>
      (await window.showInformationMessage(message, CAPTURE_ACTION)) ===
      CAPTURE_ACTION,
    confirmWrite: async ({ actorId, track, revisionId, title }) =>
      (await window.showWarningMessage(
        `Record “${title}” on ${revisionId} as ${actorId} in track ${track}?`,
        { modal: true },
        RECORD_ACTION,
      )) === RECORD_ACTION,
    capture: async () => commands.executeCommand("pointbreak.capture"),
    showInformationMessage: async (message) =>
      window.showInformationMessage(message),
    showErrorMessage: async (message) => window.showErrorMessage(message),
  };
}

async function offerCapture(
  dependencies: AddObservationDependencies,
  message: string,
): Promise<void> {
  if (await dependencies.offerCapture(message)) {
    await dependencies.capture();
  }
}
