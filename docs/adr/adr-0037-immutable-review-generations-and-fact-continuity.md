# ADR-0037: Immutable Review Generations, Relation Evidence, and Fact Continuity

**Status:** Accepted (owner-approved 2026-07-19)
**Date:** 2026-07-19
**See also:** [ADR-0014](./adr-0014-reviewunit-commit-range-lifecycle.md),
[ADR-0017](./adr-0017-eventtarget-identity-layering-and-engagement-naming.md),
[ADR-0018](./adr-0018-event-borne-supersession-replaces-lineage.md),
[ADR-0026](./adr-0026-fact-to-fact-response-relationship.md),
[ADR-0038](./adr-0038-relation-proof-and-auxiliary-document-resources.md), and
[ADR-0039](./adr-0039-exact-logical-bundles-and-import-receipts.md).

## Context

The current revision proposal stores sorted replacement pointers and derives replacement heads, but it has
no non-replacing continuation edge (`src/session/workflow/capture.rs:157-171,425-452`; `src/session/projection/supersession.rs:18-38`).
Commit associations are convergent structural history. Their path-overlap guard is explicitly advisory and
does not prove that an associated commit contains the reviewed bytes (ADR-0014). Observations, validation,
requests, and assessments nevertheless target only a revision, so recording post-edit facts on the same
revision can make an old captured artifact appear to describe new live content.

The object artifact is already immutable and content-addressed. The missing contract is an exact logical
address for that reviewed state, an explicit split between replacement and continuation, attributed evidence
for what an association means, and cross-generation continuity that never moves original facts.

## Decision

### D1. `GenerationRefV1` is the exact reviewed-content address

The frozen generation authority is:

```text
GenerationRefV1 {
  revision_id,
  object_artifact_content_hash,
}
```

The pair, not either member alone, identifies one immutable reviewed state. Exact generation reads return
the artifact bound by that pair or an explained `available | removed | missing` state. A thread,
association, Git endpoint, proof, native projection, or fallback may not change or substitute those bytes.
A supplied hash that does not match the revision is a hard resource mismatch.

### D2. Reviewable content changes create a new generation

A coherent change to reviewed code, tests, documentation, generated output, or capture scope requires a new
capture before new-state observations, validation, requests, or assessment are recorded. Clarification,
responses, attribution, and a landing association may remain on the generation only when reviewed content
is unchanged. Captures occur at coherent review boundaries, not on every editor save.

Every fact retains its origin `GenerationRefV1`. Validation and assessment never transfer. An accepting
state requires a current accepting assessment and relied-on validation on every selected current
generation, with no applicable operative request open.

### D3. `supersedes` and `continues` are distinct capture-time relations

`supersedes` alone means replacement and alone drives stale/current replacement heads. Add omitted-when-
empty `continues: Vec<RevisionId>` to the revision proposal for non-replacing work that remains in the same
review thread. Both vectors are sorted and deduplicated in the stored payload, are disjoint, are immutable
after capture, and do not enter `revision_id`.

The thread projection uses the undirected connected component over the union of `supersedes` and
`continues`. Replacement currency uses only `supersedes`. Several non-superseded generations may therefore
remain current; no scalar winner is inferred. Missing relation targets are accepted with self-healing
diagnostics. Git ancestry, timestamps, shared paths, and planning membership never infer either intent.

### D4. Commit association meaning is separate attributed evidence

The existing association and withdrawal events remain structural. With no surviving attestation, a commit
association projects as `unknown + unverified`. Add `RevisionRelationAttested` with wire name
`revision_relation_attested` and the canonical material:

```text
RelationAttestationV1 {
  relation_attestation_id,
  generation: GenerationRefV1,
  commit_association_id,
  semantic_relation,
  proof_status,
  proof_method,
  proof_algorithm_version,
  capture_scope,
  comparison_base_or_parent,
  endpoint_oids,
  evidence_content_hash?,
  result_digest,
}
```

