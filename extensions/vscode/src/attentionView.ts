import {
  type Disposable,
  type Event,
  EventEmitter,
  ThemeIcon,
  type TreeDataProvider,
  TreeItem,
  TreeItemCollapsibleState,
  type TreeView,
  type WorkspaceFolder,
} from "vscode";
import type { AttentionListDoc, PointbreakCli, RevisionListDoc } from "./cli";
import type { TargetResolution } from "./targetResolver";

export const POLL_INTERVAL_MS = 15_000;

interface NodeBase {
  kind: string;
  label: string;
  targetKey?: string;
  folder?: WorkspaceFolder;
}

export interface TargetNode extends NodeBase {
  kind: "target";
  targetKey: string;
  folder: WorkspaceFolder;
  children: TreeNode[];
}

export interface AttentionSectionNode extends NodeBase {
  kind: "attention-section";
  targetKey: string;
  folder: WorkspaceFolder;
  children: AttentionItemNode[];
}

export interface RevisionsSectionNode extends NodeBase {
  kind: "revisions-section";
  targetKey: string;
  folder: WorkspaceFolder;
  children: RevisionItemNode[];
}

export interface AttentionItemNode extends NodeBase {
  kind: "attention-item";
  targetKey: string;
  folder: WorkspaceFolder;
  description: string;
  revisionId?: string;
  command?: "pointbreak.openInReview";
}

export interface RevisionItemNode extends NodeBase {
  kind: "revision-item";
  targetKey: string;
  folder: WorkspaceFolder;
  description: string;
  revisionId: string;
  command: "pointbreak.openInReview";
}

export interface OnboardingNode extends NodeBase {
  kind: "onboarding";
  targetKey: string;
  folder: WorkspaceFolder;
  command: "pointbreak.capture";
}

export interface RepairNode extends NodeBase {
  kind: "repair";
  message: string;
  folder: WorkspaceFolder;
}

export type TreeNode =
  | TargetNode
  | AttentionSectionNode
  | RevisionsSectionNode
  | AttentionItemNode
  | RevisionItemNode
  | OnboardingNode
  | RepairNode;

const activeProviders = new Set<AttentionTreeProvider>();

export function deriveTree(
  resolutions: TargetResolution[],
  attentionByTarget: ReadonlyMap<string, AttentionListDoc>,
  revisionsByTarget: ReadonlyMap<string, RevisionListDoc>,
): TreeNode[] {
  const errors = resolutions
    .filter((resolution) => resolution.kind === "error")
    .map(repairNode);
  const resolved = distinctResolvedTargets(resolutions);
  const sections = resolved.map((resolution) => ({
    resolution,
    children: deriveTargetChildren(
      resolution,
      attentionByTarget.get(resolution.target.key),
      revisionsByTarget.get(resolution.target.key),
    ),
  }));

  if (sections.length === 1) {
    return [...sections[0].children, ...errors];
  }
  return [
    ...sections.map(
      ({ resolution, children }): TargetNode => ({
        kind: "target",
        label: resolution.folder.name,
        targetKey: resolution.target.key,
        folder: resolution.folder,
        children,
      }),
    ),
    ...errors,
  ];
}

export function attentionCount(
  attentionByTarget: ReadonlyMap<string, AttentionListDoc>,
): number {
  let count = 0;
  for (const doc of attentionByTarget.values()) {
    count += doc.items.length;
  }
  return count;
}

export async function refreshAfterWrite(): Promise<void> {
  await Promise.all([...activeProviders].map((provider) => provider.refresh()));
}

export class AttentionTreeProvider implements TreeDataProvider<TreeNode> {
  private readonly emitter = new EventEmitter<TreeNode | undefined | null>();
  readonly onDidChangeTreeData: Event<TreeNode | undefined | null> =
    this.emitter.event;
  private readonly attentionByTarget = new Map<string, AttentionListDoc>();
  private readonly revisionsByTarget = new Map<string, RevisionListDoc>();
  private readonly loadErrors = new Map<string, string>();
  private interval: ReturnType<typeof setInterval> | undefined;
  private treeView: TreeView<TreeNode> | undefined;
  private visible = false;
  private visibilityGeneration = 0;

  constructor(
    private readonly cli: PointbreakCli,
    private readonly resolutions: TargetResolution[],
  ) {
    activeProviders.add(this);
  }

  attachTreeView(treeView: TreeView<TreeNode>): Disposable {
    this.treeView = treeView;
    this.updateBadge();
    const visibility = treeView.onDidChangeVisibility((event) => {
      void this.setVisible(event.visible);
    });
    void this.setVisible(treeView.visible);
    return visibility;
  }

  async setVisible(visible: boolean): Promise<void> {
    this.visible = visible;
    const generation = ++this.visibilityGeneration;
    this.stopPolling();
    if (!visible) {
      return;
    }

    await this.refresh();
    if (this.visible && generation === this.visibilityGeneration) {
      this.interval = setInterval(() => {
        void this.refresh();
      }, POLL_INTERVAL_MS);
    }
  }

