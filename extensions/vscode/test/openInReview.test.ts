import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import type { ResolvedBinary } from "../src/binary";
import type { PointbreakCli } from "../src/cli";
import {
  probeReview,
  type ReviewIdentity,
  type ReviewProbeResult,
  reviewDeepLink,
  runOpenInReviewCommand,
} from "../src/commands/openInReview";
import type { FetchFn } from "../src/inspectClient";
import type { ReviewCapability } from "../src/reviewTerminal";
import type { TargetResolution } from "../src/targetResolver";
import { VERSION_DOC } from "./fixtures";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => ({
  executeCommand: vi.fn(),
  openLocalhostLinks: false,
  openExternal: vi.fn(),
  reviewUrl: "",
  remoteName: undefined as string | undefined,
  showErrorMessage: vi.fn(),
  showInformationMessage: vi.fn(),
  showQuickPick: vi.fn(),
}));

vi.mock("vscode", () => ({
  commands: { executeCommand: vscodeMocks.executeCommand },
  Uri: { parse: (value: string) => value },
  env: {
    get remoteName() {
      return vscodeMocks.remoteName;
    },
    openExternal: vscodeMocks.openExternal,
  },
  window: {
    showErrorMessage: vscodeMocks.showErrorMessage,
    showInformationMessage: vscodeMocks.showInformationMessage,
    showQuickPick: vscodeMocks.showQuickPick,
  },
  workspace: {
    getConfiguration: (section: string) => ({
      get: (_key: string, fallback: unknown) => {
        if (section === "workbench.browser") {
          return vscodeMocks.openLocalhostLinks;
        }
        if (section === "pointbreak") {
          return vscodeMocks.reviewUrl;
        }
        return fallback;
      },
    }),
  },
}));

const IDENTITY: ReviewIdentity = {
  storeIdentity: "store:sha256:store",
  contextIdentity: "context:sha256:repo",
};
const CAPABILITY: ReviewCapability = {
  origin: "http://127.0.0.1:63831",
  token: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-",
};
const MATCH: ReviewProbeResult = { kind: "match" };
const UNREACHABLE: ReviewProbeResult = { kind: "unreachable" };

