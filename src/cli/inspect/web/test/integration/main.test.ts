import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mountInspectorDom, resetDom } from "../support/dom";
import { installFetchMock, uninstallFetchMock } from "../support/fetch";

// End-to-end composition tests: drive the real `main()` bootstrap over the fixtures
// and exercise the wired interactions through the delegates, asserting the painted
// DOM / route end-to-end. `main` is the only place `subscribe(render)` is called and
// the only place the two document delegates + the bootstrap tail live. The modules
// are singletons, so reset + re-import before each test.
type Store = typeof import("../../src/store");
type Main = typeof import("../../src/main");
let store: Store;
let main: Main;

const REV =
  "rev:sha256:9a7626ca7cb2801721ed992402184460210477aadfd4f7228628b65ff11a6efd";

beforeEach(async () => {
  vi.resetModules();
  store = await import("../../src/store");
  main = await import("../../src/main");
  mountInspectorDom();
  installFetchMock();
  history.replaceState(null, "", "/");
  // Apply a stored theme so the prefs-before-paint step is observable.
  localStorage.setItem("shore-inspect-theme", "light");
});

afterEach(() => {
  uninstallFetchMock();
  localStorage.clear();
  resetDom();
});

describe("first paint + bootstrap tail", () => {
  it("applies prefs, loads, and paints the master/detail from the routed state", async () => {
    await main.main();
    // Prefs applied (before first paint).
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    // Loaded counts + the timeline lens painted from the default route.
    expect(document.querySelector("#stat-events")?.textContent).toBe(
      "8 events",
    );
    expect(document.querySelector("#master #timeline")).not.toBeNull();
    expect(
      (document.querySelectorAll("#master #timeline .event").length ?? 0) > 0,
    ).toBe(true);
    // The bootstrap tail flipped the refresh status.
    expect(document.querySelector("#refresh")?.textContent).toBe("watching");
  });
});

describe("the single subscriber repaints on every commit", () => {
  it("a later commit repaints without a second registration", async () => {
    await main.main();
    store.commit({ lens: "list" });
    expect(document.querySelector("#master #units .unit-card")).not.toBeNull();
    store.commit({ lens: "threads" });
    expect(
      document.querySelector("#master #revisions .thread-card"),
    ).not.toBeNull();
  });
});

describe("wired interactions drive the DOM/route through the delegates", () => {
  it("a lens-tab click switches the master lens", async () => {
    await main.main();
    document
      .querySelector<HTMLElement>('.lens-tab[data-lens="list"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().lens).toBe("list");
    expect(document.querySelector("#master #units")).not.toBeNull();
  });

  it("a #filter-types toggle click narrows the enabled types", async () => {
    await main.main();
    const before = store
      .getState()
      .enabledTypes.has("review_observation_recorded");
    expect(before).toBe(true);
    document
      .querySelector<HTMLElement>('[data-type="review_observation_recorded"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      store.getState().enabledTypes.has("review_observation_recorded"),
    ).toBe(false);
  });

  it("a timeline-row click selects the event and paints the detail", async () => {
    await main.main();
    const row = document.querySelector<HTMLElement>(
      "#master #timeline .event[data-event-id]",
    );
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().selected.kind).toBe("event");
    expect(document.querySelector("#detail dl.kv")).not.toBeNull();
  });

  it("a ref-chip click anywhere routes through the document delegate", async () => {
    await main.main();
    const detail = document.querySelector("#detail");
    if (detail)
      detail.innerHTML = `<span class="ref" role="link" data-ref-kind="rev" data-ref-id="${REV}">chip</span>`;
    document
      .querySelector<HTMLElement>("[data-ref-kind]")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().selected).toEqual({ kind: "revision", id: REV });
  });

  it("Cmd-K opens the palette and a run executes a command", async () => {
    await main.main();
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true }),
    );
    expect(
      document.querySelector("#cmd-palette")?.classList.contains("hidden"),
    ).toBe(false);
    const input = document.querySelector<HTMLInputElement>("#cmd-input");
    if (input) {
      input.value = "Switch to list lens";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    }
    expect(store.getState().lens).toBe("list");
  });

  it("keyboard stepping selects through the active lens", async () => {
    await main.main();
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "j", bubbles: true }),
    );
    expect(store.getState().selected.kind).toBe("event");
  });

  it("opening a diff from a list card then closing reconciles the modal", async () => {
    await main.main();
    store.commit({ lens: "list" });
    const diffBtn = document.querySelector<HTMLElement>(
      "#master [data-open-diff]",
    );
    diffBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().diff).not.toBeNull();
    // The render reconciler opened the modal.
    await Promise.resolve();
    expect(
      document.querySelector("#diff-modal")?.classList.contains("hidden"),
    ).toBe(false);
    document
      .querySelector("#diff-close")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().diff).toBeNull();
  });

  it("a hashchange re-applies the route", async () => {
    await main.main();
    history.replaceState(null, "", "#/list");
    window.dispatchEvent(new Event("hashchange"));
    expect(store.getState().lens).toBe("list");
    expect(document.querySelector("#master #units")).not.toBeNull();
  });
});

describe("advisory framing (rendered DOM, reader-relative, never a gate)", () => {
  it("surfaces the read-only / advisory posture and exposes no gate affordance", async () => {
    await main.main();
    // The advisory posture now lives as a footnote in the store-identity popover
    // (the persistent "read-only · advisory" badge was retired for a quieter bar).
    const note = document.querySelector(".store-identity-note");
    expect(note?.textContent).toContain("never gates writes");
    expect(note?.textContent).toContain("reader-relative");
    // No approve / merge / gate control anywhere in the rendered chrome.
    const buttons = Array.from(document.querySelectorAll("button")).map(
      (b) => b.textContent ?? "",
    );
    expect(buttons.some((t) => /\b(approve|merge|gate)\b/i.test(t))).toBe(
      false,
    );
  });

  it("the detail readback frames verification as reader-relative", async () => {
    await main.main();
    const event = store.getState().history?.entries?.[2];
    store.commit({
      selected: { kind: "event", id: event?.eventId ?? null },
      open: true,
    });
    expect(document.querySelector("#detail")?.textContent).toContain(
      "reader-relative",
    );
  });
});
