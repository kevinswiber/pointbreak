import { expect, it } from "vitest";
import type { ResolvedBinary } from "../src/binary";
import {
  type BuildIdentityV1,
  type ExecFn,
  PointbreakCli,
  REQUIRED_DOCUMENTS,
  verifyHandshake,
} from "../src/cli";
import { VERSION_DOC, VERSION_JSON } from "./fixtures";

const binary: ResolvedBinary = {
  path: "/bin/arbitrarily-named-review-cli",
  source: "setting",
};

it("pins the exact extension document handshake", () => {
  expect(REQUIRED_DOCUMENTS).toEqual({
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
  });
});

it("fails closed when a required document version mismatches", () => {
  const doc = {
    ...VERSION_DOC,
    documents: {
      ...VERSION_DOC.documents,
      "pointbreak.attention-list": 2,
    },
  };

  const result = verifyHandshake(doc);

  expect(result.ok).toBe(false);
  expect(result.ok === false && result.reason).toMatch(/attention-list/);
});

it("accepts the exact Pointbreak 0.7 handshake through an arbitrary path", () => {
  expect(verifyHandshake(VERSION_DOC)).toEqual({
    ok: true,
    cliVersion: "0.7.0",
  });
});

it("tolerates additive build identity and unrelated soft-shell fields", () => {
  const build: BuildIdentityV1 = {
    source: "git",
    commit: "d2bc01650076314897bb8c30ba57623640c0d257",
    describe: "v0.7.0-34-gd2bc016",
    dirty: false,
  };

  expect(
    verifyHandshake({ ...VERSION_DOC, build, futureSoftShellField: true }),
  ).toEqual({ ok: true, cliVersion: "0.7.0" });
});

it("still fails closed on an unknown version envelope with additive fields", () => {
  const result = verifyHandshake({
    ...VERSION_DOC,
    version: 2,
    futureSoftShellField: true,
  } as unknown as typeof VERSION_DOC);

  expect(result.ok).toBe(false);
  expect(result.ok === false && result.reason).toMatch(/version 1/);
});

it("executes an arbitrary configured path only through the exact handshake", async () => {
  const invocations: Array<{ file: string; args: string[] }> = [];
  const exec: ExecFn = async (file, args) => {
    invocations.push({ file, args });
    return { stdout: VERSION_JSON, stderr: "", exitCode: 0 };
  };
  const cli = new PointbreakCli(binary, exec);

  await expect(cli.version("/repo")).resolves.toEqual(VERSION_DOC);
  expect(invocations).toEqual([{ file: binary.path, args: ["version"] }]);
});

it("fails closed when the CLI minor is incompatible", () => {
  const result = verifyHandshake({ ...VERSION_DOC, cliVersion: "0.8.0" });

  expect(result.ok).toBe(false);
  expect(result.ok === false && result.reason).toMatch(/0\.8\.0/);
});

it("fails closed when the document map omits a required member", () => {
  const documents = { ...VERSION_DOC.documents };
  delete documents["pointbreak.store-status"];
  const result = verifyHandshake({
    ...VERSION_DOC,
    documents,
  });

  expect(result.ok).toBe(false);
  expect(result.ok === false && result.reason).toMatch(/store-status|missing/i);
});

it("fails closed when the version document body is malformed", () => {
  const result = verifyHandshake({
    schema: "pointbreak.version",
    version: 1,
    diagnostics: [],
  } as unknown as typeof VERSION_DOC);

  expect(result.ok).toBe(false);
});

it("fails closed with Pointbreak-only wording when version is unavailable", async () => {
  const exec: ExecFn = async () => ({
    stdout: "",
    stderr: "unknown subcommand 'version'",
    exitCode: 2,
  });
  const cli = new PointbreakCli(binary, exec);

  await expect(cli.version("/repo")).rejects.toThrow(
    /pointbreak version failed/i,
  );
});
