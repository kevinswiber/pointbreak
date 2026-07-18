import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AttentionItem } from "../src/store";
import {
  attentionHandoffs,
  copyWorkflowCommand,
  firstReviewHandoff,
  renderWorkflowHandoff,
  revisionHandoffs,
  type WorkflowCommandHandoff,
} from "../src/workflow-handoff";

// `workflow-handoff.ts` is the ONE producer of copyable CLI command text in the
// inspector: pure command construction from authoritative loaded ids plus
// explicit visible `<placeholder>` tokens, one shell-escape path, one
// HTML-escape path, and one clipboard helper. Review shows and copies commands;
// it never executes them.

const REV =
  "rev:sha256:4a94102ca0b2309bbd85f2ce8b9435c526e2000eeb8a234ec62f1ad0e9000a0c";
const REQ =
  "input-request:sha256:5e421401b50450d8adba1147550caac70e51d655e983955ca8344f60bf33baf8";
const A1 =
  "assess:sha256:021a4c2c462925e26601171bde3e7ec056738c2129e1e0b9dfe810382e726e1e";
const A2 =
  "assess:sha256:d63873796f65ab3f540af01867fb29062f28ca0e96368ab17fb1919dc2f36fb6";
const H1 =
  "rev:sha256:44d8d31d25e42f3ab4c9a6c5b290b9910b58378fa27fa3f2e9ec2515c42a2b6c";
const H2 =
  "rev:sha256:c7fd0544d4bca27cb052555f0dea7575a65fbbf59f88d9c40bfb9aa6c7395738";

const OUTCOMES = "<approved|rejected|dismissed|superseded|abandoned>";
const STATUSES = "<passed|failed|errored|skipped>";
const CALLS =
  "<accepted|accepted-with-follow-up|needs-changes|needs-clarification>";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("firstReviewHandoff", () => {
  it("suggests exactly the short-path capture with one visible placeholder", () => {
    const handoff = firstReviewHandoff();
    expect(handoff.command).toBe(
      'pointbreak capture --summary "<what changed>"',
    );
    expect(handoff.placeholders).toEqual(["<what changed>"]);
    expect(handoff.label).toBe("Capture your first revision");
  });
});

describe("revisionHandoffs", () => {
  it("offers the stage templates in order: claim, evidence, question, call, association", () => {
    const handoffs = revisionHandoffs(REV);
    expect(handoffs.map((h) => h.label)).toEqual([
      "Add a claim (observation)",
      "Record evidence (validation)",
      "Ask a question (input request)",
      "Make the call (assessment)",
      "Record a landed commit on this same revision",
    ]);
    expect(handoffs.map((h) => h.command)).toEqual([
      `pointbreak observation add --exact-revision ${REV} --track <your-track> --title "<claim title>" --body "<why it matters>"`,
      `pointbreak validation add --exact-revision ${REV} --track <your-track> --check-name "<check name>" --status ${STATUSES} --command "<command you ran>" --exit-code <exit-code> --summary "<what the run showed>"`,
      `pointbreak input-request open --revision ${REV} --track <your-track> --title "<question>" --reason manual-decision-required --mode advisory --body "<what needs an answer>"`,
      `pointbreak assessment add --exact-revision ${REV} --track <your-track> --assessment ${CALLS} --summary "<why this call>"`,
      `pointbreak association record --revision ${REV} --track <your-track> --commit <landed-commit>`,
    ]);
  });

  it("lists every visible placeholder for each template", () => {
    const handoffs = revisionHandoffs(REV);
    expect(handoffs[0].placeholders).toEqual([
      "<your-track>",
      "<claim title>",
      "<why it matters>",
    ]);
    expect(handoffs[1].placeholders).toEqual([
      "<your-track>",
      "<check name>",
      STATUSES,
      "<command you ran>",
      "<exit-code>",
      "<what the run showed>",
    ]);
    expect(handoffs[4].placeholders).toEqual([
      "<your-track>",
      "<landed-commit>",
    ]);
  });

  it("offers nothing without a loaded revision id", () => {
    expect(revisionHandoffs("")).toEqual([]);
  });
});

function item(overrides: Partial<AttentionItem>): AttentionItem {
  return {
    id: "x",
    kind: "unknown",
    tier: "primary",
    ...overrides,
  } as AttentionItem;
}

