import { commands, window } from "vscode";
import type { AttentionItemNode } from "../attentionView";
import type {
  AssessmentShowDoc,
  AssessmentValue,
  AssessmentView,
  PointbreakCli,
} from "../cli";
import type {
  HumanWriteContext,
  HumanWriteCoordinator,
} from "../humanWriteCoordinator";

type AssessmentAttentionItem = Extract<
  AttentionItemNode["item"],
  {
    kind: "ambiguous_assessment" | "stale_assessment" | "failed_validation";
  }
>;

interface AssessmentConfirmation extends HumanWriteContext {
  revisionId: string;
  assessment: AssessmentValue;
  summary?: string;
  replacementIds: string[];
}

interface AssessmentFromAttentionDependencies {
  humanWrites?: HumanWriteCoordinator;
  pickAssessment(
    item: AssessmentAttentionItem,
  ): Promise<AssessmentValue | undefined>;
  promptSummary(
    item: AssessmentAttentionItem,
    assessment: AssessmentValue,
  ): Promise<string | undefined>;
  pickReplacements(
    candidates: AssessmentView[],
  ): Promise<AssessmentView[] | undefined>;
  confirmAssessment(context: AssessmentConfirmation): Promise<boolean>;
  routeHeadResolution(node: AttentionItemNode): Promise<unknown>;
  showInformationMessage(message: string): Promise<unknown>;
  showErrorMessage(message: string): Promise<unknown>;
}

const ASSESSMENTS: Array<{ label: string; assessment: AssessmentValue }> = [
  { label: "Accepted", assessment: "accepted" },
  {
    label: "Accepted with follow-up",
    assessment: "accepted-with-follow-up",
  },
  { label: "Needs changes", assessment: "needs-changes" },
  { label: "Needs clarification", assessment: "needs-clarification" },
];
const CONFIRM_ASSESSMENT_ACTION = "Record assessment";

