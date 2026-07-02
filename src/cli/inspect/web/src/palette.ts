// The command palette: one searchable overlay unifying jump-to-entity + actions.
// Ported from the served app.js palette cluster (`buildCommands` / `openPalette` /
// `closePalette` / `togglePalette` / `filterPalette` / `renderPalette` /
// `movePaletteActive` / `runPaletteActive` / `copyCurrentViewLink` + the contextual
// command helpers + the `cmd*` globals), in the revision vocabulary.
//
// It opens and tears down through the overlay manager (`register("palette", …)` +
// `open("palette")`) and imports no sibling overlay — the served `if (state.diff)
// closeDiff()` / `closeKeyHelp()` calls are dropped, because `overlay.open` already
// tears down whatever overlay is active (the diff↔palette↔help cycle cut). Every
// command navigates via the router or runs a read/copy action — none is operative
// or gating. The `cmd*` view state stays module-local; commands never call render
// (the store subscriber repaints on commit).

import { CLASS, cmdItemClass } from "./classNames";
import { openDiff } from "./diff/controller";
import { $ } from "./dom";
import { escapeHtml } from "./escape";
import { presentTypes, revisionForId } from "./model";
import {
  close as closeOverlay,
  type OverlayCloseOptions,
  open as openOverlay,
  register,
} from "./overlay";
import {
  assessmentLabel,
  attentionTokens,
  entryRevisionId,
  entryTitle,
  entryTrack,
  type Revision,
} from "./projection";
import { shortId, shortRef } from "./refs";
import { navigate, serializeState } from "./router";
import { getState } from "./store";
import { typeLabel } from "./types";

/** One palette command: its group, label, hint, action, and assigned DOM index. */
interface Command {
  kind: string;
  label: string;
  hint: string;
  run: () => void;
  domIndex?: number;
}

// The built commands, the filtered view, and the active cursor — transient view
// state, never on the store.
let cmdItems: Command[] = [];
let cmdFiltered: Command[] = [];
let cmdActive = 0;

// ---------------------------------------------------------------------------
// Copy actions
// ---------------------------------------------------------------------------

function copyText(text: string): void {
  const clip = navigator.clipboard;
  if (clip?.writeText) void clip.writeText(text);
}

/** Copy the absolute, canonical link for the current view to the clipboard. */
export function copyCurrentViewLink(): void {
  copyText(
    location.origin +
      location.pathname +
      serializeState(getState(), presentTypes()),
  );
}

// ---------------------------------------------------------------------------
// Command construction
// ---------------------------------------------------------------------------

function assignCommandOptionIds(cmds: Command[]): Command[] {
  cmds.forEach((cmd, index) => {
    cmd.domIndex = index;
  });
  return cmds;
}

function selectedRevisionId(): string {
  const sel = getState().selected;
  if (sel.kind === "revision") return sel.id ?? "";
  if (sel.kind === "event") {
    const event = (getState().history?.entries ?? []).find(
      (e) => e.eventId === sel.id,
    );
    return event ? entryRevisionId(event) : "";
  }
  return "";
}

function revisionCommandLabel(u: Revision): string {
  const targetDisplay = u.targetDisplay ?? {};
  const overview = u.overview ?? {};
  const current = overview.currentAssessment ?? {};
  const target = targetDisplay.label || shortId(u.revisionId);
  const assessment = current.assessment
    ? assessmentLabel(current.assessment)
    : current.status || "unassessed";
  return `${target} · ${assessment} · ${shortId(u.revisionId)}`;
}

function revisionCommandHint(u: Revision): string {
  const overview = u.overview ?? {};
  const cues = attentionTokens(overview).map((cue) => cue.label);
  const latest = overview.latestActivity?.title;
  return [cues.join(", ") || "review context", latest, shortId(u.snapshotId)]
    .filter(Boolean)
    .join(" · ");
}

