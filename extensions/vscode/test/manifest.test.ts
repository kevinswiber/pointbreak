import { expect, it } from "vitest";
import pkg from "../package.json";

it("never activates on startup (lazy activation only)", () => {
  expect(pkg.activationEvents ?? []).not.toContain("onStartupFinished");
  expect(pkg.activationEvents ?? []).not.toContain("*");
});

it("carries the untouchable identity and license", () => {
  expect(pkg.publisher).toBe("pointbreak");
  expect(pkg.name).toBe("pointbreak");
  expect(pkg.license).toBe("Apache-2.0");
});

it("contributes the lazy Review view and its commands", () => {
  expect(pkg.activationEvents).toEqual([
    "onView:pointbreak.attention",
    "onCommand:pointbreak.refreshAttention",
    "onCommand:pointbreak.capture",
    "onCommand:pointbreak.openInReview",
  ]);
  expect(pkg.contributes.views.pointbreak).toContainEqual({
    id: "pointbreak.attention",
    name: "Review",
  });
  expect(pkg.contributes.commands.map(({ command }) => command)).toEqual([
    "pointbreak.refreshAttention",
    "pointbreak.capture",
    "pointbreak.openInReview",
  ]);
  expect(
    pkg.contributes.configuration.properties["pointbreak.reviewUrl"],
  ).toMatchObject({
    default: "http://127.0.0.1:7878",
  });
});
