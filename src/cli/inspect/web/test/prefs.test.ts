import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  applyDensity,
  applyPrefs,
  applySplit,
  applyTheme,
  cycleTheme,
  initControls,
  preferredSplit,
  preferredTheme,
  toggleDensity,
  watchColorScheme,
} from "../src/prefs";
import { mountInspectorDom, resetDom } from "./support/dom";

// The persisted storage keys (the reader-local preference contract; mirrors app.js).
const THEME_KEY = "shore-inspect-theme";
const DENSITY_KEY = "shore-inspect-density";
const SPLIT_KEY = "shore-inspect-split";

const realMatchMedia = window.matchMedia;

function fakeMediaQueryList(matches: boolean, media: string): MediaQueryList {
  return {
    matches,
    media,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  };
}

/** Make `prefers-color-scheme: light` resolve deterministically. */
function stubPrefersLight(prefersLight: boolean): void {
  window.matchMedia = (query: string) =>
    fakeMediaQueryList(prefersLight && query.includes("light"), query);
}

/** A matchMedia stub whose OS preference can flip live, firing registered `change` handlers. */
function stubControllableColorScheme(initialPrefersLight: boolean): {
  setPrefersLight(next: boolean): void;
} {
  let prefersLight = initialPrefersLight;
  const handlers: Array<(e: MediaQueryListEvent) => void> = [];
  window.matchMedia = (query: string): MediaQueryList => {
    const isLightQuery = query.includes("light");
    return {
      get matches() {
        return isLightQuery ? prefersLight : !prefersLight;
      },
      media: query,
      onchange: null,
      addEventListener: (
        _type: string,
        cb: EventListenerOrEventListenerObject,
      ) => {
        handlers.push(cb as (e: MediaQueryListEvent) => void);
      },
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    } as MediaQueryList;
  };
  return {
    setPrefersLight(next: boolean): void {
      prefersLight = next;
      for (const cb of handlers) cb({ matches: next } as MediaQueryListEvent);
    },
  };
}

beforeEach(() => {
  mountInspectorDom();
  localStorage.clear();
  stubPrefersLight(false);
});

afterEach(() => {
  resetDom();
  localStorage.clear();
  window.matchMedia = realMatchMedia;
});

describe("preferredTheme", () => {
  it("returns the stored theme when it is light or dark", () => {
    localStorage.setItem(THEME_KEY, "light");
    expect(preferredTheme()).toBe("light");
    localStorage.setItem(THEME_KEY, "dark");
    expect(preferredTheme()).toBe("dark");
  });

  it("falls back to the OS color-scheme preference when unset", () => {
    stubPrefersLight(true);
    expect(preferredTheme()).toBe("light");
    stubPrefersLight(false);
    expect(preferredTheme()).toBe("dark");
  });

  it("ignores a junk stored value and uses the OS preference", () => {
    localStorage.setItem(THEME_KEY, "neon");
    stubPrefersLight(true);
    expect(preferredTheme()).toBe("light");
  });
});

