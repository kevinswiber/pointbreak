import { describe, expect, it } from "vitest";
import {
  matchDiffFiles as matchInspectorFiles,
  renderDiff as renderInspectorDiff,
} from "../../../src/cli/inspect/web/src/diff/render";
import inspectorSnapshot from "../../../src/cli/inspect/web/test/fixtures/snapshot.json";
import type {
  Annotation,
  DiffArtifact,
  DiffFile,
} from "../src/webview/diff/render";
import {
  matchDiffFiles as matchWebviewFiles,
  renderDiff as renderWebviewDiff,
} from "../src/webview/diff/render";

describe("webview renderer parity", () => {
  it("matches the inspector renderer across the complete semantic fixture", () => {
    const artifact = parityArtifact();
    const annotations = parityAnnotations();

    const inspector = renderInspectorDiff(
      "obj:sha256:renderer-parity",
      artifact,
      annotations,
    );
    const webview = renderWebviewDiff(
      "obj:sha256:renderer-parity",
      artifact,
      annotations,
    );

    expect(normalizeHtml(webview.html)).toBe(normalizeHtml(inspector.html));
    expect(navigatorFacts(webview.ctx)).toEqual(navigatorFacts(inspector.ctx));
    expect(navigatorFacts(webview.ctx)).toEqual({
      anchored: 3,
      files: 2,
      unanchored: 1,
    });

    expect(webview.html).toContain("tok tok-keyword");
    expect(webview.html).toMatch(/class="[^"]*\bemph\b[^"]*"/);
    expect(webview.html).toContain("anno-observation");
    expect(webview.html).toContain("anno-input-request");
    expect(webview.html).toContain("anno-assessment");
    expect(webview.html).toContain("markdown-body");
    expect(webview.html).toContain("<strong>renderer parity</strong>");
    expect(webview.html).toContain("not anchored to a diff line");
    expect(webview.html).toContain('data-fact-vicinity="true"');

    for (const query of ["path:lib", "has:facts", "is:unanchored"]) {
      expect(fileLabels(matchWebviewFiles(webview.ctx, query).files)).toEqual(
        fileLabels(matchInspectorFiles(inspector.ctx, query).files),
      );
    }
  });
});

function parityArtifact(): DiffArtifact {
  const artifact = structuredClone(inspectorSnapshot) as DiffArtifact;
  const firstFile = artifact.snapshot?.files?.[0];
  const addedRow = firstFile?.hunks?.[0]?.rows?.find(
    (row) => row.kind === "added",
  );
  if (!artifact.snapshot || !firstFile || !addedRow) {
    throw new Error(
      "renderer parity fixture is missing its captured diff rows",
    );
  }
  addedRow.emphasis = [{ start: 4, end: addedRow.text?.length ?? 4 }];
  artifact.snapshot.files?.push(largeFile());
  return artifact;
}

function parityAnnotations(): Annotation[] {
  return [
    {
      kind: "observation",
      id: "obs:sha256:renderer-parity",
      title: "Observed numeric change",
      track: "agent:fixture",
      body: "Verify **renderer parity**.",
      bodyContentType: "text/markdown",
      tags: ["rendering"],
      target: {
        kind: "range",
        filePath: "src/lib.rs",
        startLine: 2,
        endLine: 2,
      },
    },
    {
      kind: "input-request",
      id: "request:sha256:renderer-parity",
      title: "Confirm the captured file",
      track: "agent:fixture",
      body: "The file-level request stays anchored.",
      tags: ["advisory"],
      target: { kind: "file", filePath: "src/lib.rs" },
    },
    {
      kind: "assessment",
      id: "assess:sha256:renderer-parity",
      title: "assessment: accepted",
      track: "agent:fixture",
      body: "",
      tags: [],
      target: { kind: "revision" },
    },
    {
      kind: "observation",
      id: "obs:sha256:large-file",
      title: "Inspect the large-file vicinity",
      track: "agent:fixture",
      body: "Only the fact vicinity is eager.",
      tags: [],
      target: { kind: "file", filePath: "src/large.rs" },
    },
  ];
}

function largeFile(): DiffFile {
  return {
    status: "modified",
    old_path: "src/large.rs",
    new_path: "src/large.rs",
    hunks: [
      {
        header: "@@ -1,501 +1,501 @@",
        rows: Array.from({ length: 501 }, (_, index) => ({
          kind: "context",
          old_line: index + 1,
          new_line: index + 1,
          text: `line ${index + 1}`,
        })),
      },
    ],
  };
}

function normalizeHtml(html: string): string {
  return html
    .replace(/\sid="[^"]*"/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function navigatorFacts(ctx: {
  files: readonly DiffFile[];
  anchored: readonly Annotation[];
  unanchored: readonly Annotation[];
}): { files: number; anchored: number; unanchored: number } {
  return {
    files: ctx.files.length,
    anchored: ctx.anchored.length,
    unanchored: ctx.unanchored.length,
  };
}

function fileLabels(files: readonly DiffFile[]): string[] {
  return files.map((file) => file.new_path ?? file.old_path ?? "");
}
