# Event versioning

Shoreline separates four version axes so that different kinds of change cost what they should. The
guiding principle:

> **Identity is frozen and signed; interpretation is versioned and hash-excluded.**

A change that touches *what a fact is* (its family, its identity-bearing fields) is a deliberate,
migrated, signed-store break. A change that touches *how a fact's payload is read* is a cheap,
signature-neutral version bump handled at read time. Keeping these on separate axes is what lets a
display rename or a payload-shape tweak avoid re-signing the store.

## The axes

| Axis | Field | Signed / hashed? | Identifies | Changing it costs |
| ---- | ----- | ---------------- | ---------- | ----------------- |
| **Family identity** | `eventType` — a frozen type code (`t:NN`) | **yes** — both signed digests and the stored envelope | *what kind of fact this is* | a **new append-only code** for a genuinely new family; an existing code never changes |
| **Payload view** | `payloadVersion: u32` | **no** — hash-excluded | *which shape the decoded payload takes* | a **cheap, signature-neutral bump** on a hash-excluded axis (no read-time upcast ships at the floor) |
| **Envelope schema** | `version: u32` | included | *the whole envelope contract* | a reject-only gate — a bump is a schema break |
| **Signing scheme** | `sigVersion` | part of the signature record | *how the bytes are signed* | a signing-mechanism break (currently pinned at `1`) |

### `eventType` — a frozen, opaque family code

The stored envelope and both signed digests (`EventToBeSigned`, `EventRecordView`) bind an opaque
**type code** (`t:01`, `t:02`, …) from an append-only registry, not the renamable snake_case name. The
code is assigned once, when a family is first introduced, and is **never reassigned**: renaming the
Rust variant or its display string never changes the code, and a retired family keeps its code
reserved forever so old signed events stay decodable. The display name (`EventType::as_str`) is a
projection-only lookup.

The code is a bare opaque token whose meaning lives **only** in the registry. It carries **no embedded
version** on purpose — a code identifies a *family*, and a family's identity must not move when its
payload shape evolves. Versioning the code would drag every payload-shape bump into the signed
identity (re-keying every event and forking the store), which is exactly the treadmill the opaque
coding exists to retire. Payload-shape versioning is a different axis; see below.

### `payloadVersion` — the hash-excluded payload-shape version

`payloadVersion` is a hash-excluded envelope field that names *which shape the decoded payload takes*.
Because it is excluded from every digest, bumping it is **signature-neutral** — no re-mint, no
migrator. It is the axis on which a **read-time view upcast** *could* live: because the field is
hash-excluded, a future interpretation-only change could re-present an older-shaped payload at
projection time with no stored bytes changed, without re-signing the store. At the 1.0 store-format
floor no such upcast ships: the strict reader is fail-loud, so an older payload view is refused, not
re-presented (see `store-migration.md` §1a). The design point stands — interpretation versioning
belongs here, on a hash-excluded axis, never on the signed type code.

## Decision procedure: I want to change an event

1. **Rename a family's display name or Rust variant** → change the display lookup (`as_str`) only. The
   type code, both digests, and the stored envelope are unaffected; every consumer is a projection.
   **No migration.**
2. **Evolve a payload's shape, interpretation-only** — add, rename, or re-present a field that is
   **not** a content-id input → bump `payloadVersion` for the new shape. Hash-excluded, so
   **signature-neutral: no re-mint, no migrator.** Reading events that carry the *older* shape would
   require a read-time view upcast keyed on `payloadVersion`; none ships at the floor, where the
   strict reader is fail-loud (`store-migration.md` §1a).
3. **Change a payload field that feeds a content id** — an idempotency-key or content-id input → this
   is a **signed-store break**: re-derive every affected content id via the live builders in
   dependency order and gate correctness on the content-id-convergence test (`store-migration.md` §8).
   Rare, deliberate, owner-migrated.
4. **Introduce a genuinely new or replacing family** → assign the next **append-only** type code; the
   retired family keeps its code reserved forever so its old signed events stay decodable. This — not
   a version suffix on an existing code — is how "this is a different kind of fact now" is expressed.
5. **Change the signing mechanism itself** → bump `sigVersion` (ADR-0004). Not done lightly; a signed
   store never holds two signing schemes at once.

## Why identity and interpretation are held apart

- **Cost matches intent.** A display rename or a payload-shape tweak is common and should be free;
  those ride `as_str` / `payloadVersion`. A change to *what a fact is* is rare and expensive; that
  rides a migrated signed-store break. Conflating the axes (for example, versioning the type code)
  would make the common case pay the rare case's price.
- **The signed layer does not dual-read.** A signed-identity break is clean and migrated: the strict
  reader rejects the old shape and a one-shot migrator converts it, so two signed shapes never
  coexist. Versioned coexistence would be a property of the unsigned, hash-excluded payload-view
  layer where `payloadVersion` lives — but at the store-format floor even that layer is fail-loud: no
  read-time upcast ships, so there is no "older format" to *select* on either axis.

## See also

- `docs/adr/adr-0004-event-signatures.md` — the signed-identity model and the opaque-coded-identity /
  view-upcast / storage-descriptor amendment.
- `docs/store-migration.md` — §1a the fail-loud floor (the read-time view-upcast exception is
  retired); §8 the content-id-convergence gate that makes a signed-store break safe.
- `docs/storage-model.md` — the canonical-JSON hashing the digests are computed over.
