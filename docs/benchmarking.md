# Benchmarking the durable store

The `store_backend` benchmark (`cargo bench --features bench`) measures the three metrics a future
log-structured backend would be compared against for the file backend: whole-log read latency
(`list_events`), single-append latency, and on-disk amplification. The synthetic groups
(100 / 1k / 10k events) are generated in-process and need nothing external — anyone can run them, and
they carry the portable baseline.

## Foundation workload smoke

The `store_foundation` target freezes two backend-neutral qualification workloads before any
alternative store is implemented:

- `synthetic-legacy-shape` is a small public fixture that exercises legacy event, object-artifact,
  and note-body records.
- `modeled-foundation-workload` covers root and replacement generations, continuation, every
  relation-proof status, every supported fact-port relation, relation-proof content, auxiliary
  documents, and multi-round artifact growth.

Run its non-timing smoke mode with:

```sh
cargo bench --features bench --bench store_foundation -- --smoke
```

The target prints one JSON record containing build identity, a Cargo lockfile hash, Rust version,
OS, filesystem, configuration, logical capabilities, and manifest hashes/counts. It does not select
or time a storage implementation.

An optional versioned external workload must be a separately supplied read-only copy:

```sh
export POINTBREAK_QUALIFICATION_CORPUS=/path/to/external-corpus-copy
cargo bench --features bench --bench store_foundation -- --smoke
```

Never point this variable at a live Pointbreak store. The loader rejects source-tree paths and
symbolic links, reads only `events/`, `artifacts/objects/`, and `artifacts/notes/`, and emits only
hashes, counts, byte totals, and sanitized status. It never prints the supplied path or record
bytes. When the variable is absent, the public workloads still validate and the external row is
reported as `not_configured`.

The current external workload contains 6,706 files totaling 58,212,172 decoded bytes. Its manifest
carries 6,702 logical workload records (6,392 events, 309 object artifacts, and one note body;
58,210,604 decoded bytes). The loader separately checks the four store-metadata files and their 1,568
bytes without reading their content, then verifies the versioned manifest hash. Any logical, metadata,
or manifest mismatch fails closed. The earlier 6,437-file frozen-legacy workload and its loader remain
available only for reproducing historical reports; it is not relabeled as the current workload.

## Frozen performance qualification contract

The machine-readable performance qualification contract is compiled into the benchmark target. Print
the canonical contract, its SHA-256 identity, and its generated human decision table with:

```sh
cargo bench --features bench --bench store_foundation -- --qualification-contract
```

The contract applies the same four complete operations—durable append, strict replay, keyed read, and
fresh-process open/recovery—to the SQLite WAL and bounded-segment candidates against one common loose-file
baseline. Required quantitative rows are the external workload on macOS/APFS and the modeled workload on
macOS/APFS, native non-container Linux/ext4, and native Windows/NTFS. Public-smoke rows use the same
protocol and semantic receipts on all three platforms, but their timing and allocation remain diagnostic.

Each operation receives three untimed warm-up pairs and 30 measured adjacent pairs, alternating which role
runs first. Two independently prepared runs are required for every workload/platform row. The evaluator
retains every sample, computes candidate-to-baseline ratios, and reports nearest-rank p50 and p95, the full
range, and population standard deviation. Every quantitative run passes only when each operation's p95 is
at or below 125%; runs are never pooled. Event-scope and complete-profile native allocations must also be
strictly lower than the loose baseline in steady, reopened, and high-water states. Allocation parity fails.

Windows allocation uses `FILE_STANDARD_INFO.AllocationSize`; its native fixture test covers one-byte and
multi-cluster ordinary files, sparse allocated ranges, and compressed data. Missing, stale, unsupported,
duplicate, or hash-mismatched evidence is rejected or evaluated as unknown, never as a pass.

## Native foundation qualification

The developer-gated foundation runner applies one deterministic matrix to the SQLite WAL and bounded-
segment candidates on both public workloads. It uses fresh disposable roots and real child processes for
locking, reader/writer, backup/writer, kill/reopen, and maintenance overlap. Results include exact build and
dependency identity, stable per-row seed identities reserved for future seeded placement, fixed scenario
boundary labels, filesystem policy, native allocated-byte inventories, raw samples, and a generated
completeness report.

