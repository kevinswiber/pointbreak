import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";

vi.mock("vscode", () => ({
  commands: { executeCommand: vi.fn() },
  Range: class {},
  Selection: class {},
  TextEditorRevealType: { InCenter: 0 },
  Uri: { file: vi.fn() },
  ViewColumn: { One: 1 },
  window: {
    activeTextEditor: undefined,
    createTextEditorDecorationType: vi.fn(),
    showErrorMessage: vi.fn(),
    showInformationMessage: vi.fn(),
    showInputBox: vi.fn(),
    showTextDocument: vi.fn(),
    showWarningMessage: vi.fn(),
  },
  workspace: {
    getConfiguration: vi.fn(),
    openTextDocument: vi.fn(),
  },
}));

import { runAddObservationFromSelectionCommand } from "../src/commands/addObservationFromSelection";
import { SourceReviewContextStore } from "../src/commands/openInSource";
import type { HumanWriteCoordinator } from "../src/humanWriteCoordinator";
import type { TargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const document = sourceDocument(["alpha", "new", "omega"]);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("runAddObservationFromSelectionCommand", () => {
  it("records a verified selection through actor confirmation and refreshes", async () => {
    const contexts = contextStore(document);
    const cli = cliMock();
    const dependencies = deps(editor(document, 1, 0, 1, 3));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(dependencies.confirmDrift).not.toHaveBeenCalled();
    expect(dependencies.confirmUnverified).not.toHaveBeenCalled();
    expect(dependencies.humanWrites.run).toHaveBeenCalledOnce();
    expect(dependencies.confirmWrite).toHaveBeenCalledWith({
      actorId: "actor:git-email:human@example.com",
      track: "human:local",
      revisionId: "rev:sha256:one",
      title: "Check this range",
    });
    expect(cli.addObservation).toHaveBeenCalledWith("/repo", {
      revisionId: "rev:sha256:one",
      track: "human:local",
      title: "Check this range",
      file: "src/lib.rs",
      side: "new",
      startLine: 2,
      endLine: 2,
    });
    expect(dependencies.showInformationMessage).toHaveBeenCalledWith(
      "Observation recorded.",
    );
    expect(dependencies.refresh).toHaveBeenCalledOnce();
  });

  it("cancels final actor confirmation without appending", async () => {
    const contexts = contextStore(document);
    const cli = cliMock();
    const dependencies = deps(editor(document, 1, 0, 1, 3));
    dependencies.confirmWrite.mockResolvedValueOnce(false);

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(cli.addObservation).not.toHaveBeenCalled();
    expect(dependencies.refresh).not.toHaveBeenCalled();
  });

  it("refreshes a completed write without waiting for its notification", async () => {
    const notice = deferred<void>();
    const contexts = contextStore(document);
    const dependencies = {
      ...deps(editor(document, 1, 0, 1, 3)),
      showInformationMessage: vi.fn(() => notice.promise),
    };
    const command = runAddObservationFromSelectionCommand(
      cliMock() as never,
      resolutions(),
      contexts,
      dependencies,
    );
    await vi.waitFor(() =>
      expect(dependencies.showInformationMessage).toHaveBeenCalled(),
    );
    const refreshesBeforeDismissal = dependencies.refresh.mock.calls.length;
    notice.resolve();
    await command;

    expect(refreshesBeforeDismissal).toBe(1);
  });

  it.each([
    {
      name: "drifted",
      lines: ["alpha", "changed", "omega"],
      selected: [1, 0, 1, 3] as const,
      confirmation: "confirmDrift" as const,
      other: "confirmUnverified" as const,
    },
    {
      name: "unverified",
      lines: ["alpha", "new", "omega", "outside"],
      selected: [2, 0, 3, 7] as const,
      confirmation: "confirmUnverified" as const,
      other: "confirmDrift" as const,
    },
  ])("requires the distinct $name confirmation", async ({
    lines,
    selected,
    confirmation,
    other,
  }) => {
    const activeDocument = sourceDocument(lines);
    const contexts = contextStore(activeDocument);
    const cli = cliMock();
    const dependencies = deps(
      editor(
        activeDocument,
        selected[0],
        selected[1],
        selected[2],
        selected[3],
      ),
    );

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(dependencies[confirmation]).toHaveBeenCalledOnce();
    expect(dependencies[other]).not.toHaveBeenCalled();
    expect(cli.addObservation).toHaveBeenCalledOnce();
  });

  it("confirms a uniquely shifted selection reached through a drifted landing", async () => {
    const shiftedDocument = sourceDocument([
      "inserted",
      "alpha",
      "new",
      "omega",
    ]);
    const contexts = contextStore(shiftedDocument);
    contexts.set(shiftedDocument, {
      targetKey: "store/context",
      revisionId: "rev:sha256:one",
      snapshot: snapshot(),
      filePath: "src/lib.rs",
      side: "new",
      target: {
        filePath: "src/lib.rs",
        side: "new",
        startLine: 2,
        endLine: 2,
      },
      lastLanding: {
        kind: "drifted",
        filePath: "src/lib.rs",
        range: {
          start: { line: 2, character: 0 },
          end: { line: 2, character: 3 },
        },
        message: "The live source has changed since this review was captured.",
      },
    });
    const cli = cliMock();
    const dependencies = deps(editor(shiftedDocument, 2, 0, 2, 3));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(dependencies.confirmDrift).toHaveBeenCalledOnce();
    expect(cli.addObservation).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({ startLine: 2, endLine: 2 }),
    );
  });

  it.each([
    "confirmDrift",
    "confirmUnverified",
  ] as const)("does not write when %s is declined", async (confirmation) => {
    const drifted = confirmation === "confirmDrift";
    const activeDocument = sourceDocument(
      drifted
        ? ["alpha", "changed", "omega"]
        : ["alpha", "new", "omega", "outside"],
    );
    const contexts = contextStore(activeDocument);
    const cli = cliMock();
    const selection = drifted
      ? editor(activeDocument, 1, 0, 1, 3)
      : editor(activeDocument, 2, 0, 3, 7);
    const dependencies = deps(selection);
    dependencies[confirmation].mockResolvedValueOnce(false);

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(cli.addObservation).not.toHaveBeenCalled();
    expect(dependencies.promptTitle).not.toHaveBeenCalled();
  });

  it("blocks missing context with open-source guidance", async () => {
    const cli = cliMock();
    const dependencies = deps(editor(document, 1, 0, 1, 3));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      new SourceReviewContextStore(),
      dependencies,
    );

    expect(cli.addObservation).not.toHaveBeenCalled();
    expect(dependencies.showInformationMessage).toHaveBeenCalledWith(
      expect.stringMatching(/open.+Pointbreak Review/i),
    );
  });

  it("blocks a superseded source context before recording", async () => {
    const contexts = contextStore(document);
    const cli = cliMock();
    const dependencies = deps(editor(document, 1, 0, 1, 3));
    dependencies.isRevisionCurrent.mockResolvedValueOnce(false);

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(cli.addObservation).not.toHaveBeenCalled();
    expect(dependencies.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/reopen|current/i),
    );
  });

  it("blocks old-side source context instead of mixing old and new coordinates", async () => {
    const oldSideDocument = sourceDocument(["added", "alpha"]);
    const contexts = contextStore(oldSideDocument);
    contexts.set(oldSideDocument, {
      targetKey: "store/context",
      revisionId: "rev:sha256:one",
      snapshot: snapshot(),
      filePath: "src/lib.rs",
      side: "old",
      target: {
        filePath: "src/lib.rs",
        side: "old",
        startLine: 1,
        endLine: 1,
      },
      lastLanding: {
        kind: "drifted",
        filePath: "src/lib.rs",
        range: {
          start: { line: 1, character: 0 },
          end: { line: 1, character: 5 },
        },
        message: "The live source has changed since this review was captured.",
      },
    });
    const cli = cliMock();
    const dependencies = deps(editor(oldSideDocument, 1, 0, 1, 5));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(cli.addObservation).not.toHaveBeenCalled();
    expect(dependencies.promptTitle).not.toHaveBeenCalled();
    expect(dependencies.showInformationMessage).toHaveBeenCalledWith(
      expect.stringMatching(/new-side|added source/i),
    );
  });

  it.each([
    ["outside repository", "/outside/lib.rs"],
    ["different snapshot path", "/repo/src/other.rs"],
  ])("blocks %s without revision-wide fallback", async (_name, filePath) => {
    const activeDocument = sourceDocument(["alpha", "new", "omega"], filePath);
    const contexts = contextStore(activeDocument);
    const cli = cliMock();
    const dependencies = deps(editor(activeDocument, 1, 0, 1, 3));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(cli.addObservation).not.toHaveBeenCalled();
    expect(dependencies.promptTitle).not.toHaveBeenCalled();
  });

  it("preserves source context and shows a path-free failure", async () => {
    const contexts = contextStore(document);
    const cli = cliMock();
    cli.addObservation.mockRejectedValueOnce(
      new Error("failed in /repo with token=secret 127.0.0.1:7878"),
    );
    const dependencies = deps(editor(document, 1, 0, 1, 3));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(contexts.get(document)).toBeDefined();
    expect(dependencies.refresh).not.toHaveBeenCalled();
    expect(
      JSON.stringify(dependencies.showErrorMessage.mock.calls),
    ).not.toMatch(/\/repo|secret|127\.0\.0\.1|7878/);
  });

  it("fails closed when the CLI reports a different revision", async () => {
    const contexts = contextStore(document);
    const cli = cliMock();
    cli.addObservation.mockResolvedValueOnce({
      schema: "pointbreak.review-observation-add",
      version: 1,
      revisionId: "rev:sha256:other",
      observationId: "obs:sha256:one",
    });
    const dependencies = deps(editor(document, 1, 0, 1, 3));

    await runAddObservationFromSelectionCommand(
      cli as never,
      resolutions(),
      contexts,
      dependencies,
    );

    expect(dependencies.refresh).not.toHaveBeenCalled();
    expect(dependencies.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/could not add/i),
    );
  });
});

