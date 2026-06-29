import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fetchJSON } from "../src/http";

// `fetchJSON` is the fetch leaf. These tests drive it against a hand-stubbed
// global `fetch` so the status / body / error-field combinations of its error
// contract can be exercised directly (the route-serving fixture mock lives in the
// data-layer tests).
let savedFetch: typeof fetch;

beforeEach(() => {
  savedFetch = globalThis.fetch;
});

afterEach(() => {
  globalThis.fetch = savedFetch;
});

/** Make the next fetch resolve with `body` at the given HTTP status. */
function stub(body: string, status = 200): void {
  globalThis.fetch = () => Promise.resolve(new Response(body, { status }));
}

describe("fetchJSON", () => {
  it("resolves the parsed JSON for a 200 response", async () => {
    stub(JSON.stringify({ ok: true, n: 3 }));
    await expect(fetchJSON("/api/history")).resolves.toEqual({
      ok: true,
      n: 3,
    });
  });

  it("throws a non-JSON error naming the path and status when the body is not JSON", async () => {
    stub("<html>nope</html>", 502);
    await expect(fetchJSON("/api/history")).rejects.toThrow(
      "/api/history: non-JSON response (502)",
    );
  });

  it("throws the payload's error message even on a 200", async () => {
    stub(JSON.stringify({ error: "boom" }), 200);
    await expect(fetchJSON("/api/revisions")).rejects.toThrow("boom");
  });

  it("throws an HTTP status error for a non-OK response without an error field", async () => {
    stub(JSON.stringify({ data: 1 }), 500);
    await expect(fetchJSON("/api/objects")).rejects.toThrow(
      "/api/objects: HTTP 500",
    );
  });

  it("prefers the payload error message over the HTTP status on a non-OK response", async () => {
    stub(JSON.stringify({ error: "explicit reason" }), 503);
    await expect(fetchJSON("/api/freshness")).rejects.toThrow(
      "explicit reason",
    );
  });
});
