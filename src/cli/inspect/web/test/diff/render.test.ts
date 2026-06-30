import { describe, expect, it } from "vitest";
import type { Annotation, DiffArtifact, DiffFile } from "../../src/diff/render";
import {
  classifyLowSignal,
  fileFactCount,
  fileForFact,
  filePathLabel,
  fileRowCount,
  rangeTouchesCapturedRows,
  renderAnnotation,
  renderDiff,
  renderDiffFactVicinity,
  renderDiffFileBody,
  renderDiffFileHeader,
  renderDiffNavFilters,
  renderDiffNavSummary,
  unanchoredReason,
} from "../../src/diff/render";
import snapshotJson from "../fixtures/snapshot.json";

function parse(html: string): Document {
  return new DOMParser().parseFromString(html, "text/html");
}

const artifact = snapshotJson as unknown as DiffArtifact;
const libFile = (artifact.snapshot?.files ?? [])[0] as DiffFile;

// A range observation anchored to src/lib.rs:2 on the new side. In the fixture
// new_line 2 is the "    42" added row, so this fact anchors to a captured row.
const anchoredObs: Annotation = {
  kind: "observation",
  id: "obs:sha256:anchored",
  title: "Observed change",
  track: "agent:codex",
  body: "looks good",
  tags: ["needs-tests"],
  target: { kind: "range", filePath: "src/lib.rs", startLine: 2, endLine: 2 },
};

// A revision-level assessment never anchors to a diff line.
const unanchoredAssessment: Annotation = {
  kind: "assessment",
  id: "assess:sha256:broad",
  title: "assessment: accepted",
  track: "agent:codex",
  body: "",
  tags: [],
  target: { kind: "revision" },
};

function largeFile(rows: number): DiffFile {
  return {
    status: "modified",
    new_path: "src/big.rs",
    old_path: "src/big.rs",
    hunks: [
      {
        header: "@@ -1,1 +1,1 @@",
        rows: Array.from({ length: rows }, (_, i) => ({
          kind: "context",
          old_line: i + 1,
          new_line: i + 1,
          text: `line ${i + 1}`,
        })),
      },
    ],
  };
}

describe("filePathLabel", () => {
  it("shows both sides for a rename, else the single path", () => {
    expect(filePathLabel({ old_path: "a.rs", new_path: "b.rs" })).toBe(
      "a.rs → b.rs",
    );
    expect(filePathLabel({ old_path: "a.rs", new_path: "a.rs" })).toBe("a.rs");
    expect(filePathLabel({ new_path: "only.rs" })).toBe("only.rs");
    expect(filePathLabel({ old_path: "gone.rs" })).toBe("gone.rs");
    expect(filePathLabel({})).toBe("(unknown path)");
  });
});

describe("fileRowCount", () => {
  it("sums the rows across every hunk", () => {
    expect(fileRowCount(libFile)).toBe(9);
    expect(fileRowCount({})).toBe(0);
    expect(fileRowCount(largeFile(501))).toBe(501);
  });
});

describe("classifyLowSignal", () => {
  it("names binary and mode-only files", () => {
    expect(classifyLowSignal({ is_binary: true })).toBe("binary");
    expect(classifyLowSignal({ is_mode_only: true })).toBe("mode change only");
  });

  it("names a pure rename, with the similarity when present", () => {
    expect(
      classifyLowSignal({
        status: "renamed",
        old_path: "a.rs",
        new_path: "b.rs",
        hunks: [],
        similarity: 95,
      }),
    ).toBe("rename 95%");
    expect(
      classifyLowSignal({
        old_path: "a.rs",
        new_path: "b.rs",
        hunks: [],
      }),
    ).toBe("rename");
  });

  it("flags a file over the large-file row budget", () => {
    expect(classifyLowSignal(largeFile(501))).toBe("large file");
    expect(classifyLowSignal(largeFile(500))).toBeNull();
  });

  it("returns null for a normal content-bearing file", () => {
    expect(classifyLowSignal(libFile)).toBeNull();
  });
});

