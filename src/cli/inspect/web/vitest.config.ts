import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "happy-dom",
    // Re-binds the Web Storage globals to happy-dom's after the environment is
    // set up — Node ≥25 (or --experimental-webstorage) otherwise shadows them
    // with its own file-backed storage and every write throws.
    setupFiles: ["test/support/webstorage.ts"],
    include: ["test/**/*.test.ts"],
  },
});
