import {
  commands,
  env,
  Uri,
  type WorkspaceFolder,
  window,
  workspace,
} from "vscode";
import { POINTBREAK_CLI_NAME, type ResolvedBinary } from "../binary";
import type { PointbreakCli } from "../cli";
import { revisionDiscoveryDisplay } from "../idDisplay";
import {
  type FetchFn,
  InspectClient,
  InspectClientError,
} from "../inspectClient";
import {
  type ReviewCapability,
  runningReviewCapability,
  startReviewTerminal,
  stopReviewTerminal,
} from "../reviewTerminal";
import { newestRevisionEntries } from "../revisionOrder";
import { pickFolder, type TargetResolution } from "../targetResolver";

const RETRY_ATTEMPTS = 10;
const RETRY_DELAY_MS = 1_000;
const START_REVIEW_ACTION = `Start \`${POINTBREAK_CLI_NAME} inspect\` here`;
const OPEN_INTEGRATED_BROWSER_COMMAND = "workbench.action.browser.open";
const LOCAL_BROWSER_HOSTS = new Set([
  "localhost",
  "127.0.0.1",
  "[::1]",
  "0.0.0.0",
  "[::]",
]);

export interface ReviewNode {
  revisionId: string;
  targetKey: string;
  folder: WorkspaceFolder;
  lens?: "attention";
}

export interface ReviewIdentity {
  storeIdentity: string;
  contextIdentity: string;
}

export type ReviewProbeResult =
  | { kind: "match" }
  | { kind: "unreachable" }
  | { kind: "unauthorized" }
  | { kind: "version-incompatible" }
  | { kind: "protocol" }
  | { kind: "identity-mismatch" };

type ReviewProbe = (
  capability: ReviewCapability,
  identity: ReviewIdentity,
) => Promise<ReviewProbeResult>;

interface OpenInReviewDependencies {
  pick?: typeof pickFolder;
  probe?: ReviewProbe;
  reviewUrl?: string;
  running?: typeof runningReviewCapability;
  sleep?: (milliseconds: number) => Promise<void>;
  start?: typeof startReviewTerminal;
  stop?: typeof stopReviewTerminal;
}

export function reviewDeepLink(
  origin: string,
  revisionId: string,
  lens?: "attention",
  token?: string,
): string {
  const params = new URLSearchParams();
  if (lens) {
    params.set("lens", lens);
  }
  if (token) {
    params.set("token", token);
  }
  const query = params.size > 0 ? `?${params.toString()}` : "";
  return `${trimTrailingSlash(origin)}/#/revision/${revisionId}${query}`;
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
  const resolution = await selectedResolution(selection, resolutions);
  if (!resolution) {
    return;
  }

  const configuredValue =
    dependencies.reviewUrl ?? configuredReviewUrl(selection.folder);
  if (configuredValue.trim()) {
    const configured = normalizeExternalReviewUrl(configuredValue);
    if (!configured) {
      await window.showErrorMessage(
        "The configured Pointbreak Review URL must be an HTTP or HTTPS server origin without credentials or fragments.",
      );
      return;
    }
    await openReviewUrl(
      reviewDeepLink(configured, selection.revisionId, selection.lens),
    );
    return;
  }

  const targetKey = resolution.target.key;
  const identity = identityFor(resolution);
  const probe = dependencies.probe ?? probeReview;
  const stop = dependencies.stop ?? stopReviewTerminal;
  const running = await knownRunningCapability(
    (dependencies.running ?? runningReviewCapability)(targetKey),
  );
  if (running) {
    const result = await probe(running, identity);
    if (result.kind === "match") {
      await openCapability(running, selection);
      return;
    }
    await Promise.resolve(stop(targetKey));
  }

  const action = await window.showInformationMessage(
    "Pointbreak Review is not running for this repository.",
    START_REVIEW_ACTION,
  );
  if (action !== START_REVIEW_ACTION) {
    return;
  }

  let capability: ReviewCapability;
  try {
    capability = await (dependencies.start ?? startReviewTerminal)(
      binary,
      selection.folder,
      targetKey,
    );
  } catch {
    await Promise.resolve(stop(targetKey));
    await window.showErrorMessage(
      "Pointbreak could not start the local Review service.",
    );
    return;
  }

  const result = await retryProbe(
    capability,
    identity,
    probe,
    dependencies.sleep ?? delay,
  );
  if (result.kind === "match") {
    await openCapability(capability, selection);
    return;
  }

  await Promise.resolve(stop(targetKey));
  await window.showErrorMessage(startedServerFailureMessage(result));
}

