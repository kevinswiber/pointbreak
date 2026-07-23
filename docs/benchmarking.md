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

## Plain LMDB build-closure proof

The non-default `lmdb-proof` feature compiles a source-only, developer-gated LMDB closure. It pins
the reviewed heed3 wrapper and upstream `mdb.master3` native source, uses no wrapper, native,
encryption, bindgen, sanitizer, Valgrind, or alternate-key-size features, and links the native
`liblmdb.a` archive statically. It does not select a store, read a Pointbreak store, route production
records, or enable encryption.

Validate the embedded closure contract and exercise one plain open/close against a disposable
directory with:

```sh
cargo bench --locked --features bench,lmdb-proof --bench store_foundation -- \
  --lmdb-proof-open-close
```

The JSON report records the exact wrapper and native source commits, linked LMDB version, plain and
encrypted status, dynamic-host-dependency status, and the disposable carrier filenames. The focused
LMDB closure tests fail closed if the source trees, build inputs, generated bindings,
licenses/notices, feature set, release target matrix, or default-package exclusion drift from
`vendor/lmdb-proof/closure.json`; they run in the default test suite. The portable open/close command
validates the embedded structural contract without requiring a source checkout at runtime. The proof
sources are excluded from default Cargo packages and release archives; default builds do not resolve
or compile heed3 or LMDB.

The native tree retains two ordered, hash-bound source corrections over the immutable LMDB commit:
an explicit byte-pointer cast required by MSVC and the `SYNCHRONIZE` process access required before
Windows can test whether a retained process object has exited. Failed process opens and failed waits
remain conservative and never establish that a reader is dead.

The same feature contains a plain qualification-only journal/profile core with physical identity
`qualification-lmdb-plain-v1`. It uses one journal database, versioned metadata and value envelopes,
default durable commits, exact create-once retries, deterministic byte-ordered replay, and independent
content carriers. Its fixed map policy starts at 16 MiB, grows in 64 MiB increments under a
cross-process resize lock, stops at 256 MiB, and permits at most four resize attempts. Those values are
derived from the public 64 MiB G2 ceiling and reserve four times its decoded bytes; they are not tuned
from candidate timings.

Run its non-timing G0 semantic and receipt smoke against a fresh disposable root with:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
cargo bench --locked --features bench,lmdb-proof --bench store_foundation -- --lmdb-smoke
```

The smoke emits no timing samples or feasibility verdict. It verifies create-once journal writes,
sorted replay, exact decoded hashes, the oldest/middle/newest/absent read schedule, and the deterministic
head marker.

Run the separate non-timing lifecycle smoke with:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
cargo bench --locked --features bench,lmdb-proof --bench store_foundation -- \
  --lmdb-lifecycle-smoke
```

This mode uses deterministic process barriers and only generated public inputs in disposable roots.
It proves that a pinned reader retains its old snapshot while later writers commit, clears a dead
reader slot without evicting a live reader, and keeps the fixed reader-retention workload within a
16 MiB native-allocation bound. After the reader is released, an additional fixed write cohort may
grow native allocation by at most 2 MiB; ordinary page reuse satisfied that predeclared bound. These
are lifecycle bounds derived from the fixed map and workload, not feasibility thresholds.

Backup uses heed3's ordinary online-copy primitive with compaction disabled; it never copies a live
`data.mdb` through filesystem APIs. Candidate and independent-content carriers are published through
the shared backup manifest contract, and the completion marker is written last. The smoke overlaps an
online copy with a writer cohort, accepts only an exact coherent cohort prefix, rejects interrupted or
incomplete destinations, restores in a fresh process without changing the backup, and repairs by
replaying validated logical truth into a fresh copy. Restore and repair both compare the exact
database/content carrier-set identity as well as profile, head, journal, and content receipts. Repair
never replaces an open environment or modifies source carriers in place.

