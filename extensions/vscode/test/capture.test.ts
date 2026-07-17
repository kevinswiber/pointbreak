import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import {
  type CaptureOptions,
  type PointbreakCli,
  PointbreakCliError,
} from "../src/cli";
import { runCaptureCommand } from "../src/commands/capture";
import type { HumanWriteCoordinator } from "../src/humanWriteCoordinator";
import type { TargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => ({
  showErrorMessage: vi.fn(),
  showInputBox: vi.fn(),
  showInformationMessage: vi.fn(),
  showQuickPick: vi.fn(),
  showWarningMessage: vi.fn(),
}));

vi.mock("vscode", () => ({ window: vscodeMocks }));

beforeEach(() => {
  vscodeMocks.showErrorMessage.mockReset();
  vscodeMocks.showInputBox.mockReset();
  vscodeMocks.showInputBox.mockResolvedValue("");
  vscodeMocks.showInformationMessage.mockReset();
  vscodeMocks.showQuickPick.mockReset();
  vscodeMocks.showWarningMessage.mockReset();
  vscodeMocks.showWarningMessage.mockResolvedValue("Capture");
});

describe("runCaptureCommand", () => {
  it("offers allow-empty only after the CLI reports zero changed files", async () => {
    const capture = vi
      .fn<(repo: string, options: CaptureOptions) => Promise<never>>()
      .mockRejectedValueOnce(
        new PointbreakCliError(
          "pointbreak capture failed",
          1,
          "capture produced no changed files; pass --allow-empty",
        ),
      )
      .mockRejectedValueOnce(new Error("stop after retry"));
    const cli = { capture } as unknown as PointbreakCli;
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "My current work",
      choice: "worktree",
    });
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Tracked files only",
      includeUntracked: false,
    });
    vscodeMocks.showInformationMessage.mockResolvedValueOnce(
      "Capture empty revision",
    );
    const refresh = vi.fn(async () => undefined);

    await runCaptureCommand(cli, [resolved()], {
      pick: vi.fn(async (items) => items[0] as never),
      humanWrites: humanWrites(refresh),
    });

    expect(capture.mock.calls.map((call) => call[1].allowEmpty)).toEqual([
      false,
      true,
    ]);
    expect(vscodeMocks.showInformationMessage).toHaveBeenCalledWith(
      "Capture an empty revision?",
      "Capture empty revision",
    );
    expect(
      vscodeMocks.showQuickPick.mock.calls
        .flatMap((call) => call[0])
        .map((item) => item.label),
    ).not.toContain("Capture empty revision");
  });

  it("routes through pickFolder and refreshes the view on success", async () => {
    const capture = vi.fn(async () => ({
      schema: "pointbreak.review-capture" as const,
      version: 1 as const,
      revision: { id: "rev:sha256:1234567890abcdef" },
      diagnostics: [],
    }));
    const cli = { capture } as unknown as PointbreakCli;
    const pick = vi.fn(async (items) => items[0] as never);
    const refresh = vi.fn(async () => undefined);
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Staged only",
      choice: "staged",
    });

    await runCaptureCommand(cli, [resolved()], {
      pick,
      humanWrites: humanWrites(refresh),
    });

    expect(pick).toHaveBeenCalledOnce();
    expect(capture).toHaveBeenCalledWith("/repo", {
      choice: "staged",
      includeUntracked: false,
      allowEmpty: false,
    });
    expect(refresh).toHaveBeenCalledOnce();
    expect(vscodeMocks.showInformationMessage).toHaveBeenCalledWith(
      "Captured revision 1234567890ab",
    );
  });

  it("records an optional summary and includes it in the completion notice", async () => {
    const capture = vi.fn(async () => ({
      schema: "pointbreak.review-capture" as const,
      version: 1 as const,
      revision: {
        id: "rev:sha256:1234567890abcdef",
        summary: "Make revision discovery readable",
      },
      diagnostics: [],
    }));
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Staged only",
      choice: "staged",
    });
    vscodeMocks.showInputBox.mockResolvedValueOnce(
      "  Make revision discovery readable  ",
    );

    await runCaptureCommand(
      { capture } as unknown as PointbreakCli,
      [resolved()],
      {
        pick: vi.fn(async (items) => items[0] as never),
        humanWrites: humanWrites(vi.fn(async () => undefined)),
      },
    );

    expect(capture).toHaveBeenCalledWith("/repo", {
      choice: "staged",
      includeUntracked: false,
      allowEmpty: false,
      summary: "Make revision discovery readable",
    });
    expect(vscodeMocks.showInformationMessage).toHaveBeenCalledWith(
      "Captured “Make revision discovery readable” (1234567890ab)",
    );
  });

  it("refreshes a completed write without waiting for its notification", async () => {
    const notice = deferred<void>();
    const refresh = vi.fn(async () => undefined);
    const cli = {
      capture: vi.fn(async () => ({
        schema: "pointbreak.review-capture",
        version: 1,
        revision: { id: "rev:sha256:1234567890abcdef" },
        diagnostics: [],
      })),
    } as unknown as PointbreakCli;
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Staged only",
      choice: "staged",
    });
    vscodeMocks.showInformationMessage.mockReturnValueOnce(notice.promise);

    const command = runCaptureCommand(cli, [resolved()], {
      pick: vi.fn(async (items) => items[0] as never),
      humanWrites: humanWrites(refresh),
    });
    await vi.waitFor(() =>
      expect(vscodeMocks.showInformationMessage).toHaveBeenCalled(),
    );
    const refreshesBeforeDismissal = refresh.mock.calls.length;
    notice.resolve();
    await command;

    expect(refreshesBeforeDismissal).toBe(1);
  });

  it("marks every matching target populated before refreshing", async () => {
    const capture = vi.fn(async () => ({
      schema: "pointbreak.review-capture" as const,
      version: 1 as const,
      revision: { id: "rev:sha256:1234567890abcdef" },
      diagnostics: [],
    }));
    const cli = { capture } as unknown as PointbreakCli;
    const first = resolved(true);
    const second = {
      ...resolved(true),
      folder: workspaceFolder("/linked", "linked") as WorkspaceFolder,
    };
    const refresh = vi.fn(async () => undefined);
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Staged only",
      choice: "staged",
    });

    await runCaptureCommand(cli, [first, second], {
      pick: vi.fn(async () => second as never),
      humanWrites: humanWrites(refresh),
    });

    expect(first).toMatchObject({ emptyInventory: false });
    expect(second).toMatchObject({ emptyInventory: false });
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("never offers include-untracked for staged capture", async () => {
    const cli = {
      capture: vi.fn(async () => ({
        schema: "pointbreak.review-capture",
        version: 1,
        revision: { id: "rev:sha256:a" },
      })),
    } as unknown as PointbreakCli;
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Staged only",
      choice: "staged",
    });

    const refresh = vi.fn(async () => undefined);
    await runCaptureCommand(cli, [resolved()], {
      pick: vi.fn(async (items) => items[0] as never),
      humanWrites: humanWrites(refresh),
    });

    expect(vscodeMocks.showQuickPick).toHaveBeenCalledTimes(1);
  });

  it("cancels actor confirmation without capturing or refreshing", async () => {
    const capture = vi.fn(async () => ({
      schema: "pointbreak.review-capture" as const,
      version: 1 as const,
      revision: { id: "rev:sha256:a" },
      diagnostics: [],
    }));
    const cli = { capture } as unknown as PointbreakCli;
    const refresh = vi.fn(async () => undefined);
    const coordinator = humanWrites(refresh);
    vscodeMocks.showQuickPick.mockResolvedValueOnce({
      label: "Staged only",
      choice: "staged",
    });
    vscodeMocks.showWarningMessage.mockResolvedValueOnce(undefined);

    await runCaptureCommand(cli, [resolved()], {
      pick: vi.fn(async (items) => items[0] as never),
      humanWrites: coordinator,
    });

    expect(coordinator.run).toHaveBeenCalledOnce();
    expect(capture).not.toHaveBeenCalled();
    expect(refresh).not.toHaveBeenCalled();
  });
});

function resolved(emptyInventory = false): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
    target: {
      key: "store/context",
      label: "repo",
      storeIdentity: "store",
      contextIdentity: "context",
    },
    emptyInventory,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function humanWrites(refresh: () => Promise<void>): HumanWriteCoordinator {
  return {
    run: vi.fn(async (request: HumanWriteRequestMock) => {
      const context = {
        actorId: "actor:git-email:human@example.com",
        track: "human:local",
      };
      if (!(await request.confirm(context))) return undefined;
      const result = await request.write(context);
      await refresh();
      return { document: result, refreshed: true };
    }),
  } as unknown as HumanWriteCoordinator;
}

interface HumanWriteRequestMock {
  confirm(context: { actorId: string; track: string }): Promise<boolean>;
  write(context: { actorId: string; track: string }): Promise<unknown>;
}
