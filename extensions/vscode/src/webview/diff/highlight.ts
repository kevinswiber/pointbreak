// Pure, fail-safe attributed-segment emit for a diff row. Slices the raw row text
// by UTF-16 offsets (matching the wire span offsets), escapes each segment, and
// wraps each segment in the classes of the channels covering it: syntax token kind
// (`tok tok-<kind>`) and/or intraline emphasis (`emph`). A leaf module: no DOM, no
// state.

import { CLASS, tokClass } from "./classNames";
import { escapeHtml } from "./escape";

/** A token span over a row's text. `start`/`end` are UTF-16 code-unit offsets. */
export interface TokenSpan {
  start: number;
  end: number;
  kind: string;
}

/**
 * An intraline emphasis span over a row's text (UTF-16 offsets). No `kind` — emphasis
 * is a boolean decoration channel.
 */
export interface EmphSpan {
  start: number;
  end: number;
}

/**
 * A channel is valid when its spans are integer, sorted, non-overlapping, in range.
 * Typed to the start/end shape both `TokenSpan` and `EmphSpan` share.
 */
function validChannel(
  spans: Array<{ start: number; end: number }>,
  len: number,
): boolean {
  let cursor = 0;
  for (const span of spans) {
    if (
      !Number.isInteger(span.start) ||
      !Number.isInteger(span.end) ||
      span.start < cursor ||
      span.end < span.start ||
      span.end > len
    ) {
      return false;
    }
    cursor = span.end;
  }
  return true;
}

/**
 * The class attribute for a segment covered by the given channels, or null when the
 * segment is plain (both channels absent) so the caller emits bare escaped text.
 */
function segClass(kind: string | undefined, isEmph: boolean): string | null {
  const parts = [
    kind ? tokClass(kind) : null,
    isEmph ? CLASS.emph : null,
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(" ") : null;
}

/**
 * Render a row's text with syntax tokens and intraline emphasis. With neither channel
 * present (or both malformed) this returns exactly `escapeHtml(text)`, so an
 * unhighlighted, unemphasized row is byte-identical to the plain renderer.
 *
 * The emit is a left-to-right sweep over the union of both channels' segment
 * boundaries: each minimal segment is escaped once and wrapped in the classes of the
 * channels covering it (a token-covered segment gets `tok tok-<kind>`, an
 * emphasis-covered segment additionally gets `emph`, gaps stay plain). Each channel is
 * validated independently, so a malformed emphasis set drops emphasis only, and a
 * malformed token set drops tokens only.
 */
export function highlightRowText(
  text: string,
  tokens?: TokenSpan[],
  emphasis?: EmphSpan[],
): string {
  const toks = tokens && validChannel(tokens, text.length) ? tokens : [];
  const emph = emphasis && validChannel(emphasis, text.length) ? emphasis : [];
  if (toks.length === 0 && emph.length === 0) return escapeHtml(text);

  // The union of both channels' boundaries, plus the row endpoints, deduped and sorted.
  const points = [
    ...new Set([
      0,
      text.length,
      ...toks.flatMap((t) => [t.start, t.end]),
      ...emph.flatMap((e) => [e.start, e.end]),
    ]),
  ].sort((a, b) => a - b);

  let out = "";
  for (let i = 0; i + 1 < points.length; i++) {
    const a = points[i];
    const b = points[i + 1];
    if (a >= b) continue;
    const seg = escapeHtml(text.slice(a, b));
    const kind = toks.find((t) => t.start <= a && a < t.end)?.kind;
    const isEmph = emph.some((e) => e.start <= a && a < e.end);
    const cls = segClass(kind, isEmph);
    out += cls ? `<span class="${cls}">${seg}</span>` : seg;
  }
  return out;
}