The report schema is `pointbreak.qualification-lmdb-lifecycle-smoke.v1` with report mode
`non_timing_lifecycle_receipts`. It serializes exact receipt hashes and sanitized inventory only:
carrier classes, counts, set hashes, encoded bytes, and native allocated bytes. The exhaustive classes
are database, lock, resize lock, independent content, copy, temporary, obsolete, pinned, repair, and
sidecar. Native allocation uses filesystem allocation metadata and excludes the virtual map
reservation. Separate steady, reopened, and all-carrier high-water snapshots use the shared sanitized
inventory document. Every owned class remains explicit even when the engine has no distinct carrier for
it; for example, reader-pin state is proven by the lifecycle receipt and LMDB lock carrier, so the separate
`pinned` class has a zero count rather than invented bytes. Runner barrier/request/result files are control
evidence, not candidate storage carriers. Windows runs additionally require open-handle replacement to fail while mapped,
succeed after close, reopen exactly, and clean interrupted-copy carriers. The mode emits no timing
samples, performance evaluation, feasibility verdict, selection, migration, or production routing.

## Generated public scale workloads

The foundation target also owns three public, versioned scale workloads. They are generated in
process and do not read an external corpus, a Pointbreak store, the filesystem, environment paths,
the clock, locale state, or operating-system randomness.

| Workload | Records | Decoded bytes | Cohorts | Generator spec SHA-256 | Manifest SHA-256 | Operation schedule SHA-256 |
| --- | ---: | ---: | ---: | --- | --- | --- |
| `G0` | 128 | 1,048,576 | 4 | `5dd08fab4e371f90f9de401ea78c6e281d442627967a3a16db55f724eb32c928` | `b35ebf4bd7bf09a40133e2066cce43cb901a07bf06d5b1caa0f4881bdad27595` | `8f2c69c54a1ea590d05c139cc5405a3e3081be1c9ca50278e3a5ec03df8f788b` |
| `G1` | 1,024 | 8,388,608 | 8 | `9a4b6c1ef8363866005d47860206f94f089a0ad0e2b0e89471dd7254098d368a` | `f520817b751d672810bd8fbe842bb2983b5ff437cce1ad4db3341d79c9b4bf4f` | `a8a094aee8b4154d1c6d1c8c1dcf82f1bf2ecd12d22d4d8ffa4391960e1c0f58` |
| `G2` | 8,192 | 67,108,864 | 8 | `d19e86ed2ca9c0ccc03c1356d721216d3a8a9cba0c49ce19c29c3d52fc1a567c` | `295240840539fbd500796d0cd125d3c1e5266cb61a9feba4aeab2a4d0c2c9158` | `e9f0e9e983873c5251b2ca401718e0ae2bfbde32a046c31b6ece7295c88199a9` |

The canonical generator schema is `pointbreak.qualification-workload-generator.v1`. Its public
seed is
`f4da49601a212010bae444e6ca2de6c6bf28b5ec1b0a05bf42154a533ca513ff`.
Every deterministic decision uses domain-separated counter expansion:

```text
SHA-256(public_seed || schema || workload_id || domain || counter_be_u64)
```

Each tier repeats the same eight decoded-size bins—512, 1,024, 2,048, 4,096, 8,192,
12,288, 16,384, and 20,992 bytes—so every eight records total exactly 65,536 bytes. Records
cycle through low, medium, and high compressibility padding and all nine public record kinds. They
also cover root-only, root-to-replacement, continuation, forked-replacement, carried-open,
resolved, removable-content-present, removed-content-absent, and restored-from-backup lifecycle
motifs. Each record kind and lifecycle motif occurs at least once in every tier.

Logical keys are lowercase portable ASCII and are sorted by their raw UTF-8 bytes before manifest
hashing. Exactly 50% use a digest-uniform shape, 25% use one long common prefix with independent
suffixes, and 25% use cohort-prefixed ordered suffixes. Record ordinals are divided into contiguous,
equal-width logical-age cohorts. The operation schedule selects distinct existing records from the
oldest, middle, and newest cohorts plus one independently derived absent key. Its 30 unique append
indices address the canonical sorted manifest, not host directory order.

