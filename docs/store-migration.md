# Migrating a signed event store across a breaking shape change

Pointbreak's event store is append-only and content-addressed: an event's id, record
hash, and on-disk filename are all derived from its bytes, and signatures attest those
bytes. That makes a **breaking change to the event shape** — renaming an enum tag,
reshaping a target, splitting an id — a hard break: old events no longer satisfy the new
reader, and you cannot rewrite them in place without invalidating every id, hash, and
signature that depends on them.

This document captures the architecture that migrates such a store across one clean break.
It is written from a real migration but is deliberately generic: the same shape works for
any future signed-store break. The migrating tool itself is a **throwaway** — run once,
then deleted — so this doc, plus the fail-loud reader, is the durable record.

## The shape of the migration

```
legacy store ──read (relaxed)──▶ reshape (not rename) ──write (strict)──▶ fresh store
                                       │                                      │
                                  re-key + re-sign                       self-check
                                  re-home co-sigs                       (gate swap-in)
```

A migration is a pure function of `(legacy bytes, held keystore)` → a fresh store. It never
mutates the source; the operator swaps the fresh store in only after its self-check passes.

## 1. A fail-loud strict reader, not a dual-read

The new reader rejects an old-shape envelope outright, with a typed error that names the
break and points at the migration. There is **no silent dual-read path** that quietly
accepts both shapes — that defers the break indefinitely and lets half-migrated stores
masquerade as healthy. The break is clean; the one-shot tool is what bridges it. Removing a
prior version's dual-read branch is part of committing to the break and is permanent (it
outlives the tool).

### 1a. No read-time view upcast at the floor: the strict reader is fail-loud

§1's "no silent dual-read" governs the **signed identity** of an event — its `eventType`, `target`,
and `payloadHash`, and anything those digests bind. A change to that identity is still a **clean
break**: the strict reader rejects the old shape, and the one-shot migrator bridges it (§2-§8). There
is no dual-read of the signed bytes.

Earlier revisions carved a bounded exception into that rule for a *different* class of change —
re-*interpreting* an event whose signed identity is unchanged, for example surfacing an old payload
field under a new name in the rendered view. Such a change was allowed to run a pure
`upcast(old_value) -> current_model` in the projection layer, keyed on the hash-excluded per-payload
`payloadVersion`, leaving the stored bytes and every digest untouched. **That exception is retired.**
1.0 is the store-format floor: there is no read-time view upcast, and the strict reader is fail-loud
with no bounded exception. A pre-1.0 payload view — for example a revision capture that bound its
object artifact under the retired `snapshotArtifactContentHash` wire key instead of the current
`objectArtifactContentHash` — is **refused with a clear error at read time, never re-presented**.

The `payloadVersion` field stays hash-excluded (see `event-versioning.md`) so that a *future*
interpretation-only change could still be versioned without re-signing the store, but no such upcast
ships at the floor, and no pre-1.0 format is bridged by one — those are refused, not upcast. When the
signed identity itself must change, you are in §1's clean-break discipline: reject on read, bridge
with a one-shot migrator (§2-§8).

## 2. Read legacy → reshape → write through the strict path

Read each legacy event with a **relaxed** reader (raw JSON / `serde_json::Value`), never the
strict reader, which would reject it. Then **reshape** rather than rename: project the old
envelope into the new structure (e.g. split an over-loaded target into a typed identity
triple; separate a content object from its revision position). A field-rename mindset is a
trap — it produces a byte-compatible-looking event whose *derived* ids are wrong. Recompute
the idempotency key → event id → filename → payload hash from the reshaped bytes, then write
into a **fresh** store through the ordinary strict write path (`record_event_once`). The
strict path re-validates every id and hash on the way in, so a green write *is* a proof the
reshaped event is internally consistent.

## 3. Signed-store handling: re-sign inline, re-home co-signatures in one pass

Reshaping changes an event's record hash, so every signature over it must be reproduced:

- **Inline signatures** are re-signed with the original signer's **held** key. If the key is
  not held, the event is written **unsigned** with a warning — never fabricate a signature.
- **Detached co-signatures** (carriers that attest another event by its id + record hash)
  are **re-homed in a single pass after the whole event set is rebuilt**, so the old→new
  id map is complete. Defer *every* carrier to this pass, then for each:
  - if its target **kept its id** (a verbatim passthrough), preserve the carrier verbatim —
    even if the attester key is not held, because nothing it binds changed;
  - if its target was **re-keyed**, re-attest over the reshaped target with the attester's
    **held** key, or **drop + warn + count** it if the key is not held (foreign or
    transcribed-untrusted attesters cannot be re-signed).

  The trigger is **whether the target was re-keyed, not whether the target was "legacy."** A
  current-envelope target can still be re-keyed (see §7c); a carrier keyed only on
  legacy-ness will silently orphan it, leaving a carrier pointing at a vanished id.

Preserve domain discrimination when you re-emit: a generative move that ranges over multiple
work-object kinds must keep enough on the event for a reader to tell the arms apart, rather
than collapsing them.

## 4. Content-addressed re-keying