`semantic_relation` is `exact_materialization | equivalent_rewrite | content_preserving_extension |
landing_provenance | related_provenance | unknown`. `proof_status` is `verified | asserted | unverified |
indeterminate | refuted`. Only verified exact, equivalent, or extension evidence authorizes the matching
content-qualified presentation. Asserted landing/related provenance is useful attribution but never diff
substitution or assessment expansion.

The attestation ID is the canonical hash of every field above except itself. Writer and track stay envelope
provenance and do not fragment otherwise identical evidence. Different results coexist; the projection
surfaces `conflicting` and withholds stronger authorization rather than choosing by time. Missing
association, generation, or evidence members are accepted with self-healing diagnostics.

### D5. Cross-generation continuity uses `ReviewFactPorted`

Add `ReviewFactPorted` with wire name `review_fact_ported`:

```text
ReviewFactPortedV1 {
  port_id,
  origin_generation: GenerationRefV1,
  origin_fact: FactRefV1,
  target_generation: GenerationRefV1,
  relation: context_only | reanchored_as | carried_open_as | resolved_by,
  target_fact: FactRefV1?,
  rationale_content_hash?,
}
```

`context_only` presents the origin as context without creating a target fact. `reanchored_as` requires a
separately recorded target fact with its own anchor. `carried_open_as` requires a new open target-generation
request. `resolved_by` records that a later generation resolved an origin request/finding while the response
remains on its original request.

The port ID is per-writer and per-track: it hashes the canonical payload material plus writer actor and
track. Competing ports therefore coexist with preserved attribution. A port never moves the origin anchor,
makes an origin fact current, changes supersession, or transfers validation or assessment. Missing members
are diagnosed and may self-heal on backfill.

### D6. Exact generation, thread, association comparison, and interdiff are separate resources

Readers expose four distinct identities:

- captured generation: `GenerationRefV1`;
- thread/current selection: a derived thread key plus its exact generation set, with no content bytes;
- associated commit comparison: generation, association, commit, base/parent, and surviving attestation;
- interdiff: from-generation, to-generation, comparison kind, algorithm version, and scope.

Routes, DTOs, cache keys, labels, and editor document URIs keep these identities separate. An open exact
generation resource never serves different bytes. Native diff remains derived and requires two exact,
complete, immutable textual endpoints; live worktree bytes, reconstructed hunks, normalized patch IDs, and
plain associations are not endpoint proof.

### D7. One logical capability cohort gates first use

Before the first new event is written, every production reader/writer, bundle tool, repair/migration tool,
Inspector client/server, editor extension, and review-loop skill must advertise and require at least:
`review_continuation_v1`, `commit_relation_attestation_v1`, and `review_fact_port_v1`. An incomplete reader
fails before projection or import. Ignoring a relationship is not forward compatibility because it changes
thread membership, evidence authorization, or acceptance currency.

Legacy-byte preservation is mandatory during migration: existing events, IDs, hashes, signatures, and
attribution remain historical evidence. Legacy associations remain `unknown + unverified`. There is no
semantic backfill from ancestry, path overlap, timestamps, or census classes; stronger history is appended
only when reproducible evidence exists.

## Consequences

### Accepted

- Each review judgment has one immutable, auditable content boundary.
- Replacement forks and non-replacing continuations remain visible without a mutable review parent.
- Association history, content proof, and assessment scope are no longer conflated.
- Agent loops must switch generation after content edits, rerun validation, and obtain a new assessment.
- The logical cohort can be specified independently of physical storage while still cutting over together.

### Rejected

- Same-revision fact accretion after reviewable edits.
- A mutable stable review object that owns facts or bytes.
- Inferred continuation, landing, equivalence, content coverage, or acceptance.
- Copying validation or assessment to a successor.
- Serving associated-commit or live-worktree bytes under an exact generation identity.

## Revisit Triggers

- A demonstrated need requires a fact-port relation beyond the four fixed variants.
- A content-unchanged assessment-reuse policy can be specified without changing the stored target or base
  strict-default acceptance rule.
- Generation or thread projections become hot enough to justify a derived cache; the cache remains
  rebuildable and non-authoritative.