Generation has a collected API for bounded consumers and a streaming API that retains key/ordinal
plans but no payload collection. The streaming path buffers one generated record at a time and
computes the same canonical `pointbreak.qualification-corpus.v1` hash incrementally. The repository
commits only the small identity fixture, not generated record bytes.

Regenerate all three public identities without timing or candidate mutation with:

```sh
cargo bench --features bench --bench store_foundation -- --generated-workload-smoke
```

The command emits only the canonical generator, seed, spec, manifest, schedule, count, byte, and
declared-coverage summaries. The JSON omits runtime identity and external-corpus fields so the same
source emits byte-identical output on macOS, Linux, and Windows. `G3` is not an executable workload
in this target.

## Longitudinal access contracts

The bench-gated longitudinal surface freezes two disjoint, public-input-only contracts before any
root materializer or evidence runner exists. `pointbreak.longitudinal-workload.v1` retains the
1,024/7,168/25,600/102,400-event product-operation workload and its proposed envelopes.
`pointbreak.longitudinal-capacity-sentinel.v1` separately defines the 10,000-object L100 variant,
required 262,144-event capacity sentinel, gated 524,288-event extension, fixed-output probes,
memory ownership fields, and bounded two-writer/one-reader exercise. Companion rows never pool
with or substitute for the product-operation workload.

Print both canonical contracts without constructing a root, reading private input, or collecting
timing, memory, or counter observations:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS POINTBREAK_BENCH_FIXTURE POINTBREAK_BENCH_REPO
cargo bench --locked --features bench --bench store_foundation -- --longitudinal-contract
```

The contract records exact release/debug/counting lane separation, retained samples, nearest-rank
statistics, semantic receipts, manifest and pair identity, attribution fields, package boundaries,
capacity gates, and non-compensation. Fast wall time cannot turn whole-history fixed-output work
into bounded work. Exact audit, export/import verification, migration, repair, backup/restore
verification, removal/compaction reconciliation, and deliberate rebuild retain their explicit
history-proportional allowance.

Contract-only package validation derives the C262 identity-complete gate input from the included
materialization pair. Package-verification completion and host-pressure clearance remain hash-bound
observations that the later raw-evidence verifier must derive; this publication does not infer them.

List the currently available longitudinal modes and their side-effect boundary with:

```sh
cargo bench --locked --features bench --bench store_foundation -- --longitudinal-help
```

At this source cut, both modes are non-timing publications. They expose no materializer, evidence
run, external input, storage selector, migration, production route, or architecture verdict.

## Loose-profile baseline evidence

The foundation target has a candidate-independent loose-profile runner. Its evidence document uses
the schema `pointbreak.qualification-loose-baseline-evidence.v1` and cannot represent a candidate,
comparison, threshold, or verdict. It measures the current loose representation directly; the output
is observational input for a later replacement contract, not a storage decision.

Run a native evidence shard from a clean exact commit on a quiesced host with:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
export POINTBREAK_QUALIFICATION_QUIESCED=1
cargo bench --features bench --bench store_foundation -- --loose-baseline-evidence
```

The runner uses only disposable roots and the frozen public generator. `G0` is a diagnostic row;
`G1` and `G2` are baseline rows. Every workload gets three warm-up iterations, 30 measured
iterations, and two independently prepared roots. All raw samples are retained and no outlier is
removed. There is deliberately no pass/fail evaluator.

Each measured root records durable append, strict replay, fresh-process open/recovery, and separate
oldest, middle, newest, and absent keyed reads. Every sample carries a sanitized semantic receipt.
Raw durations cover the verified operation rather than bare I/O: the timed window includes the
semantic verification needed to prove the receipt, and open/recovery includes child-process startup
and teardown. A later comparison must use the same operation windows.
Allocation inventories cover event and complete-profile scopes in steady, reopened, and high-water
states using the same native allocation APIs as the existing qualification runner:
`stat(2)` blocks on APFS/ext4 and `FILE_STANDARD_INFO.AllocationSize` on NTFS.

