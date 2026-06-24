# ADR-0021: Inspector Server-Side Projection Direction (Classifications, DAG Layout)

**Status:** Accepted
**Date:** 2026-06-23
**See also:** the `shore inspect` redesign research (the inspector design-direction synthesis,
q2-supersession-dag-viz, q8-feasibility-and-delivery). Relates to: **ADR-0003** (advisory-first),
**ADR-0017** (object/Engagement identity layering; the `shore inspect` substrate-vocabulary exception,
§A4 viii), **ADR-0018** (event-borne supersession; fork-tolerant competing heads), **ADR-0019**
(blackboard liveness; no-runtime / pull-only), **ADR-0020** (durable-storage seam; the truth-vs-
projection split).

## Context

The inspector is a std-only, synchronous, GET-only, thread-per-connection server
(`src/cli/inspect/server.rs`) over a handful of `serde_json` payload builders
(`src/cli/inspect/api.rs`) that each reuse a validated `shoreline::session` projection. The served
frontend is `include_str!`'d vanilla JS/CSS — no framework, no bundler, no npm, no build step, and no
JS test harness. It is a localhost developer tool, not a production server.

The asymmetry the redesign research surfaced is the crux of this decision: the **frontend is
hard-constrained** but the **server is only soft-constrained**. The frontend cannot grow a bundler, a
graph library, a WASM module, or a JS test harness without leaving the feasibility envelope; the
server, by contrast, already carries `serde`/`serde_json`/`sha2`/`ed25519-dalek`/`ratatui` and can add
a pure-synchronous Rust dependency and additive `/api/*` fields cheaply. The cheapest lever for
ambitious visual design is therefore to **compute more in Rust and keep the client thin** — a
projection of the model rather than a second place where the model is re-derived.

Two consequences of today's client-side derivation motivate the shift:

- **The per-revision supersession classification is re-derived in the browser on every render.** The
  client recomputes, from the edge maps the objects payload ships, whether each revision is a head, is
  superseded, and by which superseding successors — work the server already has in hand from the
  `SupersessionView` projection.
- **The supersession DAG — the model's signature view (ADR-0018) — renders as a flat node list with
  textual edges.** A fork is indistinguishable from a chain except by reading chip text. The DAG is the
  model's strongest concept and its weakest rendering.

Client-side derivation is also why the inspector's UI tests grep `/app.js` source for function names
and color literals: when the meaning lives in the browser, there is nothing server-assertable to test,
so the tests reach into implementation strings — brittle and refactor-fragile.

The DAG needs a real graph layout: layered ranks plus routed edges with crossing reduction. Doing that
layout in the browser would mean vendoring a graph library (a fourth, larger asset) or a Mermaid bundle
— both outside the envelope, and both moving layout into the untestable JS surface. The owner publishes
`mmdflux`, a pure-Rust layered-layout and orthogonal-edge-routing engine, which can compute the layout
server-side and ship the client only geometry to paint.

## Decision

### D1. Derivations move into `/api/*` payloads; the client is a thin painter

Derivations the client used to compute — the per-revision supersession classification, the
supersession-DAG geometry, and later facet counts — are pre-computed in `api.rs` and serialized as
**additive** payload fields (existing fields stay byte-unchanged). The client reads a field instead of
re-deriving from edge maps. This shrinks the untestable JS surface, makes each derivation assertable as
JSON over the existing HTTP harness, and honors the project ethos that the UI is a projection of the
model, not a parallel source of truth.

The per-revision supersession classification (head / superseded / isolated, plus its superseding
successors and its predecessors) is the first such field. It is computed from the `SupersessionView`
the objects builder already holds, so the client stops recomputing head/superseded/edge status from the
edge maps per render.

### D2. The supersession-DAG layout is computed server-side via `mmdflux`

The DAG layout is built in `src/cli/inspect/api.rs` over the per-component edges already on the
`SupersessionView` projection, via the published facade `mmdflux::layout::layout_graph(&Graph,
&LayoutOptions)` (added in `mmdflux` 2.6.0). The client is a **pure SVG painter** of the emitted
geometry: no client-side layout math, no client graph library, no Mermaid round-trip, and no id
aliasing (opaque revision-id node ids round-trip verbatim).

The core library's `SupersessionView` stays engine-free. `mmdflux` is a crate-wide dependency used
**only** in the inspector binary (`api.rs`); it never enters the core model. The layout is a binary-tier
concern, kept out of the library so the model carries no presentation engine.

### D3. The dependency is pure-synchronous and runtime-free (ADR-0019 pull-only)

