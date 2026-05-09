# Storage Model

## Status

This is architecture guidance for Shore's durable review/session state. It describes constraints the
first `.shore/` persistence slice should preserve, even when the implementation starts small.

## Goal

Shore should make durable state boring: write facts once, rebuild projections, and keep output,
storage, and notification side effects behind explicit seams. The storage model should avoid the
common failure modes of long-running coordination tools: hidden in-memory authority, direct delivery
before persistence, shared mutable JSON files, unbounded retries, and helper bypasses.

## Storage Layers

Use distinct storage concepts for distinct semantics:

```text
.shore/
  events/       immutable event log
  state.json    rebuildable projection
  artifacts/    immutable or content-addressed support records
```

`events/` is the authoritative log. Events are immutable, independently written, and never moved to
`failed/`, retried in place, or rewritten on read.

`state.json` is a cache/projection. It must be rebuildable from durable records. If it is missing,
stale, or invalid, Shore should rebuild it rather than treating it as authority.

A future delivery queue is a separate subsystem. Queue concepts such as `pending/`, `failed/`,
retry counts, backoff, circuit breakers, and acknowledgement markers do not belong in
`.shore/events/`.

## Event Files

Every durable event must carry a non-null `idempotencyKey`. The key should be derived from canonical
event content, not generated randomly at the call site.

Use a hash of the idempotency key as the event filename:

```text
events/<sha256(idempotencyKey)>.json
```

Keep the readable idempotency key inside the event envelope. The filename is fixed-width and safe;
the event remains inspectable.

Event creation should be exclusive. If the file already exists for the same idempotency key, the
write is idempotent. If the filename exists with conflicting content, that is a corruption or
conflict error, not a merge.

Do not add a global sequence number until Shore has a concrete allocator that does not create a
shared mutable counter. Deterministic event ordering can start from event metadata and filenames.

## Atomic Writes

All durable writes should go through one storage helper. The helper owns:

- temp file in the same directory as the target
- deterministic temp filename prefix
- file mode suitable for local review/session data
- temp file fsync for durable writes
- atomic rename into place
- parent directory fsync for durable writes
- stale temp file sweep

Any helper that can create temp files must also participate in sweeping them. Cleanup should not be
limited to queue code. On load, Shore should remove stale temp files matching its known prefixes and
older than the configured safety threshold.

Rebuildable projections may use a non-durable write mode that skips fsync, but they still should use
the same temp/rename path to avoid partial reads.

## Bounded Projections

`state.json` must stay bounded. It should summarize current state, cursors, and active projections;
it should not grow linearly with the event log.

If a projection needs unbounded history, split it into paged or content-addressed records under
`artifacts/` and keep `state.json` as an index or summary. A large `state.json` is a design smell
because it becomes a shared mutable file, a slow health check, and a crash-recovery hazard.

## Shared Mutable Files

Authoritative facts should not live in read-modify-write shared JSON documents. Per-event files are
a deliberate defense against metadata clobbering:

- two writers can write different events without merging a shared object
- one failed event does not roll back unrelated events
- a projection can be rebuilt after partial failure
- stale projections are recoverable

Shared JSON files are acceptable only for rebuildable projections or configuration whose merge rules
are explicit and tested.

## Storage API Shape

Keep the primitive storage API bytes-shaped first, with JSON as a convenience layer:

```text
storage::read_bytes
storage::read_bytes_if_exists
storage::write_bytes_atomic
storage::create_file_exclusive
storage::list_dir
storage::sweep_temp_files

storage::read_json
storage::write_json_atomic

event_store::write_event
event_store::read_event
event_store::list_events
event_store::event_exists
```

This keeps the lower layer useful for manifests, JSON, future binary artifacts, and exact conflict
checks. Event filename construction should live in `event_store`, not in command handlers.

Plan 0005 should keep this synchronous and local. Do not introduce async traits or a runtime until a
remote backend, subscription API, or second storage backend forces that decision.

## Output Boundary

CLI output is also a side effect and should have a seam.

Domain, storage, and workflow code should return values, diagnostics, or events. CLI code should
decide how to write those values to stdout and stderr. Avoid burying `println!` or `eprintln!`
inside workflow logic.

A small boundary such as `run_with_io(args, stdout: &mut dyn Write, stderr: &mut dyn Write)` is
enough. This is not a multi-channel delivery framework; it is a testability and side-effect
boundary.

## Notifications And Delivery

Notifications are hints, not authority. The durable event must land before any notification fires.
Clients that receive a notification should re-read durable state before acting.

If Shore later adds a delivery queue, every retry path must have:

- a maximum attempt count
- backoff policy
- permanent vs. transient failure classification
- a terminal failed state that removes the entry from active rotation
- target-liveness checks before resume or apply actions

Plan 0005 should not implement this queue. Local event writes should fail loudly rather than loop.

## Migrations And Doctor

Runtime code should read canonical storage. Legacy repair and migration belong in a future
`shore doctor` or equivalent explicit command.

Migration and repair work should commit independently. One successful fix should not be rolled back
because an unrelated later validation failed. This mirrors the event-log rule: one durable fact, one
independent commit.

## Lock Discipline

Plan 0005 should not need locks. If a future plan introduces locks, follow these constraints:

- keep critical sections short
- do not perform long I/O while holding a lock when it can be avoided
- use lock-acquisition timeouts
- record enough state on disk to recover after process death
- do not rely on process-exit cleanup for correctness

## Health And Status

Health checks and status commands should exercise the real path:

- load the manifest or storage root
- list `events/`
- read event envelopes through the event store
- derive fresh state
- compare or refresh the projection

A lightweight probe that bypasses event loading and state derivation can report healthy while the
real workflow is broken. The health path should be the same code path users depend on.

## Non-Goals

This document does not require:

- a daemon
- remote storage
- async storage
- a delivery queue
- filesystem locks
- global event sequence allocation
- committed `.shore/` state

The point is to keep the first storage slice small while making the safe path the easiest path to
use.
