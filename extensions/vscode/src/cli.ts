import { execFile } from "node:child_process";
import { POINTBREAK_CLI_NAME, type ResolvedBinary } from "./binary";

export type ExecFn = (
  file: string,
  args: string[],
  opts: { cwd: string; env: NodeJS.ProcessEnv; stdin?: Uint8Array },
) => Promise<{ stdout: string; stderr: string; exitCode: number }>;

export interface DiagnosticDocument {
  schema: string;
  version: number;
  diagnostics?: unknown[];
}

export interface BuildIdentityV1 {
  source: "git" | "package";
  commit: string | null;
  describe: string;
  dirty: boolean;
}

export interface VersionDoc extends DiagnosticDocument {
  schema: "pointbreak.version";
  version: 1;
  cliVersion: string;
  build?: BuildIdentityV1;
  documents: Record<string, number>;
  [field: string]: unknown;
}

export interface ReviewFactTarget {
  kind?: string;
  filePath?: string;
  startLine?: number;
  endLine?: number;
  side?: string;
  [field: string]: unknown;
}

export interface ReviewObservationDoc {
  id?: string;
  trackId?: string;
  title?: string;
  body?: string;
  bodyContentType?: string;
  tags?: string[];
  target?: ReviewFactTarget;
  status?: string;
  [field: string]: unknown;
}

export interface ReviewInputRequestDoc {
  id?: string;
  trackId?: string;
  title?: string;
  body?: string;
  bodyContentType?: string;
  mode?: string;
  reasonCode?: string;
  target?: ReviewFactTarget;
  status?: string;
  [field: string]: unknown;
}

export interface ReviewAssessmentDoc {
  id?: string;
  trackId?: string;
  assessment?: string;
  summary?: string;
  summaryContentType?: string;
  target?: ReviewFactTarget;
  status?: string;
  [field: string]: unknown;
}

export interface RevisionDoc extends DiagnosticDocument {
  schema: "pointbreak.review-revision";
  version: 2;
  revision: {
    id: string;
    summary?: string;
    objectId?: string;
    objectArtifactContentHash?: string;
    [field: string]: unknown;
  };
  observations: ReviewObservationDoc[];
  inputRequests: ReviewInputRequestDoc[];
  assessments: ReviewAssessmentDoc[];
  [field: string]: unknown;
}

export interface ReviewSnapshotRow {
  kind: string;
  old_line: number | null;
  new_line: number | null;
  text: string;
  tokens?: unknown[];
  emphasis?: unknown[];
  [field: string]: unknown;
}

export interface ReviewSnapshotHunk {
  id: string;
  header: string;
  rows: ReviewSnapshotRow[];
  [field: string]: unknown;
}

export interface ReviewSnapshotFile {
  id: string;
  hunks: ReviewSnapshotHunk[];
  [field: string]: unknown;
}

export interface ReviewSnapshotDoc {
  schema: "pointbreak.review-snapshot";
  version: 1;
  contentHash: string;
  snapshot: {
    review_id: string;
    object_id: string;
    files: ReviewSnapshotFile[];
    [field: string]: unknown;
  };
}

export interface InspectFreshnessDoc {
  schema: "pointbreak.inspect-freshness";
  version: 1;
  eventCount: number;
  commitGraphStamp?: string;
}

export interface InspectStartupDoc {
  schema: "pointbreak.inspect-startup";
  version: 1;
  host: string;
  port: number;
  token: string;
}

export interface ObservationAddDoc extends DiagnosticDocument {
  schema: "pointbreak.review-observation-add";
  version: 1;
  revisionId: string;
  observationId: string;
  eventId: string;
  trackId: string;
  target: ReviewFactTarget;
  bodyContentHash?: string;
}

export type AttentionTier = "primary" | "secondary";

export interface AttentionFreshness {
  state: "current" | "superseded";
  supersededBy?: string[];
}

