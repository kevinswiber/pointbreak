import { afterEach, describe, expect, it } from "vitest";
import type { ResolvedBinary } from "../src/binary";
import {
  type CaptureOptions,
  captureArgs,
  type ExecFn,
  type ObservationOptions,
  observationArgs,
  PointbreakCli,
} from "../src/cli";
import { ATTENTION_JSON, REVISION_LIST_JSON, VERSION_JSON } from "./fixtures";

const binary: ResolvedBinary = { path: "/bin/pointbreak", source: "setting" };

afterEach(() => {
  delete process.env.POINTBREAK_ACTOR_ID;
  delete process.env.POINTBREAK_FORMAT;
});

it("strips actor and format overrides from every spawn env", async () => {
  process.env.POINTBREAK_ACTOR_ID = "actor:agent:leaked";
  process.env.POINTBREAK_FORMAT = "text";
  const seen: NodeJS.ProcessEnv[] = [];
  const exec: ExecFn = async (_file, args, opts) => {
    seen.push(opts.env);
    const stdout = args[0] === "version" ? VERSION_JSON : ATTENTION_JSON;
    return { stdout, stderr: "", exitCode: 0 };
  };
  const cli = new PointbreakCli(binary, exec);

  await cli.attentionList("/repo");

  expect(seen).toHaveLength(2);
  for (const env of seen) {
    expect(env).not.toHaveProperty("POINTBREAK_ACTOR_ID");
    expect(env).not.toHaveProperty("POINTBREAK_FORMAT");
  }
});

it("sanitizes writes through the same spawn seam", async () => {
  process.env.POINTBREAK_ACTOR_ID = "actor:agent:leaked";
  process.env.POINTBREAK_FORMAT = "text";
  const seen: NodeJS.ProcessEnv[] = [];
  const exec: ExecFn = async (_file, args, opts) => {
    seen.push(opts.env);
    return {
      stdout:
        args[0] === "version"
          ? VERSION_JSON
          : JSON.stringify({
              schema: "pointbreak.review-capture",
              version: 1,
              revision: { id: "rev:sha256:1234" },
              diagnostics: [],
            }),
      stderr: "",
      exitCode: 0,
    };
  };
  const cli = new PointbreakCli(binary, exec);

  await cli.capture("/repo", {
    choice: "worktree",
    includeUntracked: false,
    allowEmpty: false,
  });

  expect(seen).toHaveLength(2);
  expect(seen.every((env) => env.POINTBREAK_ACTOR_ID === undefined)).toBe(true);
  expect(seen.every((env) => env.POINTBREAK_FORMAT === undefined)).toBe(true);
});

describe("captureArgs", () => {
  const args = (opts: CaptureOptions) => captureArgs(opts);

  it("maps choices to exact flags", () => {
    expect(
      args({ choice: "worktree", includeUntracked: true, allowEmpty: false }),
    ).toEqual(["capture", "--include-untracked"]);
    expect(
      args({ choice: "staged", includeUntracked: false, allowEmpty: false }),
    ).toEqual(["capture", "--staged"]);
    expect(
      args({
        choice: "unstaged",
        includeUntracked: true,
        allowEmpty: true,
        summary: "Describe the captured change",
        supersedes: ["rev:sha256:two", "rev:sha256:one"],
      }),
    ).toEqual([
      "capture",
      "--unstaged",
      "--include-untracked",
      "--allow-empty",
      "--summary",
      "Describe the captured change",
      "--supersedes",
      "rev:sha256:two",
      "--supersedes",
      "rev:sha256:one",
    ]);
  });

  it("rejects including untracked files with a staged capture", () => {
    expect(() =>
      args({ choice: "staged", includeUntracked: true, allowEmpty: false }),
    ).toThrow(/staged/i);
  });
});

