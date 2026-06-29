// The diff overlay controller: the lifecycle, lazy file bodies, navigator, and
// jump keys over the route-preserving diff overlay. Ported from the served app.js
// diff cluster (`openDiff` / `openRevisionDiff` / `closeDiff` / `renderDiffOverlay`
// / `applyDiffFocus` / `scrollToAnno` / lazy bodies / navigator / `jump*`).
//
// Two structural moves make this the cycle-cutting controller:
//   - It opens through the overlay teardown manager (`overlay.register("diff", …)`
//     + `overlay.open("diff")`) and imports NO sibling overlay (palette / help).
//     The manager's mutual exclusion tears down whatever overlay was open, so the
//     explicit `closePalette` / `closeKeyHelp` calls the served code made are gone.
//   - It clears the route through `router.navigate` and never calls render: the
//     store subscriber repaints, so `closeDiff` only changes route state and the
//     reconciler (`renderDiffOverlay`, run by render) opens/closes the modal.
//
// It consumes the pure `diff/render.renderDiff(objectId, artifact, annotations) →
// { html, ctx }`, assigning the returned `ctx` (and resetting the cursors/filter
// the pure renderer no longer writes) to module-local state. The diff cursors /
// `diffCtx` / `shownDiff*` / nav filter stay module-local — never on the store.

import { CLASS, diffStatusClass } from "../classNames";
import { $ } from "../dom";
import { escapeHtml } from "../escape";
import { fetchJSON } from "../http";
import {
  annotationsForRevision,
  objectArtifactHashForRevision,
  objectIdForRevision,
  revisionIdForObject,
} from "../model";
import { activeName, close, open, register } from "../overlay";
import { shortId } from "../refs";
import { navigate } from "../router";
import { getState } from "../store";
import {
  type DiffArtifact,
  type DiffCtx,
  type DiffNavFilter,
  type DiffNavSummary,
  fileFactCount,
  filePathLabel,
  renderDiff,
  renderDiffFileBody,
  renderDiffNavFilters,
  renderDiffNavSummary,
  unanchoredReason,
} from "./render";

// The object artifact currently painted in the modal, so a re-render with an
// unchanged overlay does not re-fetch.
let shownDiffObject: string | null = null;
let shownDiffHash: string | null = null;
// Module-local render context for the open diff: the files + anchored facts the
// delegated #diff-body / #diff-nav listeners read to lazily fill a collapsed file
// body or expand-then-scroll to a fact. Set when renderDiff paints, cleared when
// the overlay closes. NOT route state (state.diff stays the object-id string|null).
let diffCtx: DiffCtx | null = null;
// Cursors for the diff-local jump keys (next/prev fact, next/prev change) and the
// navigator filter, reset each time a new diff renders.
let diffFactCursor = -1;
let diffChangeCursor = -1;
let diffNavFilter: DiffNavFilter = "all";

const DIFF_NAV_FILTERS: readonly DiffNavFilter[] = [
  "all",
  "with-facts",
  "unanchored",
];

function isDiffNavFilter(value: string): value is DiffNavFilter {
  return (DIFF_NAV_FILTERS as readonly string[]).includes(value);
}

// ---------------------------------------------------------------------------
// Route-only open / close (the open/close DOM is the reconciler's job)
// ---------------------------------------------------------------------------

// DIFF_LENS_ROUTE_SEAM: this modal remains quick readback over `diff=` route
// state. A full-page diff lens route/data contract is deferred until it can be
// designed as its own route and payload seam rather than inferred here.
/** Open the snapshot diff for an object id (optionally focusing a fact), route-only. */
export function openDiff(
  objectId: string,
  focusId: string | null = null,
  contentHash: string | null = null,
): void {
  navigate({
    diff: objectId,
    diffHash: contentHash || null,
    focus: focusId || null,
  });
}