interface AttentionItemBase {
  id: string;
  tier: AttentionTier;
  revisionId?: string;
  freshness: AttentionFreshness;
  observedAt: string;
}

export interface OpenInputRequestAttentionItem extends AttentionItemBase {
  kind: "open_input_request";
  inputRequestId: string;
  mode: "operative" | "advisory";
  reasonCode: string;
  title: string;
  trackId: string;
  openedBy: string;
}

export interface FollowUpOutstandingAttentionItem extends AttentionItemBase {
  kind: "follow_up_outstanding";
  assessmentId: string;
  trackId: string;
  recordedBy: string;
  openInputRequestIds: string[];
}

export interface AttentionAssessmentRecord {
  assessmentId: string;
  assessment: string;
  trackId: string;
  recordedBy: string;
  recordedAt: string;
  relatedObservationIds?: string[];
  relatedInputRequestIds?: string[];
}

export interface AmbiguousAssessmentAttentionItem extends AttentionItemBase {
  kind: "ambiguous_assessment";
  assessments: AttentionAssessmentRecord[];
}

export interface CompetingHeadsAttentionItem extends AttentionItemBase {
  kind: "competing_heads";
  headRevisionIds: string[];
  threadRevisionCount: number;
}

export interface StaleAssessmentAttentionItem extends AttentionItemBase {
  kind: "stale_assessment";
  assessmentId: string;
  assessment: string;
  trackId: string;
  recordedBy: string;
  headRevisionIds?: string[];
}

export interface FailedValidationAttentionItem extends AttentionItemBase {
  kind: "failed_validation";
  validationCheckId: string;
  checkName: string;
  status: "failed" | "errored";
  exitCode?: number;
  trackId: string;
  recordedBy: string;
  logArtifactContentHashes?: string[];
}

export type AttentionItem =
  | OpenInputRequestAttentionItem
  | FollowUpOutstandingAttentionItem
  | AmbiguousAssessmentAttentionItem
  | CompetingHeadsAttentionItem
  | StaleAssessmentAttentionItem
  | FailedValidationAttentionItem;

export type InputRequestResponseAttentionItem =
  | OpenInputRequestAttentionItem
  | FollowUpOutstandingAttentionItem;

export interface AttentionListDoc extends DiagnosticDocument {
  schema: "pointbreak.attention-list";
  version: 1;
  items: AttentionItem[];
}

export interface RevisionListDoc extends DiagnosticDocument {
  schema: "pointbreak.review-revision-list";
  version: 1;
  entries: Array<{
    revisionId: string;
    summary?: string;
    capturedAt: string;
    mergeStatus: string;
    [field: string]: unknown;
  }>;
  revisionCount: number;
  eventCount: number;
  eventSetHash: string;
}

export interface RevisionListOptions {
  filter?: string;
}

export interface StoreStatusDoc extends DiagnosticDocument {
  schema: "pointbreak.store-status";
  version: 1;
  storeIdentity: string;
  contextIdentity: string;
  repositoryFamilyRef?: string;
  storeRef?: string;
  inventory: {
    eventCount: number;
    artifactCount: number;
    revisionObjects: unknown[];
    [field: string]: unknown;
  };
}

export interface CaptureDoc extends DiagnosticDocument {
  schema: "pointbreak.review-capture";
  version: 1;
  revision: { id: string; summary?: string };
}

export type CaptureChoice = "worktree" | "staged" | "unstaged";

export interface CaptureOptions {
  choice: CaptureChoice;
  includeUntracked: boolean;
  allowEmpty: boolean;
  summary?: string;
  supersedes?: readonly string[];
}

interface ObservationOptionsBase {
  revisionId: string;
  track: string;
  title: string;
  body?: string;
  bodyContentType?: "text/plain" | "text/markdown";
}

export interface RevisionObservationOptions extends ObservationOptionsBase {
  target: { kind: "revision" };
}

export interface RangeObservationOptions extends ObservationOptionsBase {
  target: {
    kind: "range";
    file: string;
    side: "old" | "new";
    startLine: number;
    endLine: number;
  };
}

