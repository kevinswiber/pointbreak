import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import revisionsJson from "../fixtures/revisions.json";
import threadsJson from "../fixtures/threads.json";
import { mountInspectorDom, resetDom } from "../support/dom";
import {
  installFetchMock,
  resetHistoryResponse,
  setHistoryResponse,
  uninstallFetchMock,
} from "../support/fetch";

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

function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function deferredResponse(payload: unknown): {
  promise: Promise<Response>;
  resolve: () => void;
} {
  let resolve!: () => void;
  const promise = new Promise<Response>((done) => {
    resolve = () =>
      done(
        new Response(JSON.stringify(payload), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
  });
  return { promise, resolve };
}

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
  main.stopPolling();
  uninstallFetchMock();
  resetHistoryResponse();
  vi.restoreAllMocks();
  localStorage.clear();
  sessionStorage.clear();
  resetDom();
});

describe("first paint + bootstrap tail", () => {
  it("enters a clean document before routing or making API requests", async () => {
    const token = "bootstrap_secret_0123456789";
    history.replaceState(
      { safe: true },
      "",
      `/#/attention?types=observation&token=${token}`,
    );
    const inner = globalThis.fetch;
    const reload = vi.fn();
    let requests = 0;
    let cleanBeforeFetch = true;
    let authorized = true;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      requests += 1;
      cleanBeforeFetch &&= !location.href.includes(token);
      const headers = new Headers(init?.headers);
      authorized &&= headers.get("Authorization") === `Bearer ${token}`;
      return inner(input, init);
    }) as typeof fetch;
    try {
      await main.main({ reload });
      expect(reload).toHaveBeenCalledOnce();
      expect(requests).toBe(0);
      expect(location.hash).toBe("#/attention?types=observation");
      expect(location.href.includes(token)).toBe(false);
      expect(document.body.textContent?.includes(token)).toBe(false);
      expect(JSON.stringify(history.state).includes(token)).toBe(false);

      await main.main({ reload });
      expect(requests).toBeGreaterThan(0);
      expect(cleanBeforeFetch).toBe(true);
      expect(authorized).toBe(true);
      expect(reload).toHaveBeenCalledOnce();
      expect(location.hash).toBe("#/attention?types=observation");
      expect(location.href.includes(token)).toBe(false);
      expect(document.body.textContent?.includes(token)).toBe(false);
      expect(JSON.stringify(history.state).includes(token)).toBe(false);
    } finally {
      globalThis.fetch = inner;
    }
  });

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
    // The bootstrap tail flipped the liveness dot to watching.
    expect(document.querySelector("#refresh")?.getAttribute("data-state")).toBe(
      "watching",
    );
  });

  it("starts freshness polling after Retry recovers an initial outage", async () => {
    const interval = vi.spyOn(globalThis, "setInterval");
    const inner = globalThis.fetch;
    let unavailable = true;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      if (unavailable) return Promise.reject(new Error("offline"));
      return inner(input, init);
    }) as typeof fetch;
    try {
      await main.main();
      expect(interval).not.toHaveBeenCalled();
      expect(document.querySelector("#connection-action")?.textContent).toBe(
        "Retry",
      );

      unavailable = false;
      document.querySelector<HTMLButtonElement>("#connection-action")?.click();

      await vi.waitFor(() => {
        expect(
          document.querySelector("#refresh")?.getAttribute("data-state"),
        ).toBe("watching");
        expect(interval).toHaveBeenCalledTimes(1);
      });
      expect(interval).toHaveBeenCalledWith(expect.any(Function), 3000);
    } finally {
      globalThis.fetch = inner;
    }
  });

  it("paints the timeline before revisions and threads finish loading", async () => {
    const revisions = deferredResponse(revisionsJson);
    const threads = deferredResponse(threadsJson);
    const inner = globalThis.fetch;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = new URL(url, "http://inspector.test").pathname;
      if (path === "/api/revisions") return revisions.promise;
      if (path === "/api/threads") return threads.promise;
      return inner(input as RequestInfo, init);
    }) as typeof fetch;
    try {
      const boot = main.main();
      await flush();
      expect(document.querySelector("#master #timeline")).not.toBeNull();
      expect(
        (document.querySelectorAll("#master #timeline .event").length ?? 0) > 0,
      ).toBe(true);
      expect(document.querySelector("#stat-units")?.textContent).toBe(
        "— units",
      );

      revisions.resolve();
      threads.resolve();
      await boot;
      expect(document.querySelector("#stat-units")?.textContent).toBe(
        "1 units",
      );
    } finally {
      globalThis.fetch = inner;
    }
  });
});