function cliMock() {
  return {
    addObservation: vi.fn(async () => ({
      schema: "pointbreak.review-observation-add",
      version: 1,
      revisionId: "rev:sha256:one",
      observationId: "obs:sha256:one",
    })),
  };
}

function deps(activeEditor: ReturnType<typeof editor>) {
  const refresh = vi.fn(async () => undefined);
  const confirmWrite = vi.fn(async () => true);
  const humanWrites = {
    run: vi.fn(async (request: HumanWriteRequestMock) => {
      const context = {
        actorId: "actor:git-email:human@example.com",
        track: "human:local",
      };
      if (!(await request.confirm(context))) return undefined;
      const result = await request.write(context);
      await refresh();
      return result;
    }),
  } as unknown as HumanWriteCoordinator;
  return {
    activeEditor: vi.fn(() => activeEditor),
    observationTrack: vi.fn(() => "human:local"),
    humanWrites,
    confirmWrite,
    isRevisionCurrent: vi.fn(async () => true),
    promptTitle: vi.fn(async () => "Check this range"),
    confirmDrift: vi.fn(async () => true),
    confirmUnverified: vi.fn(async () => true),
    offerCapture: vi.fn(async () => false),
    capture: vi.fn(async () => undefined),
    showInformationMessage: vi.fn(async () => undefined),
    showErrorMessage: vi.fn(async () => undefined),
    refresh,
  };
}