export type ObservationOptions =
  | RevisionObservationOptions
  | RangeObservationOptions;

export interface IdentityWhoamiDoc extends DiagnosticDocument {
  schema: "pointbreak.identity-whoami";
  version: 1;
  actorId: string;
}

export type AssessmentValue =
  | "accepted"
  | "accepted-with-follow-up"
  | "needs-changes"
  | "needs-clarification";

export interface AssessmentView {
  id: string;
  trackId: string;
  target: ReviewFactTarget;
  assessment: string;
  status: string;
  createdAt?: string;
  writer: { actorId: string; [field: string]: unknown };
  [field: string]: unknown;
}

export interface AssessmentShowDoc extends DiagnosticDocument {
  schema: "pointbreak.review-assessment-show";
  version: 1;
  revisionId: string;
  filters: {
    trackId?: string;
    all: boolean;
    includeSummary: boolean;
  };
  current: {
    status: string;
    assessmentId?: string;
    assessment?: string;
    candidates?: AssessmentView[];
  };
  assessments: AssessmentView[];
}

export interface AssessmentAddDoc extends DiagnosticDocument {
  schema: "pointbreak.review-assessment-add";
  version: 1;
  revisionId: string;
  assessmentId: string;
  eventId: string;
  trackId: string;
  target: ReviewFactTarget;
  assessment: string;
}

export interface AssessmentShowOptions {
  revisionId: string;
  track?: string;
}

export interface AssessmentAddOptions {
  revisionId: string;
  track: string;
  assessment: AssessmentValue;
  summary?: string;
  replaces?: readonly string[];
}

export type InputRequestOutcome =
  | "approved"
  | "rejected"
  | "dismissed"
  | "superseded"
  | "abandoned";

export interface InputRequestRespondDoc extends DiagnosticDocument {
  schema: "pointbreak.review-input-request-respond";
  version: 1;
  inputRequestId: string;
  inputRequestResponseId: string;
  eventId: string;
  outcome: string;
}

export interface InputRequestRespondOptions {
  inputRequestId: string;
  outcome: InputRequestOutcome;
  reason?: string;
}

export type ValidationStatus = "passed" | "failed" | "errored" | "skipped";
export type ValidationTrigger = "manual" | "push" | "pull-request";

export interface ValidationAddDoc extends DiagnosticDocument {
  schema: "pointbreak.review-validation-add";
  version: 1;
  revisionId: string;
  validationCheckId: string;
  eventId: string;
  trackId: string;
  target: ReviewFactTarget;
  status: ValidationStatus;
}

export interface ValidationAddOptions {
  revisionId: string;
  track: string;
  checkName: string;
  status: ValidationStatus;
  command?: string;
  exitCode?: number;
  startedAt?: string;
  completedAt?: string;
  trigger?: ValidationTrigger;
  summary?: string;
}

export const REQUIRED_DOCUMENTS: Record<string, number> = {
  "pointbreak.version": 1,
  "pointbreak.attention-list": 1,
  "pointbreak.identity-whoami": 1,
  "pointbreak.review-assessment-add": 1,
  "pointbreak.review-assessment-show": 1,
  "pointbreak.review-revision-list": 1,
  "pointbreak.review-revision": 2,
  "pointbreak.review-capture": 1,
  "pointbreak.review-input-request-respond": 1,
  "pointbreak.review-observation-add": 1,
  "pointbreak.review-snapshot": 1,
  "pointbreak.review-validation-add": 1,
  "pointbreak.inspect-freshness": 1,
  "pointbreak.inspect-startup": 1,
  "pointbreak.store-status": 1,
};

// This extension targets the CLI minor that first provides the version handshake.
export const COMPATIBLE_CLI_RANGE = "0.7";

export type HandshakeResult =
  | { ok: true; cliVersion: string }
  | { ok: false; reason: string };