describe("the single subscriber repaints on every commit", () => {
  it("a later commit repaints without a second registration", async () => {
    await main.main();
    store.commit({ lens: "list" });
    expect(document.querySelector("#master #units .unit-card")).not.toBeNull();
    store.commit({ lens: "attention" });
    expect(document.querySelector("#master #attention")).not.toBeNull();
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

  it("selecting a type row leaves the Filters panel open across its repaint", async () => {
    // The row click commits via navigate, whose subscribe(render) repaint
    // replaces the menu rows BEFORE the same click bubbles to the document
    // outside-click listener — the detached target must still classify as an
    // in-container click, or every selection slams the menu shut.
    await main.main();
    document
      .querySelector<HTMLElement>("#filters-toggle")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      document.querySelector("#filters-panel")?.classList.contains("hidden"),
    ).toBe(false);
    document
      .querySelector<HTMLElement>('[data-type="review_observation_recorded"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      store.getState().enabledTypes.has("review_observation_recorded"),
    ).toBe(false); // the toggle committed…
    expect(
      document.querySelector("#filters-panel")?.classList.contains("hidden"),
    ).toBe(false); // …and the panel stayed open for the next selection
  });

  it("the Filters panel's Escape closes it without reaching the global Escape ladder", async () => {
    // Only this harness can observe a leak: main() wires the document-level
    // onKey, whose Escape ladder would clear the query if the popover's
    // locally-scoped Escape ever propagated past #filter-types.
    await main.main();
    store.commit({ filterText: "keepme" });
    document
      .querySelector<HTMLElement>("#filters-toggle")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      document.querySelector("#filters-panel")?.classList.contains("hidden"),
    ).toBe(false);
    document
      .querySelector<HTMLElement>("#filter-controls")
      ?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    expect(
      document.querySelector("#filters-panel")?.classList.contains("hidden"),
    ).toBe(true);
    expect(store.getState().filterText).toBe("keepme");
  });

  it("keeps only one lightweight disclosure open and closes it on outside click", async () => {
    await main.main();
    const filters = document.querySelector<HTMLElement>("#filters-toggle");
    const view = document.querySelector<HTMLElement>("#view-toggle");
    filters?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      document.querySelector("#filters-panel")?.classList.contains("hidden"),
    ).toBe(false);
    view?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      document.querySelector("#filters-panel")?.classList.contains("hidden"),
    ).toBe(true);
    expect(
      document.querySelector("#view-panel")?.classList.contains("hidden"),
    ).toBe(false);
    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(
      document.querySelector("#view-panel")?.classList.contains("hidden"),
    ).toBe(true);
  });

  it("typing paints search suggestions; Escape dismisses without blurring or clearing the query", async () => {
    // Only this harness can observe a leak: main() wires the document-level
    // onKey, whose global Escape would blur the focused field (and a stray
    // clear-query rung would wipe filterText) if the suggestion handler's
    // locally-consumed Escape ever propagated past #filter-text.
    await main.main();
    const input = document.querySelector<HTMLInputElement>("#filter-text");
    if (!input) throw new Error("#filter-text missing");
    input.focus();
    input.value = "tra";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const list = document.querySelector("#filter-suggestions");
    expect(list?.classList.contains("hidden")).toBe(false);
    expect(list?.textContent).toContain("track:");
    expect(store.getState().filterText).toBe("tra");
    const escapeKeydown = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    const notCancelled = input.dispatchEvent(escapeKeydown);
    expect(list?.classList.contains("hidden")).toBe(true);
    expect(store.getState().filterText).toBe("tra"); // the query survived…
    expect(document.activeElement).toBe(input); // …and the field kept focus
    // input[type=search] has a NATIVE Escape default (clear the field) that
    // stopPropagation cannot suppress — the handler must cancel it, which
    // dispatchEvent reports by returning false.
    expect(notCancelled).toBe(false);
  });

  it("clicking a suggestion row inserts the completed clause from the payload's distinct values", async () => {
    await main.main();
    const input = document.querySelector<HTMLInputElement>("#filter-text");
    if (!input) throw new Error("#filter-text missing");
    input.focus();
    input.value = "track:cod";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const row = document.querySelector<HTMLElement>(
      '#filter-suggestions [data-index="0"]',
    );
    expect(row?.textContent).toBe("track:agent:codex");
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().filterText).toBe("track:agent:codex ");
    expect(input.value).toBe("track:agent:codex ");
    expect(
      document
        .querySelector("#filter-suggestions")
        ?.classList.contains("hidden"),
    ).toBe(true);
  });

  it("an outside click dismisses the suggestion popover", async () => {
    await main.main();
    const input = document.querySelector<HTMLInputElement>("#filter-text");
    if (!input) throw new Error("#filter-text missing");
    input.focus();
    input.value = "tra";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const list = document.querySelector("#filter-suggestions");
    expect(list?.classList.contains("hidden")).toBe(false);
    document
      .querySelector<HTMLElement>("#master")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(list?.classList.contains("hidden")).toBe(true);
  });

  it("ArrowDown + Enter accepts the highlighted suggestion without the global Enter running", async () => {
    // Enter on the timeline search input normally moves focus to the timeline
    // (onKey's "done searching" case) — a consumed suggestion-accept Enter
    // must not also do that.
    await main.main();
    const input = document.querySelector<HTMLInputElement>("#filter-text");
    if (!input) throw new Error("#filter-text missing");
    input.focus();
    input.value = "is:";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
    );
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    expect(store.getState().filterText).toBe("is:open ");
    expect(document.activeElement).toBe(input);
  });

  it("type: suggestions offer only types present in the loaded store", async () => {
    // The fixture has observations but no review_initialized events — a
    // `type:init` suggestion would complete a clause that can only match
    // nothing.
    await main.main();
    const input = document.querySelector<HTMLInputElement>("#filter-text");
    if (!input) throw new Error("#filter-text missing");
    input.focus();
    input.value = "type:";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const labels = [
      ...document.querySelectorAll("#filter-suggestions [data-index]"),
    ].map((el) => el.textContent);
    expect(labels).toContain("type:observation");
    expect(labels).not.toContain("type:init");
  });

  it("Enter without a highlighted suggestion moves focus onward and dismisses the popover", async () => {
    // The global "done searching" Enter still runs (no row is highlighted),
    // and the popover must not linger over the list once focus has left the
    // input.
    await main.main();
    const input = document.querySelector<HTMLInputElement>("#filter-text");
    if (!input) throw new Error("#filter-text missing");
    input.focus();
    input.value = "tra";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const list = document.querySelector("#filter-suggestions");
    expect(list?.classList.contains("hidden")).toBe(false);
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    expect(document.activeElement).not.toBe(input);
    expect(list?.classList.contains("hidden")).toBe(true);
  });

  it("a sort-picker change on the list lens navigates with the new sortKey", async () => {
    await main.main();
    document
      .querySelector<HTMLElement>('.lens-tab[data-lens="list"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    const picker = document.querySelector<HTMLSelectElement>("#sort-picker");
    expect(picker).not.toBeNull();
    if (picker) picker.value = "activity";
    picker?.dispatchEvent(new Event("change", { bubbles: true }));
    expect(store.getState().sortKey).toBe("activity");
    expect(location.hash).toBe("#/list?sort=activity");
  });

  it("an explicit View order choice replaces the timeline order", async () => {
    await main.main();
    const oldest = document.querySelector<HTMLInputElement>("#order-oldest");
    if (oldest) oldest.checked = true;
    oldest?.dispatchEvent(new Event("change", { bubbles: true }));
    expect(store.getState().order).toBe("asc");
    expect(location.hash).toBe("#/timeline?order=asc");
  });

  it("a timeline-row click selects the event without disabling follow", async () => {
    await main.main();
    const row = document.querySelector<HTMLElement>(
      "#master #timeline .event[data-event-id]",
    );
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().selected.kind).toBe("event");
    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineHeadAnchor).not.toBeNull();
    expect(document.querySelector("#detail dl.kv")).not.toBeNull();
  });

  it("a palette Events navigation parks the read without disabling follow", async () => {
    await main.main();
    const event = store.getState().history?.entries?.[0];
    const { entryTitle } = await import("../../src/projection");
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true }),
    );
    const input = document.querySelector<HTMLInputElement>("#cmd-input");
    if (!input || !event) throw new Error("expected palette and history event");
    input.value = entryTitle(event);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    expect(store.getState().selected.id).toBe(event.eventId);
    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineHeadAnchor).not.toBeNull();
  });

  it("timeline step, page, and boundary keys leave follow enabled", async () => {
    await main.main();
    for (const key of ["j", "f", "b", "d", "u", "g", "G"]) {
      store.commit({
        followByLens: {
          ...store.getState().followByLens,
          timeline: true,
        },
        timelineHeadAnchor: null,
      });
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key, bubbles: true }),
      );
      await flush();
      expect(store.getState().followByLens.timeline, key).toBe(true);
      expect(store.getState().timelineHeadAnchor, key).not.toBeNull();
    }
  });

  it("scrolling off the descending live edge parks without disabling follow", async () => {
    await main.main();
    const timeline = document.querySelector<HTMLElement>("#timeline");
    if (!timeline) throw new Error("expected timeline");
    timeline.scrollTop = 12;
    timeline.dispatchEvent(new Event("scroll"));
    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineHeadAnchor).not.toBeNull();
  });

  it("scrolling back to the displayed head verifies catch-up before going live", async () => {
    await main.main();
    const timeline = document.querySelector<HTMLElement>("#timeline");
    if (!timeline) throw new Error("expected timeline");
    timeline.scrollTop = 12;
    timeline.dispatchEvent(new Event("scroll"));
    const newHead = {
      eventId: "evt:sha256:new-live-head",
      eventType: "review_observation_recorded",
      occurredAt: "2026-07-13T21:00:00.000Z",
    };
    setHistoryResponse({
      entries: [newHead],
      diagnostics: [],
      offset: 0,
      matchCount: 1,
      facets: {},
    });

    timeline.scrollTop = 0;
    timeline.dispatchEvent(new Event("scroll"));

    await vi.waitFor(() => {
      expect(store.getState().timelineHeadAnchor).toBeNull();
      expect(store.getState().history?.entries[0]?.eventId).toBe(
        newHead.eventId,
      );
    });
  });

  it("renders catch-up at the top of the timeline and preserves follow and selection", async () => {
    await main.main();
    const row = document.querySelectorAll<HTMLElement>(
      "#timeline .event[data-event-id]",
    )[2];
    const selected = row?.dataset.eventId ?? null;
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    store.commit({ timelineNewCount: 4 });

    const pill = document.querySelector<HTMLElement>(
      "#master > .timeline-shell > #timeline-new-pill",
    );
    expect(pill?.textContent).toBe("Show 4 new events");
    expect(document.querySelector("header #timeline-new-pill")).toBeNull();
    expect(store.getState().selected.id).toBe(selected);

    pill?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().selected.id).toBe(selected);
    await flush();

    expect(store.getState().followByLens.timeline).toBe(true);
    expect(store.getState().timelineNewCount).toBe(0);
    expect(store.getState().history?.offset).toBe(0);
    expect(store.getState().selected.id).toBe(selected);
  });

  it("the disclosed Latest and Oldest actions drive chronological boundaries without disabling follow", async () => {
    await main.main();
    const entries = store.getState().history?.entries ?? [];
    const later = entries[2]?.eventId ?? null;
    store.commit({ selected: { kind: "event", id: later } });

    document
      .querySelector<HTMLElement>("#view-toggle")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document
      .querySelector<HTMLElement>("#jump-latest")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(store.getState().selected.id).toBe(entries[0]?.eventId ?? null);
    expect(store.getState().followByLens.timeline).toBe(true);

    store.commit({
      followByLens: { ...store.getState().followByLens, timeline: true },
      timelineHeadAnchor: null,
    });
    document
      .querySelector<HTMLElement>("#view-toggle")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document
      .querySelector<HTMLElement>("#jump-oldest")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(store.getState().selected.id).toBe(entries.at(-1)?.eventId ?? null);
    expect(store.getState().followByLens.timeline).toBe(true);
  });

  it("the follow control explicitly toggles streaming off and on", async () => {
    await main.main();
    const control = document.querySelector<HTMLElement>("#follow-toggle");

    control?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(store.getState().followByLens.timeline).toBe(false);
    expect(control?.getAttribute("aria-pressed")).toBe("false");

    control?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(store.getState().followByLens.timeline).toBe(true);
    expect(control?.getAttribute("aria-pressed")).toBe("true");
  });

  it("a background repaint does not recenter an unchanged selection", async () => {
    await main.main();
    const selected = store.getState().history?.entries?.[2]?.eventId ?? null;
    store.commit({ selected: { kind: "event", id: selected }, open: true });
    const timeline = document.querySelector<HTMLElement>("#timeline");
    if (!timeline) throw new Error("expected timeline");
    timeline.scrollTop = 42;

    store.commit({ attentionDelta: 1 });

    expect(timeline.scrollTop).toBe(42);
  });

  it("a ref-chip click anywhere routes through the document delegate", async () => {
    await main.main();
    const detail = document.querySelector("#detail");
    if (detail)
      detail.innerHTML = `<span class="ref" role="link" data-ref-kind="rev" data-ref-id="${REV}">chip</span>`;
    // The injected chip specifically: the timeline rows now carry actor chips of
    // their own, so a bare [data-ref-kind] query would hit one of those first.
    document
      .querySelector<HTMLElement>("#detail [data-ref-kind]")
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

  it("opening a diff from a list card then closing reconciles the page", async () => {
    await main.main();
    store.commit({ lens: "list" });
    const diffBtn = document.querySelector<HTMLElement>(
      "#master [data-open-diff]",
    );
    diffBtn?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    // The click navigated onto the routed page; the subscriber's repaint
    // swapped the frame synchronously (the lens layout hides underneath).
    expect(store.getState().diffPage).toBe(true);
    expect(
      document.querySelector("#diff-page")?.classList.contains("hidden"),
    ).toBe(false);
    expect(
      document.querySelector("#master")?.classList.contains("hidden"),
    ).toBe(true);
    document
      .querySelector("#diff-page-close")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().diffPage).toBe(false);
    expect(store.getState().diff).toBeNull();
    expect(
      document.querySelector("#diff-page")?.classList.contains("hidden"),
    ).toBe(true);
    expect(
      document.querySelector("#master")?.classList.contains("hidden"),
    ).toBe(false);
  });

  it("a topbar lens tab exits the diff page onto the lens", async () => {
    await main.main();
    store.commit({ diffPage: true, diffRevision: REV });
    expect(
      document.querySelector("#diff-page")?.classList.contains("hidden"),
    ).toBe(false);
    document
      .querySelector('.lens-tab[data-lens="list"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(store.getState().diffPage).toBe(false);
    expect(store.getState().lens).toBe("list");
    expect(location.hash).toBe("#/list");
    expect(
      document.querySelector("#diff-page")?.classList.contains("hidden"),
    ).toBe(true);
    expect(
      document.querySelector("#master")?.classList.contains("hidden"),
    ).toBe(false);
  });

  it("a hashchange re-applies the route", async () => {
    await main.main();
    history.replaceState(null, "", "#/list");
    window.dispatchEvent(new Event("hashchange"));
    expect(store.getState().lens).toBe("list");
    expect(document.querySelector("#master #units")).not.toBeNull();
  });
});

