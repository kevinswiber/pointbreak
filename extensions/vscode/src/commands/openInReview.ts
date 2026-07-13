import {
  env,
  type Terminal,
  Uri,
  type WorkspaceFolder,
  window,
  workspace,
} from "vscode";
import type { ResolvedBinary } from "../binary";
import type { PointbreakCli } from "../cli";
import { pickFolder, type TargetResolution } from "../targetResolver";

const DEFAULT_REVIEW_URL = "http://127.0.0.1:7878";
const RETRY_ATTEMPTS = 10;
const RETRY_DELAY_MS = 1_000;
const REVIEW_TERMINAL_NAME = "Pointbreak Review";
const START_REVIEW_ACTION = "Start `shore inspect` here";
const terminalTargets = new WeakMap<Terminal, string>();

export interface ReviewNode {
  revisionId: string;
  folder: WorkspaceFolder;
}

interface OpenInReviewDependencies {
  pick?: typeof pickFolder;
  probe?: (baseUrl: string) => Promise<boolean>;
  reviewUrl?: string;
  sleep?: (milliseconds: number) => Promise<void>;
}

export function reviewDeepLink(baseUrl: string, revisionId: string): string {
  return `${trimTrailingSlash(baseUrl)}/#/revision/${revisionId}`;
}

export async function runOpenInReviewCommand(
  cli: PointbreakCli,
  binary: ResolvedBinary,
  resolutions: TargetResolution[],
  node?: ReviewNode,
  dependencies: OpenInReviewDependencies = {},
): Promise<void> {
  if (env.remoteName) {
    await window.showInformationMessage(
      "Open in Pointbreak Review is not available in remote workspaces yet.",
    );
    return;
  }

  const selection =
    node ??
    (await pickRevision(cli, resolutions, dependencies.pick ?? pickFolder));
  if (!selection) {
    return;
  }

  const baseUrl = trimTrailingSlash(
    dependencies.reviewUrl ??
      workspace
        .getConfiguration("pointbreak")
        .get<string>("reviewUrl", DEFAULT_REVIEW_URL),
  );
  const probe = dependencies.probe ?? probeReview;
  const deepLink = reviewDeepLink(baseUrl, selection.revisionId);
  if (await probe(baseUrl)) {
    await env.openExternal(Uri.parse(deepLink));
    return;
  }

  const action = await window.showInformationMessage(
    `Pointbreak Review isn't running at ${baseUrl}`,
    START_REVIEW_ACTION,
  );
  if (action !== START_REVIEW_ACTION) {
    return;
  }

  startReviewTerminal(binary, selection.folder);

  const available = await retryProbe(
    baseUrl,
    probe,
    dependencies.sleep ?? delay,
  );
  if (available) {
    await env.openExternal(Uri.parse(deepLink));
    return;
  }

  await window.showErrorMessage(
    "Pointbreak Review is still unavailable. Start `shore inspect` manually or fix pointbreak.reviewUrl.",
  );
}

export async function probeReview(baseUrl: string): Promise<boolean> {
  try {
    await fetch(`${trimTrailingSlash(baseUrl)}/`, {
      method: "GET",
      signal: AbortSignal.timeout(1_000),
    });
    return true;
  } catch {
    return false;
  }
}

async function pickRevision(
  cli: PointbreakCli,
  resolutions: TargetResolution[],
  pick: typeof pickFolder,
): Promise<ReviewNode | undefined> {
  const resolution = await pick(resolutions);
  if (!resolution) {
    return undefined;
  }
  try {
    const revisions = await cli.revisionList(resolution.folder.uri.fsPath);
    const items = revisions.entries.slice(0, 20).map((entry) => ({
      label: shortRevisionId(entry.revisionId),
      description: entry.mergeStatus,
      detail: entry.capturedAt,
      revisionId: entry.revisionId,
    }));
    if (items.length === 0) {
      await window.showInformationMessage(
        "Pointbreak has no captured revisions in this target yet.",
      );
      return undefined;
    }
    const picked = await window.showQuickPick(items, {
      placeHolder: "Choose a revision to open in Pointbreak Review",
    });
    return picked
      ? { revisionId: picked.revisionId, folder: resolution.folder }
      : undefined;
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    await window.showErrorMessage(
      `Pointbreak could not list revisions: ${detail}`,
    );
    return undefined;
  }
}

async function retryProbe(
  baseUrl: string,
  probe: (baseUrl: string) => Promise<boolean>,
  sleep: (milliseconds: number) => Promise<void>,
): Promise<boolean> {
  for (let attempt = 0; attempt < RETRY_ATTEMPTS; attempt += 1) {
    await sleep(RETRY_DELAY_MS);
    if (await probe(baseUrl)) {
      return true;
    }
  }
  return false;
}

function startReviewTerminal(
  binary: ResolvedBinary,
  folder: WorkspaceFolder,
): void {
  const cwd = folder.uri.fsPath;
  if (binary.source !== "path") {
    const terminal = window.createTerminal({
      name: REVIEW_TERMINAL_NAME,
      cwd,
      shellPath: binary.path,
      shellArgs: ["inspect"],
    });
    terminal.show();
    return;
  }

  const existing = window.terminals.find(
    (terminal) =>
      terminal.name === REVIEW_TERMINAL_NAME &&
      terminalTargets.get(terminal) === cwd,
  );
  const terminal =
    existing ?? window.createTerminal({ name: REVIEW_TERMINAL_NAME, cwd });
  terminalTargets.set(terminal, cwd);
  terminal.sendText("shore inspect");
  terminal.show();
}

function trimTrailingSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

function shortRevisionId(revisionId: string): string {
  return revisionId.split(":").at(-1)?.slice(0, 12) ?? revisionId;
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