interface HumanWriteRequestMock {
  confirm(context: { actorId: string; track: string }): Promise<boolean>;
  write(context: { actorId: string; track: string }): Promise<unknown>;
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function sourceDocument(lines: readonly string[], fsPath = "/repo/src/lib.rs") {
  return {
    uri: { fsPath },
    lineCount: lines.length,
    lineAt: (line: number) => ({ text: lines[line] }),
  };
}

function editor(
  activeDocument: ReturnType<typeof sourceDocument>,
  startLine: number,
  startCharacter: number,
  endLine: number,
  endCharacter: number,
) {
  return {
    document: activeDocument,
    selection: {
      start: { line: startLine, character: startCharacter },
      end: { line: endLine, character: endCharacter },
    },
  };
}

function contextStore(activeDocument: object) {
  const contexts = new SourceReviewContextStore();
  contexts.set(activeDocument, {
    targetKey: "store/context",
    revisionId: "rev:sha256:one",
    snapshot: snapshot(),
    filePath: "src/lib.rs",
    side: "new",
    target: {
      filePath: "src/lib.rs",
      side: "new",
      startLine: 2,
      endLine: 2,
    },
    lastLanding: {
      kind: "exact",
      filePath: "src/lib.rs",
      range: {
        start: { line: 1, character: 0 },
        end: { line: 1, character: 3 },
      },
    },
  });
  return contexts;
}

function resolutions(): TargetResolution[] {
  return [
    {
      kind: "resolved",
      folder: workspaceFolder("/repo", "repo") as WorkspaceFolder,
      target: {
        key: "store/context",
        label: "repo",
        storeIdentity: "store",
        contextIdentity: "context",
      },
      emptyInventory: false,
    },
  ];
}

function snapshot() {
  return {
    schema: "pointbreak.review-snapshot" as const,
    version: 1 as const,
    contentHash: "sha256:snapshot",
    snapshot: {
      review_id: "review:default",
      object_id: "obj:sha256:snapshot",
      files: [
        {
          id: "src/lib.rs",
          old_path: "src/lib.rs",
          new_path: "src/lib.rs",
          hunks: [
            {
              id: "hunk",
              header: "@@",
              rows: [
                { kind: "context", old_line: 1, new_line: 1, text: "alpha" },
                { kind: "added", old_line: null, new_line: 2, text: "new" },
                { kind: "context", old_line: 2, new_line: 3, text: "omega" },
              ],
            },
          ],
        },
      ],
    },
  };
}
