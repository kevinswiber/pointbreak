// The diff controller: the lifecycle, lazy file bodies, navigator, and jump keys
// over the two diff surfaces — the route-preserving overlay (legacy, reconciled
// from `state.diff`) and the routed diff page (reconciled from `state.diffPage`/
// `diffRevision`). Ported from the served app.js diff cluster (`openDiff` /
// `openRevisionDiff` / `closeDiff` / `renderDiffOverlay` / `applyDiffFocus` /
// `scrollToAnno` / lazy bodies / navigator / `jump*`).
//
// Structural moves:
//   - The overlay opens through the overlay teardown manager (`register("diff", …)`
//     + `open("diff")`) and imports NO sibling overlay (palette / help). The page
//     never touches the manager — it is a route surface (`activeName()` stays
//     null on it), so palette/help can still open above it.
//   - Route changes go through `router.navigate` and never call render: the
//     store subscriber repaints, and the reconcilers (`renderDiffOverlay` /
//     `renderDiffPage`, run by render) open/close/paint their surfaces.
//   - The page's payload comes from the composite revision document (the
//     detail module's exported `ensureRevisionComposite` seam): annotations AND
//     snapshot identity derive from it, so cold and grouped-away deep links
//     paint annotated with nothing loaded. Bytes come from `/api/snapshots/{id}`
//     on both surfaces.
//
// It consumes the pure `diff/render.renderDiff(snapshotId, artifact, annotations) →
// { html, ctx }`, assigning the returned `ctx` (and resetting the cursors/filter
// the pure renderer no longer writes) to module-local state. The diff cursors /
// `diffCtx` / `shownDiff*` / the overlay's nav filter stay module-local — never on
// the store; the page's nav filter is route state (`state.diffNav`).

import { CLASS, diffStatusClass } from "../classNames";
import { compositeAnnotations, ensureRevisionComposite } from "../detail";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fetchJSON } from "../http";
import {
  annotationsForRevision,
  revisionIdForSnapshot,
  snapshotContentHashForRevision,
  snapshotIdForRevision,
} from "../model";
import { activeName, close, open, register } from "../overlay";
import { shortId } from "../refs";
import { navigate } from "../router";
import { getState } from "../store";
import {
  type Annotation,
  type DiffArtifact,
  type DiffCtx,
  type DiffNavFilter,
  type DiffNavSummary,
  fileFactCount,
  filePathLabel,
  isDiffNavFilter,
  renderDiff,
  renderDiffFileBody,
  renderDiffNavFilters,
  renderDiffNavSummary,
  unanchoredReason,
} from "./render";

// The two diff surfaces' fixed-id hosts. Only one surface paints at a time (the
// page owns the frame while `state.diffPage` is set; render never runs the
// overlay reconciler then), and every shared helper resolves its container
// through the active surface.
interface DiffSurface {
  title: string;
  nav: string;
  body: string;
}
const MODAL_SURFACE: DiffSurface = {
  title: "#diff-title",
  nav: "#diff-nav",
  body: "#diff-body",
};
const PAGE_SURFACE: DiffSurface = {
  title: "#diff-page-title",
  nav: "#diff-page-nav",
  body: "#diff-page-body",
};

function activeSurface(): DiffSurface {
  return getState().diffPage ? PAGE_SURFACE : MODAL_SURFACE;
}

function surfaceBody(): HTMLElement | null {
  return $(activeSurface().body);
}

function surfaceNav(): HTMLElement | null {
  return $(activeSurface().nav);
}

// The identity of the diff currently painted — surface plus payload address —
// so a re-render with an unchanged route does not re-fetch. Set before the
// fetch, so repaints landing while it is in flight fall into the cheap
// reconcile branch instead of stacking fetches.
let shownDiffKey: string | null = null;
// Module-local render context for the open diff: the files + anchored facts the
// delegated body / nav listeners read to lazily fill a collapsed file body or
// expand-then-scroll to a fact. Set when renderDiff paints, cleared when the
// surface closes. NOT route state (state.diff stays the snapshot-id string|null).
let diffCtx: DiffCtx | null = null;
// Cursors for the diff-local jump keys (next/prev fact, next/prev change),
// reset each time a new diff renders. `diffNavFilter` is the OVERLAY's
// navigator filter; the page reads `state.diffNav` instead (route state).
let diffFactCursor = -1;
let diffChangeCursor = -1;
let diffNavFilter: DiffNavFilter = "all";
// The page's last-painted navigator filter and `?file=` target, so a repaint
// only re-renders the navigator / re-scrolls when the route actually moved.
let shownDiffNavFilter: DiffNavFilter = "all";
let shownDiffFile: string | null = null;

