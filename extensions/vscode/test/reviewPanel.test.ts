import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceFolder } from "vscode";
import snapshotFixture from "../../../src/cli/inspect/web/test/fixtures/snapshot.json";
import type { ReviewSnapshotDoc } from "../src/cli";
import type { SourceOpener } from "../src/commands/openInSource";
import type { DiffDataSource, DiffRenderData } from "../src/diffDataSource";
import {
  type ReviewPanelLocation,
  ReviewPanelManager,
} from "../src/reviewPanel";
import type { ResolvedTargetResolution } from "../src/targetResolver";
import { workspaceFolder } from "./helpers/vscodeMock";

const vscodeMocks = vi.hoisted(() => {
  class Uri {
    constructor(readonly path: string) {}

    static joinPath(base: Uri, ...parts: string[]): Uri {
      return new Uri([base.path.replace(/\/$/, ""), ...parts].join("/"));
    }

    toString(): string {
      return `file://${this.path}`;
    }
  }

  class EventEmitter<T> {
    private readonly listeners: Array<(value: T) => void> = [];
    readonly event = (listener: (value: T) => void) => {
      this.listeners.push(listener);
      return { dispose: vi.fn() };
    };
    fire(value: T): void {
      for (const listener of this.listeners) listener(value);
    }
    dispose = vi.fn();
  }

  class MockPanel {
    readonly reveal = vi.fn((_column?: number, preserveFocus = false) => {
      if (!preserveFocus) this.setActive(true);
    });
    readonly dispose = vi.fn(() => {
      for (const listener of this.disposeListeners) listener();
    });
    readonly disposeListeners: Array<() => void> = [];
    readonly viewStateListeners: Array<
      (event: { webviewPanel: MockPanel }) => void
    > = [];
    readonly messageListeners: Array<(message: unknown) => void> = [];
    title = "";
    active = true;
    visible = true;
    readonly webview = {
      cspSource: "vscode-webview://panel",
      html: "",
      asWebviewUri: vi.fn((uri: Uri) => ({
        toString: () => `vscode-resource:${uri.path}`,
      })),
      postMessage: vi.fn(async () => true),
      onDidReceiveMessage: vi.fn((listener: (message: unknown) => void) => {
        this.messageListeners.push(listener);
        return { dispose: vi.fn() };
      }),
    };
    onDidDispose(listener: () => void) {
      this.disposeListeners.push(listener);
      return { dispose: vi.fn() };
    }
    onDidChangeViewState(
      listener: (event: { webviewPanel: MockPanel }) => void,
    ) {
      this.viewStateListeners.push(listener);
      return { dispose: vi.fn() };
    }
    emitMessage(message: unknown): void {
      for (const listener of this.messageListeners) listener(message);
    }
    setVisible(visible: boolean): void {
      this.visible = visible;
      for (const listener of this.viewStateListeners) {
        listener({ webviewPanel: this });
      }
    }
    setActive(active: boolean): void {
      if (active) {
        for (const panel of panels) {
          if (panel !== this && panel.active) panel.setActive(false);
        }
      }
      this.active = active;
      for (const listener of this.viewStateListeners) {
        listener({ webviewPanel: this });
      }
    }
  }

  const panels: MockPanel[] = [];
  const createWebviewPanel = vi.fn((...args: unknown[]) => {
    const preserveFocus = !!(args[2] as { preserveFocus?: boolean })
      ?.preserveFocus;
    if (!preserveFocus) {
      for (const panel of panels) panel.setActive(false);
    }
    const panel = new MockPanel();
    panel.active = !preserveFocus;
    panels.push(panel);
    return panel;
  });

  return { createWebviewPanel, EventEmitter, MockPanel, panels, Uri };
});

vi.mock("vscode", () => ({
  EventEmitter: vscodeMocks.EventEmitter,
  Uri: vscodeMocks.Uri,
  ViewColumn: { Active: -1 },
  window: { createWebviewPanel: vscodeMocks.createWebviewPanel },
}));

beforeEach(() => {
  vscodeMocks.createWebviewPanel.mockClear();
  vscodeMocks.panels.splice(0);
});

