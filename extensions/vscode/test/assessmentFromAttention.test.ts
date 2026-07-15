import { describe, expect, it, vi } from "vitest";

vi.mock("vscode", () => ({
  commands: { executeCommand: vi.fn() },
  window: {
    showErrorMessage: vi.fn(),
    showInformationMessage: vi.fn(),
    showInputBox: vi.fn(),
    showQuickPick: vi.fn(),
    showWarningMessage: vi.fn(),
  },
}));

import type { AttentionItemNode } from "../src/attentionView";
import type {
  AssessmentShowDoc,
  AssessmentValue,
  AssessmentView,
  PointbreakCli,
  StaleAssessmentAttentionItem,
} from "../src/cli";
import {
  runAssessmentFromAttentionCommand,
  sameHumanRevisionCandidates,
} from "../src/commands/assessmentFromAttention";
import { HumanWriteCoordinator } from "../src/humanWriteCoordinator";
import { workspaceFolder } from "./helpers/vscodeMock";

describe("sameHumanRevisionCandidates", () => {
  it("keeps only current revision-scoped records from the exact actor and track", () => {
    const candidates = sameHumanRevisionCandidates(
      assessmentShow([
        assessment("same", "actor:human:kevin", "human:local"),
        assessment("other-actor", "actor:human:other", "human:local"),
        assessment("other-track", "actor:human:kevin", "agent:review"),
        assessment("range", "actor:human:kevin", "human:local", {
          kind: "range",
          revisionId: "rev:sha256:one",
        }),
        {
          ...assessment("replaced", "actor:human:kevin", "human:local"),
          status: "replaced",
        },
      ]),
      {
        actorId: "actor:human:kevin",
        track: "human:local",
      },
      "rev:sha256:one",
    );

    expect(candidates.map(({ id }) => id)).toEqual(["assess:sha256:same"]);
  });
});

