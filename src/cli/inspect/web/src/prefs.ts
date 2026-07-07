// Reader-local display preferences (theme / density / split width). Ported from
// the served app.js prefs cluster. These are reader-local choices, persisted in
// localStorage and never encoded in the URL/hash — they are not shareable view
// state, so they live off the store, applied straight to the document root as
// app.js does. The split width is an integer master-pane percent (25–75) applied
// as the `--split-master` custom property the `.split` grid reads; unset means
// the 50/50 default.
//
// Unlike app.js, the apply-on-load is NOT a top-level import side-effect: `main`
// calls `applyPrefs()` explicitly before the first paint.

import { $ } from "./dom";

const THEME_KEY = "shore-inspect-theme";
const DENSITY_KEY = "shore-inspect-density";
const SPLIT_KEY = "shore-inspect-split";

const SPLIT_MIN = 25;
const SPLIT_MAX = 75;

// Strong references to the MediaQueryLists we've attached `change` listeners to.
// WebKit/Safari garbage-collects a MediaQueryList that is reachable only through
// its own listener, which silently stops the listener from ever firing; holding a
// reference here keeps the query alive (Chromium/Firefox retain it regardless).
const liveMediaQueries: MediaQueryList[] = [];

// The three theme modes the toggle cycles through. `system` follows the OS color
// scheme live (see watchColorScheme); `light`/`dark` pin an explicit choice.
// Persisted under THEME_KEY — anything else (unset or junk) reads as `system`, so
// a fresh reader follows the OS.
type ThemeMode = "system" | "light" | "dark";

const THEME_CYCLE: Record<ThemeMode, ThemeMode> = {
  system: "light",
  light: "dark",
  dark: "system",
};