describe("fileFactCount", () => {
  it("counts anchored facts whose target file is either side of the file", () => {
    expect(fileFactCount(libFile, [anchoredObs])).toBe(1);
    expect(fileFactCount(libFile, [unanchoredAssessment])).toBe(0);
    expect(
      fileFactCount({ old_path: "a.rs", new_path: "b.rs" }, [
        { ...anchoredObs, target: { filePath: "a.rs" } },
      ]),
    ).toBe(1);
  });
});

describe("fileForFact", () => {
  it("finds the file matching either path, else null", () => {
    const files = artifact.snapshot?.files ?? [];
    expect(fileForFact(files, "src/lib.rs")).toBe(libFile);
    expect(fileForFact(files, "nope.rs")).toBeNull();
  });
});

describe("rangeTouchesCapturedRows", () => {
  it("is true when a captured row falls in the fact's line span", () => {
    expect(rangeTouchesCapturedRows(anchoredObs, libFile)).toBe(true);
  });

  it("is false when the span is outside every captured row", () => {
    expect(
      rangeTouchesCapturedRows(
        { ...anchoredObs, target: { kind: "range", startLine: 99 } },
        libFile,
      ),
    ).toBe(false);
  });

  it("treats a missing file or non-range target as touching (or not)", () => {
    expect(rangeTouchesCapturedRows(anchoredObs, null)).toBe(false);
    expect(
      rangeTouchesCapturedRows(
        { ...anchoredObs, target: { kind: "file" } },
        libFile,
      ),
    ).toBe(true);
  });
});

describe("unanchoredReason", () => {
  const filePaths = new Set(["src/lib.rs"]);

  it("labels a broad assessment", () => {
    expect(unanchoredReason(unanchoredAssessment, filePaths)).toBe(
      "broad assessment",
    );
  });

  it("labels a revision-level or fileless target", () => {
    expect(
      unanchoredReason(
        { ...anchoredObs, kind: "observation", target: { kind: "revision" } },
        filePaths,
      ),
    ).toBe("revision-level");
    expect(
      unanchoredReason(
        { ...anchoredObs, kind: "observation", target: {} },
        filePaths,
      ),
    ).toBe("revision-level");
  });

  it("labels a range whose file is captured but line is outside the rows", () => {
    expect(
      unanchoredReason(
        {
          ...anchoredObs,
          kind: "observation",
          target: { kind: "range", filePath: "src/lib.rs" },
        },
        filePaths,
      ),
    ).toBe("line outside captured rows");
  });

  it("labels a file missing from the snapshot", () => {
    expect(
      unanchoredReason(
        {
          ...anchoredObs,
          kind: "observation",
          target: { kind: "file", filePath: "gone.rs" },
        },
        filePaths,
      ),
    ).toBe("file missing from snapshot");
  });
});

describe("renderAnnotation", () => {
  it("renders a kinded, tracked annotation with its body and tags", () => {
    const doc = parse(renderAnnotation(anchoredObs, false));
    const anno = doc.querySelector(".anno");
    expect(anno?.classList.contains("anno-observation")).toBe(true);
    expect(anno?.getAttribute("data-anno")).toBe("obs:sha256:anchored");
    expect(doc.querySelector(".anno-kind-observation")?.textContent).toBe(
      "observation",
    );
    expect(doc.querySelector(".anno-track")?.textContent).toBe("agent:codex");
    expect(doc.querySelector(".anno-title")?.textContent).toContain(
      "Observed change",
    );
    expect(doc.querySelector(".badge")?.textContent).toBe("needs-tests");
    expect(doc.querySelector(".anno-body")?.textContent).toContain(
      "looks good",
    );
  });

  it("includes a location only when asked and the target has a file", () => {
    expect(
      parse(renderAnnotation(anchoredObs, true)).querySelector(".anno-loc")
        ?.textContent,
    ).toBe("src/lib.rs:2-2");
    expect(
      parse(renderAnnotation(anchoredObs, false)).querySelector(".anno-loc"),
    ).toBeNull();
    expect(
      parse(renderAnnotation(unanchoredAssessment, true)).querySelector(
        ".anno-loc",
      ),
    ).toBeNull();
  });

  it("renders a markdown body when the content type selects it", () => {
    const doc = parse(
      renderAnnotation(
        {
          ...anchoredObs,
          body: "# Heading",
          bodyContentType: "text/markdown",
        },
        false,
      ),
    );
    expect(doc.querySelector(".markdown-body h1")?.textContent).toBe("Heading");
  });
});