/** The navigator filter the active surface renders under. */
function activeNavFilter(): DiffNavFilter {
  return getState().diffPage ? getState().diffNav : diffNavFilter;
}

// ---------------------------------------------------------------------------
// Route-only open / close (the open/close DOM is the reconciler's job)
// ---------------------------------------------------------------------------

// DIFF_LENS_ROUTE_SEAM: this modal remains quick readback over `diff=` route
// state. A full-page diff lens route/data contract is deferred until it can be
// designed as its own route and payload seam rather than inferred here.
/** Open the snapshot diff for a snapshot id (optionally focusing a fact), route-only. */
export function openDiff(
  snapshotId: string,
  focusId: string | null = null,
  contentHash: string | null = null,
): void {
  navigate({
    diff: snapshotId,
    diffHash: contentHash || null,
    focus: focusId || null,
  });
}

/** Open the diff for the snapshot a revision captured, with its content hash. */
export function openRevisionDiff(
  revisionId: string,
  focusId: string | null = null,
): void {
  const snapshotId = snapshotIdForRevision(revisionId);
  if (snapshotId)
    openDiff(snapshotId, focusId, snapshotContentHashForRevision(revisionId));
}

/** Clear the diff route (replace, so Back does not reopen it); the repaint closes it. */
export function closeDiff(): void {
  const modal = $("#diff-modal");
  if (!getState().diff && modal?.classList.contains("hidden")) return;
  navigate({ diff: null, diffHash: null, focus: null }, { replace: true });
}

// ---------------------------------------------------------------------------
// The reconciler (run by render): open/close the modal from the route + fetch
// ---------------------------------------------------------------------------

// The shared fetch-and-paint body both reconcilers use: paint the loading
// state, fetch the snapshot bytes, render them with the given annotations into
// the surface, and reset the jump cursors. Resolves true when it painted (the
// callers run their surface-specific post-paint steps), false when a later
// route change superseded the fetch.
async function paintDiffSurface(
  s: DiffSurface,
  opts: {
    snapshotId: string;
    contentHash: string | null;
    annotations: Annotation[];
    title: string;
    stillCurrent: () => boolean;
    // A quiet note painted above the bytes when the surface has no facts to
    // offer (the snapshot-only page); null renders nothing.
    factsNote: string | null;
  },
): Promise<boolean> {
  const title = $(s.title);
  if (title) title.textContent = opts.title;
  const body = $(s.body);
  if (body) body.innerHTML = `<p class="${CLASS.empty}">loading snapshot…</p>`;
  const nav = $(s.nav);
  if (nav) nav.innerHTML = "";
  let snapshotUrl = `/api/snapshots/${encodeURIComponent(opts.snapshotId)}`;
  if (opts.contentHash)
    snapshotUrl += `?contentHash=${encodeURIComponent(opts.contentHash)}`;
  try {
    const artifact = await fetchJSON(snapshotUrl);
    // A later route change may have superseded this fetch.
    if (!opts.stillCurrent()) return false;
    const { html, ctx } = renderDiff(
      opts.snapshotId,
      artifact as DiffArtifact,
      opts.annotations,
    );
    const note = opts.factsNote
      ? `<p class="${CLASS.empty}">${escapeHtml(opts.factsNote)}</p>`
      : "";
    const liveBody = $(s.body);
    if (liveBody) liveBody.innerHTML = note + html;
    diffCtx = ctx;
    diffFactCursor = -1;
    diffChangeCursor = -1;
    const liveNav = $(s.nav);
    if (liveNav) liveNav.innerHTML = renderDiffNav();
    applyDiffFocus();
    return true;
  } catch (err: unknown) {
    if (!opts.stillCurrent()) return false;
    const liveBody = $(s.body);
    if (liveBody)
      liveBody.innerHTML = `<p class="${CLASS.empty}">error: ${escapeHtml(
        err instanceof Error ? err.message : String(err),
      )}</p>`;
    return false;
  }
}