The evidence validator binds the source commit, `Cargo.lock`, generator schema and seed, workload
specification, manifest and operation schedule, platform, filesystem, allocation API, independent
run, operation, read class, receipt, and allocation inventory. Output retains aggregate receipt and
carrier-set hashes, counts, byte totals, and raw durations. It cannot serialize disposable paths,
environment values, payloads, logical keys, record-level hashes, or error text.

For a quick correctness check, use the non-timing mode:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
cargo bench --features bench --bench store_foundation -- --loose-baseline-smoke
```

The smoke document uses `pointbreak.qualification-loose-baseline-smoke.v1`. It exercises `G0`, all
four operation families, all four keyed-read classes, both allocation scopes, and all three inventory
states without serializing timing samples.

Both documents also expose the value-free
`pointbreak.qualification-prospective-contract-proposal-shape.v1` checklist. The checklist requires
the later proposal to cover operation-specific absolute ceilings, relative allowances, small-baseline
guard bands and their combination formula; small-store overhead and peak headroom; the first public
crossover; event and complete-profile savings at `G1`/`G2`; steady, reopened, and high-water states;
high-water amplification and maintenance duration; `P0`/`M0`/`G0`/`G1`/`G2` roles; manifests, seed,
generator version, schedule, and the verified-operation timing-window definition; platform,
filesystem, and allocation rules; independent keyed-read classes; external evidence authority;
provenance and privacy; and causal early stops. It names those fields only—it contains no proposed
numeric values or evaluator.

## Content-only APFS falsifier contract

The candidate-independent content-only contract is compiled into the benchmark target as
`pointbreak.qualification-content-only-contract.v1`. Its canonical SHA-256 is
`77ce55dd47363bc924d0612c3b508db92bd7969a0ca9bac8d9c7e096e985f654`. Print the contract and its
generated decision table without constructing a candidate, reading a corpus, or collecting timing or
allocation observations:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
cargo bench --features bench --bench store_foundation -- --content-only-contract
```

<!-- content-only-contract-v1:start -->
| Decision | Required value |
| --- | --- |
| Profile | `qualification-loose-journal-pbrf-content-v1` |
| Physical profile ID | 3 |
| Logical capability epoch | `pointbreak.foundation.v1` |
| Events | unchanged raw loose carriers and receipt; byte-equal to loose; 0 bps informational only |
| Content | one PBRF v1 carrier per logical key across object, note, relation-proof, document-manifest, and document-blob |
| Content codec | 192-byte PBRF v1 header; adaptive raw or zstd level 1; checksum and pledged decoded size; no dictionary or trailing bytes |
| Publication | complete same-directory temp, durable create-once, cleanup, and parent-directory durability; retry compares decoded kind, key, and bytes |
| Public workloads | G0 admission, then G1 allocation; frozen manifests and schedules |
| Native platform | macos/apfs via `stat_blocks_512` |
| Independent runs | exactly [0, 1] |
| Allocation states | steady, reopened, high-water; each gates independently |
| Complete-profile floor | at least 1000 basis points in every run/state; no pooling or reruns |
| G0 | every named semantic, lifecycle, transfer, repair, migration, privacy, provenance, and inventory row passes |
| Package admission | macOS aarch64, macOS x86_64; required only for aggregate pass |
| Meaning | bounded content-only APFS falsifier; no timing, recovery-speed, physical-profile selection, migration, activation, or rollout claim |
<!-- content-only-contract-v1:end -->

Every named G0 row, each of the two native APFS G1 runs, and every steady, reopened, and high-water
complete-profile allocation row gates independently. A missing row evaluates as `unknown`; a present row
below the 1,000-basis-point floor evaluates as `failed`. Rows are never pooled, averaged, discarded, or rerun.
The unchanged `events/` carriers and semantic receipt must be byte-equal to loose; their reported saving is
always zero and informational, not a journal qualification criterion. Default-package evidence for both macOS
release architectures is required only for an aggregate pass and cannot rescue an earlier failure.

