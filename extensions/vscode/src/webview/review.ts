/// <reference lib="dom" />

import { escapeHtml } from "./diff/escape";
import type { Annotation, DiffArtifact } from "./diff/render";
import { renderDiff } from "./diff/render";
import "./review.css";
import {
  type HostToWebview,
  isHostToWebview,
  type WebviewToHost,
} from "../webviewProtocol";

interface VsCodeApi {
  postMessage(message: WebviewToHost): void;
}

declare function acquireVsCodeApi(): VsCodeApi;

const vscode = acquireVsCodeApi();

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", start, { once: true });
} else {
  start();
}

function start(): void {
  const root = document.querySelector<HTMLElement>("#review-root");
  if (!root) {
    return;
  }
  window.addEventListener("message", ({ data }: MessageEvent<unknown>) => {
    if (isHostToWebview(data)) {
      renderMessage(root, data);
    }
  });
  vscode.postMessage({ type: "ready" });
}

function renderMessage(container: HTMLElement, message: HostToWebview): void {
  switch (message.type) {
    case "render": {
      const result = renderDiff(
        message.data.snapshotId,
        message.data.artifact as DiffArtifact,
        message.data.annotations as Annotation[],
      );
      container.innerHTML = result.html;
      return;
    }
    case "error":
      container.innerHTML = `<p class="empty" role="alert">${escapeHtml(message.message)}</p>`;
      return;
    case "freshness":
      document.body.dataset.freshness = message.changed ? "changed" : "current";
      return;
    case "focus":
      return;
  }
}