The dependency is recorded as `mmdflux = { version = "2.6", default-features = false }`. It adds no
async runtime, no daemon, and no push primitive, so it is compatible with ADR-0019's strictly pull-only
rule. ADR-0019 D3 states that the Shoreline core "stays **strictly pull-only**: every read is a pure
function of the store, with no callbacks and no in-memory subscriber set," and that "**No push
primitive enters the core** — no `notify`/`inotify`/`tokio`/subscriber-registry."

The layout call honors that rule exactly: it is a pure function of the projection, invoked synchronously
inside an existing GET handler. It introduces no runtime, no callback, and no subscriber state — it
reduces the supersession edges to geometry the same way every other inspector reduction reduces the
event log to a view, on the pull.

### D4. The layout is a derived, regenerable projection (ADR-0020 truth-vs-projection split)

The laid-out geometry is never durable truth. It is a function of the event log's supersession edges,
re-derived on every request, holding no state between calls. ADR-0020 draws the line this sits on: the
durable truth layer (the journal and content) stays plain-text, diffable, and hash-validated, while
**derived, regenerable projections** — `state.json`, the on-the-fly history/show/revisions/inspect
reductions — are where computed and structured views live, "derived and rebuildable from the journal."
The DAG layout sits squarely in that projection layer: nothing about it is persisted, signed, or
hashed; deleting it changes no truth and regenerating it is a pure replay.

### D5. The coordinate contract (what the facade returns and how the wire maps it)

The facade's canonical return type and the inspector's additive wire shape are recorded here so the ADR
and the implemented payload cannot drift.

The facade returns:

```text
LaidOutGraph {
  nodes: Vec<LaidOutNode { id, center: FPoint, width, height }>     // sorted by id
  edges: Vec<LaidOutEdge { from, to, points: Vec<FPoint>, is_backward: bool }>  // input order
  width, height
}
```

at `geometry_level: Routed` (drawable polylines). `is_backward` is an engine cycle-removal flag.
Shoreline **does not** surface `is_backward` on its wire: the client orients arrowheads by the `from`
and `to` node centers, not by `points` order, so reversed and cycle edges still render correctly without
it.

`api.rs` builds the graph `Direction::TopDown`, one node per revision in the component (node id = the
revision-id string, round-tripped verbatim, no aliasing), and one edge `B → A` for each `A` that `B`
supersedes (so `from` **supersedes** `to`). It calls the facade once per connected component.

The additive wire field on each `/api/objects` thread document:

```jsonc
"laidOut": {
  "nodes": [ { "id": "<revisionId>", "x": <f64>, "y": <f64>, "w": <f64>, "h": <f64>,
               "isHead": <bool>, "isSuperseded": <bool> } ],
  "edges": [ { "from": "<revisionId>", "to": "<revisionId>", "path": [[x,y], ...] } ],
  "bounds": { "w": <f64>, "h": <f64> }
}
```

Mapping: `x,y` = `LaidOutNode.center.{x,y}` (a **center**, never a corner); `w,h` =
`LaidOutNode.{width,height}`; `path` = `LaidOutEdge.points` flattened. `isHead` and `isSuperseded` come
from the `SupersessionView` projection, **not** from `mmdflux`.

**Origin policy (load-bearing).** `mmdflux`'s `width`/`height` are extents whose bounds are not
guaranteed to start at the origin, so `api.rs` normalizes the geometry to a `(0,0)` top-left over the
content bounding box (node boxes plus edge points) before serializing, and emits `bounds.{w,h}` as the
**normalized content extent** (`max − min`), **not** `LaidOutGraph.{width,height}` directly. After
normalization every node box and every edge point lies within `0..bounds.w` × `0..bounds.h`, so the
client paints into `<svg viewBox="0 0 bounds.w bounds.h">` with no clipping. A single-node thread emits
one node and no edges.

### D6. No trunk / no winner by construction; advisory posture preserved (ADR-0018 / ADR-0003)

Every in-degree-0 head is an equal rank-0 peer. Node insertion is keyed by **revision-id sort** for
stable columns. No head is centered, bolded, or ordered-first as a primary. A fork draws as one rail
splitting into N equal rails, each topped by a head node; the neutral "competing revisions (N)" callout
is retained. This continues ADR-0018's fork-tolerant model: a fork surfaces every competing head as a
peer, never nulled, never with an auto-chosen winner.

