import { describe, expect, it, vi } from "vitest";
import type { PointbreakCli } from "../src/cli";
import { HumanWriteCoordinator } from "../src/humanWriteCoordinator";

describe("HumanWriteCoordinator", () => {
  it("requires a fresh confirmation when the resolved actor changes", async () => {
    const identityWhoami = vi
      .fn()
      .mockResolvedValueOnce(identity("actor:git-email:first@example.com"))
      .mockResolvedValueOnce(identity("actor:git-email:second@example.com"))
      .mockResolvedValueOnce(identity("actor:git-email:second@example.com"));
    const dependencies = coordinatorDependencies();
    const coordinator = new HumanWriteCoordinator(
      { identityWhoami } as unknown as PointbreakCli,
      dependencies,
    );
    const confirm = vi.fn(
      async (_context: { actorId: string; track: string }) => true,
    );
    const write = vi.fn(async () => document());

    await expect(
      coordinator.run({
        repo: "/repo",
        resource: { fsPath: "/repo" },
        confirm,
        write,
      }),
    ).resolves.toEqual({ document: document(), refreshed: true });

    expect(confirm.mock.calls.map(([context]) => context.actorId)).toEqual([
      "actor:git-email:first@example.com",
      "actor:git-email:second@example.com",
    ]);
    expect(write).toHaveBeenCalledOnce();
    expect(write).toHaveBeenCalledWith(
      {
        actorId: "actor:git-email:second@example.com",
        track: "human:local",
      },
      undefined,
    );
    expect(dependencies.refresh).toHaveBeenCalledOnce();
  });

  it("cancels without writing or refreshing", async () => {
    const coordinator = new HumanWriteCoordinator(
      {
        identityWhoami: vi.fn(async () =>
          identity("actor:git-email:human@example.com"),
        ),
      } as unknown as PointbreakCli,
      coordinatorDependencies(),
    );
    const write = vi.fn(async () => document());

    await expect(
      coordinator.run({
        repo: "/repo",
        resource: { fsPath: "/repo" },
        confirm: vi.fn(async () => false),
        write,
      }),
    ).resolves.toBeUndefined();

    expect(write).not.toHaveBeenCalled();
  });

  it("uses an attention track override and surfaces landed diagnostics", async () => {
    const dependencies = coordinatorDependencies();
    dependencies.refresh.mockRejectedValueOnce(new Error("refresh failed"));
    const coordinator = new HumanWriteCoordinator(
      {
        identityWhoami: vi.fn(async () =>
          identity("actor:git-email:human@example.com"),
        ),
      } as unknown as PointbreakCli,
      dependencies,
    );
    const landed = document([
      { code: "candidate_remains", message: "Candidate remains." },
    ]);
    const write = vi.fn(async () => landed);

    await expect(
      coordinator.run({
        repo: "/repo",
        resource: { fsPath: "/repo" },
        trackOverride: " agent:review-lane ",
        confirm: vi.fn(async () => true),
        write,
      }),
    ).resolves.toEqual({
      document: landed,
      refreshed: false,
    });

    expect(dependencies.resolveTrack).not.toHaveBeenCalled();
    expect(write).toHaveBeenCalledWith(
      {
        actorId: "actor:git-email:human@example.com",
        track: "agent:review-lane",
      },
      undefined,
    );
    expect(dependencies.showDiagnostic).toHaveBeenCalledWith(
      "Candidate remains.",
    );
    expect(dependencies.showRefreshError).toHaveBeenCalledOnce();
  });

  it.each([
    {
      name: "actor",
      actors: ["actor:human:first", "actor:human:second", "actor:human:second"],
      tracks: ["human:local", "human:local", "human:local"],
    },
    {
      name: "track",
      actors: ["actor:human:first", "actor:human:first", "actor:human:first"],
      tracks: ["human:first", "human:second", "human:second"],
    },
  ])("re-prepares a write when the resolved $name changes", async ({
    actors,
    tracks,
  }) => {
    const identityWhoami = vi.fn(async () => identity(actors.shift() ?? ""));
    const dependencies = coordinatorDependencies();
    dependencies.resolveTrack.mockImplementation(() => tracks.shift() ?? "");
    const coordinator = new HumanWriteCoordinator(
      { identityWhoami } as unknown as PointbreakCli,
      dependencies,
    );
    const prepare = vi.fn(
      async (context: { actorId: string; track: string }) =>
        `${context.actorId}@${context.track}`,
    );
    const confirm = vi.fn(async () => true);
    const write = vi.fn(
      async (
        _context: { actorId: string; track: string },
        _preparation: string,
      ) => document(),
    );

    await coordinator.run({
      repo: "/repo",
      resource: { fsPath: "/repo" },
      prepare,
      confirm,
      write,
    });

    expect(prepare).toHaveBeenCalledTimes(2);
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(write).toHaveBeenCalledOnce();
    expect(write.mock.calls[0]?.[1]).toBe(await prepare.mock.results[1]?.value);
  });
});

function coordinatorDependencies() {
  return {
    resolveTrack: vi.fn(() => "human:local"),
    showDiagnostic: vi.fn(async () => undefined),
    refresh: vi.fn(async () => undefined),
    showRefreshError: vi.fn(async () => undefined),
  };
}

function identity(actorId: string) {
  return {
    schema: "pointbreak.identity-whoami" as const,
    version: 1 as const,
    actorId,
    diagnostics: [],
  };
}

function document(diagnostics: unknown[] = []) {
  return {
    schema: "pointbreak.review-assessment-add" as const,
    version: 1 as const,
    diagnostics,
  };
}