export class PointbreakCliError extends Error {
  constructor(
    message: string,
    readonly exitCode?: number,
    readonly stderr = "",
  ) {
    super(message);
    this.name = "PointbreakCliError";
  }
}

export function sanitizedEnv(
  base: NodeJS.ProcessEnv = process.env,
): NodeJS.ProcessEnv {
  const env = { ...base };
  delete env.POINTBREAK_ACTOR_ID;
  delete env.POINTBREAK_FORMAT;
  return env;
}

export function verifyHandshake(doc: VersionDoc): HandshakeResult {
  if (doc.schema !== "pointbreak.version" || doc.version !== 1) {
    return {
      ok: false,
      reason: "the CLI does not provide pointbreak.version version 1",
    };
  }

  if (
    typeof doc.cliVersion !== "string" ||
    typeof doc.documents !== "object" ||
    doc.documents === null
  ) {
    return {
      ok: false,
      reason: "the CLI returned an invalid pointbreak.version document",
    };
  }

  const minor = doc.cliVersion
    .match(/^(\d+)\.(\d+)(?:\.|$)/)
    ?.slice(1, 3)
    .join(".");
  if (minor !== COMPATIBLE_CLI_RANGE) {
    return {
      ok: false,
      reason: `CLI ${doc.cliVersion} is outside compatible minor ${COMPATIBLE_CLI_RANGE}`,
    };
  }

  for (const [schema, version] of Object.entries(REQUIRED_DOCUMENTS)) {
    if (doc.documents[schema] !== version) {
      return {
        ok: false,
        reason: `${schema} version ${String(doc.documents[schema] ?? "missing")} is incompatible; expected ${version}`,
      };
    }
  }

  return { ok: true, cliVersion: doc.cliVersion };
}

export class PointbreakCli {
  private handshake?: Promise<void>;

  constructor(
    private readonly binary: ResolvedBinary,
    private readonly exec: ExecFn = defaultExec,
  ) {}

  async version(repo: string): Promise<VersionDoc> {
    const doc = await this.readVersion(repo);
    this.handshake = Promise.resolve();
    return doc;
  }

  async attentionList(repo: string): Promise<AttentionListDoc> {
    return this.runDocument(
      repo,
      ["attention", "list"],
      "pointbreak.attention-list",
    );
  }

  async revisionList(
    repo: string,
    options: RevisionListOptions = {},
  ): Promise<RevisionListDoc> {
    return this.runDocument(
      repo,
      revisionListArgs(options),
      "pointbreak.review-revision-list",
    );
  }

  async storeStatus(repo: string): Promise<StoreStatusDoc> {
    return this.runDocument(
      repo,
      ["store", "status"],
      "pointbreak.store-status",
    );
  }

  async capture(repo: string, opts: CaptureOptions): Promise<CaptureDoc> {
    return this.runDocument(
      repo,
      captureArgs(opts),
      "pointbreak.review-capture",
    );
  }

  async addObservation(
    repo: string,
    options: ObservationOptions,
  ): Promise<ObservationAddDoc> {
    const command = observationCommand(options);
    return this.runDocument(
      repo,
      command.args,
      "pointbreak.review-observation-add",
      command.stdin,
    );
  }

  async identityWhoami(repo: string): Promise<IdentityWhoamiDoc> {
    return this.runDocument(
      repo,
      ["identity", "whoami"],
      "pointbreak.identity-whoami",
    );
  }

  async showAssessments(
    repo: string,
    options: AssessmentShowOptions,
  ): Promise<AssessmentShowDoc> {
    return this.runDocument(
      repo,
      assessmentShowArgs(options),
      "pointbreak.review-assessment-show",
    );
  }

  async addAssessment(
    repo: string,
    options: AssessmentAddOptions,
  ): Promise<AssessmentAddDoc> {
    const command = assessmentAddCommand(options);
    return this.runDocument(
      repo,
      command.args,
      "pointbreak.review-assessment-add",
      command.stdin,
    );
  }