describe("renderDiffFileHeader", () => {
  it("exposes the disclosure state and the eager fact-count badge", () => {
    const header = parse(
      renderDiffFileHeader(libFile, [anchoredObs], null, true),
    ).querySelector("header.dfile-head");
    expect(header?.getAttribute("role")).toBe("button");
    expect(header?.getAttribute("aria-expanded")).toBe("true");
    expect(header?.querySelector(".dstatus")?.textContent).toBe("modified");
    expect(header?.querySelector(".dpath")?.textContent).toBe("src/lib.rs");
    expect(header?.querySelector(".dfile-notes")?.textContent).toBe("1 note");
  });

  it("surfaces the low-signal reason and drops the badge with no facts", () => {
    const header = parse(
      renderDiffFileHeader(
        { is_binary: true, status: "modified", new_path: "logo.png" },
        [],
        "binary",
        false,
      ),
    ).querySelector("header.dfile-head");
    expect(header?.getAttribute("aria-expanded")).toBe("false");
    expect(header?.querySelector(".dfile-summary")?.textContent).toBe("binary");
    expect(header?.querySelector(".dfile-notes")).toBeNull();
  });
});

describe("renderDiffFileBody", () => {
  it("anchors a range fact to its captured row via the side:line map", () => {
    const doc = parse(renderDiffFileBody(libFile, [anchoredObs]));
    const noted = doc.querySelector(".drow-noted");
    expect(noted?.getAttribute("data-anno")).toBe("obs:sha256:anchored");
    // new_line 2 is the "    42" added row the fact anchors to.
    expect(noted?.querySelector(".dtext")?.textContent).toBe("    42");
    expect(noted?.classList.contains("drow-added")).toBe(true);
    // The annotation renders inline, once, after its row.
    expect(doc.querySelectorAll(".anno[data-anno]")).toHaveLength(1);
    expect(doc.querySelector(".dhunk")?.textContent).toBe("@@ -1,7 +1,7 @@");
  });

  it("renders a no-content note for an empty content-bearing file", () => {
    const doc = parse(
      renderDiffFileBody({ status: "added", new_path: "empty.rs" }, []),
    );
    expect(doc.querySelector(".drow-meta")?.textContent).toContain(
      "(no captured content)",
    );
  });
});

describe("renderDiffFactVicinity", () => {
  it("summarizes facts first with a hydrate-all affordance", () => {
    const doc = parse(renderDiffFactVicinity(libFile, [anchoredObs]));
    const vicinity = doc.querySelector(".diff-fact-vicinity");
    expect(vicinity?.getAttribute("data-fact-vicinity")).toBe("true");
    const btn = doc.querySelector("button[data-render-diff-file]");
    expect(btn?.textContent).toBe("Render all rows");
    expect(doc.querySelectorAll(".anno[data-anno]")).toHaveLength(1);
  });
});

describe("renderDiffNavSummary", () => {
  it("renders the file/fact/unanchored counts", () => {
    const doc = parse(
      renderDiffNavSummary({ fileCount: 3, factCount: 5, unanchoredCount: 2 }),
    );
    const summary = doc.querySelector(".diff-nav-summary");
    expect(summary?.getAttribute("aria-label")).toBe("diff summary");
    const bolds = Array.from(
      summary?.querySelectorAll("b") ?? [],
      (b) => b.textContent,
    );
    expect(bolds).toEqual(["3", "5", "2"]);
  });
});

describe("renderDiffNavFilters", () => {
  it("renders the three filters and presses the active one", () => {
    for (const active of ["all", "with-facts", "unanchored"] as const) {
      const doc = parse(renderDiffNavFilters(active));
      const buttons = doc.querySelectorAll("button[data-diff-nav-filter]");
      expect(buttons).toHaveLength(3);
      const pressed = doc.querySelector('button[aria-pressed="true"]');
      expect(pressed?.getAttribute("data-diff-nav-filter")).toBe(active);
    }
  });
});

