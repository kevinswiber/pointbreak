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
import type {
  AttentionItem,
  AttentionListDoc,
  PointbreakCli,
  RevisionListDoc,
} from "./cli";
import { revisionDiscoveryDisplay } from "./idDisplay";
import { newestRevisionEntries } from "./revisionOrder";
import type { TargetResolution } from "./targetResolver";

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
  attentionId: string;
  item: AttentionItem;
  lens: "attention";
  command?: "pointbreak.openAnnotatedDiff";
}

export interface RevisionItemNode extends NodeBase {
  kind: "revision-item";
  targetKey: string;
  folder: WorkspaceFolder;
  description: string;
  revisionId: string;
  command: "pointbreak.openAnnotatedDiff";
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
  private readonly visibilityEmitter = new EventEmitter<boolean>();
  readonly onDidChangeVisibility: Event<boolean> = this.visibilityEmitter.event;
  private readonly attentionByTarget = new Map<string, AttentionListDoc>();
  private readonly revisionsByTarget = new Map<string, RevisionListDoc>();
  private readonly loadErrors = new Map<string, string>();
  private treeView: TreeView<TreeNode> | undefined;
  private visible = false;

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
    if (visible === this.visible) return;
    this.visible = visible;
    this.visibilityEmitter.fire(visible);
    if (!visible) {
      return;
    }

    await this.refresh();
  }

  async refresh(): Promise<void> {
    await this.refreshResolutions(distinctResolvedTargets(this.resolutions));
  }

  async refreshTarget(targetKey: string, signal?: AbortSignal): Promise<void> {
    const resolution = distinctResolvedTargets(this.resolutions).find(
      (candidate) => candidate.target.key === targetKey,
    );
    if (resolution) {
      await this.refreshResolutions([resolution], signal);
    }
  }

  isVisible(): boolean {
    return this.visible;
  }

  findAttentionItem(
    targetKey: string,
    attentionId: string,
  ): AttentionItem | undefined {
    return this.attentionByTarget
      .get(targetKey)
      ?.items.find((item) => item.id === attentionId);
  }

  private async refreshResolutions(
    resolutions: ResolvedTarget[],
    signal?: AbortSignal,
  ): Promise<void> {
    await Promise.all(
      resolutions.map(async (resolution) => {
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
          if (signal?.aborted) return;
          this.attentionByTarget.set(key, attention);
          this.revisionsByTarget.set(key, revisions);
          this.loadErrors.delete(key);
        } catch (error) {
          if (signal?.aborted) return;
          this.attentionByTarget.delete(key);
          this.revisionsByTarget.delete(key);
          this.loadErrors.set(key, errorMessage(error));
        }
      }),
    );
    if (signal?.aborted) return;
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
    if (node.kind === "revision-item") {
      item.contextValue = "pointbreak.revision";
    } else if (node.kind === "attention-item") {
      item.contextValue = attentionContextValue(node.item);
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
    activeProviders.delete(this);
    this.emitter.dispose();
    this.visibilityEmitter.dispose();
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
      label: attentionLabel(item),
      targetKey: resolution.target.key,
      folder: resolution.folder,
      description: item.tier,
      revisionId: item.revisionId,
      attentionId: item.id,
      item,
      lens: "attention",
      command: item.revisionId ? "pointbreak.openAnnotatedDiff" : undefined,
    }),
  );
  const revisionChildren: RevisionItemNode[] = newestRevisionEntries(
    revisions?.entries ?? [],
  )
    .slice(0, 5)
    .map((entry) => ({
      kind: "revision-item",
      ...revisionDiscoveryDisplay(entry),
      targetKey: resolution.target.key,
      folder: resolution.folder,
      revisionId: entry.revisionId,
      command: "pointbreak.openAnnotatedDiff",
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

function attentionContextValue(item: AttentionItem): string | undefined {
  if (
    item.kind === "open_input_request" ||
    item.kind === "follow_up_outstanding"
  ) {
    return "pointbreak.attention.inputRequest";
  }
  if (
    item.kind === "ambiguous_assessment" ||
    item.kind === "stale_assessment"
  ) {
    return "pointbreak.attention.assessment";
  }
  if (item.kind === "failed_validation") {
    return "pointbreak.attention.failedValidation";
  }
  if (item.kind === "competing_heads") {
    return "pointbreak.attention.headResolution";
  }
  return undefined;
}

function attentionLabel(item: AttentionItem): string {
  return item.kind === "open_input_request" ? item.title : item.kind;
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
