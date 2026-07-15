import type { DiagnosticDocument, PointbreakCli } from "./cli";

export interface HumanWriteContext {
  actorId: string;
  track: string;
}

export interface HumanWriteResult<T extends DiagnosticDocument> {
  document: T;
  refreshed: boolean;
}

export interface HumanWriteRequest<
  T extends DiagnosticDocument,
  TPreparation = undefined,
> {
  repo: string;
  resource: unknown;
  trackOverride?: string;
  prepare?(context: HumanWriteContext): Promise<TPreparation>;
  confirm(
    context: HumanWriteContext,
    preparation: TPreparation,
  ): Promise<boolean>;
  write(context: HumanWriteContext, preparation: TPreparation): Promise<T>;
}

export interface HumanWriteCoordinatorDependencies {
  resolveTrack(resource: unknown): string;
  showDiagnostic(message: string): Promise<unknown>;
  refresh(): Promise<void>;
  showRefreshError(message: string): Promise<unknown>;
}

export class HumanWriteCoordinator {
  constructor(
    private readonly cli: PointbreakCli,
    private readonly dependencies: HumanWriteCoordinatorDependencies,
  ) {}

  async run<T extends DiagnosticDocument, TPreparation = undefined>(
    request: HumanWriteRequest<T, TPreparation>,
  ): Promise<HumanWriteResult<T> | undefined> {
    let context = await this.resolveContext(request);
    while (true) {
      const preparation = request.prepare
        ? await request.prepare(context)
        : (undefined as TPreparation);
      if (!(await request.confirm(context, preparation))) {
        return undefined;
      }
      const latest = await this.resolveContext(request);
      if (!sameContext(context, latest)) {
        context = latest;
        continue;
      }

      const document = await request.write(latest, preparation);
      await this.showDiagnostics(document.diagnostics);
      let refreshed = true;
      try {
        await this.dependencies.refresh();
      } catch {
        refreshed = false;
        await this.dependencies.showRefreshError(
          "Pointbreak recorded the write, but could not refresh the review.",
        );
      }
      return { document, refreshed };
    }
  }

  private async resolveContext<
    T extends DiagnosticDocument,
    TPreparation = undefined,
  >(request: HumanWriteRequest<T, TPreparation>): Promise<HumanWriteContext> {
    const track = (
      request.trackOverride ?? this.dependencies.resolveTrack(request.resource)
    ).trim();
    if (!track) {
      throw new Error("Pointbreak human write track must not be empty.");
    }
    const identity = await this.cli.identityWhoami(request.repo);
    if (!identity.actorId.trim()) {
      throw new Error("Pointbreak resolved an empty writer actor.");
    }
    return { actorId: identity.actorId, track };
  }

  private async showDiagnostics(diagnostics: unknown[] | undefined) {
    for (const diagnostic of diagnostics ?? []) {
      await this.dependencies.showDiagnostic(diagnosticMessage(diagnostic));
    }
  }
}

function sameContext(
  left: HumanWriteContext,
  right: HumanWriteContext,
): boolean {
  return left.actorId === right.actorId && left.track === right.track;
}

function diagnosticMessage(diagnostic: unknown): string {
  if (typeof diagnostic === "string") return diagnostic;
  if (typeof diagnostic === "object" && diagnostic !== null) {
    const message = (diagnostic as { message?: unknown }).message;
    if (typeof message === "string") return message;
    const serialized = JSON.stringify(diagnostic);
    if (serialized) return serialized;
  }
  return "Pointbreak reported a write diagnostic.";
}
