// Pure HTML escaping. Ported from the served app.js `escapeHtml`.

const ENTITIES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

/** Escape the five HTML-significant characters, coercing non-strings via String(). */
export function escapeHtml(value: unknown): string {
  return String(value).replace(/[&<>"']/g, (char) => ENTITIES[char]);
}