Evidence and packages bind clean source and tree, `Cargo.lock`, contract, profile, codec and runner sources,
frozen public G0/G1 manifests and schedules, native platform, run index, row identities, and canonical hashes.
Configured private-corpus input and stale, mixed, duplicate, unsupported, or hash-mismatched inputs are
rejected before evaluation. The publication contains no candidate
measurements or observed APFS results, and the contract makes no timing or recovery-speed claim. It does not
select or route a physical profile, authorize migration or activation, or expand qualification beyond this
bounded APFS falsifier.

## Prospective feasibility contract

The approved prospective contract is compiled into the benchmark target as
`pointbreak.qualification-prospective-feasibility-contract.v1`. Its canonical SHA-256 is
`8e9fb5bffef230d97d3f4abc8a70c79958e4372668af8bde19b3aa815382857d`, and it binds the exact approved
proposal SHA-256
`83446c8a40eb71fa4696ee5d71043c47beb8624fc97e2360b62337e489ad67e8`. Print the contract and its
generated decision table without running a candidate, reading an evidence corpus, or collecting timing:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
cargo bench --features bench --bench store_foundation -- --prospective-contract
```

The contract requires two independent native runs on macOS/APFS, non-container Linux/ext4, and native
Windows/NTFS. Each run retains 30 raw samples after three warm-ups. `G0` is a diagnostic early-stop row;
`G1` is the first required allocation crossover; `G2` is the representative public scale row. `P0` and
`M0` separately gate small-store fixed overhead and peak headroom. Evidence binds the admitted loose
baseline authorities, exact source commit and tree, `Cargo.lock`, generator, public seed, manifests,
operation schedules, native allocation API, semantic receipt, and contract identity. Missing evidence
evaluates as unknown, while stale, duplicate, mixed, or hash-mismatched evidence is rejected.

For durable append, replay, fresh-process open/recovery, and each oldest/middle/newest/absent keyed read,
the candidate p95 must satisfy both the operation's absolute ceiling and this dynamic ceiling:

```text
min(absolute ceiling, max(ceil(loose p95 * 125 / 100), loose p95 + guard band))
```

The absolute ceilings are 50 ms for durable append, 500 ms for replay, 750 ms for fresh-process open,
and 5 ms for each keyed read. Guard bands are respectively 5 ms, 10 ms, 25 ms, and 1 ms. Equality passes;
one nanosecond above the resulting limit fails.

Event allocation must save at least 25% and complete-profile allocation at least 10% versus the paired
loose baseline at both `G1` and `G2`, in steady, reopened, and high-water states. `G1` must also be strictly
smaller in both scopes and every state. High-water allocation may be no more than 150% of candidate steady
allocation while still satisfying the savings floor. Small-store fixed-overhead and peak-headroom caps
are 1 MiB for event scope and 2 MiB for complete-profile scope. Maintenance foreground p95 is capped at
250 ms, with total budgets of 5 seconds at `G1` and 30 seconds at `G2`; a genuinely inapplicable
maintenance mechanism requires a hash-bound mechanism proof.

Only public native rows decide prospective feasibility. An owner-local sanitized snapshot may veto later
adoption but cannot rescue a public failure, is never pooled with the public rows, and is excluded from
the contract publication. The publication also excludes candidate observations and results. Passing this
contract establishes prospective plain-store feasibility only: it does not select a storage profile,
authorize production use or migration, or alter the historical H8 qualification artifacts below.

## Plain LMDB prospective evidence runner

The `lmdb-proof` feature includes a native evidence runner and a separate package assembler for the
approved prospective contract. The runner emits one
`pointbreak.qualification-lmdb-prospective-evidence-shard.v1` document for the current native
platform. The assembler accepts exactly one macOS/APFS, one non-container Linux/ext4, and one native
Windows/NTFS shard, then emits a
`pointbreak.qualification-lmdb-prospective-package.v1` document containing the unchanged
`pointbreak.qualification-prospective-feasibility-evidence.v1` aggregate and its frozen
`pointbreak.qualification-prospective-feasibility-evaluation.v1` evaluator output.

Before collecting evidence, exercise the runner, the plain LMDB semantic and lifecycle preflights,
and deterministic package assembly without collecting normative timing or allocation samples:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
cargo bench --locked --features bench,lmdb-proof --bench store_foundation -- \
  --lmdb-prospective-smoke
```