beforeEach(() => {
  vscodeMocks.executeCommand.mockReset();
  vscodeMocks.openLocalhostLinks = false;
  vscodeMocks.openExternal.mockReset();
  vscodeMocks.reviewUrl = "";
  vscodeMocks.remoteName = undefined;
  vscodeMocks.showErrorMessage.mockReset();
  vscodeMocks.showInformationMessage.mockReset();
  vscodeMocks.showQuickPick.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("reviewDeepLink", () => {
  it("builds one route-preserving fragment carrying the browser bearer", () => {
    const link = reviewDeepLink(
      CAPABILITY.origin,
      "rev:sha256:abc",
      "attention",
      CAPABILITY.token,
    );

    expect(link).toBe(
      `${CAPABILITY.origin}/#/revision/rev:sha256:abc?lens=attention&token=${CAPABILITY.token}`,
    );
    expect(link.match(/#/g)).toHaveLength(1);
  });
});

describe("probeReview", () => {
  it("uses only the secret-free origin for authenticated version and identity requests", async () => {
    const fetch = vi
      .fn<FetchFn>()
      .mockResolvedValueOnce(response(VERSION_DOC))
      .mockResolvedValueOnce(
        response({ schema: "pointbreak.inspect-identity", ...IDENTITY }),
      );

    await expect(probeReview(CAPABILITY, IDENTITY, fetch)).resolves.toEqual(
      MATCH,
    );

    expect(fetch.mock.calls.map(([url]) => url.toString())).toEqual([
      `${CAPABILITY.origin}/api/version`,
      `${CAPABILITY.origin}/api/identity`,
    ]);
    for (const [, init] of fetch.mock.calls) {
      expect(init.headers.Authorization).toBe(`Bearer ${CAPABILITY.token}`);
    }
  });

  it("classifies authenticated failures without returning credentials", async () => {
    const fetch = vi.fn<FetchFn>(async () => ({
      status: 401,
      text: async () => CAPABILITY.token,
    }));

    const result = await probeReview(CAPABILITY, IDENTITY, fetch);

    expect(result).toEqual({ kind: "unauthorized" });
    expect(JSON.stringify(result)).not.toContain(CAPABILITY.token);
  });
});

describe("runOpenInReviewCommand", () => {
  it("offers every revision newest-first in the explicit picker", async () => {
    const target = resolved();
    const entries = Array.from({ length: 25 }, (_, index) => ({
      revisionId: `rev:sha256:${String(index + 1).padStart(2, "0")}`,
      summary: index === 24 ? "Newest readable revision" : undefined,
      mergeStatus: "open",
      capturedAt: `2026-07-${String(index + 1).padStart(2, "0")}T00:00:00Z`,
    }));
    const pointbreak = {
      revisionList: vi.fn(async () => ({ entries })),
    } as unknown as PointbreakCli;
    vscodeMocks.showQuickPick.mockImplementation(
      async (items: readonly { revisionId: string }[]) => items[0],
    );

    await runOpenInReviewCommand(pointbreak, binary(), [target], undefined, {
      pick: vi.fn(async () => target as never),
      reviewUrl: "https://review.example.com",
    });

    const items = vscodeMocks.showQuickPick.mock.calls[0][0];
    expect(items).toHaveLength(25);
    expect(items[0]).toMatchObject({
      revisionId: "rev:sha256:25",
      label: "Newest readable revision",
      description: "25 · open",
    });
    expect(vscodeMocks.openExternal).toHaveBeenCalledWith(
      "https://review.example.com/#/revision/rev:sha256:25",
    );
  });

  it("opens a verified running text-web capability without starting another process", async () => {
    const probe = vi.fn(async () => MATCH);
    const start = vi.fn();

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      running: () => Promise.resolve(CAPABILITY),
      start,
    });

    expect(probe).toHaveBeenCalledWith(CAPABILITY, IDENTITY);
    expect(start).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).toHaveBeenCalledWith(
      `${CAPABILITY.origin}/#/revision/rev:sha256:abc?token=${CAPABILITY.token}`,
    );
  });

  it("starts only the explicit text-web surface and cleans it up when probing fails", async () => {
    vscodeMocks.showInformationMessage.mockResolvedValue(
      "Start `pointbreak inspect` here",
    );
    const probe = vi.fn(async () => UNREACHABLE);
    const start = vi.fn(async () => CAPABILITY);
    const stop = vi.fn();

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      running: () => undefined,
      sleep: vi.fn(),
      start,
      stop,
    });

    expect(start).toHaveBeenCalledWith(
      binary(),
      expect.objectContaining({ name: "repo" }),
      resolvedTargetKey(),
    );
    expect(probe).toHaveBeenCalledTimes(10);
    expect(stop).toHaveBeenCalledWith(resolvedTargetKey());
    expect(vscodeMocks.openExternal).not.toHaveBeenCalled();
    expect(
      vscodeMocks.showErrorMessage.mock.calls.flat().join(" "),
    ).not.toMatch(/secret|127\.0\.0\.1|63831|file:|\/repo/);
  });

  it("opens an optional externally managed browser URL without restoring or persisting it", async () => {
    const probe = vi.fn();
    const start = vi.fn();

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: "https://review.example.com",
      running: () => undefined,
      start,
    });

    expect(probe).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).toHaveBeenCalledWith(
      "https://review.example.com/#/revision/rev:sha256:abc",
    );
  });

  it("rejects a token-bearing configured URL instead of retaining or probing it", async () => {
    const probe = vi.fn();
    const start = vi.fn();

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: `${CAPABILITY.origin}/#/timeline?token=${CAPABILITY.token}`,
      running: () => undefined,
      start,
    });

    expect(probe).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).not.toHaveBeenCalled();
    expect(vscodeMocks.showErrorMessage).toHaveBeenCalledWith(
      expect.not.stringContaining(CAPABILITY.token),
    );
  });

  it("never treats the API-only child manager as a browser destination", async () => {
    vscodeMocks.showInformationMessage.mockResolvedValue(undefined);
    const start = vi.fn();

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe: vi.fn(),
      running: () => undefined,
      start,
    });

    expect(start).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).not.toHaveBeenCalled();
  });

  it("keeps configured non-local Review servers in the external browser", async () => {
    vscodeMocks.openLocalhostLinks = true;

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      reviewUrl: "https://review.example.com",
      running: () => undefined,
    });

    expect(vscodeMocks.executeCommand).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).toHaveBeenCalledOnce();
  });

  it("disables itself honestly in remote workspaces", async () => {
    vscodeMocks.remoteName = "ssh-remote";
    const start = vi.fn();

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      running: () => undefined,
      start,
    });

    expect(start).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).not.toHaveBeenCalled();
  });
});

function response(document: unknown) {
  return { status: 200, text: async () => JSON.stringify(document) };
}

function binary(): ResolvedBinary {
  return { path: "/usr/local/bin/pointbreak", source: "path" };
}

function cli(): PointbreakCli {
  return {} as PointbreakCli;
}

function resolved(): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
    target: {
      key: resolvedTargetKey(),
      label: "repo",
      ...IDENTITY,
    },
    emptyInventory: false,
  };
}

function resolvedTargetKey(): string {
  return "store:sha256:store/context:sha256:repo";
}

function reviewNode() {
  return {
    revisionId: "rev:sha256:abc",
    targetKey: resolvedTargetKey(),
    folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
  };
}