/** Open the diff for the object a revision captured, with its artifact content hash. */
export function openRevisionDiff(
  revisionId: string,
  focusId: string | null = null,
): void {
  const objectId = objectIdForRevision(revisionId);
  if (objectId)
    openDiff(objectId, focusId, objectArtifactHashForRevision(revisionId));
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

/**
 * Reconcile the diff modal DOM with `state.diff`/`state.focus`. Part of the render
 * path (the store subscriber calls it): it both opens (user action, deep link,
 * Back/Forward) and closes. Returns the in-flight fetch so a caller can await the
 * paint; render ignores the return.
 */
export function renderDiffOverlay(): Promise<void> {
  const state = getState();
  if (!state.diff) {
    close("diff");
    shownDiffObject = null;
    shownDiffHash = null;
    diffCtx = null;
    return Promise.resolve();
  }
  if (state.diff === shownDiffObject && state.diffHash === shownDiffHash) {
    // Re-show only if the diff is not already the active overlay, so an unrelated
    // repaint while the diff is open never re-steals focus to the close button.
    if (activeName() !== "diff") open("diff", "#diff-close");
    applyDiffFocus();
    return Promise.resolve();
  }
  shownDiffObject = state.diff;
  shownDiffHash = state.diffHash;
  const objectId = state.diff;
  const contentHash = state.diffHash;
  const revisionId = revisionIdForObject(objectId, contentHash);
  const label = revisionId ? shortId(revisionId) : "";
  const title = $("#diff-title");
  if (title)
    title.textContent = label
      ? `${label} · snapshot ${shortId(objectId)}`
      : shortId(objectId);
  const body = $("#diff-body");
  if (body) body.innerHTML = `<p class="${CLASS.empty}">loading snapshot…</p>`;
  const nav = $("#diff-nav");
  if (nav) nav.innerHTML = "";
  // Opening through the manager tears down any prior overlay (palette/help) with
  // no focus restore — the indirection that replaces the served explicit closes.
  open("diff", "#diff-close");
  // The object endpoint is object-scoped (no revision id on the wire); the revision
  // id is recovered from the revisions list for annotation lookup.
  let objectUrl = `/api/object?id=${encodeURIComponent(objectId)}`;
  if (contentHash)
    objectUrl += `&contentHash=${encodeURIComponent(contentHash)}`;
  return fetchJSON(objectUrl)
    .then((artifact) => {
      // A later overlay change may have superseded this fetch.
      if (state.diff !== objectId || state.diffHash !== contentHash) return;
      const annotations = revisionId ? annotationsForRevision(revisionId) : [];
      const { html, ctx } = renderDiff(
        objectId,
        artifact as DiffArtifact,
        annotations,
      );
      const liveBody = $("#diff-body");
      if (liveBody) liveBody.innerHTML = html;
      diffCtx = ctx;
      diffFactCursor = -1;
      diffChangeCursor = -1;
      diffNavFilter = "all";
      const liveNav = $("#diff-nav");
      if (liveNav) liveNav.innerHTML = renderDiffNav();
      applyDiffFocus();
    })
    .catch((err: unknown) => {
      if (state.diff !== objectId || state.diffHash !== contentHash) return;
      const liveBody = $("#diff-body");
      if (liveBody)
        liveBody.innerHTML = `<p class="${CLASS.empty}">error: ${escapeHtml(
          err instanceof Error ? err.message : String(err),
        )}</p>`;
    });
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
  const body = $("#diff-body");
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
  const { files, anchored, unanchored, filePaths } = diffCtx;
  const visibleFiles = files
    .map((f, i) => ({ f, i, factCount: fileFactCount(f, anchored) }))
    .filter((item) => {
      if (diffNavFilter === "with-facts") return item.factCount > 0;
      if (diffNavFilter === "unanchored") return false;
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
    renderDiffNavSummary(diffNavSummary()) +
    renderDiffNavFilters(diffNavFilter);
  if (diffNavFilter !== "unanchored") {
    html += `<ol class="${CLASS.diffNavFiles}">${fileItems}</ol>`;
  }
  if (unanchored.length && diffNavFilter !== "with-facts") {
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

/** Set the navigator's file/fact filter and re-render the nav (no route state). */
export function setDiffNavFilter(filter: string): void {
  if (!isDiffNavFilter(filter)) return;
  diffNavFilter = filter;
  const nav = $("#diff-nav");
  if (nav) nav.innerHTML = renderDiffNav();
}

// ---------------------------------------------------------------------------
// Jump keys (next/prev fact, next/prev change)
// ---------------------------------------------------------------------------

// All rendered fact anchors in document order (inline annotations + unanchored
// bodies) — the ordering n/p cycles through.
function diffFactTargets(): HTMLElement[] {
  return Array.from(
    $("#diff-body")?.querySelectorAll<HTMLElement>(".anno[data-anno]") ?? [],
  );
}

// All change anchors (hunk headers) in rendered file bodies — the ordering ]/[
// cycles through.
function diffChangeTargets(): HTMLElement[] {
  return Array.from(
    $("#diff-body")?.querySelectorAll<HTMLElement>(".dhunk") ?? [],
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

/**
 * Wire the diff overlay's fixed-id controls and register its teardown with the
 * overlay manager. The delegated #diff-body / #diff-nav listeners read the
 * module-local `diffCtx`; they are installed once here, never at the open call site.
 */
export function initControls(): void {
  const modal = $<HTMLElement>("#diff-modal");
  if (modal) register("diff", { node: modal, onClose: closeDiff });
  $("#diff-close")?.addEventListener("click", () => closeDiff());
  modal?.addEventListener("click", (ev) => {
    if (ev.target === modal) closeDiff();
  });
  // A file header toggles its section; a render-all button hydrates a fact-vicinity
  // body; an annotated row's gutter scrolls to its annotation. Typed HTMLElement so
  // the keydown listener narrows to KeyboardEvent.
  const body = $<HTMLElement>("#diff-body");
  body?.addEventListener("click", (ev) => {
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
  });
  body?.addEventListener("keydown", (ev) => {
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
  });
  // The navigator sidebar: a filter button re-renders the nav; a file entry
  // expands + scrolls its section; an unanchored-fact entry scrolls to its body.
  const nav = $("#diff-nav");
  nav?.addEventListener("click", (ev) => {
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
      const section = $("#diff-body")?.querySelector<HTMLElement>(
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
  });
}