describe("attentionHandoffs (kind-specific, authoritative ids only)", () => {
  it("open_input_request offers the exact respond command", () => {
    const handoffs = attentionHandoffs(
      item({ kind: "open_input_request", inputRequestId: REQ }),
    );
    expect(handoffs.map((h) => h.command)).toEqual([
      `pointbreak input-request respond ${REQ} --outcome ${OUTCOMES} --reason "<answer>"`,
    ]);
    expect(handoffs[0].placeholders).toEqual([OUTCOMES, "<answer>"]);
  });

  it("open_input_request without its request id offers nothing", () => {
    expect(attentionHandoffs(item({ kind: "open_input_request" }))).toEqual([]);
    expect(
      attentionHandoffs(
        item({ kind: "open_input_request", inputRequestId: "" }),
      ),
    ).toEqual([]);
  });

  it("ambiguous_assessment replaces every current candidate in wire order", () => {
    const handoffs = attentionHandoffs(
      item({
        kind: "ambiguous_assessment",
        revisionId: REV,
        assessments: [
          { assessmentId: A1, assessment: "accepted", trackId: "human:kevin" },
          {
            assessmentId: A2,
            assessment: "needs_changes",
            trackId: "agent:codex",
          },
        ],
      }),
    );
    expect(handoffs.map((h) => h.command)).toEqual([
      `pointbreak assessment add --exact-revision ${REV} --track <your-track> --assessment ${CALLS} --summary "<why this call>" --replaces ${A1} --replaces ${A2}`,
    ]);
  });

  it("ambiguous_assessment offers nothing when incomplete", () => {
    // No revision anchor.
    expect(
      attentionHandoffs(
        item({
          kind: "ambiguous_assessment",
          assessments: [{ assessmentId: A1 }, { assessmentId: A2 }],
        }),
      ),
    ).toEqual([]);
    // Fewer than two current candidates is not ambiguous.
    expect(
      attentionHandoffs(
        item({
          kind: "ambiguous_assessment",
          revisionId: REV,
          assessments: [{ assessmentId: A1 }],
        }),
      ),
    ).toEqual([]);
    // A candidate missing its id would make the replacement set a guess.
    expect(
      attentionHandoffs(
        item({
          kind: "ambiguous_assessment",
          revisionId: REV,
          assessments: [{ assessmentId: A1 }, { assessment: "accepted" }],
        }),
      ),
    ).toEqual([]);
  });

  it("failed_validation fills revision, recorded track, and check name exactly", () => {
    const handoffs = attentionHandoffs(
      item({
        kind: "failed_validation",
        revisionId: REV,
        trackId: "agent:codex",
        checkName: "cargo clippy",
        status: "failed",
      }),
    );
    expect(handoffs.map((h) => h.command)).toEqual([
      `pointbreak validation add --exact-revision ${REV} --track agent:codex --check-name 'cargo clippy' --status ${STATUSES} --command "<command you ran>" --exit-code <exit-code> --summary "<what the re-run showed>"`,
    ]);
  });

  it("failed_validation offers nothing when incomplete", () => {
    const complete = {
      kind: "failed_validation",
      revisionId: REV,
      trackId: "agent:codex",
      checkName: "cargo clippy",
    };
    for (const missing of ["revisionId", "trackId", "checkName"] as const) {
      const incomplete = { ...complete, [missing]: undefined };
      expect(attentionHandoffs(item(incomplete))).toEqual([]);
    }
  });

  it("follow_up_outstanding offers one respond command per open request", () => {
    const other =
      "input-request:sha256:aaaa111111111111111111111111111111111111111111111111111111111111";
    const handoffs = attentionHandoffs(
      item({
        kind: "follow_up_outstanding",
        openInputRequestIds: [REQ, other],
      }),
    );
    expect(handoffs.map((h) => h.command)).toEqual([
      `pointbreak input-request respond ${REQ} --outcome ${OUTCOMES} --reason "<answer>"`,
      `pointbreak input-request respond ${other} --outcome ${OUTCOMES} --reason "<answer>"`,
    ]);
    expect(handoffs.map((h) => h.label)).toEqual([
      "Respond to open follow-up request 1 of 2",
      "Respond to open follow-up request 2 of 2",
    ]);
  });

  it("follow_up_outstanding offers nothing without complete request ids", () => {
    expect(attentionHandoffs(item({ kind: "follow_up_outstanding" }))).toEqual(
      [],
    );
    expect(
      attentionHandoffs(
        item({ kind: "follow_up_outstanding", openInputRequestIds: [] }),
      ),
    ).toEqual([]);
    expect(
      attentionHandoffs(
        item({ kind: "follow_up_outstanding", openInputRequestIds: [REQ, ""] }),
      ),
    ).toEqual([]);
  });

  it("competing_heads names every loaded head and conditions the capture visibly", () => {
    const handoffs = attentionHandoffs(
      item({ kind: "competing_heads", headRevisionIds: [H1, H2] }),
    );
    expect(handoffs.map((h) => h.command)).toEqual([
      `pointbreak capture --summary "<what changed>" --supersedes ${H1} --supersedes ${H2}`,
    ]);
    expect(handoffs[0].label).toBe(
      "Capture a replacement only when genuinely new content replaces every head",
    );
  });

  it("competing_heads offers nothing without at least two complete heads", () => {
    expect(
      attentionHandoffs(
        item({ kind: "competing_heads", headRevisionIds: [H1] }),
      ),
    ).toEqual([]);
    expect(
      attentionHandoffs(
        item({ kind: "competing_heads", headRevisionIds: [H1, ""] }),
      ),
    ).toEqual([]);
  });

  it("unsupported kinds never receive a guessed command", () => {
    expect(
      attentionHandoffs(
        item({ kind: "stale_assessment", revisionId: REV, assessmentId: A1 }),
      ),
    ).toEqual([]);
    expect(attentionHandoffs(item({ kind: "future_kind" }))).toEqual([]);
  });

  it("quotes a leading-= value so zsh cannot expand it to a command path", () => {
    // zsh =word expansion: an unquoted `=echo` becomes /bin/echo, so a check
    // name with a leading = must leave the safe charset and be quoted.
    const handoffs = attentionHandoffs(
      item({
        kind: "failed_validation",
        revisionId: REV,
        trackId: "agent:codex",
        checkName: "=echo",
      }),
    );
    expect(handoffs[0].command).toContain("--check-name '=echo'");
  });

  it("shell-escapes hostile authoritative values through the one quote path", () => {
    const hostile = 'rev:sha256:x"; rm -rf ~';
    const handoffs = attentionHandoffs(
      item({ kind: "competing_heads", headRevisionIds: [H1, hostile] }),
    );
    expect(handoffs[0].command).toContain(
      `--supersedes ${H1} --supersedes 'rev:sha256:x"; rm -rf ~'`,
    );
  });
});