Run the non-timing matrix used by native CI with:

```sh
just store-foundation-qualification-smoke
```

The legacy repeated matrix remains available for historical comparison with:

```sh
just store-foundation-qualification
```

That command no longer produces new qualification evidence. Its performance rows fail closed until a
complete `pointbreak.qualification-performance-evidence.v2` package is assembled and evaluated. Timing
thresholds never run in default tests or the CI smoke lane.

These matrix commands use only the checked-in public workloads. They do not read
`POINTBREAK_QUALIFICATION_CORPUS`; validate an explicitly supplied external copy separately with the
`--smoke` command above, and keep its record bytes outside the repository and generated reports.

## Non-gating performance diagnostics

The foundation target also has an explicit diagnostic mode that explains the candidate and loose-file
operation totals without changing the qualification verdict:

```sh
cargo bench --features bench --bench store_foundation -- --qualification-diagnostics
```

It runs warm-up and alternating paired samples for durable append, strict replay, keyed read, and strict
open/recovery. The JSON report binds the source commit, Cargo lockfile, diagnostic contract, candidate
profile, workload, platform, pair order, raw totals, stage totals, and steady/reopened/high-water
inventories. Diagnostic results are observations: exceeding the historical 125% ceiling does not make this
command fail and does not select a storage profile.

For order-sensitivity controls, repeat the command with
`--qualification-pair-order=candidate_then_baseline` and
`--qualification-pair-order=baseline_then_candidate`. An alternating report remains the primary paired
observation; either fixed-order report on its own is incomplete diagnostic evidence.

When `POINTBREAK_QUALIFICATION_CORPUS` names a validated external workload copy, the same process also adds
that workload. The path, logical keys, and decoded bytes are not serialized. Never point it at a live store,
a path inside a Git worktree, or `~/.pointbreak`; an absent external path leaves the public diagnostic run
complete without claiming external-corpus evidence.

## Real-world read-all sample: `POINTBREAK_BENCH_FIXTURE`

The `read_all/fixture` group runs only when `POINTBREAK_BENCH_FIXTURE` points at a **store directory** — the
directory that contains `events/`. For a captured repo that is the shared common-dir store at
`<git-common-dir>/pointbreak`. When the variable is unset, or the store does not read back, the group is **skipped,
not failed**, so the harness has no baked-in paths.

The API-level benches (`revision_overviews`, `freshness`) instead want a repo root: set
`POINTBREAK_BENCH_REPO=<repo>`, or, for the standard `<repo>/.git/pointbreak` layout, let it be
derived from `POINTBREAK_BENCH_FIXTURE`. Linked worktrees and separate Git directories must set
`POINTBREAK_BENCH_REPO` explicitly.

## Schema currency matters

The fixture store must be authored by the **current** Pointbreak schema. A store from a retired schema
(for example the legacy `writer.role` envelope, pre-0076/0079) hard-errors under the strict
`list_events`, so the real-world group silently skips — which is exactly why a rotted fixture is easy to
miss.

Two things guard against that:

- A schema-currency guard test (`bench_support` →
  `a_current_schema_store_reads_back_through_the_harness`) authors a store with the current code and
  asserts it reads back through the harness. If a schema break ever regresses this, that test fails
  loudly in CI rather than the benchmark quietly skipping.
- The fixture is **regenerated**, not committed as a binary blob, so it can't drift out of schema.

## Getting a current-schema fixture

Capture a current-schema repository, ask Pointbreak for its canonical common store, and point the
benchmark at that directory:

```sh
REPO=/path/to/captured/repo
export POINTBREAK_BENCH_REPO="$REPO"
export POINTBREAK_BENCH_FIXTURE="$(pointbreak store paths --repo "$REPO" --format json | jq -r .commonStore)"
cargo bench --features bench
```

Re-capture or regenerate the source repository after any store-schema break to keep the fixture current.

> A future alternate backend must be measured on the **same** filesystem as the file backend — disk
> amplification is filesystem-specific (~8× on APFS for sub-block event files). See
> [ADR-0020](./adr/adr-0020-durable-storage-backend-seam.md).
