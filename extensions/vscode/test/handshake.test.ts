import { expect, it } from "vitest";
import type { ResolvedBinary } from "../src/binary";
import {
  type ExecFn,
  PointbreakCli,
  REQUIRED_DOCUMENTS,
  verifyHandshake,
} from "../src/cli";
import { VERSION_DOC } from "./fixtures";

const binary: ResolvedBinary = { path: "/bin/shore", source: "setting" };

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

it("fails closed when the CLI minor is incompatible", () => {
  const result = verifyHandshake({ ...VERSION_DOC, cliVersion: "0.7.0" });

  expect(result.ok).toBe(false);
  expect(result.ok === false && result.reason).toMatch(/0\.7\.0/);
});

it("fails closed when the version document body is malformed", () => {
  const result = verifyHandshake({
    schema: "pointbreak.version",
    version: 1,
    diagnostics: [],
  } as unknown as typeof VERSION_DOC);

  expect(result.ok).toBe(false);
});

it("fails closed when the binary does not speak shore version", async () => {
  const exec: ExecFn = async () => ({
    stdout: "",
    stderr: "unknown subcommand 'version'",
    exitCode: 2,
  });
  const cli = new PointbreakCli(binary, exec);

  await expect(cli.version("/repo")).rejects.toThrow(/version|too old/i);
});