describe("renderWorkflowHandoff (one escaped renderer, displayed == copied)", () => {
  function mount(handoff: WorkflowCommandHandoff): HTMLElement {
    document.body.innerHTML = renderWorkflowHandoff(handoff);
    const root = document.body.querySelector<HTMLElement>(
      "[data-workflow-handoff]",
    );
    if (!root) throw new Error("workflow handoff root missing");
    return root;
  }

  it("renders the label, the command as code, and a copy control", () => {
    const root = mount(firstReviewHandoff());
    expect(root.classList.contains("workflow-handoff")).toBe(true);
    expect(root.querySelector(".workflow-handoff-label")?.textContent).toBe(
      "Capture your first revision",
    );
    const code = root.querySelector<HTMLElement>("code[data-workflow-command]");
    expect(code?.textContent).toBe(
      'pointbreak capture --summary "<what changed>"',
    );
    const button = root.querySelector<HTMLButtonElement>(
      "button[data-copy-workflow-command]",
    );
    expect(button?.type).toBe("button");
    expect(button?.textContent).toBe("copy");
  });

  it("keeps placeholder tokens visible and marked without changing the text", () => {
    const [claim] = revisionHandoffs(REV);
    const root = mount(claim);
    const code = root.querySelector<HTMLElement>("[data-workflow-command]");
    expect(code?.textContent).toBe(claim.command);
    const marked = Array.from(
      root.querySelectorAll<HTMLElement>(".workflow-placeholder"),
    ).map((span) => span.textContent);
    expect(marked).toEqual(claim.placeholders);
  });

  it("HTML-escapes hostile values instead of injecting markup", () => {
    const hostile = "rev:sha256:<img src=x onerror=alert(1)>";
    const [claim] = revisionHandoffs(hostile);
    const root = mount(claim);
    expect(root.querySelector("img")).toBeNull();
    expect(root.querySelector("[data-workflow-command]")?.textContent).toBe(
      claim.command,
    );
  });
});

describe("copyWorkflowCommand (clipboard-only, advisory)", () => {
  function mountWithButton(): {
    button: HTMLElement;
    command: string;
  } {
    document.body.innerHTML = renderWorkflowHandoff(firstReviewHandoff());
    const button = document.body.querySelector<HTMLElement>(
      "[data-copy-workflow-command]",
    );
    if (!button) throw new Error("copy button missing");
    return { button, command: firstReviewHandoff().command };
  }

  function stubClipboard(writeText: (text: string) => Promise<void>): void {
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
  }

  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("copies exactly the displayed command text and restores the label", async () => {
    const { button, command } = mountWithButton();
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard(writeText);
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);

    await copyWorkflowCommand(button);
    expect(writeText).toHaveBeenCalledWith(command);
    expect(button.textContent).toBe("copied");
    vi.advanceTimersByTime(1300);
    expect(button.textContent).toBe("copy");
    // Copying is clipboard-only: no fetch, no write endpoint, no navigation.
    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it("reports failure advisorily and restores the label", async () => {
    const { button } = mountWithButton();
    stubClipboard(vi.fn().mockRejectedValue(new Error("denied")));

    await copyWorkflowCommand(button);
    expect(button.textContent).toBe("copy failed");
    vi.advanceTimersByTime(1300);
    expect(button.textContent).toBe("copy");
  });

  it("degrades to the failure label when no clipboard API exists", async () => {
    const { button } = mountWithButton();
    Object.defineProperty(navigator, "clipboard", {
      value: undefined,
      configurable: true,
    });

    await copyWorkflowCommand(button);
    expect(button.textContent).toBe("copy failed");
  });
});
