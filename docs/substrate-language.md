# Substrate Language

## Status

Stable internal language. Shore's substrate framing is supported by the code-review domain and one
agent task-supervision prototype. That is enough to use this vocabulary in source docs, architecture
discussion, and code review; it is not a claim that the substrate generalizes to every possible
software-work domain.

This document names the internal coordination vocabulary. It does not authorize a separate
substrate crate, SDK, product surface, or public API. User-facing command names and output documents
should stay domain-named.

## Purpose

Shore is built around durable software work objects. Different actors can read, write, and interpret
facts about those objects without requiring one central controller to decide the workflow.

The substrate language keeps that architecture legible. It separates:

- substrate concepts, which describe the durable shared medium and its coordination rules;
- domain concepts, which describe the current product shape, such as review units or task attempts;
- product language, which should stay clear to users and should not expose implementation jargon.

The important line is naming versus factoring. Naming the substrate is a vocabulary discipline.
Factoring it into a separate module or product boundary is a later architectural decision that needs
its own design and review.

## Core Model

Shore coordinates through these pieces:

- **Append-only event log.** Recorded facts are immutable. Corrections, supersession, retraction,
  and resolution are new facts, not edits to old facts.
- **Work objects.** Durable subjects that actors coordinate around. A review unit is one work
  object. A task attempt is another.
- **Targets within work objects.** A file range, observation, or task checkpoint can be addressed
  without becoming a peer work object.
- **Actor-attributed assertions.** Events are claims by actors, not unqualified global truth.
- **Purpose-built projections.** Read views derive from the log for a specific purpose. They can
  summarize, group, flag ambiguity, and apply explicit policy.
- **Named coordination policy.** Interpretive, attention, and executive policy should live in
  locatable code and docs, not as hidden defaults spread across readers.

## Substrate Concepts

| Term | Meaning |
|---|---|
| event | An atomic recorded fact. Authoritative as a record, not necessarily as world truth. |
| event log | The append-only stream of events. |
| projection | A derived read view over events for a specific purpose. |
| work object | A durable thing actors coordinate around, with stable identity and captured state. |
| target | A more specific address inside a work object. |
| actor | Any participant: human, coding agent, import pipeline, automation, or tool. |
| assertion | An attributed claim about a target. |
| advisory | Assertion mode meaning non-binding by default. |
| operative | Assertion mode meaning binding only under explicit projection policy. |
| stale | An action against captured state that has moved since the actor read it. |
| fingerprint | Opaque identity for a captured state. Equality is meaningful; internal structure is not. |
| lineage | A predecessor relationship between related captures. |
| provenance | Source information for a fact or imported assertion. |
| evidence | Material that supports an assertion, such as test output, logs, traces, or artifacts. |
| supersession | A new event that replaces an earlier event without mutating it. |
| retraction | A new event that withdraws an earlier event without mutating it. |
| representative selection | Stable choice of one representative for duplicate semantic facts. |
| projection diagnostic | A first-class signal that a projection saw ambiguity, conflict, drift, or data loss. |
| ambiguity preservation | The discipline of carrying disagreement as explicit state instead of picking a winner. |
| derived attention state | Projection output used to guide attention, not to authorize or block writes. |

## Policy Vocabulary

| Term | Meaning |
|---|---|
| interpretive policy | Rules that define what events or projection values mean. |
| attention policy | Rules that decide what is surfaced, hidden, grouped, sorted, or highlighted. |
| executive policy | Rules that decide what is allowed to proceed or treated as operative. |
| coordination semantics | The complete set of rules by which actors coordinate through the substrate. |

Executive policy is allowed, but it must be explicit and locatable. A projection that answers "may
this actor continue?" is acceptable when the rule is named, testable, diagnostic-rich, and derived
from events. A scheduler, hard lease system, write gate, or hidden global current-state field is a
different architecture.

## Current Domain: Code Review

| Domain term | Substrate concept | Notes |
|---|---|---|
| review unit | work object | Captured working-tree state for review. |
| review observation | assertion | An attributed claim about a review unit or target inside it. |
| review assessment | assertion | A formal review-domain call. |
| review note | imported assertion | A note imported from an external review source. |
| intervention | assertion / input request | A durable request for attention, decision, or explicit response. |
| intervention resolution | assertion / response | A durable answer to an intervention. |

Review-domain terms remain correct in command names and user-facing JSON. Do not rename user-facing
surfaces to substrate terms just because the underlying pattern generalizes.

## Current Prototype Domain: Task Supervision

| Domain term | Substrate concept | Notes |
|---|---|---|
| task attempt | work object | A bounded attempt by an agent to perform a software task. |
| checkpoint | target within a task attempt | A captured assistant-turn boundary, not a peer work object. |
| task observation | assertion | An attributed claim or captured signal about a task attempt or checkpoint. |
| resumption projection | executive-policy projection | A read view that decides whether an agent may resume under explicit rules. |

The task-supervision prototype validates that these shapes can reuse the same event-log and
projection pattern without adding scheduler, lease, or controller primitives. It does not by itself
authorize a public task command surface.

## Carry-Forward Rules

- Keep substrate vocabulary internal unless a user-facing term is clearly better than the domain
  term. Prefer `shore review unit show` over exposing "projection" in the command surface.
- Preserve ambiguity in projections. Multiple distinct facts should surface as ambiguous or
  diagnostic state, not disappear behind timestamp tie-breakers.
- Treat event IDs and filenames as storage addresses, not causal order. Use explicit fields for
  semantic ordering or lineage.
- Keep advisory and operative separate. Stored assertion mode is an actor's claim; treated-as-
  operative is a projection rule.
- Keep resource claims advisory by default. See
  [ADR-0003](adr/adr-0003-agent-resource-claims-advisory-first.md).
- Compare fingerprints as opaque values. Equality is the stable contract; domain-specific parsing
  belongs outside the substrate rule.
- Add substrate primitives only when a real projection or writer needs them. Do not fill out a
  speculative platform vocabulary.

## What This Does Not Authorize

- A separate substrate package, crate, or SDK.
- A scheduler, lease manager, daemon broker, or write-side gate.
- A global "current task" or "current actor" field for domains where ambiguity is normal.
- Productizing task supervision or any additional domain.
- Renaming existing review-domain commands simply to match the substrate vocabulary.