  async refresh(): Promise<void> {
    await Promise.all(
      distinctResolvedTargets(this.resolutions).map(async (resolution) => {
        const key = resolution.target.key;
        if (resolution.emptyInventory) {
          this.attentionByTarget.delete(key);
          this.revisionsByTarget.delete(key);
          this.loadErrors.delete(key);
          return;
        }
        try {
          const [attention, revisions] = await Promise.all([
            this.cli.attentionList(resolution.folder.uri.fsPath),
            this.cli.revisionList(resolution.folder.uri.fsPath),
          ]);
          this.attentionByTarget.set(key, attention);
          this.revisionsByTarget.set(key, revisions);
          this.loadErrors.delete(key);
        } catch (error) {
          this.attentionByTarget.delete(key);
          this.revisionsByTarget.delete(key);
          this.loadErrors.set(key, errorMessage(error));
        }
      }),
    );
    this.updateBadge();
    this.emitter.fire(undefined);
  }

  getTreeItem(node: TreeNode): TreeItem {
    const collapsible =
      "children" in node
        ? TreeItemCollapsibleState.Expanded
        : TreeItemCollapsibleState.None;
    const item = new TreeItem(node.label, collapsible);
    if ("description" in node) {
      item.description = node.description;
    }
    if (node.kind === "repair") {
      item.tooltip = node.message;
      item.iconPath = new ThemeIcon("warning");
    } else if (node.kind === "onboarding") {
      item.iconPath = new ThemeIcon("add");
    }
    if ("command" in node && node.command) {
      item.command = {
        command: node.command,
        title: node.label,
        arguments: [node],
      };
    }
    return item;
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (node && "children" in node) {
      return node.children;
    }
    if (node) {
      return [];
    }
    return deriveTree(
      this.resolutionsWithLoadErrors(),
      this.attentionByTarget,
      this.revisionsByTarget,
    );
  }

  dispose(): void {
    this.visible = false;
    this.visibilityGeneration += 1;
    this.stopPolling();
    activeProviders.delete(this);
    this.emitter.dispose();
  }

  private resolutionsWithLoadErrors(): TargetResolution[] {
    return this.resolutions.map((resolution) => {
      if (resolution.kind === "error") {
        return resolution;
      }
      const message = this.loadErrors.get(resolution.target.key);
      return message
        ? { kind: "error", folder: resolution.folder, message }
        : resolution;
    });
  }

  private updateBadge(): void {
    if (!this.treeView) {
      return;
    }
    const value = attentionCount(this.attentionByTarget);
    this.treeView.badge = {
      value,
      tooltip: `${value} Pointbreak attention item${value === 1 ? "" : "s"}`,
    };
  }

  private stopPolling(): void {
    if (this.interval) {
      clearInterval(this.interval);
      this.interval = undefined;
    }
  }
}

type ResolvedTarget = TargetResolution & { kind: "resolved" };

function distinctResolvedTargets(
  resolutions: TargetResolution[],
): ResolvedTarget[] {
  const distinct = new Map<string, ResolvedTarget>();
  for (const resolution of resolutions) {
    if (
      resolution.kind === "resolved" &&
      !distinct.has(resolution.target.key)
    ) {
      distinct.set(resolution.target.key, resolution);
    }
  }
  return [...distinct.values()];
}

function deriveTargetChildren(
  resolution: ResolvedTarget,
  attention: AttentionListDoc | undefined,
  revisions: RevisionListDoc | undefined,
): TreeNode[] {
  if (resolution.emptyInventory) {
    return [
      {
        kind: "onboarding",
        label: "Capture your current work",
        targetKey: resolution.target.key,
        folder: resolution.folder,
        command: "pointbreak.capture",
      },
    ];
  }

  const attentionChildren: AttentionItemNode[] = (attention?.items ?? []).map(
    (item) => ({
      kind: "attention-item",
      label: item.title ?? item.kind,
      targetKey: resolution.target.key,
      folder: resolution.folder,
      description: item.tier,
      revisionId: item.revisionId,
      command: item.revisionId ? "pointbreak.openInReview" : undefined,
    }),
  );
  const revisionChildren: RevisionItemNode[] = (revisions?.entries ?? [])
    .slice(0, 5)
    .map((entry) => ({
      kind: "revision-item",
      label: shortRevisionId(entry.revisionId),
      targetKey: resolution.target.key,
      folder: resolution.folder,
      description: entry.mergeStatus,
      revisionId: entry.revisionId,
      command: "pointbreak.openInReview",
    }));
  return [
    {
      kind: "attention-section",
      label: "Attention",
      targetKey: resolution.target.key,
      folder: resolution.folder,
      children: attentionChildren,
    },
    {
      kind: "revisions-section",
      label: "Recent revisions",
      targetKey: resolution.target.key,
      folder: resolution.folder,
      children: revisionChildren,
    },
  ];
}

function repairNode(
  resolution: TargetResolution & { kind: "error" },
): RepairNode {
  return {
    kind: "repair",
    label: resolution.folder.name,
    message: resolution.message,
    folder: resolution.folder,
  };
}

function shortRevisionId(revisionId: string): string {
  return revisionId.split(":").at(-1)?.slice(0, 12) ?? revisionId;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