describe("observationArgs", () => {
  const options: ObservationOptions = {
    revisionId: "rev:sha256:one",
    track: "human:local",
    title: "Check this range",
    target: {
      kind: "range",
      file: "src/lib.rs",
      side: "new",
      startLine: 7,
      endLine: 9,
    },
  };

  it("always emits the complete explicit range target", () => {
    expect(observationArgs(options)).toEqual([
      "observation",
      "add",
      "--exact-revision",
      "rev:sha256:one",
      "--track",
      "human:local",
      "--title",
      "Check this range",
      "--file",
      "src/lib.rs",
      "--side",
      "new",
      "--start-line",
      "7",
      "--end-line",
      "9",
    ]);
    expect(observationArgs(options)).not.toContain("--actor");
  });

  it("decodes observation-add through the cold handshake", async () => {
    const calls: string[][] = [];
    const exec: ExecFn = async (_file, args) => {
      calls.push(args);
      return {
        stdout:
          args[0] === "version"
            ? VERSION_JSON
            : JSON.stringify({
                schema: "pointbreak.review-observation-add",
                version: 1,
                revisionId: "rev:sha256:one",
                observationId: "obs:sha256:one",
                eventId: "evt:sha256:one",
                trackId: "human:local",
                target: { kind: "range" },
                diagnostics: [],
              }),
        stderr: "",
        exitCode: 0,
      };
    };
    const cli = new PointbreakCli(binary, exec);

    await expect(cli.addObservation("/repo", options)).resolves.toMatchObject({
      schema: "pointbreak.review-observation-add",
      observationId: "obs:sha256:one",
    });
    expect(calls).toEqual([["version"], observationArgs(options)]);
  });

  it("emits a revision target with Markdown body only on stdin", async () => {
    const calls: Array<{ args: string[]; stdin?: Uint8Array }> = [];
    const revisionOptions: ObservationOptions = {
      revisionId: "rev:sha256:one",
      track: "human:local",
      title: "VS Code Problems snapshot",
      target: { kind: "revision" },
      body: "# Problems snapshot\n\nExact bytes.\n",
      bodyContentType: "text/markdown",
    };
    const exec: ExecFn = async (_file, args, opts) => {
      calls.push({ args, stdin: opts.stdin });
      return {
        stdout:
          args[0] === "version"
            ? VERSION_JSON
            : JSON.stringify({
                schema: "pointbreak.review-observation-add",
                version: 1,
                revisionId: "rev:sha256:one",
                observationId: "obs:sha256:problems",
                eventId: "evt:sha256:problems",
                trackId: "human:local",
                target: { kind: "revision" },
                diagnostics: [],
              }),
        stderr: "",
        exitCode: 0,
      };
    };
    const cli = new PointbreakCli(binary, exec);

    await cli.addObservation("/repo", revisionOptions);

    expect(observationArgs(revisionOptions)).toEqual([
      "observation",
      "add",
      "--exact-revision",
      "rev:sha256:one",
      "--track",
      "human:local",
      "--title",
      "VS Code Problems snapshot",
      "--body-stdin",
      "--body-content-type",
      "text/markdown",
    ]);
    expect(observationArgs(revisionOptions)).not.toContain("--file");
    expect(calls.map(({ args }) => args)).toEqual([
      ["version"],
      observationArgs(revisionOptions),
    ]);
    expect(calls.map(({ stdin }) => bytes(stdin))).toEqual([
      undefined,
      revisionOptions.body,
    ]);
  });
});

it("uses typed exact-write documents, sanitized env, and stdin bytes", async () => {
  process.env.POINTBREAK_ACTOR_ID = "actor:agent:leaked";
  process.env.POINTBREAK_FORMAT = "text";
  const calls: Array<{
    args: string[];
    opts: { cwd: string; env: NodeJS.ProcessEnv; stdin?: Uint8Array };
  }> = [];
  const exec: ExecFn = async (_file, args, opts) => {
    calls.push({ args, opts });
    return {
      stdout: args[0] === "version" ? VERSION_JSON : documentFor(args),
      stderr: "",
      exitCode: 0,
    };
  };
  const cli = new PointbreakCli(binary, exec);

  await expect(cli.identityWhoami("/repo")).resolves.toMatchObject({
    actorId: "actor:git-email:human@example.com",
  });
  await cli.showAssessments("/repo", {
    revisionId: "rev:sha256:one",
    track: "human:local",
  });
  await cli.showAssessments("/repo", {
    revisionId: "rev:sha256:one",
  });
  await cli.addAssessment("/repo", {
    revisionId: "rev:sha256:one",
    track: "human:local",
    assessment: "needs-changes",
    summary: "Fix the boundary.",
    replaces: ["assess:sha256:one", "assess:sha256:two"],
  });
  await cli.respondInputRequest("/repo", {
    inputRequestId: "input-request:sha256:one",
    outcome: "approved",
    reason: "Approved after review.",
  });
  await cli.addValidation("/repo", {
    revisionId: "rev:sha256:one",
    track: "human:local",
    checkName: "vscode-task:npm:workspace:test",
    status: "passed",
    command: "npm test",
    exitCode: 0,
    startedAt: "2026-07-15T20:00:00.000Z",
    completedAt: "2026-07-15T20:00:01.250Z",
    trigger: "manual",
    summary: "The selected task passed.",
  });

  expect(calls.map(({ args }) => args)).toEqual([
    ["version"],
    ["identity", "whoami"],
    [
      "assessment",
      "show",
      "--exact-revision",
      "rev:sha256:one",
      "--track",
      "human:local",
      "--all",
      "--include-summary",
    ],
    [
      "assessment",
      "show",
      "--exact-revision",
      "rev:sha256:one",
      "--all",
      "--include-summary",
    ],
    [
      "assessment",
      "add",
      "--exact-revision",
      "rev:sha256:one",
      "--track",
      "human:local",
      "--assessment",
      "needs-changes",
      "--summary-stdin",
      "--replaces",
      "assess:sha256:one",
      "--replaces",
      "assess:sha256:two",
    ],
    [
      "input-request",
      "respond",
      "input-request:sha256:one",
      "--outcome",
      "approved",
      "--reason-stdin",
    ],
    [
      "validation",
      "add",
      "--exact-revision",
      "rev:sha256:one",
      "--track",
      "human:local",
      "--check-name",
      "vscode-task:npm:workspace:test",
      "--status",
      "passed",
      "--command",
      "npm test",
      "--exit-code",
      "0",
      "--started-at",
      "2026-07-15T20:00:00.000Z",
      "--completed-at",
      "2026-07-15T20:00:01.250Z",
      "--trigger",
      "manual",
      "--summary-stdin",
    ],
  ]);
  expect(calls.map(({ opts }) => bytes(opts.stdin))).toEqual([
    undefined,
    undefined,
    undefined,
    undefined,
    "Fix the boundary.",
    "Approved after review.",
    "The selected task passed.",
  ]);
  expect(
    calls.every(
      ({ opts }) =>
        opts.env.POINTBREAK_ACTOR_ID === undefined &&
        opts.env.POINTBREAK_FORMAT === undefined,
    ),
  ).toBe(true);
  expect(JSON.stringify(calls.map(({ args }) => args))).not.toContain(
    "Approved after review.",
  );
});