describe("runAssessmentFromAttentionCommand", () => {
  it("fails closed when invoked without an Attention row", async () => {
    const harness = commandHarness({ assessments: [] });

    await runAssessmentFromAttentionCommand(
      harness.cli,
      undefined,
      harness.dependencies,
    );

    expect(harness.dependencies.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/matching.*attention row/i),
    );
    expect(harness.identityWhoami).not.toHaveBeenCalled();
  });

  it("records an exact assessment without replacement when no candidate exists", async () => {
    const harness = commandHarness({ assessments: [] });
    harness.pickAssessment.mockResolvedValue("needs-clarification");
    harness.promptSummary.mockResolvedValue("Clarify the owner gate.");

    await runAssessmentFromAttentionCommand(
      harness.cli,
      ambiguousNode(),
      harness.dependencies,
    );

    expect(harness.addAssessment).toHaveBeenCalledWith("/repo", {
      revisionId: "rev:sha256:one",
      track: "human:local",
      assessment: "needs-clarification",
      summary: "Clarify the owner gate.",
      replaces: [],
    });
    expect(harness.confirmAssessment).toHaveBeenCalledWith(
      expect.objectContaining({
        actorId: "actor:human:kevin",
        track: "human:local",
        revisionId: "rev:sha256:one",
        replacementIds: [],
      }),
    );
  });

  it("prefills one same-human candidate and exposes it in confirmation", async () => {
    const harness = commandHarness({
      assessments: [assessment("mine", "actor:human:kevin", "human:local")],
    });

    await runAssessmentFromAttentionCommand(
      harness.cli,
      ambiguousNode(),
      harness.dependencies,
    );

    expect(harness.pickReplacements).not.toHaveBeenCalled();
    expect(harness.addAssessment).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({ replaces: ["assess:sha256:mine"] }),
    );
    expect(harness.confirmAssessment).toHaveBeenCalledWith(
      expect.objectContaining({ replacementIds: ["assess:sha256:mine"] }),
    );
  });

  it("requires a picker for multiple same-human candidates", async () => {
    const candidates = [
      assessment("one", "actor:human:kevin", "human:local"),
      assessment("two", "actor:human:kevin", "human:local"),
      assessment("other", "actor:human:other", "human:local"),
    ];
    const harness = commandHarness({ assessments: candidates });
    harness.pickReplacements.mockResolvedValue([candidates[1]]);

    await runAssessmentFromAttentionCommand(
      harness.cli,
      ambiguousNode(),
      harness.dependencies,
    );

    expect(harness.pickReplacements).toHaveBeenCalledWith(
      candidates.slice(0, 2),
    );
    expect(harness.addAssessment).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({ replaces: ["assess:sha256:two"] }),
    );
  });

  it("discards prepared replacements after an actor change", async () => {
    const harness = commandHarness({
      actors: ["actor:human:first", "actor:human:second", "actor:human:second"],
      assessments: [
        assessment("first", "actor:human:first", "human:local"),
        assessment("second", "actor:human:second", "human:local"),
      ],
    });

    await runAssessmentFromAttentionCommand(
      harness.cli,
      ambiguousNode(),
      harness.dependencies,
    );

    expect(harness.showAssessments).toHaveBeenCalledTimes(2);
    expect(
      harness.confirmAssessment.mock.calls.map(
        ([value]) => value.replacementIds,
      ),
    ).toEqual([["assess:sha256:first"], ["assess:sha256:second"]]);
    expect(harness.addAssessment).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({ replaces: ["assess:sha256:second"] }),
    );
  });

  it("discards prepared replacements after a track change", async () => {
    const harness = commandHarness({
      tracks: ["human:first", "human:second", "human:second"],
      assessments: [
        assessment("first", "actor:human:kevin", "human:first"),
        assessment("second", "actor:human:kevin", "human:second"),
      ],
    });

    await runAssessmentFromAttentionCommand(
      harness.cli,
      ambiguousNode(),
      harness.dependencies,
    );

    expect(
      harness.showAssessments.mock.calls.map(([, options]) => options.track),
    ).toEqual(["human:first", "human:second"]);
    expect(harness.addAssessment).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({
        track: "human:second",
        replaces: ["assess:sha256:second"],
      }),
    );
  });

  it("targets one exact current head and routes multiple current heads to resolution", async () => {
    const one = commandHarness({ assessments: [] });
    await runAssessmentFromAttentionCommand(
      one.cli,
      staleNode(["rev:sha256:head"], ["rev:sha256:intermediate"]),
      one.dependencies,
    );
    expect(one.addAssessment).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({ revisionId: "rev:sha256:head" }),
    );

    const many = commandHarness({ assessments: [] });
    const node = staleNode(["rev:sha256:one", "rev:sha256:two"]);
    await runAssessmentFromAttentionCommand(many.cli, node, many.dependencies);
    expect(many.routeHeadResolution).toHaveBeenCalledWith(node);
    expect(many.addAssessment).not.toHaveBeenCalled();
  });

  it("fails closed when an older attention v1 item has no current-head field", async () => {
    const harness = commandHarness({ assessments: [] });

    await runAssessmentFromAttentionCommand(
      harness.cli,
      legacyStaleNode(),
      harness.dependencies,
    );

    expect(harness.addAssessment).not.toHaveBeenCalled();
    expect(harness.dependencies.showErrorMessage).toHaveBeenCalledWith(
      expect.stringMatching(/exact current head/i),
    );
  });

  it("preserves a failed-validation row track and refreshes after landed diagnostics", async () => {
    const harness = commandHarness({
      assessments: [],
      diagnostic: {
        code: "assessment_competing_candidates",
        message: "Another assessment candidate remains current.",
      },
    });

    await runAssessmentFromAttentionCommand(
      harness.cli,
      failedValidationNode(),
      harness.dependencies,
    );

    expect(harness.showAssessments).toHaveBeenCalledWith("/repo", {
      revisionId: "rev:sha256:failed",
      track: "agent:validation",
    });
    expect(harness.addAssessment).toHaveBeenCalledWith(
      "/repo",
      expect.objectContaining({
        revisionId: "rev:sha256:failed",
        track: "agent:validation",
      }),
    );
    expect(harness.showWarningMessage).toHaveBeenCalledWith(
      "Another assessment candidate remains current.",
    );
    expect(harness.refresh).toHaveBeenCalledOnce();
  });

  it("cancels final confirmation without appending", async () => {
    const harness = commandHarness({ assessments: [] });
    harness.confirmAssessment.mockResolvedValue(false);

    await runAssessmentFromAttentionCommand(
      harness.cli,
      ambiguousNode(),
      harness.dependencies,
    );

    expect(harness.addAssessment).not.toHaveBeenCalled();
    expect(harness.refresh).not.toHaveBeenCalled();
  });
});

interface HarnessOptions {
  actors?: string[];
  tracks?: string[];
  assessments: AssessmentView[];
  diagnostic?: unknown;
}

