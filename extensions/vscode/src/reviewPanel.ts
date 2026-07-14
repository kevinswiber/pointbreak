import { createHash, randomBytes } from "node:crypto";
import {
  type Disposable,
  type Event,
  EventEmitter,
  Uri,
  ViewColumn,
  type Webview,
  type WebviewPanel,
  window,
} from "vscode";
import type { SourceOpener } from "./commands/openInSource";
import type { DiffDataSource } from "./diffDataSource";
import type { ResolvedTargetResolution } from "./targetResolver";
import {
  type HostToWebview,
  isHostToWebview,
  isWebviewToHost,
  type ReviewPanelFocus,
} from "./webviewProtocol";

const VIEW_TYPE = "pointbreak.annotatedDiff";
const PANEL_TITLE = "Pointbreak Review";
const LOAD_ERROR = "Pointbreak could not load this annotated diff.";

export interface ReviewPanelLocation {
  readonly resolution: ResolvedTargetResolution;
  readonly revisionId: string;
  readonly focus?: ReviewPanelFocus;
}

export interface ReviewPanelOpenOptions {
  readonly preserveFocus?: boolean;
}

interface PanelSession {
  readonly key: string;
  readonly panel: WebviewPanel;
  location: ReviewPanelLocation;
  generation: number;
  ready: boolean;
  pendingState: HostToWebview | undefined;
  active: boolean;
  visible: boolean;
  snapshot: import("./cli").ReviewSnapshotDoc | undefined;
}

/** Owns one annotated-diff presentation surface per review document. */
export class ReviewPanelManager implements Disposable {
  private readonly visibilityEmitter = new EventEmitter<boolean>();
  readonly onDidChangeVisibility: Event<boolean> = this.visibilityEmitter.event;

  private readonly sessions = new Map<string, PanelSession>();
  private currentKey: string | undefined;
  private reportedVisibility = false;
  private disposed = false;

  constructor(
    private readonly extensionUri: Uri,
    private readonly dataSource: DiffDataSource,
    private readonly sourceOpener?: SourceOpener,
  ) {}

  async open(
    location: ReviewPanelLocation,
    options: ReviewPanelOpenOptions = {},
  ): Promise<void> {
    if (this.disposed) {
      throw new Error("Pointbreak Review is no longer available.");
    }

    const key = reviewDocumentKey(location);
    const existing = this.sessions.get(key);
    this.currentKey = key;

    if (existing) {
      existing.panel.reveal(ViewColumn.Active, !!options.preserveFocus);
      const sameFocus = equalFocus(existing.location.focus, location.focus);
      existing.location = location;
      existing.panel.title = reviewDocumentTitle(location);
      if (!sameFocus) {
        this.sendFocus(existing, location.focus);
      }
      return;
    }

    const session = this.createSession(key, location, !!options.preserveFocus);
    await this.load(session);
  }

  async reloadActive(targetKey?: string, signal?: AbortSignal): Promise<void> {
    const session = this.currentKey
      ? this.sessions.get(this.currentKey)
      : undefined;
    if (!session?.visible) {
      return;
    }
    if (
      targetKey !== undefined &&
      session.location.resolution.target.key !== targetKey
    ) {
      return;
    }
    await this.load(session, signal, true);
  }

  isVisible(): boolean {
    return this.reportedVisibility;
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    const sessions = [...this.sessions.values()];
    this.sessions.clear();
    this.currentKey = undefined;
    for (const session of sessions) {
      session.generation += 1;
      session.panel.dispose();
    }
    this.visibilityEmitter.dispose();
  }