export async function probeReview(
  capability: ReviewCapability,
  identity: ReviewIdentity,
  fetch?: FetchFn,
): Promise<ReviewProbeResult> {
  try {
    const client = new InspectClient(
      capability.origin,
      capability.token,
      fetch,
    );
    await client.verify(identity);
    return { kind: "match" };
  } catch (error) {
    if (!(error instanceof InspectClientError)) {
      return { kind: "protocol" };
    }
    return { kind: error.kind };
  }
}

async function openCapability(
  capability: ReviewCapability,
  selection: ReviewNode,
): Promise<void> {
  await openReviewUrl(
    reviewDeepLink(
      capability.origin,
      selection.revisionId,
      selection.lens,
      capability.token,
    ),
  );
}

async function openReviewUrl(url: string): Promise<void> {
  const openLocalhostLinks = workspace
    .getConfiguration("workbench.browser")
    .get<boolean>("openLocalhostLinks", false);
  if (openLocalhostLinks && isLocalBrowserUrl(url)) {
    try {
      await commands.executeCommand(OPEN_INTEGRATED_BROWSER_COMMAND, url);
      return;
    } catch {
      // Older supported hosts fall back to the external browser.
    }
  }
  await env.openExternal(Uri.parse(url));
}

function isLocalBrowserUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      (url.protocol === "http:" || url.protocol === "https:") &&
      LOCAL_BROWSER_HOSTS.has(url.hostname)
    );
  } catch {
    return false;
  }
}

async function knownRunningCapability(
  pending: Promise<ReviewCapability> | undefined,
): Promise<ReviewCapability | undefined> {
  try {
    return pending ? await pending : undefined;
  } catch {
    return undefined;
  }
}

type ResolvedTarget = TargetResolution & { kind: "resolved" };

async function selectedResolution(
  selection: ReviewNode,
  resolutions: TargetResolution[],
): Promise<ResolvedTarget | undefined> {
  const resolution = resolutions.find(
    (candidate) =>
      candidate.kind === "resolved" &&
      candidate.target.key === selection.targetKey,
  );
  if (resolution?.kind === "resolved") {
    return resolution;
  }
  await window.showErrorMessage(
    "Pointbreak could not identify this review target. Refresh the extension and try again.",
  );
  return undefined;
}

function identityFor(resolution: ResolvedTarget): ReviewIdentity {
  return {
    storeIdentity: resolution.target.storeIdentity,
    contextIdentity: resolution.target.contextIdentity,
  };
}

function configuredReviewUrl(folder: WorkspaceFolder): string {
  return workspace
    .getConfiguration("pointbreak", folder.uri)
    .get<string>("reviewUrl", "");
}

function normalizeExternalReviewUrl(value: string): string | undefined {
  const trimmed = value.trim();
  try {
    const url = new URL(trimmed);
    if (
      (url.protocol !== "http:" && url.protocol !== "https:") ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      return undefined;
    }
    return url.origin;
  } catch {
    return undefined;
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
    const items = newestRevisionEntries(revisions.entries).map((entry) => ({
      ...revisionDiscoveryDisplay(entry),
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
      ? {
          revisionId: picked.revisionId,
          targetKey: resolution.target.key,
          folder: resolution.folder,
        }
      : undefined;
  } catch {
    await window.showErrorMessage(
      "Pointbreak could not list revisions for this review target.",
    );
    return undefined;
  }
}

async function retryProbe(
  capability: ReviewCapability,
  identity: ReviewIdentity,
  probe: ReviewProbe,
  sleep: (milliseconds: number) => Promise<void>,
): Promise<ReviewProbeResult> {
  for (let attempt = 0; attempt < RETRY_ATTEMPTS; attempt += 1) {
    const result = await probe(capability, identity);
    if (result.kind !== "unreachable") {
      return result;
    }
    if (attempt + 1 < RETRY_ATTEMPTS) {
      await sleep(RETRY_DELAY_MS);
    }
  }
  return { kind: "unreachable" };
}

function startedServerFailureMessage(result: ReviewProbeResult): string {
  if (result.kind === "unauthorized") {
    return "Pointbreak Review rejected its startup credential.";
  }
  if (result.kind === "version-incompatible") {
    return "Pointbreak Review is incompatible with this extension.";
  }
  if (result.kind === "identity-mismatch") {
    return "Pointbreak Review belongs to a different review target.";
  }
  if (result.kind === "protocol") {
    return "Pointbreak Review returned an invalid startup response.";
  }
  return "Pointbreak Review did not become available.";
}

function trimTrailingSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