function commandHarness(options: HarnessOptions) {
  const actors = [
    ...(options.actors ?? ["actor:human:kevin", "actor:human:kevin"]),
  ];
  const tracks = [...(options.tracks ?? ["human:local", "human:local"])];
  const identityWhoami = vi.fn(async () => ({
    schema: "pointbreak.identity-whoami" as const,
    version: 1 as const,
    actorId: actors.shift() ?? "actor:human:kevin",
    diagnostics: [],
  }));
  const showAssessments = vi.fn(
    async (_repo: string, _values: { revisionId: string; track: string }) =>
      assessmentShow(options.assessments),
  );
  const addAssessment = vi.fn(
    async (
      _repo: string,
      values: {
        revisionId: string;
        track: string;
        assessment: AssessmentValue;
      },
    ) => ({
      schema: "pointbreak.review-assessment-add" as const,
      version: 1 as const,
      revisionId: values.revisionId,
      assessmentId: "assess:sha256:new",
      eventId: "evt:sha256:new",
      trackId: values.track,
      target: { kind: "revision", revisionId: values.revisionId },
      assessment: values.assessment,
      diagnostics: options.diagnostic ? [options.diagnostic] : [],
    }),
  );
  const cli = {
    identityWhoami,
    showAssessments,
    addAssessment,
  } as unknown as PointbreakCli;
  const refresh = vi.fn(async () => undefined);
  const showWarningMessage = vi.fn(async () => undefined);
  const humanWrites = new HumanWriteCoordinator(cli, {
    resolveTrack: vi.fn(() => tracks.shift() ?? "human:local"),
    showDiagnostic: showWarningMessage,
    refresh,
    showRefreshError: showWarningMessage,
  });
  const pickAssessment = vi.fn(
    async () => "accepted" as AssessmentValue | undefined,
  );
  const promptSummary = vi.fn(async () => "" as string | undefined);
  const pickReplacements = vi.fn(
    async (candidates: AssessmentView[]) => candidates,
  );
  const confirmAssessment = vi.fn(
    async (_context: {
      actorId: string;
      track: string;
      revisionId: string;
      assessment: AssessmentValue;
      summary?: string;
      replacementIds: string[];
    }) => true,
  );
  const routeHeadResolution = vi.fn(
    async (_node: AttentionItemNode) => undefined,
  );
  return {
    cli,
    identityWhoami,
    showAssessments,
    addAssessment,
    refresh,
    showWarningMessage,
    pickAssessment,
    promptSummary,
    pickReplacements,
    confirmAssessment,
    routeHeadResolution,
    dependencies: {
      humanWrites,
      pickAssessment,
      promptSummary,
      pickReplacements,
      confirmAssessment,
      routeHeadResolution,
      showInformationMessage: vi.fn(async () => undefined),
      showErrorMessage: vi.fn(async () => undefined),
    },
  };
}

function assessmentShow(assessments: AssessmentView[]): AssessmentShowDoc {
  return {
    schema: "pointbreak.review-assessment-show",
    version: 1,
    revisionId: "rev:sha256:one",
    filters: { trackId: "human:local", all: true, includeSummary: true },
    current: { status: "ambiguous", candidates: assessments },
    assessments,
    diagnostics: [],
  };
}

function assessment(
  id: string,
  actorId: string,
  trackId: string,
  target = { kind: "revision", revisionId: "rev:sha256:one" },
): AssessmentView {
  return {
    id: `assess:sha256:${id}`,
    trackId,
    target,
    assessment: "needs_changes",
    status: "current",
    writer: { actorId },
  };
}

function ambiguousNode(): AttentionItemNode {
  return node({
    id: "ambiguous_assessment:rev:sha256:one",
    kind: "ambiguous_assessment",
    tier: "primary",
    revisionId: "rev:sha256:one",
    freshness: { state: "current" },
    observedAt: "2026-07-15T00:00:00Z",
    assessments: [],
  });
}

function staleNode(
  headRevisionIds: string[],
  supersededBy = headRevisionIds,
): AttentionItemNode {
  return node({
    id: "stale_assessment:assess:sha256:stale",
    kind: "stale_assessment",
    tier: "primary",
    revisionId: "rev:sha256:stale",
    freshness: { state: "superseded", supersededBy },
    observedAt: "2026-07-15T00:00:00Z",
    assessmentId: "assess:sha256:stale",
    assessment: "accepted",
    trackId: "agent:review",
    recordedBy: "actor:agent:reviewer",
    headRevisionIds,
  });
}

function legacyStaleNode(): AttentionItemNode {
  const item: StaleAssessmentAttentionItem = {
    id: "stale_assessment:assess:sha256:legacy",
    kind: "stale_assessment",
    tier: "primary",
    revisionId: "rev:sha256:legacy",
    freshness: {
      state: "superseded",
      supersededBy: ["rev:sha256:intermediate"],
    },
    observedAt: "2026-07-15T00:00:00Z",
    assessmentId: "assess:sha256:legacy",
    assessment: "accepted",
    trackId: "agent:review",
    recordedBy: "actor:agent:reviewer",
  };
  return node(item);
}

function failedValidationNode(): AttentionItemNode {
  return node({
    id: "failed_validation:validation:sha256:failed",
    kind: "failed_validation",
    tier: "primary",
    revisionId: "rev:sha256:failed",
    freshness: { state: "current" },
    observedAt: "2026-07-15T00:00:00Z",
    validationCheckId: "validation:sha256:failed",
    checkName: "test",
    status: "failed",
    trackId: "agent:validation",
    recordedBy: "actor:agent:validator",
  });
}

function node(item: AttentionItemNode["item"]): AttentionItemNode {
  return {
    kind: "attention-item",
    label: item.kind,
    targetKey: "repo",
    folder: workspaceFolder("/repo") as never,
    description: item.tier,
    revisionId: item.revisionId,
    attentionId: item.id,
    item,
    lens: "attention",
    command: item.revisionId ? "pointbreak.openAnnotatedDiff" : undefined,
  };
}