Derive the new identity from content, not from succession. An **object id** is a projection
of content alone; a **revision id** is succession-independent. Two clones of the same work
then converge to the same ids, and recording that a revision supersedes a predecessor never
re-keys the predecessor. The migrator re-mints ids this way rather than carrying the legacy
strings forward.

## 5. Self-check as the swap-in gate, with a summary

After writing the fresh store, **rebuild it through the strict read path** (list every event,
re-run the projections). A green self-check is the precondition for swapping the fresh store
in for the legacy one. Emit a **migration summary** — events migrated, inline signatures
re-signed, co-signatures re-attested / dropped, content-removed records preserved — so the
operator sees exactly what happened and can spot an unexpected drop count before committing.

## 6. Throwaway lifecycle

The one-shot tool is **deleted after the run**. It carries no operator specifics: store paths,
key locations, and store inventory live in a private runbook, never in the generic code. Only
this architecture is public. Git history retains the deleted tool's blob if it is ever needed
as a reference, so deleting it loses no knowledge — the strict reader and this doc are the
durable record.

## 7. The transform must preserve integrity (the parts that are easy to get wrong)

A "reshape every field" transform is deceptively simple and quietly drops correctness. Three
guards, each learned the hard way:

**(a) Validate the source before re-emitting it.** Before laundering a legacy artifact into
the clean new format, verify its stored content hash **and** its body↔path identity
consistency (the body's self-claimed id matches the path it was bound at). Reject a tampered
artifact (bad content hash) or a swapped one (valid body, wrong id) rather than re-emitting it
as a trusted clean-format artifact. A content-removed record (the bytes were intentionally
discarded) stays migratable: preserve its binding hash and warn, don't fail.

**(b) Preserve transport / provenance metadata across the transform.** Fields that ride
*outside* the identity digest — ingest provenance, source references, a non-default assertion
mode — are exactly the ones a field-by-field reshape forgets to copy. Some are load-bearing
for verification (ingest provenance gates a binding arm), so dropping them silently changes
behavior. Carry them through uniformly.

**(c) Discriminate "already migrated?" on full wire shape with a STRUCTURAL guard, not on the
envelope alone.** A mixed-shape store accumulates during a multi-step rename: an event can
have a **current envelope** yet still carry a **stale wire token** that a later rename step
touches. Two real cases aborted an envelope-only migrator:

  - an event written *after* the envelope reshape but *before* a later event-type rename
    carried the new envelope and the old event-type string;
  - an event written by an interim binary (one rename landed, a sibling rename missed) carried
    the new envelope and a stale enum tag in its payload (`{ kind: <old> }`).

  Keying "already migrated, pass through" on the envelope routes both to the strict passthrough,
  which rejects the stale token and aborts the whole run. The durable fix: decide passthrough
  with a **recursive structural check for any stale wire token in WIRE POSITIONS** — enum tag
  *values* and id-field *keys* — that **excludes free-text bodies**, so a note or observation
  whose body merely mentions the old term is not a false positive (re-keying it would be wrong
  and would cascade into needless co-signature re-homes). Route any still-stale event through
  the re-keying transform, which then cascades correctly into the co-signature re-home (§3).

**Re-derive dependent ids, don't carry them.** When a reshaped field feeds a downstream id
digest, recompute that id with the **same** function a native write uses (not a private copy
that can drift), and re-derive anything keyed off it. For example, an id that digests a target
must be re-minted once the target reshapes, or a re-run of the same operation against the
migrated store will fail to deduplicate against it.

## 8. Content-id convergence is the real gate (the self-check is necessary, not sufficient)

The self-check in §5 proves the migrated store *reads* cleanly. It does **not** prove the store is
**convergent** — that a fresh re-record of the same fact deduplicates against what is already there
instead of forking a second copy. The two are different, and the gap is easy to miss because the
strict read-path validator never closes it.

A content id (an observation id, an assessment id, the object id, a revision id) is a digest of the
fact's own canonical content. The read-path validator checks an event's stored idempotency key,
event id, payload hash, and filename against each other — it never re-derives a *content* id against
the builder that mints it. So a content id is **opaque** to validation: a store can pass every
read-path and self-check while every content id is frozen at the material it was first minted from. If
a later reshape changes the **content a content id digests** but only re-keys the payload — carrying
the old id string forward — the store still reads fine, yet a fresh re-record (the live builder over
the current payload) mints a *different* id and **forks**. The store was silently non-convergent, and
no read-time check would ever say so.

The durable rule, then: **on any change to a content-id digest input, re-derive every affected content
id via the live builders, in dependency order, remapping references** — exactly as §4 and §7's
"re-derive dependent ids" require — rather than re-keying payloads and carrying the ids. And gate
correctness on a **convergence test**, not the self-check alone:

- drive the live write workflow to record the same fact the migrated store already holds, and assert
  it deduplicates — `events_created == 0` — against the migrated event;
- pin the migrator's id derivation to the live builders directly (assert the migrator's computed
  content id equals the live builder's id for every event family it touches), so the two cannot drift.

A reshape that skips this passes its self-check and ships a store that forks on the next write. The
self-check is necessary; convergence is the property that actually matters, and only a re-record test
proves it.
