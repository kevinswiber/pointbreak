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

An optional frozen legacy corpus must be a separately supplied external copy:

```sh
export POINTBREAK_QUALIFICATION_CORPUS=/path/to/external-corpus-copy
cargo bench --features bench --bench store_foundation -- --smoke
```

Never point this variable at a live Pointbreak store. The loader rejects source-tree paths and
symbolic links, reads only `events/`, `artifacts/objects/`, and `artifacts/notes/`, and emits only
hashes, counts, byte totals, and sanitized status. It never prints the supplied path or record
bytes. When the variable is absent, the public workloads still validate and the external row is
reported as `not_configured`.

The frozen snapshot contains 6,437 files totaling 57,041,682 decoded bytes. Its manifest carries
the 6,433 logical workload records (6,131 events, 301 object artifacts, and one note body; 57,040,114
decoded bytes). The loader separately checks the four store-metadata files and their 1,568 bytes
without reading their content. Any logical or metadata mismatch is returned as a structured drift
report rather than silently accepting a different corpus.

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

Run the developer evidence lane with repeated candidate and freshly populated loose-baseline samples with:

```sh
just store-foundation-qualification
```

The repeated lane applies the 125% p95 ceiling to durable append, strict replay, keyed read, and strict
reopen/recovery. A complete report can still contain failed rows; the command exits nonzero whenever any
row fails. Timing thresholds never run in default tests or the CI smoke lane.

These matrix commands use only the checked-in public workloads. They do not read
`POINTBREAK_QUALIFICATION_CORPUS`; validate an explicitly supplied external copy separately with the
`--smoke` command above, and keep its record bytes outside the repository and generated reports.

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
