import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import type { ResolvedBinary } from "../src/binary";
import type { PointbreakCli } from "../src/cli";
import {
  reviewDeepLink,
  runOpenInReviewCommand,
} from "../src/commands/openInReview";
import type { TargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => ({
  createTerminal: vi.fn(),
  openExternal: vi.fn(),
  remoteName: undefined as string | undefined,
  showErrorMessage: vi.fn(),
  showInformationMessage: vi.fn(),
  showQuickPick: vi.fn(),
  terminals: [] as Array<{ name: string }>,
}));

vi.mock("vscode", () => ({
  Uri: { parse: (value: string) => value },
  env: {
    get remoteName() {
      return vscodeMocks.remoteName;
    },
    openExternal: vscodeMocks.openExternal,
  },
  window: {
    createTerminal: vscodeMocks.createTerminal,
    get terminals() {
      return vscodeMocks.terminals;
    },
    showErrorMessage: vscodeMocks.showErrorMessage,
    showInformationMessage: vscodeMocks.showInformationMessage,
    showQuickPick: vscodeMocks.showQuickPick,
  },
  workspace: {
    getConfiguration: () => ({
      get: (_key: string, fallback: string) => fallback,
    }),
  },
}));

beforeEach(() => {
  vscodeMocks.createTerminal.mockReset();
  vscodeMocks.openExternal.mockReset();
  vscodeMocks.remoteName = undefined;
  vscodeMocks.showErrorMessage.mockReset();
  vscodeMocks.showInformationMessage.mockReset();
  vscodeMocks.showQuickPick.mockReset();
  vscodeMocks.terminals = [];
});

describe("reviewDeepLink", () => {
  it("builds the revision deep link exactly", () => {
    expect(reviewDeepLink("http://127.0.0.1:7878", "rev:sha256:abc")).toBe(
      "http://127.0.0.1:7878/#/revision/rev:sha256:abc",
    );
  });
});

