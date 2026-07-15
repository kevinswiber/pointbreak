import { commands, type ExtensionContext, window, workspace } from "vscode";
import { AttentionTreeProvider } from "./attentionView";
import { resolveBinary } from "./binary";
import { PointbreakCli } from "./cli";
import { runAddObservationFromSelectionCommand } from "./commands/addObservationFromSelection";
import { runAssessmentFromAttentionCommand } from "./commands/assessmentFromAttention";
import { runAttentionHeadResolutionCommand } from "./commands/attentionHeadResolution";
import { runCaptureCommand } from "./commands/capture";
import { runOpenAnnotatedDiffCommand } from "./commands/openAnnotatedDiff";
import { runOpenInReviewCommand } from "./commands/openInReview";
import {
  OpenInSourceCommand,
  SourceReviewContextStore,
} from "./commands/openInSource";
import { runRespondInputRequestCommand } from "./commands/respondInputRequest";
import { InspectApiDiffDataSource } from "./diffDataSource";
import { FreshnessCoordinator } from "./freshnessCoordinator";
import { HumanWriteCoordinator } from "./humanWriteCoordinator";
import { InspectChildManager } from "./inspectChild";
import { revisionIsCurrent } from "./inspectClient";
import { InspectConnectionStore } from "./inspectConnectionStore";
import { Logger } from "./logger";
import { ReviewPanelManager } from "./reviewPanel";
import { resolveTargets } from "./targetResolver";

let activeInspectManager: InspectChildManager | undefined;
const SOURCE_CONTEXT_KEY = "pointbreak.hasSourceReviewContext";

export async function activate(context: ExtensionContext): Promise<void> {
  const output = window.createOutputChannel("Pointbreak");
  const logger = new Logger(output);
  context.subscriptions.push(logger);

  const config = workspace.getConfiguration("pointbreak");
  let cli: PointbreakCli;
  let binary: ReturnType<typeof resolveBinary>;
  try {
    binary = resolveBinary(
      {
        binaryPath: config.get<string>("binaryPath"),
        useGlobalCli: config.get<boolean>("useGlobalCli", false),
        announceFallback: (message) => logger.warn(message),
      },
      context.extensionPath,
    );
    cli = new PointbreakCli(binary);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    logger.error(message);
    await window.showErrorMessage(message);
    return;
  }

  const resolutions = await resolveTargets(
    cli,
    workspace.workspaceFolders ?? [],
  );
  const inspectConnections = new InspectConnectionStore(
    context.workspaceState,
    context.secrets,
  );
  const inspectManager = new InspectChildManager(binary, inspectConnections);
  const diffDataSource = new InspectApiDiffDataSource(inspectManager);
  let sourceContexts: SourceReviewContextStore;
  const updateSourceContext = async (): Promise<void> => {
    const document = window.activeTextEditor?.document;
    await commands.executeCommand(
      "setContext",
      SOURCE_CONTEXT_KEY,
      !!document && !!sourceContexts.get(document),
    );
  };
  sourceContexts = new SourceReviewContextStore(() => {
    void updateSourceContext();
  });
  const openInSource = new OpenInSourceCommand(sourceContexts);
  const reviewPanel = new ReviewPanelManager(
    context.extensionUri,
    diffDataSource,
    openInSource,
  );
  activeInspectManager = inspectManager;
  const provider = new AttentionTreeProvider(cli, resolutions);
  const treeView = window.createTreeView("pointbreak.attention", {
    treeDataProvider: provider,
  });
  const freshness = new FreshnessCoordinator(
    inspectManager,
    provider,
    reviewPanel,
    {
      reportError: (error) => {
        void window.showWarningMessage(error.message);
      },
    },
  );
  const humanWrites = new HumanWriteCoordinator(cli, {
    resolveTrack: (resource) =>
      workspace
        .getConfiguration("pointbreak", resource as never)
        .get<string>("observationTrack", "human:local"),
    showDiagnostic: async (message) => window.showWarningMessage(message),
    refresh: () => freshness.refreshAfterWrite(),
    showRefreshError: async (message) => window.showWarningMessage(message),
  });
  context.subscriptions.push(
    provider,
    treeView,
    provider.attachTreeView(treeView),
    inspectManager,
    openInSource,
    reviewPanel,
    freshness,
    window.onDidChangeActiveTextEditor(() => {
      void updateSourceContext();
    }),
    workspace.onDidCloseTextDocument((document) =>
      sourceContexts.delete(document),
    ),
    commands.registerCommand("pointbreak.refreshAttention", () =>
      freshness.refreshAll(),
    ),
    commands.registerCommand("pointbreak.capture", () =>
      runCaptureCommand(cli, resolutions, {
        humanWrites,
      }),
    ),
    commands.registerCommand("pointbreak.openAnnotatedDiff", (node) =>
      runOpenAnnotatedDiffCommand(cli, resolutions, reviewPanel, node),
    ),
    commands.registerCommand("pointbreak.openInReview", (node) =>
      runOpenInReviewCommand(cli, binary, resolutions, node),
    ),
    commands.registerCommand("pointbreak.stopInspect", () =>
      inspectManager.stop(),
    ),
    commands.registerCommand("pointbreak.addObservationFromSelection", () =>
      runAddObservationFromSelectionCommand(cli, resolutions, sourceContexts, {
        humanWrites,
        isRevisionCurrent: async (resolution, revisionId) => {
          const { client } = await inspectManager.ensure(resolution);
          return revisionIsCurrent(
            await client.revision(revisionId),
            revisionId,
          );
        },
      }),
    ),
    commands.registerCommand("pointbreak.respondInputRequest", (node) =>
      runRespondInputRequestCommand(cli, node, { humanWrites }),
    ),
    commands.registerCommand("pointbreak.assessAttention", (node) =>
      runAssessmentFromAttentionCommand(cli, node, { humanWrites }),
    ),
    commands.registerCommand("pointbreak.captureAttentionResolution", (node) =>
      runAttentionHeadResolutionCommand(cli, resolutions, node, {
        humanWrites,
        findAttentionItem: (targetKey, attentionId) =>
          provider.findAttentionItem(targetKey, attentionId),
        routeAssessment: async (refreshedNode) =>
          commands.executeCommand("pointbreak.assessAttention", refreshedNode),
      }),
    ),
  );
  await updateSourceContext();
}

export async function deactivate(): Promise<void> {
  const manager = activeInspectManager;
  activeInspectManager = undefined;
  await manager?.stop();
}