/** The reader's stored theme mode; unset or junk reads as `system` (follow the OS). */
function preferredThemeMode(): ThemeMode {
  const stored = localStorage.getItem(THEME_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

/** True when the reader has pinned light/dark, so the OS preference is ignored. */
function hasPinnedTheme(): boolean {
  return preferredThemeMode() !== "system";
}

/** The OS color-scheme preference, resolved to `light`/`dark`. */
function osTheme(): string {
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

/** The effective theme: the pinned `light`/`dark`, else the OS preference. */
export function preferredTheme(): string {
  const mode = preferredThemeMode();
  return mode === "system" ? osTheme() : mode;
}

// Glyphs prefixing each toggle's value. A leading monochrome mark carries the
// *dimension/mode* the word doesn't: for theme, ☼ = pinned light, ☾ = pinned
// dark, and ◐ = following the OS (auto) — so the glyph alone distinguishes a
// pinned choice from system-following, and the word is left to show the effective
// light/dark. For density, ≡ (rows) names a control whose values (comfortable /
// compact) don't self-announce as density. Kept text-presentation, matching the
// restrained `▾ ○ ✕` vocabulary.
const THEME_GLYPH: Record<ThemeMode, string> = {
  light: "☼",
  dark: "☾",
  system: "◐",
};
const DENSITY_GLYPH = "≡";

// Keep the topbar toggles state-legible: the visible label is a mode/dimension
// glyph plus the effective value (the value alone mirrors `#order-toggle`'s
// "newest first"). The glyph stays OUT of the accessible name — instead the
// aria-label carries `dimension: ariaValue`, where `ariaValue` spells out what
// the glyph shows visually (e.g. `system (dark)`), so a screen reader hears the
// mode in words and the name still contains the visible value (WCAG "Label in
// Name"). Every value change funnels through applyTheme/applyDensity, so syncing
// here covers first paint, the toggle click, and the live OS watcher alike. The
// lookup is defensive: the control may be absent (e.g. a headless mount).
function labelControl(
  id: string,
  dimension: string,
  glyph: string,
  value: string,
  ariaValue: string = value,
): void {
  const btn = $(`#${id}`);
  if (!btn) return;
  btn.textContent = `${glyph} ${value}`;
  btn.setAttribute("aria-label", `${dimension}: ${ariaValue}`);
}

/**
 * Apply a theme by setting `data-theme` on the document root and labeling the
 * toggle. The visible word is always the effective `light`/`dark`; the glyph
 * carries the mode, so `system` reads `◐ <resolved>` (the ◐ is the "following the
 * OS" cue and re-resolves live as the OS flips). The aria-label still says
 * `system (…)` so non-sighted readers get the mode the glyph conveys.
 */
export function applyTheme(theme: string): void {
  document.documentElement.setAttribute("data-theme", theme);
  const mode = preferredThemeMode();
  const ariaValue = mode === "system" ? `system (${theme})` : theme;
  labelControl(
    "theme-toggle",
    "Color theme",
    THEME_GLYPH[mode],
    theme,
    ariaValue,
  );
}

/** Advance the theme mode `system → light → dark → system`, persist it, and apply. */
export function cycleTheme(): void {
  const next = THEME_CYCLE[preferredThemeMode()];
  localStorage.setItem(THEME_KEY, next);
  applyTheme(preferredTheme());
}

/** The stored density, defaulting to `comfortable` when unset. */
function preferredDensity(): string {
  return localStorage.getItem(DENSITY_KEY) || "comfortable";
}

/** Apply a density by toggling the `compact` class on the document root and labeling the toggle. */
export function applyDensity(mode: string): void {
  const value = mode === "compact" ? "compact" : "comfortable";
  document.documentElement.classList.toggle("compact", value === "compact");
  labelControl("density-toggle", "Density", DENSITY_GLYPH, value);
}

/** Flip the density between `compact` and `comfortable`, persist, apply. */
export function toggleDensity(): void {
  const next = document.documentElement.classList.contains("compact")
    ? "comfortable"
    : "compact";
  localStorage.setItem(DENSITY_KEY, next);
  applyDensity(next);
}

/** The stored split width (integer master percent, 25–75), or null for the 50/50 default. */
export function preferredSplit(): number | null {
  const raw = localStorage.getItem(SPLIT_KEY);
  const n = raw === null ? Number.NaN : Number.parseInt(raw, 10);
  return Number.isInteger(n) && n >= SPLIT_MIN && n <= SPLIT_MAX ? n : null;
}

/**
 * Apply (and persist) a split width, clamped to the valid range so the stored
 * pref can never hold junk from our own writers; null clears back to the 50/50
 * default (property and key both removed). The divider controller is the only
 * post-paint caller — every width write goes through here.
 */
export function applySplit(pct: number | null): void {
  if (pct === null) {
    document.documentElement.style.removeProperty("--split-master");
    localStorage.removeItem(SPLIT_KEY);
    return;
  }
  const clamped = Math.round(Math.min(SPLIT_MAX, Math.max(SPLIT_MIN, pct)));
  document.documentElement.style.setProperty("--split-master", `${clamped}%`);
  localStorage.setItem(SPLIT_KEY, String(clamped));
}

/**
 * Apply the persisted theme + density + split width. `main` calls this before
 * the first paint so the chosen theme is in place immediately (reproduces
 * app.js's top-level `applyTheme(preferredTheme())` / `applyDensity(...)`, as an
 * explicit call). A stored-but-invalid split key is left untouched and simply
 * not applied (the default grid wins).
 */
export function applyPrefs(): void {
  applyTheme(preferredTheme());
  applyDensity(preferredDensity());
  const split = preferredSplit();
  if (split !== null) applySplit(split);
}

/**
 * Follow live OS color-scheme changes while the reader hasn't pinned a theme.
 * `applyPrefs` reads the OS preference once before first paint; without this,
 * a later system light/dark switch only takes effect on the next page load.
 * A pinned light/dark choice (via the toggle) still wins over the OS.
 *
 * The query is retained in `liveMediaQueries` so Safari doesn't garbage-collect
 * it out from under its own listener (see the note on that binding).
 */
export function watchColorScheme(): void {
  const query = window.matchMedia("(prefers-color-scheme: light)");
  liveMediaQueries.push(query);
  query.addEventListener("change", () => {
    if (hasPinnedTheme()) return;
    applyTheme(preferredTheme());
  });
}

/** Wire the `#theme-toggle` / `#density-toggle` buttons and the OS color-scheme watcher. */
export function initControls(): void {
  $("#theme-toggle")?.addEventListener("click", cycleTheme);
  $("#density-toggle")?.addEventListener("click", toggleDensity);
  watchColorScheme();
}