  async respondInputRequest(
    repo: string,
    options: InputRequestRespondOptions,
  ): Promise<InputRequestRespondDoc> {
    const command = inputRequestRespondCommand(options);
    return this.runDocument(
      repo,
      command.args,
      "pointbreak.review-input-request-respond",
      command.stdin,
    );
  }

  async addValidation(
    repo: string,
    options: ValidationAddOptions,
  ): Promise<ValidationAddDoc> {
    const command = validationAddCommand(options);
    return this.runDocument(
      repo,
      command.args,
      "pointbreak.review-validation-add",
      command.stdin,
    );
  }

  private async ensureHandshake(repo: string): Promise<void> {
    this.handshake ??= this.readVersion(repo).then(() => undefined);
    return this.handshake;
  }

  private async readVersion(repo: string): Promise<VersionDoc> {
    let doc: VersionDoc;
    try {
      doc = await this.executeDocument(repo, ["version"], "pointbreak.version");
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw new PointbreakCliError(
        `Pointbreak CLI is too old or incompatible: ${POINTBREAK_CLI_NAME} version failed (${detail})`,
        error instanceof PointbreakCliError ? error.exitCode : undefined,
        error instanceof PointbreakCliError ? error.stderr : "",
      );
    }

    const result = verifyHandshake(doc);
    if (!result.ok) {
      throw new PointbreakCliError(
        `${result.reason}. Update the extension or the Pointbreak CLI.`,
      );
    }
    return doc;
  }

  private async runDocument<T extends DiagnosticDocument>(
    repo: string,
    args: string[],
    schema: string,
    stdin?: Uint8Array,
  ): Promise<T> {
    await this.ensureHandshake(repo);
    return this.executeDocument(repo, args, schema, stdin);
  }

  private async executeDocument<T extends DiagnosticDocument>(
    repo: string,
    args: string[],
    schema: string,
    stdin?: Uint8Array,
  ): Promise<T> {
    const result = await this.exec(this.binary.path, args, {
      cwd: repo,
      env: sanitizedEnv(),
      stdin,
    });
    if (result.exitCode !== 0) {
      const detail = result.stderr.trim() || "no error output";
      throw new PointbreakCliError(
        `${POINTBREAK_CLI_NAME} ${args.join(" ")} failed: ${detail}`,
        result.exitCode,
        result.stderr,
      );
    }

    let parsed: unknown;
    try {
      parsed = JSON.parse(result.stdout);
    } catch {
      throw new PointbreakCliError(
        `${POINTBREAK_CLI_NAME} ${args.join(" ")} returned invalid JSON`,
      );
    }
    if (!isDocument(parsed)) {
      throw new PointbreakCliError(
        `${POINTBREAK_CLI_NAME} ${args.join(" ")} returned an invalid document`,
      );
    }

    const expectedVersion = REQUIRED_DOCUMENTS[schema];
    if (parsed.schema !== schema || parsed.version !== expectedVersion) {
      throw new PointbreakCliError(
        `Expected ${schema} version ${expectedVersion}, received ${parsed.schema} version ${parsed.version}`,
      );
    }
    return parsed as T;
  }
}

export function captureArgs(opts: CaptureOptions): string[] {
  if (opts.choice === "staged" && opts.includeUntracked) {
    throw new Error("Staged capture cannot include untracked files.");
  }

  const args = ["capture"];
  if (opts.choice === "staged") {
    args.push("--staged");
  } else if (opts.choice === "unstaged") {
    args.push("--unstaged");
  }
  if (opts.includeUntracked) {
    args.push("--include-untracked");
  }
  if (opts.allowEmpty) {
    args.push("--allow-empty");
  }
  if (opts.summary !== undefined) {
    args.push("--summary", opts.summary);
  }
  for (const revisionId of opts.supersedes ?? []) {
    args.push("--supersedes", revisionId);
  }
  return args;
}

function revisionListArgs(options: RevisionListOptions): string[] {
  const args = ["revision", "list"];
  if (options.filter !== undefined) {
    args.push(`--filter=${options.filter}`);
  }
  return args;
}