describe("ReviewPanelManager", () => {
  it("routes only a validated source target with the loaded review context", async () => {
    const sourceOpener = { open: vi.fn(async () => undefined) };
    const current = location("a", "rev:sha256:one");
    const manager = createManager(
      { load: vi.fn(async () => data(current.revisionId)) },
      sourceOpener,
    );

    await manager.open(current);
    const panel = vscodeMocks.panels[0];
    panel.emitMessage({
      type: "openSource",
      target: {
        filePath: "src/lib.rs",
        side: "new",
        startLine: 2,
        endLine: 2,
      },
    });
    panel.emitMessage({
      type: "openSource",
      target: {
        filePath: "/private/repo/src/lib.rs",
        side: "new",
        startLine: 2,
        endLine: 2,
      },
    });
    await settled();

    expect(sourceOpener.open).toHaveBeenCalledTimes(1);
    expect(sourceOpener.open).toHaveBeenCalledWith({
      repoRoot: "/private/a",
      targetKey: "a",
      revisionId: current.revisionId,
      snapshot: expect.objectContaining({
        schema: "pointbreak.review-snapshot",
      }),
      target: {
        filePath: "src/lib.rs",
        side: "new",
        startLine: 2,
        endLine: 2,
      },
    });
  });

  it("keys one reusable panel by target and revision while focus stays presentation-only", async () => {
    const load = vi.fn(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    const first = location("a", "rev:sha256:one");

    await manager.open(first, { preserveFocus: true });
    const panel = vscodeMocks.panels[0];
    panel.emitMessage({ type: "ready" });
    await settled();

    expect(vscodeMocks.createWebviewPanel).toHaveBeenCalledTimes(1);
    expect(vscodeMocks.createWebviewPanel.mock.calls[0]?.[2]).toEqual({
      viewColumn: -1,
      preserveFocus: true,
    });
    expect(load).toHaveBeenCalledTimes(1);
    expect(panel.webview.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "render" }),
    );

    await manager.open(first, { preserveFocus: true });
    expect(load).toHaveBeenCalledTimes(1);
    expect(panel.reveal).toHaveBeenLastCalledWith(-1, true);

    await manager.open({
      ...first,
      focus: { kind: "attention", id: "open_input_request:request:one" },
    });
    await settled();
    expect(load).toHaveBeenCalledTimes(2);
    expect(panel.webview.postMessage).toHaveBeenLastCalledWith({
      type: "render",
      data: expect.objectContaining({ revisionId: first.revisionId }),
      focus: { kind: "attention", id: "open_input_request:request:one" },
    });

    const second = location("a", "rev:sha256:two");
    const secondOpen = manager.open(second);
    expect(vscodeMocks.createWebviewPanel).toHaveBeenCalledTimes(2);
    const secondPanel = vscodeMocks.panels[1];
    secondPanel.emitMessage({ type: "ready" });
    await secondOpen;
    await settled();
    expect(load).toHaveBeenCalledTimes(3);
    expect(secondPanel.webview.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        type: "render",
        data: expect.objectContaining({ revisionId: second.revisionId }),
      }),
    );

    await manager.open(first, { preserveFocus: true });
    expect(vscodeMocks.createWebviewPanel).toHaveBeenCalledTimes(2);
    expect(load).toHaveBeenCalledTimes(3);
    expect(panel.reveal).toHaveBeenLastCalledWith(-1, true);

    const sameRevisionOnAnotherTarget = location("b", first.revisionId);
    const thirdOpen = manager.open(sameRevisionOnAnotherTarget);
    expect(vscodeMocks.createWebviewPanel).toHaveBeenCalledTimes(3);
    const thirdPanel = vscodeMocks.panels[2];
    expect(panel.title).toBe("Pointbreak Review: a · one");
    expect(vscodeMocks.createWebviewPanel.mock.calls[2]?.[1]).toBe(
      "Pointbreak Review: b · one",
    );
    thirdPanel.emitMessage({ type: "ready" });
    await thirdOpen;
    expect(load).toHaveBeenCalledTimes(4);
    expect(thirdPanel.title).toBe("Pointbreak Review: b · one");
  });

  it("keeps overlapping document loads independent and errors path-free", async () => {
    const firstLoad = deferred<DiffRenderData>();
    const secondLoad = deferred<DiffRenderData>();
    const load = vi
      .fn<DiffDataSource["load"]>()
      .mockReturnValueOnce(firstLoad.promise)
      .mockReturnValueOnce(secondLoad.promise);
    const manager = createManager({ load });

    const firstOpen = manager.open(location("a", "rev:sha256:first"));
    const firstPanel = vscodeMocks.panels[0];
    const secondLocation = {
      ...location("b", "rev:sha256:new"),
      focus: { kind: "attention", id: "stale_assessment:assess:new" } as const,
    };
    const secondOpen = manager.open(secondLocation);
    expect(vscodeMocks.panels).toHaveLength(2);
    const secondPanel = vscodeMocks.panels[1];
    firstPanel.emitMessage({ type: "ready" });
    secondPanel.emitMessage({ type: "ready" });
    firstLoad.reject(new Error("/private/repo token=secret 127.0.0.1:7878"));
    secondLoad.resolve(data(secondLocation.revisionId));
    await Promise.all([firstOpen, secondOpen]);
    await settled();

    expect(firstPanel.webview.postMessage).toHaveBeenCalledWith({
      type: "error",
      message: "Pointbreak could not load this annotated diff.",
    });
    expect(secondPanel.webview.postMessage).toHaveBeenCalledWith({
      type: "render",
      data: expect.objectContaining({ revisionId: secondLocation.revisionId }),
      focus: secondLocation.focus,
    });
    expect(JSON.stringify(vscodeMocks.panels)).not.toMatch(
      /private|secret|127\.0\.0\.1|7878/,
    );
  });

  it("pairs a pending document load with its latest attention focus", async () => {
    const pending = deferred<DiffRenderData>();
    const load = vi
      .fn<DiffDataSource["load"]>()
      .mockReturnValue(pending.promise);
    const manager = createManager({ load });
    const initial = location("a", "rev:sha256:pending");
    const firstOpen = manager.open(initial);
    const panel = vscodeMocks.panels[0];

    await manager.open({
      ...initial,
      focus: { kind: "attention", id: "open_input_request:request:pending" },
    });
    panel.emitMessage({ type: "ready" });
    pending.resolve(data(initial.revisionId));
    await firstOpen;
    await settled();

    expect(vscodeMocks.createWebviewPanel).toHaveBeenCalledTimes(1);
    expect(load).toHaveBeenCalledTimes(1);
    expect(panel.webview.postMessage).toHaveBeenCalledWith({
      type: "render",
      data: expect.objectContaining({ revisionId: initial.revisionId }),
      focus: {
        kind: "attention",
        id: "open_input_request:request:pending",
      },
    });
  });

  it("lets only the latest reload update one document session", async () => {
    const stale = deferred<DiffRenderData>();
    const current = deferred<DiffRenderData>();
    const revisionId = "rev:sha256:reload";
    const load = vi
      .fn<DiffDataSource["load"]>()
      .mockResolvedValueOnce(data(revisionId))
      .mockReturnValueOnce(stale.promise)
      .mockReturnValueOnce(current.promise);
    const manager = createManager({ load });

    await manager.open(location("a", revisionId));
    const panel = vscodeMocks.panels[0];
    panel.emitMessage({ type: "ready" });
    await settled();
    panel.webview.postMessage.mockClear();

    panel.emitMessage({ type: "reload" });
    panel.emitMessage({ type: "reload" });
    panel.emitMessage({ type: "ready" });
    current.resolve({ ...data(revisionId), snapshotId: "obj:current" });
    await settled();
    stale.reject(new Error("stale /private/path token=secret"));
    await settled();

    expect(panel.webview.postMessage).toHaveBeenCalledTimes(1);
    expect(panel.webview.postMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "render",
        data: expect.objectContaining({ snapshotId: "obj:current" }),
      }),
    );
    expect(JSON.stringify(panel.webview.postMessage.mock.calls)).not.toMatch(
      /private|secret/,
    );
  });

  it("closes and recreates only one document while reloadActive follows the latest open", async () => {
    const pending = deferred<DiffRenderData>();
    const load = vi
      .fn<DiffDataSource["load"]>()
      .mockReturnValueOnce(pending.promise)
      .mockImplementation(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    const first = location("a", "rev:sha256:first");
    const second = location("a", "rev:sha256:second");
    const firstOpen = manager.open(first);
    const firstPanel = vscodeMocks.panels[0];
    const secondOpen = manager.open(second);
    const secondPanel = vscodeMocks.panels[1];
    secondPanel.emitMessage({ type: "ready" });
    await secondOpen;

    firstPanel.dispose();
    pending.resolve(data("rev:sha256:first"));
    await firstOpen;
    expect(firstPanel.webview.postMessage).not.toHaveBeenCalled();

    await manager.reloadActive();
    expect(load).toHaveBeenLastCalledWith({
      resolution: second.resolution,
      revisionId: second.revisionId,
    });

    const reopen = manager.open(first);
    const reopenedPanel = vscodeMocks.panels[2];
    reopenedPanel.emitMessage({ type: "ready" });
    await reopen;
    await settled();
    expect(vscodeMocks.createWebviewPanel).toHaveBeenCalledTimes(3);
    expect(reopenedPanel.webview.postMessage).toHaveBeenCalledWith(
      expect.objectContaining({ type: "render" }),
    );

    secondPanel.webview.postMessage.mockClear();
    secondPanel.emitMessage({ type: "reload" });
    secondPanel.emitMessage({ type: "ready" });
    await settled();
    expect(load).toHaveBeenLastCalledWith({
      resolution: second.resolution,
      revisionId: second.revisionId,
    });
    expect(secondPanel.webview.postMessage).toHaveBeenCalledWith(
      expect.objectContaining({ type: "render" }),
    );

    await manager.reloadActive();
    expect(load).toHaveBeenLastCalledWith({
      resolution: first.resolution,
      revisionId: first.revisionId,
    });
  });

  it("uses local-only CSP and reports aggregate keyed-panel visibility", async () => {
    const manager = createManager({ load: vi.fn(async () => data("rev:one")) });
    const visible: boolean[] = [];
    manager.onDidChangeVisibility((value) => visible.push(value));
    await manager.open(location("a", "rev:one"));
    await manager.open(location("a", "rev:two"));
    const [panel, secondPanel] = vscodeMocks.panels;
    const options = vscodeMocks.createWebviewPanel.mock.calls[0]?.[3];

    expect(options).toMatchObject({
      enableScripts: true,
      localResourceRoots: [expect.objectContaining({ path: "/extension/out" })],
    });
    expect(panel.webview.html).toContain("default-src 'none'");
    expect(panel.webview.html).toContain(
      "vscode-resource:/extension/out/review.js",
    );
    expect(panel.webview.html).toContain(
      "vscode-resource:/extension/out/review.css",
    );
    expect(panel.webview.html).not.toMatch(
      /https?:|127\.0\.0\.1|token|\/private\/repo/,
    );

    expect(visible).toEqual([true]);
    panel.setVisible(false);
    expect(visible).toEqual([true]);
    secondPanel.setVisible(false);
    panel.setVisible(true);
    expect(visible).toEqual([true, false, true]);
    expect(manager.isVisible()).toBe(true);
  });

  it("reloads the active diff only for its active connection target", async () => {
    const load = vi.fn(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    await manager.open(location("a", "rev:one"));

    await manager.reloadActive("b");
    expect(load).toHaveBeenCalledOnce();

    await manager.reloadActive("a");
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("reloads the panel the user activated instead of the last opened panel", async () => {
    const load = vi.fn(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    const first = location("a", "rev:one");
    const second = location("a", "rev:two");
    await manager.open(first);
    await manager.open(second);
    vscodeMocks.panels[0].setActive(true);
    load.mockClear();

    await manager.reloadActive("a");

    expect(load).toHaveBeenCalledOnce();
    expect(load).toHaveBeenCalledWith({
      resolution: first.resolution,
      revisionId: first.revisionId,
    });
  });

  it("reloads a visible panel that was opened with preserved focus", async () => {
    const load = vi.fn(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    const current = location("a", "rev:one");
    await manager.open(current, { preserveFocus: true });
    const panel = vscodeMocks.panels[0];
    load.mockClear();

    expect(panel.visible).toBe(true);
    expect(panel.active).toBe(false);
    await manager.reloadActive("a");

    expect(load).toHaveBeenCalledOnce();
    expect(load).toHaveBeenCalledWith({
      resolution: current.resolution,
      revisionId: current.revisionId,
    });
  });

  it("refreshes a panel immediately when the user activates it", async () => {
    const load = vi.fn(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    const first = location("a", "rev:one");
    await manager.open(first);
    await manager.open(location("b", "rev:two"));
    load.mockClear();

    vscodeMocks.panels[0].setActive(true);
    await settled();

    expect(load).toHaveBeenCalledOnce();
    expect(load).toHaveBeenCalledWith({
      resolution: first.resolution,
      revisionId: first.revisionId,
    });
  });

  it.each([
    {
      name: "hides",
      leaveCurrent: (panel: (typeof vscodeMocks.panels)[number]) =>
        panel.setVisible(false),
    },
    {
      name: "closes",
      leaveCurrent: (panel: (typeof vscodeMocks.panels)[number]) =>
        panel.dispose(),
    },
  ])("loads the visible fallback target when the current panel $name", async ({
    leaveCurrent,
  }) => {
    const load = vi.fn(async ({ revisionId }) => data(revisionId));
    const manager = createManager({ load });
    const fallback = location("b", "rev:fallback");
    await manager.open(fallback);
    await manager.open(location("a", "rev:current"));
    load.mockClear();

    leaveCurrent(vscodeMocks.panels[1]);
    await settled();

    expect(load).toHaveBeenCalledOnce();
    expect(load).toHaveBeenCalledWith({
      resolution: fallback.resolution,
      revisionId: fallback.revisionId,
    });
  });

  it("does not publish a freshness reload after its generation is aborted", async () => {
    const pending = deferred<DiffRenderData>();
    const load = vi
      .fn<DiffDataSource["load"]>()
      .mockResolvedValueOnce(data("rev:one"))
      .mockReturnValueOnce(pending.promise);
    const manager = createManager({ load });
    await manager.open(location("a", "rev:one"));
    const panel = vscodeMocks.panels[0];
    panel.emitMessage({ type: "ready" });
    await settled();
    const renderedHtml = panel.webview.html;
    panel.webview.postMessage.mockClear();
    const controller = new AbortController();
    const reload = manager.reloadActive("a", controller.signal);

    expect(panel.webview.html).toBe(renderedHtml);
    controller.abort();
    pending.resolve({ ...data("rev:one"), snapshotId: "obj:stale" });
    await reload;
    panel.emitMessage({ type: "ready" });
    await settled();

    expect(panel.webview.html).toBe(renderedHtml);
    expect(panel.webview.postMessage).not.toHaveBeenCalled();
  });

  it("keys persisted controller state by the complete host data location", async () => {
    const manager = createManager({
      load: vi.fn(async ({ revisionId }) => data(revisionId)),
    });
    await manager.open(location("a", "rev:same"));
    const firstPanel = vscodeMocks.panels[0];
    const firstKey = firstPanel.webview.html.match(
      /data-location-key="([^"]+)"/,
    )?.[1];

    await manager.open(location("b", "rev:same"));
    const secondPanel = vscodeMocks.panels[1];
    const secondKey = secondPanel.webview.html.match(
      /data-location-key="([^"]+)"/,
    )?.[1];

    expect(firstKey).toMatch(/^[a-f0-9]{64}$/);
    expect(secondKey).toMatch(/^[a-f0-9]{64}$/);
    expect(secondKey).not.toBe(firstKey);
    expect(firstPanel.webview.html).not.toContain("store:a/context:a");
    expect(secondPanel.webview.html).not.toContain("store:b/context:b");
  });

  it("disposes every keyed document session with the manager", async () => {
    const manager = createManager({
      load: vi.fn(async ({ revisionId }) => data(revisionId)),
    });
    await manager.open(location("a", "rev:one"));
    await manager.open(location("a", "rev:two"));

    manager.dispose();

    expect(vscodeMocks.panels[0].dispose).toHaveBeenCalledOnce();
    expect(vscodeMocks.panels[1].dispose).toHaveBeenCalledOnce();
    await expect(manager.open(location("a", "rev:three"))).rejects.toThrow(
      "Pointbreak Review is no longer available.",
    );
  });
});

function createManager(
  source: DiffDataSource,
  sourceOpener?: SourceOpener,
): ReviewPanelManager {
  return new ReviewPanelManager(
    new vscodeMocks.Uri("/extension") as never,
    source,
    sourceOpener,
  );
}

function location(target: string, revisionId: string): ReviewPanelLocation {
  return { resolution: resolution(target), revisionId };
}

function resolution(key: string): ResolvedTargetResolution {
  return {
    kind: "resolved",
    folder: workspaceFolder(`/private/${key}`, key) as WorkspaceFolder,
    target: {
      key,
      label: key,
      storeIdentity: `store:${key}`,
      contextIdentity: `context:${key}`,
    },
    emptyInventory: false,
  };
}

function data(revisionId: string): DiffRenderData {
  return {
    revisionId,
    snapshotId: `obj:${revisionId}`,
    artifact: snapshotFixture as ReviewSnapshotDoc,
    annotations: [],
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((next, fail) => {
    resolve = next;
    reject = fail;
  });
  return { promise, reject, resolve };
}

async function settled(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}