export async function runAssessmentFromAttentionCommand(
  cli: PointbreakCli,
  node: AttentionItemNode | undefined,
  overrides: Partial<AssessmentFromAttentionDependencies> = {},
): Promise<void> {
  const dependencies = { ...defaultDependencies(), ...overrides };
  if (!node) {
    await dependencies.showErrorMessage(
      "Use this command from a matching Pointbreak Attention row.",
    );
    return;
  }
  if (!isAssessmentItem(node.item)) {
    await dependencies.showErrorMessage(
      "This Pointbreak attention item cannot accept an assessment.",
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
  const item = node.item;
  const revisionId = await assessmentRevision(node, item, dependencies);
  if (!revisionId) return;
  const assessment = await dependencies.pickAssessment(item);
  if (!assessment) return;
  const enteredSummary = await dependencies.promptSummary(item, assessment);
  if (enteredSummary === undefined) return;
  const summary = enteredSummary.trim() || undefined;

  try {
    const result = await humanWrites.run({
      repo: node.folder.uri.fsPath,
      resource: node.folder.uri,
      trackOverride:
        item.kind === "failed_validation" ? item.trackId : undefined,
      prepare: async (context) => {
        const document = await cli.showAssessments(node.folder.uri.fsPath, {
          revisionId,
          track: context.track,
        });
        const candidates = sameHumanRevisionCandidates(
          document,
          context,
          revisionId,
        );
        if (candidates.length <= 1) {
          return { replacementIds: candidates.map(({ id }) => id) };
        }
        const selected = await dependencies.pickReplacements(candidates);
        return selected
          ? { replacementIds: selected.map(({ id }) => id) }
          : undefined;
      },
      confirm: (context, preparation) => {
        if (!preparation) return Promise.resolve(false);
        return dependencies.confirmAssessment({
          ...context,
          revisionId,
          assessment,
          summary,
          replacementIds: preparation.replacementIds,
        });
      },
      write: (context, preparation) => {
        if (!preparation) {
          throw new Error("assessment preparation was cancelled");
        }
        return cli.addAssessment(node.folder.uri.fsPath, {
          revisionId,
          track: context.track,
          assessment,
          summary,
          replaces: preparation.replacementIds,
        });
      },
    });
    if (!result) return;
  } catch (error) {
    await dependencies.showErrorMessage(
      `Pointbreak could not record the assessment: ${errorMessage(error)}`,
    );
    return;
  }
  await dependencies.showInformationMessage("Assessment recorded.");
}

export function sameHumanRevisionCandidates(
  document: AssessmentShowDoc,
  context: HumanWriteContext,
  revisionId: string,
): AssessmentView[] {
  return document.assessments.filter(
    (candidate) =>
      candidate.status === "current" &&
      candidate.trackId === context.track &&
      candidate.writer.actorId === context.actorId &&
      candidate.target.kind === "revision" &&
      candidate.target.revisionId === revisionId,
  );
}

function isAssessmentItem(
  item: AttentionItemNode["item"],
): item is AssessmentAttentionItem {
  return (
    item.kind === "ambiguous_assessment" ||
    item.kind === "stale_assessment" ||
    item.kind === "failed_validation"
  );
}

async function assessmentRevision(
  node: AttentionItemNode,
  item: AssessmentAttentionItem,
  dependencies: AssessmentFromAttentionDependencies,
): Promise<string | undefined> {
  if (item.kind !== "stale_assessment") {
    if (item.revisionId) return item.revisionId;
    await dependencies.showErrorMessage(
      "Pointbreak assessment attention is missing its exact revision.",
    );
    return undefined;
  }
  const heads = item.headRevisionIds ?? [];
  if (heads.length === 1) return heads[0];
  if (heads.length > 1) {
    await dependencies.routeHeadResolution(node);
    return undefined;
  }
  await dependencies.showErrorMessage(
    "Pointbreak could not find the stale assessment's exact current head.",
  );
  return undefined;
}

function defaultDependencies(): AssessmentFromAttentionDependencies {
  return {
    pickAssessment: async () =>
      (
        await window.showQuickPick(ASSESSMENTS, {
          placeHolder: "Choose an assessment",
        })
      )?.assessment,
    promptSummary: async (_item, assessment) =>
      window.showInputBox({
        prompt: `Optional summary for ${assessment}`,
        placeHolder: "Leave blank to record without a summary",
      }),
    pickReplacements: async (candidates) => {
      const choices = candidates.map((candidate) => ({
        label: candidate.id,
        description: candidate.assessment,
        detail: candidate.createdAt,
        candidate,
        picked: true,
      }));
      return (
        await window.showQuickPick(choices, {
          canPickMany: true,
          placeHolder: "Choose your earlier assessments to replace",
        })
      )?.map(({ candidate }) => candidate);
    },
    confirmAssessment: async ({
      actorId,
      track,
      revisionId,
      assessment,
      summary,
      replacementIds,
    }) =>
      (await window.showWarningMessage(
        assessmentConfirmation({
          actorId,
          track,
          revisionId,
          assessment,
          summary,
          replacementIds,
        }),
        { modal: true },
        CONFIRM_ASSESSMENT_ACTION,
      )) === CONFIRM_ASSESSMENT_ACTION,
    routeHeadResolution: async (node) =>
      commands.executeCommand("pointbreak.captureAttentionResolution", node),
    showInformationMessage: async (message) =>
      window.showInformationMessage(message),
    showErrorMessage: async (message) => window.showErrorMessage(message),
  };
}

function assessmentConfirmation(context: AssessmentConfirmation): string {
  const replacements = context.replacementIds.length
    ? ` Replace ${context.replacementIds.join(", ")}.`
    : "";
  const summary = context.summary ? ` Summary: “${context.summary}”.` : "";
  return `Record ${context.assessment} on ${context.revisionId} as ${context.actorId} in track ${context.track}.${summary}${replacements}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
