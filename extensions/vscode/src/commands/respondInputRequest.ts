import { window } from "vscode";
import type { AttentionItemNode } from "../attentionView";
import type {
  InputRequestOutcome,
  InputRequestResponseAttentionItem,
  PointbreakCli,
} from "../cli";
import type {
  HumanWriteContext,
  HumanWriteCoordinator,
} from "../humanWriteCoordinator";

interface ResponseConfirmation extends HumanWriteContext {
  inputRequestId: string;
  outcome: InputRequestOutcome;
  reason?: string;
}

interface RespondInputRequestDependencies {
  humanWrites?: HumanWriteCoordinator;
  pickRequestIds(
    item: InputRequestResponseAttentionItem,
  ): Promise<string[] | undefined>;
  pickOutcome(
    item: InputRequestResponseAttentionItem,
  ): Promise<InputRequestOutcome | undefined>;
  promptReason(
    item: InputRequestResponseAttentionItem,
    outcome: InputRequestOutcome,
  ): Promise<string | undefined>;
  confirmResponse(context: ResponseConfirmation): Promise<boolean>;
  showInformationMessage(message: string): Promise<unknown>;
  showWarningMessage(message: string): Promise<unknown>;
  showErrorMessage(message: string): Promise<unknown>;
}

const OUTCOMES: Array<{ label: string; outcome: InputRequestOutcome }> = [
  { label: "Approve", outcome: "approved" },
  { label: "Reject", outcome: "rejected" },
  { label: "Dismiss", outcome: "dismissed" },
  { label: "Supersede", outcome: "superseded" },
  { label: "Abandon", outcome: "abandoned" },
];
const CONFIRM_RESPONSE_ACTION = "Respond";

export async function runRespondInputRequestCommand(
  cli: PointbreakCli,
  node: AttentionItemNode | undefined,
  overrides: Partial<RespondInputRequestDependencies> = {},
): Promise<void> {
  const dependencies = { ...defaultDependencies(), ...overrides };
  if (!node) {
    await dependencies.showErrorMessage(
      "Use this command from a matching Pointbreak Attention row.",
    );
    return;
  }
  if (!isResponseItem(node.item)) {
    await dependencies.showErrorMessage(
      "This Pointbreak attention item cannot accept an input response.",
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

  const requestIds = await selectedRequestIds(node.item, dependencies);
  if (!requestIds) return;
  const outcome = await dependencies.pickOutcome(node.item);
  if (!outcome) return;
  const enteredReason = await dependencies.promptReason(node.item, outcome);
  if (enteredReason === undefined) return;
  const reason = enteredReason.trim() || undefined;

  let batchActorId: string | undefined;
  for (const inputRequestId of requestIds) {
    let confirmedActorId: string | undefined;
    let identityChanged = false;
    try {
      const result = await humanWrites.run({
        repo: node.folder.uri.fsPath,
        resource: node.folder.uri,
        trackOverride: node.item.trackId,
        confirm: async (context) => {
          if (batchActorId && context.actorId !== batchActorId) {
            identityChanged = true;
            await dependencies.showWarningMessage(
              `Pointbreak stopped because the human writer identity changed from ${batchActorId} to ${context.actorId}.`,
            );
            return false;
          }
          confirmedActorId = context.actorId;
          return dependencies.confirmResponse({
            ...context,
            inputRequestId,
            outcome,
            reason,
          });
        },
        write: () =>
          cli.respondInputRequest(node.folder.uri.fsPath, {
            inputRequestId,
            outcome,
            reason,
          }),
      });
      if (!result) {
        if (!identityChanged) {
          await dependencies.showInformationMessage(
            `Pointbreak stopped before responding to ${inputRequestId}.`,
          );
        }
        return;
      }
      batchActorId = confirmedActorId;
    } catch (error) {
      await dependencies.showErrorMessage(
        `Pointbreak could not respond to ${inputRequestId}: ${errorMessage(error)}`,
      );
      return;
    }
  }

  await dependencies.showInformationMessage(
    requestIds.length === 1
      ? "Input request response recorded."
      : `${requestIds.length} input request responses recorded.`,
  );
}

function isResponseItem(
  item: AttentionItemNode["item"],
): item is InputRequestResponseAttentionItem {
  return (
    item.kind === "open_input_request" || item.kind === "follow_up_outstanding"
  );
}

async function selectedRequestIds(
  item: InputRequestResponseAttentionItem,
  dependencies: RespondInputRequestDependencies,
): Promise<string[] | undefined> {
  if (item.kind === "open_input_request") return [item.inputRequestId];
  if (item.openInputRequestIds.length === 1) {
    return [...item.openInputRequestIds];
  }
  const selected = await dependencies.pickRequestIds(item);
  if (!selected?.length) return undefined;
  const selectedSet = new Set(selected);
  const exactSelection = item.openInputRequestIds.filter((id) =>
    selectedSet.has(id),
  );
  return exactSelection.length ? exactSelection : undefined;
}

function defaultDependencies(): RespondInputRequestDependencies {
  return {
    pickRequestIds: async (item) => {
      if (item.kind !== "follow_up_outstanding") {
        return [item.inputRequestId];
      }
      const all = {
        label: `Respond to all ${item.openInputRequestIds.length} requests`,
        requestIds: item.openInputRequestIds,
      };
      const choices = [
        all,
        ...item.openInputRequestIds.map((id) => ({
          label: `Respond to ${id}`,
          requestIds: [id],
        })),
      ];
      return (
        await window.showQuickPick(choices, {
          placeHolder: `Follow-up by ${item.recordedBy} in ${item.trackId}`,
        })
      )?.requestIds;
    },
    pickOutcome: async (item) =>
      (
        await window.showQuickPick(OUTCOMES, {
          placeHolder: responseContext(item),
        })
      )?.outcome,
    promptReason: async (_item, outcome) =>
      window.showInputBox({
        prompt: `Optional reason for ${outcome}`,
        placeHolder: "Leave blank to respond without a reason",
      }),
    confirmResponse: async ({
      actorId,
      track,
      inputRequestId,
      outcome,
      reason,
    }) =>
      (await window.showWarningMessage(
        `Respond ${outcome} to ${inputRequestId} as ${actorId} in track ${track}${reason ? ` with reason “${reason}”` : ""}?`,
        { modal: true },
        CONFIRM_RESPONSE_ACTION,
      )) === CONFIRM_RESPONSE_ACTION,
    showInformationMessage: async (message) =>
      window.showInformationMessage(message),
    showWarningMessage: async (message) => window.showWarningMessage(message),
    showErrorMessage: async (message) => window.showErrorMessage(message),
  };
}

function responseContext(item: InputRequestResponseAttentionItem): string {
  if (item.kind === "open_input_request") {
    return `${item.title} · ${item.mode} · ${item.reasonCode} · ${item.trackId} · ${item.openedBy}`;
  }
  return `Follow-up in ${item.trackId} by ${item.recordedBy}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