/**
 * Reconcile the diff modal DOM with `state.diff`/`state.focus`. Part of the render
 * path (the store subscriber calls it): it both opens (user action, deep link,
 * Back/Forward) and closes. Returns the in-flight fetch so a caller can await the
 * paint; render ignores the return.
 */
export function renderDiffOverlay(): Promise<void> {
  const state = getState();
  // While the page owns the frame, the overlay reconciler must not run: a null
  // `state.diff` there would tear down the page's render context.
  if (state.diffPage) return Promise.resolve();
  if (!state.diff) {
    close("diff");
    shownDiffKey = null;
    diffCtx = null;
    return Promise.resolve();
  }
  const snapshotId = state.diff;
  const contentHash = state.diffHash;
  const key = `modal:${snapshotId}|${contentHash ?? ""}`;
  if (key === shownDiffKey) {
    // Re-show only if the diff is not already the active overlay, so an unrelated
    // repaint while the diff is open never re-steals focus to the close button.
    if (activeName() !== "diff") open("diff", "#diff-close");
    applyDiffFocus();
    return Promise.resolve();
  }
  shownDiffKey = key;
  // The snapshot endpoint is snapshot-scoped (no revision id on the wire); the
  // revision id is recovered from the revisions list for annotation lookup.
  const revisionId = revisionIdForSnapshot(snapshotId, contentHash);
  const label = revisionId ? shortId(revisionId) : "";
  // A fresh overlay starts at the unfiltered navigator (module-local state; the
  // page's filter is route state and never resets here).
  diffNavFilter = "all";
  // Opening through the manager tears down any prior overlay (palette/help) with
  // no focus restore — the indirection that replaces the served explicit closes.
  open("diff", "#diff-close");
  return paintDiffSurface(MODAL_SURFACE, {
    snapshotId,
    contentHash,
    annotations: revisionId ? annotationsForRevision(revisionId) : [],
    title: label
      ? `${label} · snapshot ${shortId(snapshotId)}`
      : shortId(snapshotId),
    stillCurrent: () =>
      getState().diff === snapshotId && getState().diffHash === contentHash,
    factsNote: null,
  }).then(() => {});
}

// Expand (rendering on first expand) and scroll the `?file=` target into view.
// An unknown or absent path is ignored quietly; the marker keeps a repaint from
// re-scrolling until the route names a different file.
function applyDiffFileScroll(): void {
  const path = getState().diffFile;
  shownDiffFile = path;
  if (!path || !diffCtx) return;
  const idx = diffCtx.files.findIndex(
    (f) => f.new_path === path || f.old_path === path,
  );
  if (idx < 0) return;
  const section = surfaceBody()?.querySelector<HTMLElement>(
    `.dfile[data-dfile="${idx}"]`,
  );
  if (section) {
    expandDiffFile(section);
    section.scrollIntoView({ block: "start" });
  }
}

// Paint the page for a revision-primary route: annotations AND snapshot
// identity derive from the composite document (never the paged history or the
// list document, which miss cold and grouped-away revisions).
async function renderDiffPageFromRevision(revisionId: string): Promise<void> {
  const stillCurrent = () =>
    getState().diffPage && getState().diffRevision === revisionId;
  const doc = await ensureRevisionComposite(revisionId);
  if (!stillCurrent()) return;
  if (!doc) {
    const body = $(PAGE_SURFACE.body);
    if (body)
      body.innerHTML = `<p class="${CLASS.empty}">error: revision ${escapeHtml(
        shortId(revisionId),
      )} could not be loaded</p>`;
    return;
  }
  const revision = doc.revision ?? {};
  const snapshotId = revision.objectId;
  if (!snapshotId) {
    const body = $(PAGE_SURFACE.body);
    if (body)
      body.innerHTML = `<p class="${CLASS.empty}">this revision names no captured snapshot</p>`;
    return;
  }
  const painted = await paintDiffSurface(PAGE_SURFACE, {
    snapshotId,
    contentHash: revision.objectArtifactContentHash ?? null,
    annotations: compositeAnnotations(doc),
    title: `${shortId(revisionId)} · snapshot ${shortId(snapshotId)}`,
    stillCurrent,
    factsNote: null,
  });
  if (painted) {
    shownDiffNavFilter = activeNavFilter();
    applyDiffFileScroll();
  }
}

