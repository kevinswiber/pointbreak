// The fetch leaf: a single `fetchJSON` that fetches a path, parses JSON, and
// surfaces a useful error on a non-JSON body, a non-OK status, or an `error`
// field in the payload. Ported from the served app.js `fetchJSON`; imports
// nothing, so render / detail / diff can depend on it for on-demand fetches
// without pulling in the data-loading orchestration.

// The truthy `error` field of a payload, as a string, or "" when absent.
function payloadError(data: unknown): string {
  if (
    typeof data === "object" &&
    data !== null &&
    "error" in data &&
    data.error
  ) {
    return typeof data.error === "string" ? data.error : String(data.error);
  }
  return "";
}

/**
 * Fetch `path` (no-store) and return its parsed JSON. Throws when the body is not
 * JSON (naming the path + status), when the payload carries an `error` field, or
 * when the response is non-OK (preferring the payload error over the status).
 */
export async function fetchJSON(path: string): Promise<unknown> {
  const res = await fetch(path, { cache: "no-store" });
  const text = await res.text();
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    throw new Error(`${path}: non-JSON response (${res.status})`);
  }
  const error = payloadError(data);
  if (!res.ok || error) {
    throw new Error(error || `${path}: HTTP ${res.status}`);
  }
  return data;
}
