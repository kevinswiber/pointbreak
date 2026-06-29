// Test harness: a `globalThis.fetch` mock that serves the committed `/api/*`
// fixtures by route, so `data.ts` (`load`/`pollFreshness`) and the on-demand
// `fetchJSON` fetches (detail / diff) can be driven without a live server. Mocking
// at the `fetch` boundary (not `fetchJSON`) keeps the ported `http.ts`/`fetchJSON`
// in the path under test. Append-only shared surface: extend with new routes here.

import historyJson from "../fixtures/history.json";
import objectJson from "../fixtures/object.json";
import objectsJson from "../fixtures/objects.json";
import revisionJson from "../fixtures/revision.json";
import revisionsJson from "../fixtures/revisions.json";

// The committed fixtures keyed by request pathname (the query string is ignored:
// `/api/object?id=…` and `/api/revision?id=…` resolve to the single-resource
// fixtures regardless of which id is requested).
const FIXTURES: Record<string, unknown> = {
  "/api/history": historyJson,
  "/api/revisions": revisionsJson,
  "/api/objects": objectsJson,
  "/api/object": objectJson,
  "/api/revision": revisionJson,
};

// The freshness probe is not a captured fixture (it is a tiny derived summary):
// default it to history.json's hash + diagnostic count so a poll right after
// `load()` reports "unchanged", and let a test override it to drive the
// changed/reload path via {@link setFreshnessResponse}.
const historyDoc = historyJson as {
  eventSetHash?: string;
  diagnostics?: unknown[];
};
const DEFAULT_FRESHNESS: unknown = {
  eventSetHash: historyDoc.eventSetHash,
  diagnosticCount: (historyDoc.diagnostics ?? []).length,
};
let freshness: unknown = DEFAULT_FRESHNESS;

/** Override the `/api/freshness` response the mock returns (changed-hash tests). */
export function setFreshnessResponse(payload: unknown): void {
  freshness = payload;
}

/** Restore the default freshness response (history.json's hash + diagnostic count). */
export function resetFreshnessResponse(): void {
  freshness = DEFAULT_FRESHNESS;
}

/** The request target as a string, accepting the full `fetch` input union. */
function urlOf(input: RequestInfo | URL): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.href;
  return input.url;
}

/** The pathname of a request target (relative `/api/*` paths resolve against a stub origin). */
function pathnameOf(input: RequestInfo | URL): string {
  return new URL(urlOf(input), "http://inspector.test").pathname;
}

const mockFetch: typeof fetch = (input) => {
  const pathname = pathnameOf(input);
  if (pathname === "/api/freshness") {
    return Promise.resolve(
      new Response(JSON.stringify(freshness), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );
  }
  const data = FIXTURES[pathname];
  if (data === undefined) {
    return Promise.resolve(
      new Response(`no fixture for ${pathname}`, { status: 404 }),
    );
  }
  return Promise.resolve(
    new Response(JSON.stringify(data), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
};

// `null` is the "not installed" sentinel; the `=== null` guards narrow it away on
// restore, so no cast is needed.
let saved: typeof fetch | null = null;

/** Swap `globalThis.fetch` for the fixture-serving mock. Restore with {@link uninstallFetchMock}. */
export function installFetchMock(): void {
  if (saved === null) saved = globalThis.fetch;
  globalThis.fetch = mockFetch;
}

/** Restore the global `fetch` the harness replaced (a no-op if it was never installed). */
export function uninstallFetchMock(): void {
  if (saved === null) return;
  globalThis.fetch = saved;
  saved = null;
}