// Paint the page for a snapshot-only route (an unmappable legacy link): the
// bytes render best-effort with blank facts and a quiet note.
async function renderDiffPageFromSnapshot(
  snapshotId: string,
  contentHash: string | null,
): Promise<void> {
  const stillCurrent = () =>
    getState().diffPage &&
    !getState().diffRevision &&
    getState().diff === snapshotId &&
    getState().diffHash === contentHash;
  const painted = await paintDiffSurface(PAGE_SURFACE, {
    snapshotId,
    contentHash,
    annotations: [],
    title: `snapshot ${shortId(snapshotId)}`,
    stillCurrent,
    factsNote:
      "no review facts — this link names a snapshot the record cannot map to a revision",
  });
  if (painted) {
    shownDiffNavFilter = activeNavFilter();
    applyDiffFileScroll();
  }
}

/**
 * Reconcile the routed diff page with `state.diffPage`/`diffRevision`/`diff`.
 * Part of the render path while the page owns the frame. An unchanged route
 * reconciles cheaply: the navigator re-renders only when the route filter
 * moved, the `?file=` scroll re-applies only when the route names a new file,
 * and the focus route re-applies (the n/p jump path). Returns the in-flight
 * work so a caller can await the paint; render ignores the return.
 */
export function renderDiffPage(): Promise<void> {
  const state = getState();
  if (!state.diffPage) return Promise.resolve();
  const key = state.diffRevision
    ? `page:rev:${state.diffRevision}`
    : state.diff
      ? `page:snap:${state.diff}|${state.diffHash ?? ""}`
      : null;
  if (!key) {
    // Unaddressable page state (no revision, no snapshot) — nothing to paint.
    const body = $(PAGE_SURFACE.body);
    if (body)
      body.innerHTML = `<p class="${CLASS.empty}">nothing to diff — this link names no snapshot</p>`;
    return Promise.resolve();
  }
  if (key === shownDiffKey) {
    if (activeNavFilter() !== shownDiffNavFilter) {
      shownDiffNavFilter = activeNavFilter();
      const nav = $(PAGE_SURFACE.nav);
      if (nav) nav.innerHTML = renderDiffNav();
    }
    if (getState().diffFile !== shownDiffFile) applyDiffFileScroll();
    applyDiffFocus();
    return Promise.resolve();
  }
  shownDiffKey = key;
  if (state.diffRevision) return renderDiffPageFromRevision(state.diffRevision);
  return renderDiffPageFromSnapshot(
    state.diff as string,
    state.diffHash ?? null,
  );
}

function applyDiffFocus(): void {
  const focusId = getState().focus;
  if (focusId) scrollToAnno(focusId);
}

// ---------------------------------------------------------------------------
// Fact focus + scroll
// ---------------------------------------------------------------------------

function focusDiffFactRoute(id: string): boolean {
  if (!id || getState().focus === id) return false;
  navigate({ focus: id }, { replace: true });
  return true;
}

// Scroll a review fact's annotation into view and flash it, expanding its file
// first if it lives in a default-collapsed section. The single path a focus=
// deep-link, a gutter click, a navigator entry, and the n/p keys all route through.
/** Scroll to (and flash) an annotation, expanding its file if collapsed. */
export function scrollToAnno(
  id: string,
  opts: { updateRoute?: boolean } = {},
): void {
  if (opts.updateRoute && focusDiffFactRoute(id)) return;
  const sel = `.anno[data-anno="${id}"]`;
  const body = surfaceBody();
  let target = body?.querySelector<HTMLElement>(sel) ?? null;
  if (!target && diffCtx) {
    const fact = diffCtx.anchored.find((a) => a.id === id);
    const filePath = fact?.target?.filePath;
    if (filePath) {
      const idx = diffCtx.files.findIndex(
        (f) => f.new_path === filePath || f.old_path === filePath,
      );
      if (idx >= 0) {
        const section = body?.querySelector<HTMLElement>(
          `.dfile[data-dfile="${idx}"]`,
        );
        if (section) {
          expandDiffFile(section);
          target = body?.querySelector<HTMLElement>(sel) ?? null;
        }
      }
    }
  }
  if (target) {
    target.scrollIntoView({ block: "center" });
    flashAnno(target);
  }
}