describe("runOpenInReviewCommand", () => {
  it("opens externally when the probe succeeds without touching terminals", async () => {
    const probe = vi.fn(async () => true);

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: "http://127.0.0.1:7878",
    });

    expect(vscodeMocks.openExternal).toHaveBeenCalledWith(
      "http://127.0.0.1:7878/#/revision/rev:sha256:abc",
    );
    expect(vscodeMocks.createTerminal).not.toHaveBeenCalled();
  });

  it("offers a visible user-owned terminal assist after a failed probe", async () => {
    const terminal = {
      sendText: vi.fn(),
      show: vi.fn(),
      name: "Pointbreak Review",
    };
    vscodeMocks.createTerminal.mockReturnValue(terminal);
    vscodeMocks.showInformationMessage.mockResolvedValue(
      "Start `shore inspect` here",
    );
    const probe = vi
      .fn<() => Promise<boolean>>()
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true);

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: "http://127.0.0.1:7878",
      sleep: vi.fn(),
    });

    expect(vscodeMocks.createTerminal).toHaveBeenCalledWith({
      name: "Pointbreak Review",
      cwd: "/repo",
    });
    expect(terminal.sendText).toHaveBeenCalledWith("shore inspect");
    expect(terminal.show).toHaveBeenCalledOnce();
    expect(vscodeMocks.openExternal).toHaveBeenCalledOnce();
  });

  it("gives up honestly after the bounded re-probe window", async () => {
    const terminal = {
      sendText: vi.fn(),
      show: vi.fn(),
      name: "Pointbreak Review",
    };
    vscodeMocks.createTerminal.mockReturnValue(terminal);
    vscodeMocks.showInformationMessage.mockResolvedValue(
      "Start `shore inspect` here",
    );
    const probe = vi.fn(async () => false);
    const sleep = vi.fn(async () => undefined);

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: "http://127.0.0.1:7878",
      sleep,
    });

    expect(probe).toHaveBeenCalledTimes(11);
    expect(sleep).toHaveBeenCalledTimes(10);
    expect(vscodeMocks.openExternal).not.toHaveBeenCalled();
    expect(vscodeMocks.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/start `shore inspect` manually.*reviewUrl/i),
    );
  });

  it("starts a resolved binary directly without shell interpolation", async () => {
    const terminal = {
      sendText: vi.fn(),
      show: vi.fn(),
      name: "Pointbreak Review",
    };
    vscodeMocks.createTerminal.mockReturnValue(terminal);
    vscodeMocks.showInformationMessage.mockResolvedValue(
      "Start `shore inspect` here",
    );
    const probe = vi
      .fn<() => Promise<boolean>>()
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true);

    await runOpenInReviewCommand(
      cli(),
      { path: "/Pointbreak & Dev/$shore", source: "setting" },
      [resolved()],
      reviewNode(),
      { probe, reviewUrl: "http://127.0.0.1:7878", sleep: vi.fn() },
    );

    expect(vscodeMocks.createTerminal).toHaveBeenCalledWith({
      name: "Pointbreak Review",
      cwd: "/repo",
      shellPath: "/Pointbreak & Dev/$shore",
      shellArgs: ["inspect"],
    });
    expect(terminal.sendText).not.toHaveBeenCalled();
  });

  it("does not reuse a Review terminal across workspace folders", async () => {
    const terminals = [
      { sendText: vi.fn(), show: vi.fn(), name: "Pointbreak Review" },
      { sendText: vi.fn(), show: vi.fn(), name: "Pointbreak Review" },
    ];
    vscodeMocks.createTerminal.mockImplementation(() => {
      const terminal =
        terminals[vscodeMocks.createTerminal.mock.calls.length - 1];
      vscodeMocks.terminals.push(terminal);
      return terminal;
    });
    vscodeMocks.showInformationMessage.mockResolvedValue(
      "Start `shore inspect` here",
    );
    const probe = vi
      .fn<() => Promise<boolean>>()
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true);

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: "http://127.0.0.1:7878",
      sleep: vi.fn(),
    });
    await runOpenInReviewCommand(
      cli(),
      binary(),
      [resolved()],
      {
        revisionId: "rev:sha256:def",
        folder: workspaceFolder("/other", "other") as WorkspaceFolder,
      },
      {
        probe,
        reviewUrl: "http://127.0.0.1:7878",
        sleep: vi.fn(),
      },
    );

    expect(vscodeMocks.createTerminal).toHaveBeenCalledTimes(2);
    expect(
      vscodeMocks.createTerminal.mock.calls.map(([options]) => options),
    ).toEqual([
      { name: "Pointbreak Review", cwd: "/repo" },
      { name: "Pointbreak Review", cwd: "/other" },
    ]);
    expect(terminals[0].sendText).toHaveBeenCalledWith("shore inspect");
    expect(terminals[1].sendText).toHaveBeenCalledWith("shore inspect");
  });

  it("disables itself honestly in remote workspaces", async () => {
    vscodeMocks.remoteName = "ssh-remote";
    const probe = vi.fn(async () => true);

    await runOpenInReviewCommand(cli(), binary(), [resolved()], reviewNode(), {
      probe,
      reviewUrl: "http://127.0.0.1:7878",
    });

    expect(vscodeMocks.showInformationMessage).toHaveBeenCalledWith(
      expect.stringMatching(/not available in remote workspaces yet/i),
    );
    expect(probe).not.toHaveBeenCalled();
    expect(vscodeMocks.openExternal).not.toHaveBeenCalled();
    expect(vscodeMocks.createTerminal).not.toHaveBeenCalled();
  });
});

function binary(): ResolvedBinary {
  return { path: "/usr/local/bin/shore", source: "path" };
}

function cli(): PointbreakCli {
  return {} as PointbreakCli;
}

function resolved(): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
    target: { key: "store/context", label: "repo" },
    emptyInventory: false,
  };
}

function reviewNode() {
  return {
    revisionId: "rev:sha256:abc",
    folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
  };
}