describe("the disclosed density choice re-measures the timeline rows", () => {
  // Density changes row heights without resizing the #timeline box, so the
  // lens's size observer cannot see it — the composition root routes the
  // toggle to the timeline's re-measure explicitly.
  it("a density change re-derives the row estimate after the settle", async () => {
    await main.main();
    const timeline = await import("../../src/lenses/timeline");
    for (const li of document.querySelectorAll<HTMLElement>(
      "#master #timeline li.event[data-event-id]",
    )) {
      vi.spyOn(li, "getBoundingClientRect").mockReturnValue({
        height: 44,
      } as DOMRect);
    }
    vi.useFakeTimers();
    try {
      const compact =
        document.querySelector<HTMLInputElement>("#density-compact");
      if (compact) compact.checked = true;
      compact?.dispatchEvent(new Event("change", { bubbles: true }));
      vi.advanceTimersByTime(1000);
      expect(timeline.timelineRowHeight()).toBe(44);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a density change notifies every registered density listener", async () => {
    await main.main();
    const prefs = await import("../../src/prefs");
    const listener = vi.fn();
    prefs.registerDensityListener(listener);
    const compact =
      document.querySelector<HTMLInputElement>("#density-compact");
    if (compact) compact.checked = true;
    compact?.dispatchEvent(new Event("change", { bubbles: true }));
    expect(listener).toHaveBeenCalledTimes(1);
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