// Restart the flash animation even if the element was flashed before (n/p may land
// on it twice).
function flashAnno(el: HTMLElement): void {
  el.classList.remove("anno-flash");
  void el.offsetWidth;
  el.classList.add("anno-flash");
}

// ---------------------------------------------------------------------------
// Lazy file bodies (the accordion)
// ---------------------------------------------------------------------------

// Fill a collapsed file's lazy body on first expand, cached via a rendered flag.
function ensureDiffFileBody(section: HTMLElement): void {
  if (!diffCtx) return;
  const body = section.querySelector<HTMLElement>("[data-dfile-body]");
  if (!body || body.dataset.rendered) return;
  const idx = Number(section.dataset.dfile);
  body.innerHTML = renderDiffFileBody(diffCtx.files[idx], diffCtx.anchored);
  body.removeAttribute("data-fact-vicinity");
  body.dataset.rendered = "1";
}

function diffFileHeader(section: HTMLElement): HTMLElement | null {
  return section.querySelector<HTMLElement>(".dfile-head");
}

function diffFileExpanded(section: HTMLElement): boolean {
  const head = diffFileHeader(section);
  return head ? head.getAttribute("aria-expanded") === "true" : false;
}

function setDiffFileExpanded(section: HTMLElement, open: boolean): void {
  const value = String(open);
  section.dataset.expanded = value;
  const head = diffFileHeader(section);
  if (head) head.setAttribute("aria-expanded", value);
}

// Expand one accordion file section (render its body on first expand). Used by
// navigation (navigator entry, focus jump) where the target must end up open.
/** Expand a file section, filling its body on first expand. */
export function expandDiffFile(section: HTMLElement): void {
  ensureDiffFileBody(section);
  setDiffFileExpanded(section, true);
}

// Toggle one accordion file section; render its body on first expand. Transient DOM
// state, reconciled on each overlay render — not route state.
/** Toggle a file section open/closed, filling its body on first expand. */
export function toggleDiffFile(section: HTMLElement): void {
  const isOpen = diffFileExpanded(section);
  if (!isOpen) ensureDiffFileBody(section);
  setDiffFileExpanded(section, !isOpen);
}

// ---------------------------------------------------------------------------
// The file/fact navigator
// ---------------------------------------------------------------------------

// The file/fact navigator sidebar: one entry per file (status + path + fact badge)
// plus the unanchored-facts panel, so every fact — including those not anchored to
// a captured diff line — is reachable on a large changeset.
function renderDiffNav(): string {
  if (!diffCtx) return "";
  const navFilter = activeNavFilter();
  const { files, anchored, unanchored, filePaths } = diffCtx;
  const visibleFiles = files
    .map((f, i) => ({ f, i, factCount: fileFactCount(f, anchored) }))
    .filter((item) => {
      if (navFilter === "with-facts") return item.factCount > 0;
      if (navFilter === "unanchored") return false;
      return true;
    });
  const fileItems = visibleFiles
    .map(({ f, i, factCount: n }) => {
      const badge = n ? `<span class="${CLASS.dfileNotes}">${n}</span>` : "";
      return `<li><button class="${CLASS.diffNavFile}" data-nav-file="${i}">
        <span class="${diffStatusClass(escapeHtml(f.status ?? ""))}">${escapeHtml(f.status ?? "")}</span>
        <span class="${CLASS.dpath}">${escapeHtml(filePathLabel(f))}</span>${badge}</button></li>`;
    })
    .join("");
  let html =
    renderDiffNavSummary(diffNavSummary()) + renderDiffNavFilters(navFilter);
  if (navFilter !== "unanchored") {
    html += `<ol class="${CLASS.diffNavFiles}">${fileItems}</ol>`;
  }
  if (unanchored.length && navFilter !== "with-facts") {
    const entries = unanchored
      .map(
        (a) =>
          `<li><button class="${CLASS.diffNavFact}" data-anno="${escapeHtml(a.id)}"><span>${escapeHtml(a.title)}</span><span class="${CLASS.diffNavReason}">${escapeHtml(unanchoredReason(a, filePaths))}</span></button></li>`,
      )
      .join("");
    html += `<section class="${CLASS.diffUnanchored}" aria-label="unanchored review facts">
      <h3>${unanchored.length} not anchored to a diff line</h3>
      <ol>${entries}</ol></section>`;
  }
  return html;
}