describe("applyTheme / cycleTheme", () => {
  const themeBtn = () => document.getElementById("theme-toggle");

  it("applyTheme sets data-theme on the document root", () => {
    applyTheme("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("labels a pinned theme with its glyph + value (text visible, aria clean)", () => {
    localStorage.setItem(THEME_KEY, "dark");
    applyTheme("dark");
    expect(themeBtn()?.textContent).toBe("☾ dark");
    expect(themeBtn()?.getAttribute("aria-label")).toBe("Color theme: dark");
    localStorage.setItem(THEME_KEY, "light");
    applyTheme("light");
    expect(themeBtn()?.textContent).toBe("☼ light");
    expect(themeBtn()?.getAttribute("aria-label")).toBe("Color theme: light");
  });

  it("labels system mode as ◐ <resolved>, spelling out the mode only in aria", () => {
    // Unset key ⇒ system mode; applyTheme is passed the resolved theme.
    applyTheme("dark");
    expect(themeBtn()?.textContent).toBe("◐ dark");
    expect(themeBtn()?.getAttribute("aria-label")).toBe(
      "Color theme: system (dark)",
    );
  });

  it("cycleTheme advances system → light → dark → system, persisting each mode", () => {
    stubPrefersLight(false); // system resolves to dark
    applyPrefs();
    expect(themeBtn()?.textContent).toBe("◐ dark");

    cycleTheme();
    expect(localStorage.getItem(THEME_KEY)).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(themeBtn()?.textContent).toBe("☼ light");

    cycleTheme();
    expect(localStorage.getItem(THEME_KEY)).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(themeBtn()?.textContent).toBe("☾ dark");

    cycleTheme();
    expect(localStorage.getItem(THEME_KEY)).toBe("system");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(themeBtn()?.textContent).toBe("◐ dark");
  });

  it("cycleTheme back to system restores live OS-following", () => {
    localStorage.setItem(THEME_KEY, "dark"); // pinned dark
    applyPrefs();
    cycleTheme(); // dark → system
    expect(localStorage.getItem(THEME_KEY)).toBe("system");
    expect(preferredTheme()).toBe("dark"); // now resolves via the OS stub
  });
});

describe("applyDensity / toggleDensity", () => {
  it("applyDensity toggles the compact class on the root", () => {
    applyDensity("compact");
    expect(document.documentElement.classList.contains("compact")).toBe(true);
    applyDensity("comfortable");
    expect(document.documentElement.classList.contains("compact")).toBe(false);
  });

  it("applyDensity labels the #density-toggle with its glyph + value (text + aria)", () => {
    applyDensity("compact");
    const btn = document.getElementById("density-toggle");
    expect(btn?.textContent).toBe("≡ compact");
    expect(btn?.getAttribute("aria-label")).toBe("Density: compact");
    applyDensity("comfortable");
    expect(btn?.textContent).toBe("≡ comfortable");
    expect(btn?.getAttribute("aria-label")).toBe("Density: comfortable");
  });

  it("toggleDensity flips compact<->comfortable and persists the choice", () => {
    toggleDensity();
    expect(document.documentElement.classList.contains("compact")).toBe(true);
    expect(localStorage.getItem(DENSITY_KEY)).toBe("compact");
    toggleDensity();
    expect(document.documentElement.classList.contains("compact")).toBe(false);
    expect(localStorage.getItem(DENSITY_KEY)).toBe("comfortable");
  });
});

describe("applyPrefs", () => {
  it("applies the stored theme and density (the before-first-paint step)", () => {
    localStorage.setItem(THEME_KEY, "light");
    localStorage.setItem(DENSITY_KEY, "compact");
    applyPrefs();
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(document.documentElement.classList.contains("compact")).toBe(true);
  });

  it("defaults density to comfortable when unset", () => {
    applyPrefs();
    expect(document.documentElement.classList.contains("compact")).toBe(false);
  });

  it("seeds the control labels from the stored prefs at first paint", () => {
    localStorage.setItem(THEME_KEY, "light");
    localStorage.setItem(DENSITY_KEY, "compact");
    applyPrefs();
    expect(document.getElementById("theme-toggle")?.textContent).toBe(
      "☼ light",
    );
    expect(document.getElementById("density-toggle")?.textContent).toBe(
      "≡ compact",
    );
  });
});

describe("preferredSplit / applySplit (the divider width pref)", () => {
  it("applyPrefs sets --split-master from the stored width", () => {
    localStorage.setItem(SPLIT_KEY, "62");
    applyPrefs();
    expect(
      document.documentElement.style.getPropertyValue("--split-master"),
    ).toBe("62%");
  });

  it("defaults to the 50/50 grid when the width pref is unset or out of range", () => {
    applyPrefs();
    expect(
      document.documentElement.style.getPropertyValue("--split-master"),
    ).toBe("");
    localStorage.setItem(SPLIT_KEY, "9000");
    expect(preferredSplit()).toBeNull();
    applyPrefs();
    expect(
      document.documentElement.style.getPropertyValue("--split-master"),
    ).toBe("");
  });

  it("applySplit persists and clamps; null clears the property and the key", () => {
    applySplit(62);
    expect(localStorage.getItem(SPLIT_KEY)).toBe("62");
    applySplit(99);
    expect(localStorage.getItem(SPLIT_KEY)).toBe("75");
    applySplit(null);
    expect(
      document.documentElement.style.getPropertyValue("--split-master"),
    ).toBe("");
    expect(localStorage.getItem(SPLIT_KEY)).toBeNull();
  });
});

describe("watchColorScheme", () => {
  it("re-applies the theme live when the OS preference flips and no theme is pinned", () => {
    const media = stubControllableColorScheme(false);
    applyPrefs();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    watchColorScheme();
    media.setPrefersLight(true);
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(document.getElementById("theme-toggle")?.textContent).toBe(
      "◐ light",
    );
    media.setPrefersLight(false);
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(document.getElementById("theme-toggle")?.textContent).toBe("◐ dark");
  });

  it("ignores OS changes once the reader has pinned an explicit theme", () => {
    const media = stubControllableColorScheme(false);
    localStorage.setItem(THEME_KEY, "dark");
    applyPrefs();
    watchColorScheme();
    media.setPrefersLight(true);
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });
});

describe("initControls", () => {
  it("wires the #theme-toggle (cycles the mode) and #density-toggle", () => {
    applyPrefs(); // system mode, OS dark ⇒ data-theme dark
    initControls();
    document.getElementById("theme-toggle")?.click(); // system → light
    expect(localStorage.getItem(THEME_KEY)).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    document.getElementById("density-toggle")?.click();
    expect(document.documentElement.classList.contains("compact")).toBe(true);
  });
});