  private createSession(
    key: string,
    location: ReviewPanelLocation,
    preserveFocus: boolean,
  ): PanelSession {
    const outputRoot = Uri.joinPath(this.extensionUri, "out");
    const panel = window.createWebviewPanel(
      VIEW_TYPE,
      reviewDocumentTitle(location),
      { viewColumn: ViewColumn.Active, preserveFocus },
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [outputRoot],
      },
    );
    const session: PanelSession = {
      key,
      panel,
      location,
      generation: 0,
      ready: false,
      pendingState: undefined,
      active: panel.active,
      visible: panel.visible,
      snapshot: undefined,
    };
    this.sessions.set(key, session);
    this.emitVisibilityChange();
    panel.onDidDispose(() => this.clearSession(session));
    panel.onDidChangeViewState(({ webviewPanel }) => {
      if (this.sessions.get(key) === session) {
        const becameActive = webviewPanel.active && !session.active;
        session.active = webviewPanel.active;
        session.visible = webviewPanel.visible;
        if (webviewPanel.active && webviewPanel.visible) {
          this.currentKey = key;
        } else if (this.currentKey === key && !webviewPanel.visible) {
          this.selectVisibleFallback();
        }
        this.emitVisibilityChange();
        if (becameActive && webviewPanel.visible) {
          void this.load(session, undefined, true);
        }
      }
    });
    panel.webview.onDidReceiveMessage((message: unknown) => {
      void this.receive(session, message);
    });
    return session;
  }

  private async load(
    session: PanelSession,
    signal?: AbortSignal,
    preserveRender = false,
  ): Promise<void> {
    if (signal?.aborted) return;
    const generation = ++session.generation;
    const location = session.location;
    if (!preserveRender) {
      session.ready = false;
      session.pendingState = undefined;
      session.snapshot = undefined;
      session.panel.webview.html = webviewHtml(
        this.extensionUri,
        session.panel.webview,
        location,
      );
    }

    try {
      const data = await this.dataSource.load({
        resolution: location.resolution,
        revisionId: location.revisionId,
      });
      if (signal?.aborted || !this.isCurrent(session, generation)) {
        return;
      }
      const message: HostToWebview = {
        type: "render",
        data,
        focus: session.location.focus,
      };
      if (
        data.revisionId !== location.revisionId ||
        !isHostToWebview(message)
      ) {
        this.queueState(session, { type: "error", message: LOAD_ERROR });
        return;
      }
      session.panel.title = reviewDocumentTitle(location);
      session.snapshot = data.artifact;
      this.queueState(session, message);
    } catch {
      if (signal?.aborted || !this.isCurrent(session, generation)) {
        return;
      }
      this.queueState(session, { type: "error", message: LOAD_ERROR });
    }
  }

  private async receive(
    session: PanelSession,
    message: unknown,
  ): Promise<void> {
    if (
      this.sessions.get(session.key) !== session ||
      !isWebviewToHost(message)
    ) {
      return;
    }
    if (message.type === "ready") {
      session.ready = true;
      this.flushState(session);
      return;
    }
    if (message.type === "reload") {
      await this.load(session);
      return;
    }
    if (
      message.type === "openSource" &&
      session.snapshot &&
      this.sourceOpener
    ) {
      await this.sourceOpener.open({
        repoRoot: session.location.resolution.folder.uri.fsPath,
        targetKey: session.location.resolution.target.key,
        revisionId: session.location.revisionId,
        snapshot: session.snapshot,
        target: message.target,
      });
    }
  }

  private queueState(session: PanelSession, message: HostToWebview): void {
    session.pendingState = message;
    this.flushState(session);
  }

  private flushState(session: PanelSession): void {
    if (!session.ready || !session.pendingState) {
      return;
    }
    const message = session.pendingState;
    session.pendingState = undefined;
    void session.panel.webview.postMessage(message);
  }

  private sendFocus(
    session: PanelSession,
    focus: ReviewPanelFocus | undefined,
  ): void {
    if (session.pendingState?.type === "render") {
      session.pendingState = { ...session.pendingState, focus };
      this.flushState(session);
      return;
    }
    if (session.ready) {
      void session.panel.webview.postMessage({ type: "focus", focus });
    }
  }

  private isCurrent(session: PanelSession, generation: number): boolean {
    return (
      this.sessions.get(session.key) === session &&
      session.generation === generation
    );
  }

  private clearSession(session: PanelSession): void {
    if (this.sessions.get(session.key) !== session) {
      return;
    }
    session.generation += 1;
    this.sessions.delete(session.key);
    if (this.currentKey === session.key) {
      this.selectVisibleFallback();
    }
    if (!this.disposed) {
      this.emitVisibilityChange();
    }
  }

  private selectVisibleFallback(): void {
    const fallback = this.preferredVisibleSession();
    this.currentKey = fallback?.key;
    if (fallback) {
      fallback.active = fallback.panel.active;
      void this.load(fallback, undefined, true);
    }
  }

  private preferredVisibleSession(): PanelSession | undefined {
    const visible = [...this.sessions.values()].filter(
      (session) => session.visible,
    );
    return visible.find((session) => session.panel.active) ?? visible.at(-1);
  }

  private hasVisibleSession(): boolean {
    return [...this.sessions.values()].some((session) => session.visible);
  }

  private emitVisibilityChange(): void {
    const anyVisible = this.hasVisibleSession();
    if (anyVisible === this.reportedVisibility) {
      return;
    }
    this.reportedVisibility = anyVisible;
    this.visibilityEmitter.fire(anyVisible);
  }
}

function webviewHtml(
  extensionUri: Uri,
  webview: Webview,
  location: ReviewPanelLocation,
): string {
  const nonce = randomBytes(18).toString("base64");
  const locationKey = reviewDocumentKey(location);
  const script = webview.asWebviewUri(
    Uri.joinPath(extensionUri, "out", "review.js"),
  );
  const style = webview.asWebviewUri(
    Uri.joinPath(extensionUri, "out", "review.css"),
  );
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${webview.cspSource} data:; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <link rel="stylesheet" href="${style}">
  <title>${PANEL_TITLE}</title>
</head>
<body>
  <main id="review-root" data-location-key="${locationKey}"><p class="empty review-loading" role="status">Loading annotated diff…</p></main>
  <script nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
}

function reviewDocumentKey(location: ReviewPanelLocation): string {
  return createHash("sha256")
    .update(location.resolution.target.key)
    .update("\0")
    .update(location.revisionId)
    .digest("hex");
}

function equalFocus(
  left: ReviewPanelFocus | undefined,
  right: ReviewPanelFocus | undefined,
): boolean {
  return left?.kind === right?.kind && left?.id === right?.id;
}

function shortRevisionId(revisionId: string): string {
  return revisionId.split(":").at(-1)?.slice(0, 12) ?? revisionId;
}

function reviewDocumentTitle(location: ReviewPanelLocation): string {
  return `${PANEL_TITLE}: ${location.resolution.target.label} · ${shortRevisionId(location.revisionId)}`;
}