The DAG is an additional advisory lens, consistent with ADR-0003's advisory-first posture. Node
interactions navigate — drilling into a revision — and never read as a gate: there is no approve, merge,
or proceed affordance anywhere on the graph. A cycle still withholds a headline.

### D7. Determinism and topology-only tests

Same events produce the same geometry: `mmdflux` is deterministic and the version is pinned. Contract
tests therefore assert topology and structure — the node set, edge `from`/`to`, `isHead`/`isSuperseded`,
thread membership, `competing`, and "no node sits above a head" — and **never** exact pixel coordinates,
which are a property of the pinned engine version rather than a stable contract. Supersession
diagnostics (`supersession_cycle`, `supersession_target_missing`) are preserved verbatim from the
projection.

### D8. Substrate-vocabulary exception

The inspector may surface substrate vocabulary (revision, supersession, engagement/thread) where the
primary `shore review` CLI and UI surface stays domain-named. ADR-0017 records this exception (§A4
viii): the wire rename that keeps substrate terms internal is "no primary `shore review` CLI/UI surface
change (the `shore inspect` substrate-inspector excepted — §A4 (viii))." The DAG view and the
per-revision classification field inherit that exception and may name revisions and supersession
directly.

## Consequences

### Accepted

- The DAG becomes the model's strongest rendering instead of its weakest: a fork is visually a fork.
- Layout and classification are testable as JSON over the existing HTTP harness; the brittle source-grep
  UI tests retire as their subject moves server-side.
- The client stays a thin painter inside the feasibility envelope: one pure-Rust dependency, no bundler,
  no npm, no WASM, no JS test harness.
- Competing heads stay first-class by construction; the advisory, never-a-gate posture is preserved.

### Costs accepted

- A new crate-wide dependency (`mmdflux`) used only by the inspector binary.
- `/api/objects` payload growth (the additive `laidOut` geometry).
- A dependency on the `mmdflux` 2.6.0 release for the `layout` facade (see Open Questions).

## Alternatives / Rejected

- **Hand-rolled column-rail lane assignment in `api.rs`.** Dependency-free, but it re-implements layered
  layout and edge routing — crossing reduction, dummy nodes, orthogonal routing — that `mmdflux` already
  provides and maintains. Rejected to avoid carrying bespoke layout math.
- **`rust-sugiyama` (a path/workspace dependency).** Pure-Rust and on-disk, but a local path dependency
  with no published release; its coordinates need post-layout x-snapping to keep competing heads
  visually equal, because it has no no-winner notion. `mmdflux` is the owner's published engine with the
  purpose-built `layout_graph` facade and ranks in-degree-0 heads as equal peers natively. Rejected in
  favor of the published, peer-equal-by-construction engine.
- **A client-side `dagre`/Mermaid bundle.** `dagre` is a roughly 68 KB vendored fourth asset; `mermaid`
  is a roughly 2.6 MB bundle. Both move layout into the untestable JS surface and break the
  no-fourth-heavy-asset / no-build-step rule; WASM (`@mmds/wasm`) is out for the same envelope reason.
  Rejected.
- **Server-rendered SVG (the server emits finished `<svg>` markup).** Surrenders client control of theme
  tokens, density, hover-to-trace, focus/keyboard wiring, and accessibility glyphs, and couples the
  server to presentation. Rejected in favor of emitting geometry the client paints with full theme and
  interaction control.

## Open Questions / Revisit Triggers

- **Dependency provenance.** The server-side layout uses `mmdflux` 2.6.0 (published to crates.io; the
  `mmdflux::layout::layout_graph` facade), pinned as `mmdflux = { version = "2.6", default-features =
  false }` — pure-synchronous and no-runtime (ADR-0019-compatible). If the facade's return shape changes
  in a later `mmdflux` major, the coordinate contract (D5) is the revisit point.
- **Coincident cross-thread content** (ADR-0018 C4) — whether the DAG view ever links threads that share
  an object (explicitly *not* `competing_revisions`). Flagged, deferred.

## Related Docs

- The `shore inspect` redesign research: the inspector design-direction synthesis,
  q2-supersession-dag-viz (the no-trunk / peer-heads layout invariant and the server-side pre-layout
  recommendation), q8-feasibility-and-delivery (frontend hard-constrained / server soft-constrained).
- In-repo `docs/adr/`: ADR-0003 (advisory-first), ADR-0017 (identity layering; the `shore inspect`
  substrate-vocabulary exception, §A4 viii), ADR-0018 (event-borne supersession; competing heads),
  ADR-0019 (no-runtime / pull-only), ADR-0020 (durable-storage seam; truth-vs-projection split).
