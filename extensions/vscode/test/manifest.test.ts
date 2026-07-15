import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import pkg from "../package.json";

const extensionSource = sourceFiles(
  fileURLToPath(new URL("../src", import.meta.url)),
)
  .map((file) => readFileSync(file, "utf8"))
  .join("\n");

it("activates only for explicit Pointbreak surfaces", () => {
  expect(pkg.activationEvents ?? []).not.toContain("onStartupFinished");
  expect(pkg.activationEvents ?? []).not.toContain("*");
});

it("keeps retired startup restoration and raw-path state out of production", () => {
  expect(extensionSource).not.toMatch(
    /restoreReviewServers|reviewServerRegistry|folderUri|onStartupFinished/,
  );
  expect(extensionSource).not.toContain("globalState");
});

it("carries the untouchable identity and license", () => {
  expect(pkg.publisher).toBe("pointbreak");
  expect(pkg.name).toBe("pointbreak");
  expect(pkg.license).toBe("Apache-2.0");
});

it("contributes the Review view and its commands", () => {
  expect(pkg.activationEvents).toEqual([
    "onView:pointbreak.attention",
    "onCommand:pointbreak.refreshAttention",
    "onCommand:pointbreak.capture",
    "onCommand:pointbreak.openAnnotatedDiff",
    "onCommand:pointbreak.openInReview",
    "onCommand:pointbreak.stopInspect",
    "onCommand:pointbreak.addObservationFromSelection",
    "onCommand:pointbreak.respondInputRequest",
    "onCommand:pointbreak.assessAttention",
    "onCommand:pointbreak.captureAttentionResolution",
  ]);
  expect(pkg.contributes.views.pointbreak).toContainEqual({
    id: "pointbreak.attention",
    name: "Review",
  });
  expect(pkg.contributes.commands.map(({ command }) => command)).toEqual([
    "pointbreak.refreshAttention",
    "pointbreak.capture",
    "pointbreak.openAnnotatedDiff",
    "pointbreak.openInReview",
    "pointbreak.stopInspect",
    "pointbreak.addObservationFromSelection",
    "pointbreak.respondInputRequest",
    "pointbreak.assessAttention",
    "pointbreak.captureAttentionResolution",
  ]);
  expect(
    pkg.contributes.commands.find(
      ({ command }) => command === "pointbreak.addObservationFromSelection",
    ),
  ).toMatchObject({
    enablement: "pointbreak.hasSourceReviewContext",
  });
  expect(pkg.contributes.menus["view/item/context"]).toContainEqual({
    command: "pointbreak.openInReview",
    when: "view == pointbreak.attention && (viewItem == pointbreak.revision || viewItem == pointbreak.attentionItem || viewItem == pointbreak.attention.inputRequest || viewItem == pointbreak.attention.assessment)",
    group: "navigation@2",
  });
  expect(pkg.contributes.menus["view/item/context"]).toContainEqual({
    command: "pointbreak.captureAttentionResolution",
    when: "view == pointbreak.attention && viewItem == pointbreak.attention.headResolution",
    group: "inline@1",
  });
  expect(pkg.contributes.menus["view/item/context"]).toContainEqual({
    command: "pointbreak.assessAttention",
    when: "view == pointbreak.attention && viewItem == pointbreak.attention.assessment",
    group: "inline@1",
  });
  expect(pkg.contributes.menus["view/item/context"]).toContainEqual({
    command: "pointbreak.respondInputRequest",
    when: "view == pointbreak.attention && viewItem == pointbreak.attention.inputRequest",
    group: "inline@1",
  });
  expect(pkg.contributes.menus.commandPalette).toEqual([
    { command: "pointbreak.respondInputRequest", when: "false" },
    { command: "pointbreak.assessAttention", when: "false" },
    { command: "pointbreak.captureAttentionResolution", when: "false" },
  ]);
  expect(
    pkg.contributes.configuration.properties["pointbreak.reviewUrl"],
  ).toMatchObject({
    default: "",
    scope: "resource",
    description:
      "Optional URL for an externally managed Pointbreak Review server for this folder.",
  });
  expect(
    pkg.contributes.configuration.properties["pointbreak.reviewUrl"]
      .description,
  ).not.toMatch(/restore|remember|port/i);
  expect(
    pkg.contributes.configuration.properties["pointbreak.observationTrack"],
  ).toMatchObject({
    type: "string",
    default: "human:local",
    scope: "resource",
    description: "Default track for human-authored Pointbreak writes.",
  });
});

function sourceFiles(directory: string): string[] {
  return readdirSync(directory)
    .flatMap((entry) => {
      const path = join(directory, entry);
      return statSync(path).isDirectory() ? sourceFiles(path) : [path];
    })
    .filter((path) => path.endsWith(".ts"));
}