function diffNavSummary(): DiffNavSummary {
  if (!diffCtx) return { fileCount: 0, factCount: 0, unanchoredCount: 0 };
  return {
    fileCount: diffCtx.files.length,
    factCount: diffCtx.anchored.length + diffCtx.unanchored.length,
    unanchoredCount: diffCtx.unanchored.length,
  };
}

/**
 * Set the navigator's file/fact filter. On the page the filter is route state
 * (`?nav=`, a shareable refinement — replace, not push; the store subscriber's
 * repaint re-renders the navigator). On the overlay it stays module-local and
 * re-renders directly.
 */
export function setDiffNavFilter(filter: string): void {
  if (!isDiffNavFilter(filter)) return;
  if (getState().diffPage) {
    navigate({ diffNav: filter }, { replace: true });
    return;
  }
  diffNavFilter = filter;
  const nav = surfaceNav();
  if (nav) nav.innerHTML = renderDiffNav();
}

// ---------------------------------------------------------------------------
// Jump keys (next/prev fact, next/prev change)
// ---------------------------------------------------------------------------

// All rendered fact anchors in document order (inline annotations + unanchored
// bodies) — the ordering n/p cycles through.
function diffFactTargets(): HTMLElement[] {
  return Array.from(
    surfaceBody()?.querySelectorAll<HTMLElement>(".anno[data-anno]") ?? [],
  );
}

// All change anchors (hunk headers) in rendered file bodies — the ordering ]/[
// cycles through.
function diffChangeTargets(): HTMLElement[] {
  return Array.from(
    surfaceBody()?.querySelectorAll<HTMLElement>(".dhunk") ?? [],
  );
}

function jumpToTarget(
  targets: HTMLElement[],
  cursor: number,
  dir: number,
): number {
  if (!targets.length) return cursor;
  const next = (cursor + dir + targets.length) % targets.length;
  const el = targets[next];
  const section = el.closest<HTMLElement>(".dfile");
  if (section && !diffFileExpanded(section)) expandDiffFile(section);
  el.scrollIntoView({ block: "center" });
  return next;
}

/** Jump to the next/previous review fact, syncing the focus route. */
export function jumpFact(dir: number): void {
  const targets = diffFactTargets();
  if (!targets.length) return;
  diffFactCursor = (diffFactCursor + dir + targets.length) % targets.length;
  const el = targets[diffFactCursor];
  if (el) {
    const section = el.closest<HTMLElement>(".dfile");
    if (section && !diffFileExpanded(section)) expandDiffFile(section);
    const id = el.dataset.anno;
    if (id && focusDiffFactRoute(id)) return;
    el.scrollIntoView({ block: "center" });
    flashAnno(el);
  }
}

/** Jump to the next/previous change (hunk header). */
export function jumpChange(dir: number): void {
  diffChangeCursor = jumpToTarget(diffChangeTargets(), diffChangeCursor, dir);
}

// ---------------------------------------------------------------------------
// Fixed-id controls (wired once by the composition root)
// ---------------------------------------------------------------------------

