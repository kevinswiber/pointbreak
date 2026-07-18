import { REQUIRED_DOCUMENTS, type VersionDoc } from "../src/cli";

export { REQUIRED_DOCUMENTS };

export const VERSION_DOC: VersionDoc = {
  schema: "pointbreak.version",
  version: 1,
  cliVersion: "0.7.0",
  build: {
    source: "git",
    commit: "d2bc01650076314897bb8c30ba57623640c0d257",
    describe: "v0.7.0-34-gd2bc016",
    dirty: false,
  },
  documents: { ...REQUIRED_DOCUMENTS },
  diagnostics: [],
};

export const VERSION_JSON = JSON.stringify(VERSION_DOC);

export const ATTENTION_JSON = JSON.stringify({
  schema: "pointbreak.attention-list",
  version: 1,
  items: [
    {
      id: "stale_assessment:assess:sha256:87616bb4",
      tier: "primary",
      revisionId: "rev:sha256:415c68e1",
      freshness: {
        state: "superseded",
        supersededBy: ["rev:sha256:da444b13"],
      },
      observedAt: "unix-ms:1783738358085",
      kind: "stale_assessment",
      assessmentId: "assess:sha256:87616bb4",
      assessment: "accepted",
      trackId: "agent:codex-screens-r",
      recordedBy: "actor:agent:codex",
      headRevisionIds: ["rev:sha256:da444b13"],
    },
  ],
  diagnostics: [],
});

export const REVISION_LIST_JSON = JSON.stringify({
  schema: "pointbreak.review-revision-list",
  version: 1,
  entries: [
    {
      capturedAt: "2026-07-12T20:26:19.794Z",
      revisionId: "rev:sha256:9442bfeb",
      objectId: "obj:sha256:898a112f",
      source: { kind: "git_worktree", mode: "combined_head_to_working_tree" },
      mergeStatus: "merged",
      groupedRevisionIds: ["rev:sha256:9442bfeb"],
    },
  ],
  revisionCount: 1,
  eventCount: 2,
  eventSetHash: "sha256:1234",
  diagnostics: [],
});