export function observationArgs(options: ObservationOptions): string[] {
  return observationCommand(options).args;
}

function observationCommand(options: ObservationOptions): CommandInput {
  const args = [
    "observation",
    "add",
    "--exact-revision",
    options.revisionId,
    "--track",
    options.track,
    "--title",
    options.title,
  ];
  if (options.body !== undefined) {
    args.push("--body-stdin");
  }
  if (options.bodyContentType !== undefined) {
    args.push("--body-content-type", options.bodyContentType);
  }
  if (options.target.kind === "range") {
    args.push(
      "--file",
      options.target.file,
      "--side",
      options.target.side,
      "--start-line",
      String(options.target.startLine),
      "--end-line",
      String(options.target.endLine),
    );
  }
  return { args, stdin: textBytes(options.body) };
}

function assessmentShowArgs(options: AssessmentShowOptions): string[] {
  const args = ["assessment", "show", "--exact-revision", options.revisionId];
  if (options.track !== undefined) {
    args.push("--track", options.track);
  }
  args.push("--all", "--include-summary");
  return args;
}

function assessmentAddCommand(options: AssessmentAddOptions): CommandInput {
  const args = [
    "assessment",
    "add",
    "--exact-revision",
    options.revisionId,
    "--track",
    options.track,
    "--assessment",
    options.assessment,
  ];
  if (options.summary !== undefined) {
    args.push("--summary-stdin");
  }
  for (const assessmentId of options.replaces ?? []) {
    args.push("--replaces", assessmentId);
  }
  return { args, stdin: textBytes(options.summary) };
}

function inputRequestRespondCommand(
  options: InputRequestRespondOptions,
): CommandInput {
  const args = [
    "input-request",
    "respond",
    options.inputRequestId,
    "--outcome",
    options.outcome,
  ];
  if (options.reason !== undefined) {
    args.push("--reason-stdin");
  }
  return { args, stdin: textBytes(options.reason) };
}

function validationAddCommand(options: ValidationAddOptions): CommandInput {
  const args = [
    "validation",
    "add",
    "--exact-revision",
    options.revisionId,
    "--track",
    options.track,
    "--check-name",
    options.checkName,
    "--status",
    options.status,
  ];
  if (options.command !== undefined) {
    args.push("--command", options.command);
  }
  if (options.exitCode !== undefined) {
    args.push("--exit-code", String(options.exitCode));
  }
  if (options.startedAt !== undefined) {
    args.push("--started-at", options.startedAt);
  }
  if (options.completedAt !== undefined) {
    args.push("--completed-at", options.completedAt);
  }
  if (options.trigger !== undefined) {
    args.push("--trigger", options.trigger);
  }
  if (options.summary !== undefined) {
    args.push("--summary-stdin");
  }
  return { args, stdin: textBytes(options.summary) };
}

interface CommandInput {
  args: string[];
  stdin?: Uint8Array;
}

function textBytes(value: string | undefined): Uint8Array | undefined {
  return value === undefined ? undefined : Buffer.from(value, "utf8");
}

const defaultExec: ExecFn = (file, args, opts) =>
  new Promise((resolve) => {
    const child = execFile(
      file,
      args,
      {
        cwd: opts.cwd,
        env: opts.env,
        encoding: "utf8",
        maxBuffer: 1024 * 1024,
        timeout: 30_000,
        windowsHide: true,
      },
      (error, stdout, stderr) => {
        resolve({
          stdout,
          stderr,
          exitCode: commandExitCode(error),
        });
      },
    );
    child.stdin?.end(opts.stdin);
  });

function isDocument(value: unknown): value is DiagnosticDocument {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.schema === "string" &&
    typeof candidate.version === "number"
  );
}

function commandExitCode(error: { code?: string | number } | null): number {
  if (!error) {
    return 0;
  }
  return typeof error.code === "number" ? error.code : 1;
}