// The delegated body handlers, shared by both surfaces: a file header toggles
// its section; a render-all button hydrates a fact-vicinity body; an annotated
// row's gutter scrolls to its annotation.
function onDiffBodyClick(ev: Event): void {
  const t = ev.target;
  if (!(t instanceof Element)) return;
  const renderAll = t.closest("[data-render-diff-file]");
  if (renderAll) {
    const section = renderAll.closest<HTMLElement>(".dfile");
    if (section) {
      ensureDiffFileBody(section);
      setDiffFileExpanded(section, true);
    }
    return;
  }
  const head = t.closest(".dfile-head");
  if (head) {
    const section = head.closest<HTMLElement>(".dfile");
    if (section) toggleDiffFile(section);
    return;
  }
  const noted = t.closest<HTMLElement>(".drow-noted[data-anno]");
  if (noted) {
    const id = noted.dataset.anno;
    if (id) scrollToAnno(id, { updateRoute: true });
  }
}

function onDiffBodyKeydown(ev: KeyboardEvent): void {
  if (ev.key !== "Enter" && ev.key !== " ") return;
  const t = ev.target;
  if (!(t instanceof Element)) return;
  const head = t.closest(".dfile-head");
  if (head) {
    ev.preventDefault();
    const section = head.closest<HTMLElement>(".dfile");
    if (section) toggleDiffFile(section);
    return;
  }
  const noted = t.closest<HTMLElement>(".drow-noted[data-anno]");
  if (noted) {
    ev.preventDefault();
    const id = noted.dataset.anno;
    if (id) scrollToAnno(id, { updateRoute: true });
  }
}

// The navigator sidebar delegate, shared by both surfaces: a filter button
// re-renders the nav; a file entry expands + scrolls its section; an
// unanchored-fact entry scrolls to its body.
function onDiffNavClick(ev: Event): void {
  const t = ev.target;
  if (!(t instanceof Element)) return;
  const filterBtn = t.closest<HTMLElement>("[data-diff-nav-filter]");
  if (filterBtn) {
    const filter = filterBtn.dataset.diffNavFilter;
    if (filter) setDiffNavFilter(filter);
    return;
  }
  const fileBtn = t.closest<HTMLElement>("[data-nav-file]");
  if (fileBtn) {
    const idx = Number(fileBtn.dataset.navFile);
    const section = surfaceBody()?.querySelector<HTMLElement>(
      `.dfile[data-dfile="${idx}"]`,
    );
    if (section) {
      expandDiffFile(section);
      section.scrollIntoView({ block: "start" });
    }
    return;
  }
  const factBtn = t.closest<HTMLElement>(".diff-nav-fact[data-anno]");
  if (factBtn) {
    const id = factBtn.dataset.anno;
    if (id) scrollToAnno(id, { updateRoute: true });
  }
}

/**
 * Wire the diff surfaces' fixed-id controls and register the overlay's teardown
 * with the overlay manager (the page registers nothing — it is a route surface).
 * The delegated body / nav listeners read the module-local `diffCtx`; they are
 * installed once here, never at the open call site.
 */
export function initControls(): void {
  const modal = $<HTMLElement>("#diff-modal");
  if (modal)
    register("diff", {
      node: modal,
      onClose: closeDiff,
      // The diff's own jump keys, run through the overlay manager's delegation:
      // ]/[ step changes, n/p step review facts. Escape is not here — the
      // manager owns it universally.
      onKey: (ev) => {
        switch (ev.key) {
          case "]":
            ev.preventDefault();
            jumpChange(1);
            return true;
          case "[":
            ev.preventDefault();
            jumpChange(-1);
            return true;
          case "n":
            ev.preventDefault();
            jumpFact(1);
            return true;
          case "p":
            ev.preventDefault();
            jumpFact(-1);
            return true;
          default:
            return false;
        }
      },
    });
  $("#diff-close")?.addEventListener("click", () => closeDiff());
  $("#diff-page-close")?.addEventListener("click", () => closeDiff());
  modal?.addEventListener("click", (ev) => {
    if (ev.target === modal) closeDiff();
  });
  // Typed HTMLElement so the keydown listener narrows to KeyboardEvent.
  for (const sel of [MODAL_SURFACE.body, PAGE_SURFACE.body]) {
    const body = $<HTMLElement>(sel);
    body?.addEventListener("click", onDiffBodyClick);
    body?.addEventListener("keydown", onDiffBodyKeydown);
  }
  for (const sel of [MODAL_SURFACE.nav, PAGE_SURFACE.nav]) {
    $(sel)?.addEventListener("click", onDiffNavClick);
  }
}
