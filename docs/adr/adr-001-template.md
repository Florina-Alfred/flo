# ADR 001 — Template

> Copy this file to `adr-NNN-<slug>.md` for each new decision. Keep ADRs short,
> immutable once accepted, and linked from `docs/agents/notes.md` if needed.

## Status

- Proposed | Accepted | Superseded by ADR-NNN | Deprecated
- Date: YYYY-MM-DD
- Authors: @handle
- Deciders: @handle

## Context

What is the issue or opportunity? What forces are at play (tech, cost, safety,
supply chain)? Link the originating issue/PRD.

Example: “We need to choose a mesh transport. Zenoh vs MQTT. Issue #123.”

## Decision

What we decided, in the present tense. Be precise enough that a future reader
can reimplement without asking.

Example: “We will use Zenoh PUT/SUB + liveliness tokens over a mesh (see
`src/transport.rs`, `src/topic.rs`). No Zenoh Queryable.”

## Consequences

- Positive / negative consequences.
- What becomes harder or easier?
- Security / safety implications (e.g. `#[forbid(unsafe_code)]`, `FLO_HEALTH_ADDR`
  defaults, fail-closed engine `src/engine.rs:72-74`).

## Alternatives considered

- Alternative A — why not.
- Alternative B — why not.

## Links

- Issue / PRD: #
- Relevant code: `src/...`
- Related ADRs: `adr-000-record.md` if you create an index.

---

*This template lives in `docs/adr/` as required by `AGENTS.md` § Domain docs.
See also `docs/agents/domain.md` and `CONTEXT.md`.*
