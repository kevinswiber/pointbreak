// Node 25 unflagged the built-in Web Storage API (nodejs/node#57666): the test
// worker's globals then carry Node's own `localStorage`/`sessionStorage`, which
// shadow happy-dom's and — with no `--localstorage-file` backing — throw on
// every access (vitest-dev/vitest#8757). happy-dom's `window.localStorage`
// delegates to the same broken storage, so re-binding to the window doesn't
// help. The same state is reachable on Node 22/24 via
// `NODE_OPTIONS=--experimental-webstorage`, which is how one review
// environment hit it.
//
// Probe each storage global with a real write; only when it is broken or
// missing, replace it with an in-memory implementation — a healthy environment
// is left untouched. App and test code reach storage exclusively through the
// bare globals, so only `globalThis` needs the repair.

type StorageGlobals = { localStorage?: Storage; sessionStorage?: Storage };

function storageWorks(key: keyof StorageGlobals): boolean {
  try {
    const storage = (globalThis as StorageGlobals)[key];
    if (!storage) return false;
    const probe = "__webstorage-shim-probe__";
    storage.setItem(probe, "1");
    storage.removeItem(probe);
    return true;
  } catch {
    return false;
  }
}

function memoryStorage(): Storage {
  const map = new Map<string, string>();
  const storage = {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (key: string) => (map.has(key) ? (map.get(key) as string) : null),
    key: (index: number) => [...map.keys()][index] ?? null,
    removeItem: (key: string) => {
      map.delete(key);
    },
    setItem: (key: string, value: string) => {
      map.set(String(key), String(value));
    },
  };
  return storage as Storage;
}

for (const key of ["localStorage", "sessionStorage"] as const) {
  if (!storageWorks(key)) {
    const replacement = memoryStorage();
    Object.defineProperty(globalThis, key, {
      configurable: true,
      get: () => replacement,
    });
  }
}
