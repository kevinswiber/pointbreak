// Pure time formatting. Ported from the served app.js `parseMs`/`fmtTime`/`fmtDateTime`.

/** Extract the trailing millisecond count from an `occurredAt` token, or null. */
export function parseMs(occurredAt: unknown): number | null {
  if (typeof occurredAt !== "string") return null;
  const match = occurredAt.match(/(\d+)\s*$/);
  return match ? Number(match[1]) : null;
}

/** Render `HH:MM:SS.mmm` (24-hour), or the original string when it has no timestamp. */
export function fmtTime(occurredAt: string): string {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  const date = new Date(ms);
  return `${date.toLocaleTimeString([], { hour12: false })}.${String(ms % 1000).padStart(3, "0")}`;
}

/** Render a locale date-time (24-hour), or the original string when it has no timestamp. */
export function fmtDateTime(occurredAt: string): string {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  return new Date(ms).toLocaleString([], { hour12: false });
}

/** Render a compact locale date (no time), or the original string when it has no timestamp. */
export function fmtDate(occurredAt: string): string {
  const ms = parseMs(occurredAt);
  if (ms == null) return occurredAt || "";
  return new Date(ms).toLocaleDateString();
}