function currentSelectionCommand(): Command | null {
  const sel = getState().selected;
  if (!sel.id) return null;
  if (sel.kind === "revision") {
    const unit = revisionForId(sel.id);
    return {
      kind: "Current",
      label: "Open current selection",
      hint: unit ? revisionCommandLabel(unit) : shortRef(sel.id),
      run: () =>
        navigate({
          selected: { kind: "revision", id: sel.id },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    };
  }
  if (sel.kind === "event") {
    const event = (getState().history?.entries ?? []).find(
      (e) => e.eventId === sel.id,
    );
    return {
      kind: "Current",
      label: "Open current selection",
      hint: event ? entryTitle(event) : shortRef(sel.id),
      run: () =>
        navigate({
          selected: { kind: "event", id: sel.id },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    };
  }
  return null;
}

function sortedRevisionEntriesForCommands(): Revision[] {
  const selectedRevision = selectedRevisionId();
  return [...(getState().revisions?.entries ?? [])].sort((left, right) => {
    if (left.revisionId === selectedRevision) return -1;
    if (right.revisionId === selectedRevision) return 1;
    return (
      String(right.capturedAt || "").localeCompare(
        String(left.capturedAt || ""),
      ) || String(right.revisionId).localeCompare(String(left.revisionId))
    );
  });
}

// The candidate commands, built over the loaded state: actions, the current
// selection, then contextual revision / object / track / event jumps.
function buildCommands(): Command[] {
  const cmds: Command[] = [];
  const state = getState();
  cmds.push({
    kind: "Actions",
    label: "Copy current view link",
    hint: "share",
    run: copyCurrentViewLink,
  });
  cmds.push({
    kind: "Actions",
    label: "Clear filters",
    hint: "filters",
    run: () =>
      navigate(
        {
          filterText: "",
          filterTrack: "",
          filterSnapshot: "",
          enabledTypes: new Set(presentTypes()),
        },
        { replace: true },
      ),
  });
  cmds.push({
    kind: "Actions",
    label: "Switch to timeline lens",
    hint: "lens",
    run: () => navigate({ lens: "timeline" }),
  });
  cmds.push({
    kind: "Actions",
    label: "Switch to list lens",
    hint: "lens",
    run: () => navigate({ lens: "list" }),
  });
  cmds.push({
    kind: "Actions",
    label: "Switch to threads lens",
    hint: "lens",
    run: () => navigate({ lens: "threads" }),
  });
  cmds.push({
    kind: "Actions",
    label: "Toggle timeline order",
    hint: "order",
    run: () =>
      navigate(
        { order: getState().order === "desc" ? "asc" : "desc" },
        { replace: true },
      ),
  });
  cmds.push({
    kind: "Actions",
    label: "Copy selected id",
    hint: "clipboard",
    run: () => {
      const id = getState().selected.id;
      if (id) copyText(id);
    },
  });

  const current = currentSelectionCommand();
  if (current) cmds.push(current);

  for (const u of sortedRevisionEntriesForCommands()) {
    cmds.push({
      kind: "Revisions",
      label: revisionCommandLabel(u),
      hint: revisionCommandHint(u),
      run: () =>
        navigate({
          selected: { kind: "revision", id: u.revisionId ?? "" },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    });
  }
  for (const o of [
    ...new Set(
      (state.revisions?.entries ?? [])
        .map((u) => u.snapshotId)
        .filter((x): x is string => Boolean(x)),
    ),
  ]) {
    cmds.push({
      kind: "Snapshots",
      label: shortRef(o),
      hint: "open diff",
      run: () => openDiff(o),
    });
  }
  for (const t of [
    ...new Set((state.history?.entries ?? []).map(entryTrack).filter(Boolean)),
  ].sort()) {
    cmds.push({
      kind: "Tracks",
      label: t,
      hint: "filter timeline",
      run: () => navigate({ lens: "timeline", filterTrack: t }),
    });
  }
  for (const e of state.history?.entries ?? []) {
    cmds.push({
      kind: "Events",
      label: entryTitle(e),
      hint: typeLabel(e.eventType),
      run: () =>
        navigate({
          selected: { kind: "event", id: e.eventId ?? "" },
          diff: null,
          diffHash: null,
          focus: null,
        }),
    });
  }
  return assignCommandOptionIds(cmds);
}

// ---------------------------------------------------------------------------
// Open / close / filter / render / move / run
// ---------------------------------------------------------------------------

/** Open the palette: rebuild the commands, clear the input, and show via the manager. */
export function open(): void {
  cmdItems = buildCommands();
  const input = $<HTMLInputElement>("#cmd-input");
  if (input) input.value = "";
  filterPalette("");
  openOverlay("palette", "#cmd-input");
}

/** Close the palette through the overlay manager. */
export function close(opts: OverlayCloseOptions = {}): void {
  closeOverlay("palette", opts);
}

/** Open the palette when closed, close it when open. */
export function toggle(): void {
  const palette = $("#cmd-palette");
  if (palette && !palette.classList.contains("hidden")) close();
  else open();
}

/** Narrow the command list to those matching the query and re-render. */
export function filterPalette(query: string): void {
  const needle = query.trim().toLowerCase();
  cmdFiltered = needle
    ? cmdItems.filter((c) =>
        `${c.label} ${c.hint || ""}`.toLowerCase().includes(needle),
      )
    : cmdItems.slice();
  cmdActive = 0;
  renderPalette();
}

function renderPalette(): void {
  const list = $("#cmd-results");
  const input = $("#cmd-input");
  if (!list || !input) return;
  if (!cmdFiltered.length) {
    list.innerHTML = `<li id="cmd-option-empty" class="${CLASS.cmdEmpty}" role="option" aria-disabled="true">No matches</li>`;
    input.setAttribute("aria-activedescendant", "cmd-option-empty");
    return;
  }
  let html = "";
  let lastKind: string | null = null;
  cmdFiltered.forEach((c, i) => {
    if (c.kind !== lastKind) {
      lastKind = c.kind;
      html += `<li class="${CLASS.cmdGroup}" role="presentation">${escapeHtml(c.kind)}</li>`;
    }
    html += `<li id="cmd-option-${escapeHtml(String(c.domIndex ?? i))}" class="${cmdItemClass(i === cmdActive)}" role="option" data-idx="${i}" aria-selected="${i === cmdActive}"><span class="${CLASS.cmdLabel}">${escapeHtml(c.label)}</span>${c.hint ? `<span class="${CLASS.cmdHint}">${escapeHtml(c.hint)}</span>` : ""}</li>`;
  });
  list.innerHTML = html;
  const active = list.querySelector<HTMLElement>(".cmd-item.active");
  if (active) {
    input.setAttribute("aria-activedescendant", active.id);
    active.scrollIntoView({ block: "nearest" });
  }
}

/** Step the active option by `delta`, wrapping, and re-render. */
export function move(delta: number): void {
  if (!cmdFiltered.length) return;
  cmdActive = (cmdActive + delta + cmdFiltered.length) % cmdFiltered.length;
  renderPalette();
}

/** Run the active command (closing the palette first). */
export function run(): void {
  const cmd = cmdFiltered[cmdActive];
  close();
  if (cmd) cmd.run();
}

// ---------------------------------------------------------------------------
// Fixed-id controls (wired once by the composition root)
// ---------------------------------------------------------------------------

/**
 * Register the palette with the overlay manager and wire its fixed-id controls:
 * the input filters and arrow/enter drive the active option; a backdrop click or a
 * result click runs/closes. Installed once by the composition root.
 */
export function initControls(): void {
  const node = $<HTMLElement>("#cmd-palette");
  if (node)
    register("palette", {
      node,
      onClose: () => {
        cmdActive = 0;
      },
    });
  const input = $<HTMLInputElement>("#cmd-input");
  input?.addEventListener("input", () => filterPalette(input.value));
  input?.addEventListener("keydown", (ev) => {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      move(1);
    } else if (ev.key === "ArrowUp") {
      ev.preventDefault();
      move(-1);
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      run();
    }
  });
  node?.addEventListener("click", (ev) => {
    const t = ev.target;
    if (!(t instanceof Element)) return;
    if (t === node) {
      close();
      return;
    }
    const item = t.closest<HTMLElement>(".cmd-item");
    if (item) {
      cmdActive = Number(item.dataset.idx);
      run();
    }
  });
}