The smoke report schema is `pointbreak.qualification-lmdb-prospective-smoke.v1`. Its
`deterministicFixtureOnly` and `normativeMeasurementCollected` fields are respectively `true` and
`false`; its shard and package hashes cover synthetic protocol fixtures, not candidate results.

A real native shard must be built and run from the exact clean commit being evaluated, with the host
quiesced and only the generated public workloads enabled:

```sh
unset POINTBREAK_QUALIFICATION_CORPUS
export POINTBREAK_QUALIFICATION_QUIESCED=1
cargo bench --locked --features bench,lmdb-proof --bench store_foundation -- \
  --lmdb-prospective-evidence > native-shard.json
```

The runner performs the non-timing semantic and lifecycle preflights first, then executes two
independently prepared runs for `P0`, `M0`, `G0`, `G1`, and `G2`. Timing-required rows retain all 30
paired candidate and loose-baseline samples after three warm-ups; allocation rows retain event and
complete-profile inventories for steady, reopened, and high-water states. Candidate operations and
their adjacent loose controls cover durable append, strict replay, fresh-process open/recovery, and
oldest, middle, newest, and absent keyed reads with the contract's verified-operation windows.

Every shard carries two distinct provenance layers. The published contract retains the source
commit, source tree, and `Cargo.lock` identities from which its thresholds were derived. The shard's
execution envelope instead records the exact clean commit and tree that produced the measurements,
the current `Cargo.lock` SHA-256, the reviewed LMDB closure-manifest SHA-256, the frozen contract and
approved proposal hashes, the public generator and seed, the physical profile, and the run controls.
Package assembly rejects stale or mixed execution envelopes, missing platforms or runs, duplicate
rows, hash mismatches, and private-data markers. Shards and packages serialize no disposable paths,
environment values, command lines, logical keys, payload bytes, or record-level hashes.

After the three native shards have been collected from the same execution identity, assemble and
evaluate them from that same clean commit:

```sh
cargo bench --locked --features bench,lmdb-proof --bench store_foundation -- \
  --lmdb-prospective-package \
  --lmdb-prospective-input=macos-apfs.json \
  --lmdb-prospective-input=linux-ext4.json \
  --lmdb-prospective-input=windows-ntfs.json
```

The assembler validates all inputs before running the frozen evaluator and binds both the evaluation
and the final package with canonical SHA-256 identities. A well-formed package can evaluate to
passed, failed, or unknown; assembly success is not a feasibility verdict. No prospective runner or
package command selects a store, changes production routing, authorizes migration, or rewrites prior
qualification evidence.

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

Run a final evidence shard only from a clean exact commit on a quiesced native host. macOS additionally
requires the validated external workload copy; Linux and Windows reject that variable because the frozen
contract assigns the external row only to macOS:

```sh
export POINTBREAK_QUALIFICATION_QUIESCED=1
export POINTBREAK_QUALIFICATION_CORPUS=/path/to/external-corpus-copy # macOS only
cargo bench --features bench --bench store_foundation -- --qualification-final-evidence > macos.json
```

The runner discards separate warm-up roots, grows fresh measured roots monotonically, validates replay and
fresh-process open receipts, and records native event-scope and complete-profile allocation. High-water
sampling includes the candidate checkpoint or seal boundary before reopen. The JSON contains only
sanitized hashes, counts, timing samples, allocation totals, and environment identity.

After collecting the macOS, native Linux/ext4, and native Windows/NTFS shards from the same source and
contract identities, assemble and evaluate the complete performance package with:

```sh
cargo bench --features bench --bench store_foundation -- --qualification-package \
  --qualification-input=macos.json \
  --qualification-input=linux.json \
  --qualification-input=windows.json
```

Assembly rejects stale or duplicate shards and any package with a missing required run. A valid package
may still contain failed timing or allocation criteria; measurement failure is evidence, not a malformed
package.

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