describe("renderDiff", () => {
  it("returns html plus a ctx partitioning the facts (no globals)", () => {
    const { html, ctx } = renderDiff("obj:sha256:lib", artifact, [
      anchoredObs,
      unanchoredAssessment,
    ]);
    expect(ctx.objectId).toBe("obj:sha256:lib");
    expect(ctx.files).toBe(artifact.snapshot?.files);
    expect(ctx.anchored).toEqual([anchoredObs]);
    expect(ctx.unanchored).toEqual([unanchoredAssessment]);
    expect(ctx.filePaths.has("src/lib.rs")).toBe(true);

    const doc = parse(html);
    // The summary names the fact breakdown and the unanchored count.
    expect(doc.querySelector(".anno-summary")?.textContent).toContain(
      "not anchored to a diff line",
    );
    // The unanchored assessment renders in its own group up top.
    expect(doc.querySelector(".anno-group .anno-assessment")).not.toBeNull();
  });

  it("renders each file as an accordion section with the disclosure on the header", () => {
    const { html } = renderDiff("obj:sha256:lib", artifact, [anchoredObs]);
    const doc = parse(html);
    const section = doc.querySelector("section.dfile");
    expect(section?.getAttribute("data-dfile")).toBe("0");
    expect(section?.getAttribute("data-expanded")).toBe("true");
    // The section wrapper does not own the disclosure aria state.
    expect(section?.hasAttribute("aria-expanded")).toBe(false);
    expect(section?.querySelector("header.dfile-head")).not.toBeNull();
    const body = section?.querySelector(".dfile-body");
    expect(body?.getAttribute("data-dfile-body")).toBe("0");
    expect(body?.getAttribute("data-rendered")).toBe("1");
  });

  it("marks a low-signal file and renders it collapsed", () => {
    const binaryArtifact: DiffArtifact = {
      snapshot: {
        files: [{ status: "modified", new_path: "logo.png", is_binary: true }],
      },
    };
    const { html } = renderDiff("obj:sha256:bin", binaryArtifact, []);
    const section = parse(html).querySelector("section.dfile");
    expect(section?.getAttribute("data-lowsignal")).toBe("binary");
    expect(section?.getAttribute("data-expanded")).toBe("false");
    expect(section?.querySelector(".dfile-body")?.innerHTML).toBe("");
  });

  it("renders an annotated large file as a fact vicinity, not full rows", () => {
    const big = largeFile(600);
    const fact: Annotation = {
      ...anchoredObs,
      target: { kind: "file", filePath: "src/big.rs" },
    };
    const { html } = renderDiff(
      "obj:sha256:big",
      { snapshot: { files: [big] } },
      [fact],
    );
    const body = parse(html).querySelector(".dfile-body");
    expect(body?.getAttribute("data-fact-vicinity")).toBe("true");
    expect(body?.querySelector(".diff-fact-vicinity")).not.toBeNull();
    expect(body?.querySelector(".dhunk")).toBeNull();
  });

  it("notes an empty snapshot", () => {
    const { html } = renderDiff(
      "obj:sha256:empty",
      { snapshot: { files: [] } },
      [],
    );
    expect(parse(html).querySelector(".empty")?.textContent).toContain(
      "No files captured in this snapshot.",
    );
  });
});

describe("renderDiffFileBody syntax tokens", () => {
  it("renders syntax spans inside .dtext when tokens present", () => {
    const file: DiffFile = {
      status: "modified",
      old_path: "a.rs",
      new_path: "a.rs",
      hunks: [
        {
          header: "@@ -1 +1 @@",
          rows: [
            {
              kind: "added",
              old_line: null,
              new_line: 1,
              text: "let x",
              tokens: [{ start: 0, end: 3, kind: "keyword" }],
            },
          ],
        },
      ],
    };
    const html = renderDiffFileBody(file, []);
    expect(html).toContain('<span class="tok tok-keyword">let</span>');
  });

  it("renders plain escaped text when tokens absent (unchanged from today)", () => {
    const file: DiffFile = {
      status: "modified",
      old_path: "a.rs",
      new_path: "a.rs",
      hunks: [
        {
          header: "@@ -1 +1 @@",
          rows: [{ kind: "context", old_line: 1, new_line: 1, text: "a < b" }],
        },
      ],
    };
    const html = renderDiffFileBody(file, []);
    expect(html).toContain("a &lt; b");
    expect(html).not.toContain("tok-");
  });
});