it("rejects a document whose schema does not match the requested command", async () => {
  const exec: ExecFn = async (_file, args) => ({
    stdout:
      args[0] === "version"
        ? VERSION_JSON
        : JSON.stringify({ schema: "wrong.schema", version: 1 }),
    stderr: "",
    exitCode: 0,
  });
  const cli = new PointbreakCli(binary, exec);

  await expect(cli.storeStatus("/repo")).rejects.toThrow(/store-status/);
});

it("decodes the live revision-list entries shape", async () => {
  const calls: string[][] = [];
  const exec: ExecFn = async (_file, args) => {
    calls.push(args);
    return {
      stdout: args[0] === "version" ? VERSION_JSON : REVISION_LIST_JSON,
      stderr: "",
      exitCode: 0,
    };
  };
  const cli = new PointbreakCli(binary, exec);

  const document = await cli.revisionList("/repo", {
    filter: "-is:superseded",
  });

  expect(document.entries[0]).toMatchObject({
    revisionId: "rev:sha256:9442bfeb",
    mergeStatus: "merged",
  });
  expect(document.revisionCount).toBe(1);
  expect(calls).toEqual([
    ["version"],
    ["revision", "list", "--filter=-is:superseded"],
  ]);
});

function bytes(value: Uint8Array | undefined): string | undefined {
  return value === undefined ? undefined : Buffer.from(value).toString("utf8");
}

function documentFor(args: string[]): string {
  const command = `${args[0]} ${args[1] ?? ""}`;
  const documents: Record<string, unknown> = {
    "identity whoami": {
      schema: "pointbreak.identity-whoami",
      version: 1,
      actorId: "actor:git-email:human@example.com",
    },
    "assessment show": {
      schema: "pointbreak.review-assessment-show",
      version: 1,
      revisionId: "rev:sha256:one",
      filters: {
        trackId: "human:local",
        all: true,
        includeSummary: true,
      },
      current: { status: "unassessed" },
      assessments: [],
      diagnostics: [],
    },
    "assessment add": {
      schema: "pointbreak.review-assessment-add",
      version: 1,
      revisionId: "rev:sha256:one",
      assessmentId: "assess:sha256:new",
      eventId: "evt:sha256:assessment",
      trackId: "human:local",
      target: { kind: "revision", revisionId: "rev:sha256:one" },
      assessment: "needs_changes",
      diagnostics: [],
    },
    "input-request respond": {
      schema: "pointbreak.review-input-request-respond",
      version: 1,
      inputRequestId: "input-request:sha256:one",
      inputRequestResponseId: "input-request-response:sha256:one",
      eventId: "evt:sha256:response",
      outcome: "approved",
      diagnostics: [],
    },
    "validation add": {
      schema: "pointbreak.review-validation-add",
      version: 1,
      revisionId: "rev:sha256:one",
      validationCheckId: "validation:sha256:one",
      eventId: "evt:sha256:validation",
      trackId: "human:local",
      target: { kind: "revision", revisionId: "rev:sha256:one" },
      status: "passed",
      diagnostics: [],
    },
  };
  return JSON.stringify(documents[command]);
}
