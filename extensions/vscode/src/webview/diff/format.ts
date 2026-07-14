// Pure time formatting. Ported from the served app.js `parseMs`/`fmtTime`/`fmtDateTime`.

const RFC3339_UTC =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?Z$/;

function parseRfc3339UtcMillis(value: string): number | null {
  const match = value.match(RFC3339_UTC);
  if (!match) return null;

  const [
    ,
    yearText,
    monthText,
    dayText,
    hourText,
    minuteText,
    secondText,
    fraction,
  ] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  const leapYear = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const daysInMonth = [
    31,
    leapYear ? 29 : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];

  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > daysInMonth[month - 1] ||
    hour > 23 ||
    minute > 59 ||
    second > 60
  ) {
    return null;
  }

  const millis = Number((fraction ?? "").padEnd(3, "0").slice(0, 3));
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute, Math.min(second, 59), millis);
  return date.getTime() + (second === 60 ? 1000 : 0);
}

/** Parse either legal `occurredAt` form to epoch milliseconds, or return null. */
export function parseMs(occurredAt: unknown): number | null {
  if (typeof occurredAt !== "string") return null;
  if (occurredAt.startsWith("unix-ms:")) {
    const unixMillis = occurredAt.match(/^unix-ms:([+-]?\d+)$/);
    return unixMillis ? Number(unixMillis[1]) : null;
  }
  if (/^\d{4}-\d{2}-\d{2}T/.test(occurredAt))
    return parseRfc3339UtcMillis(occurredAt);
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
