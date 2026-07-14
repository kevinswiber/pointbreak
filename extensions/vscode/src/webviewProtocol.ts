import type { DiffRenderData } from "./diffDataSource";

export interface ReviewPanelFocus {
  readonly kind: "attention";
  readonly id: string;
}

export interface SnapshotRangeTarget {
  readonly filePath: string;
  readonly side: "old" | "new";
  readonly startLine: number;
  readonly endLine: number;
}

export type HostToWebview =
  | {
      readonly type: "render";
      readonly data: DiffRenderData;
      readonly focus?: ReviewPanelFocus;
    }
  | { readonly type: "focus"; readonly focus?: ReviewPanelFocus }
  | { readonly type: "error"; readonly message: string }
  | { readonly type: "freshness"; readonly changed: boolean };

export type WebviewToHost =
  | { readonly type: "ready" }
  | { readonly type: "openSource"; readonly target: SnapshotRangeTarget }
  | { readonly type: "reload" };

export function isHostToWebview(message: unknown): message is HostToWebview {
  if (!isRecord(message) || typeof message.type !== "string") {
    return false;
  }
  switch (message.type) {
    case "render":
      return isRecord(message.data) && isOptionalFocus(message.focus);
    case "focus":
      return isOptionalFocus(message.focus);
    case "error":
      return typeof message.message === "string";
    case "freshness":
      return typeof message.changed === "boolean";
    default:
      return false;
  }
}

function isOptionalFocus(
  value: unknown,
): value is ReviewPanelFocus | undefined {
  return (
    value === undefined ||
    (isRecord(value) &&
      value.kind === "attention" &&
      typeof value.id === "string")
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
