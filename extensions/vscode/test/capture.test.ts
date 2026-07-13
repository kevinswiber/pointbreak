import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import {
  type CaptureOptions,
  type PointbreakCli,
  PointbreakCliError,
} from "../src/cli";
import { runCaptureCommand } from "../src/commands/capture";
import type { TargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => ({
  showErrorMessage: vi.fn(),
  showInformationMessage: vi.fn(),
  showQuickPick: vi.fn(),
}));

vi.mock("vscode", () => ({ window: vscodeMocks }));

beforeEach(() => {
  vscodeMocks.showErrorMessage.mockReset();
  vscodeMocks.showInformationMessage.mockReset();
  vscodeMocks.showQuickPick.mockReset();
});

describe("runCaptureCommand", () => {
  it("offers allow-empty only after the CLI reports zero changed files", async () => {
    const capture = vi
      .fn<(repo: string, options: CaptureOptions) => Promise<never>>()
      .mockRejectedValueOnce(
        new PointbreakCliError(
          "shore capture failed",
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

    await runCaptureCommand(cli, [resolved()], {
      pick: vi.fn(async (items) => items[0] as never),
      refresh: vi.fn(),
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

    await runCaptureCommand(cli, [resolved()], { pick, refresh });

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
      refresh,
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

    await runCaptureCommand(cli, [resolved()], {
      pick: vi.fn(async (items) => items[0] as never),
      refresh: vi.fn(),
    });

    expect(vscodeMocks.showQuickPick).toHaveBeenCalledTimes(1);
  });
});

function resolved(emptyInventory = false): TargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
    target: { key: "store/context", label: "repo" },
    emptyInventory,
  };
}
