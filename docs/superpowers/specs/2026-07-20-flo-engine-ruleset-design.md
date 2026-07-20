# Design: `flo-engine` Cloud Rule-Server & Authenticated Robot Clients (#72–#77)

**Status:** Approved design, pending implementation plan
**Source PRD:** `docs/superpowers/prd-flo-engine.md` (wayfinder map #63; decisions #64–#70)
**Scope:** Tickets #72–#77, built on the existing `flo` crate (`engine.rs`, `rules.rs`,
`semantic.rs`, `transport.rs`, `config.rs`, `auth.rs` from #71).

This is ONE cohesive design doc with one section per ticket. The tickets share a
single data model (`Ruleset → Rule → Trigger/Predicate`) and a single Zenoh topic
contract (PRD §5), so they are specified together rather than as disjoint specs.

---

## A. Shared data model (foundation for #72/#73)

The runtime evaluation type today is `rules::Rules` (a flat `Vec<Rule>`), where each
`Trigger` carries `pred: Option<String>` evaluated by `engine::eval_predicate` over a
free-text grammar. This design replaces the free-text predicate with a **typed
`Predicate` tree** and wraps `Rules` in a **`Ruleset` envelope**.

### New types in `src/rules.rs`

```rust
// #73 — typed predicate tree (replaces free-text Trigger.pred)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Predicate {
    Comparison { op: Op, lhs: Operand, rhs: Operand },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op { Eq, Ne, Lt, Gt, Le, Ge, SameZoneAs }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Prim(PrimitiveRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimitiveRef { Site, Zone, Robot, Proximity(String), HumanPresence }

// #77 — evaluation mode per trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EvalMode { #[default] Edge, Level }
```

### Revised `Trigger` and new `Ruleset`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub topic: String,
    #[serde(default)]
    pub pred: Option<Predicate>,   // was Option<String>
    #[serde(default)]
    pub mode: EvalMode,            // #77, default Edge
}

// #72 — outer envelope wrapping the existing runtime Rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ruleset {
    pub ruleset_name: String,      // unique on server; normalized [a-z0-9-]{1,64}
    pub version: u64,              // server-owned; client may hint, never authoritative
    pub robot_owner: String,       // canonical robot_id
    pub rules: Vec<Rule>,          // existing Rule type, unchanged shape
}
```

`Rule`, `When`, `Action`, `Qos` are unchanged. The existing flat `Rules`
(`{ rules: Vec<Rule> }`) is **retained as the internal runtime eval type** the
engine subscribes against and evaluates; `Ruleset` is the **wire + storage unit**
and wraps it: `Ruleset { ..., rules: Vec<Rule> }` where `Vec<Rule>` is exactly
the former `Rules.rules`. `config::RuleStore::current()` returns the `Ruleset`
(or a compiled `Rules` view of it) for `engine::run_engine`.

**Decision:** keep `Rules` as the runtime eval type; `Ruleset` is a superset
envelope adding ownership/version metadata (`ruleset_name`/`version`/`robot_owner`)
without disturbing the evaluation path. `SemanticRuleset::compile()` yields a
`Ruleset`; the `Vec<Rule>` inside is what `engine.rs` already evaluates.

---

## B. #72 — Ruleset envelope + TOML schema (primitive-only validation)

**Authoring schema (TOML):**

```toml
ruleset_name = "acme-site-a-fleet"   # unique on server; collision => reject (#68)
version = 3                           # monotonic hint; server is authoritative
robot_owner = "robot/7"               # sole writer / re-publisher

[[rule]]
rule_name = "slow_near_human"
when.in_zone = "zone_1"               # primitive: string
when.near_human = 1.2                # primitive: float
when.human_presence = true            # primitive: bool
[[rule.actions]]
topic = "robot/7/local/drive"
qos = "reliable"
payload = { speed_mps = 0.3 }        # PRIMITIVE payload only
```

**`src/semantic.rs` changes:**

- Add `SemanticRuleset` (top-level `ruleset_name` / `version` / `robot_owner` +
  `[[rule]]` array reusing today's `SemanticRule`, with `name`→`rule_name`).
- `compile()` now yields `Ruleset` (wrapping the compiled `Rules`).
- Author-time validation: reject non-primitive action payloads (anything that is not
  bool/int/float/string) — complex payloads remain the publisher's concern
  (PRD §3). Normalize `ruleset_name` (`[a-z0-9-]{1,64}`, lowercased); invalid
  names surface as `BadRequest`, not `Conflict`.
- `Ruleset::from_toml` / `Ruleset::to_toml` mirror the existing `Rules` helpers.

**Success criteria:** a TOML doc with the envelope parses to `Ruleset`; a doc with a
non-primitive payload or duplicate/garbage `ruleset_name` is rejected at author time
with a clear `SemanticError`.

---

## C. #73 — Predicate grammar (typed `on` + parsed `Predicate` tree)

The free-text `pred: Option<String>` and the string evaluator in `engine.rs` are
**removed**. `SemanticWhen` fields compile directly into the `Predicate` tree:

| `SemanticWhen` field        | Compiled `Predicate`                                      |
|----------------------------|-----------------------------------------------------------|
| `in_zone = "z"`           | `Comparison { op: Eq, lhs: Prim(Zone), rhs: Str("z") }` |
| `not_in_zone = "z"`       | `Not(Comparison { Eq, Zone, "z" })`                      |
| `near_human = 1.2`       | `Comparison { Lt, Prim(HumanPresence), Float(1.2) }`*    |
| `proximity(robot_id) < x` | `Comparison { Lt, Prim(Proximity(id)), Float(x) }`       |
| `role = "r"`              | `Comparison { Eq, Prim(Robot), Str("r") }`               |
| `all: [...]`              | `And([...])`                                              |
| `any: [...]`              | `Or([...])`                                               |

\* `near_human` reads the human-presence/separation signal; exact primitive binding
is resolved against the PRD §5 topic contract (§D).

**`src/engine.rs` changes:**

- `eval_predicate(pred: &Option<Predicate>, payload: &Value) -> bool` walks the
  enum. `None` ⇒ always true. `Comparison` evaluates `Operand` against the JSON
  payload: `Prim(p)` resolves the field name from the PRD §5 topic the trigger
  subscribes to; literals compare directly. Float equality uses epsilon.
- `Op::SameZoneAs` is a semantic comparison (two robot IDs share a zone) evaluated
  against the zone topic; if unresolvable it fails closed (returns false), never
  fail-open.
- Grammar is non-Turing-complete, deterministic, O(1) per trigger, statically
  auditable (PL d / SIL 2 fit per PRD §4).

**Success criteria:** a `Predicate` tree evaluates identically to the prior
string grammar for the covered operators; unparseable/unsupported cases fail closed
(logged), never silently pass.

---

## D. #74 — Topic contract + hybrid p2p/cloud-router topology

**Adopt PRD §5 as the canonical namespace** (replacing the legacy
`fleet/{site}/{robot_id}/state` / `stop/fleet/cmd` scheme emitted by today's
`semantic.rs`):

| Key-expression                                | Meaning                                  | QoS |
|----------------------------------------------|------------------------------------------|-----|
| `robot/{robot_id}/local/{signal}`            | robot state: `proximity`(f),`zone`(s),`pose`(f×3),`bumper`(b),`human_present`(b),`battery`,`velocity` | 1 (state) / 2 (lidar) per signal |
| `site/{id}/{event}` / `zone/{id}/{event}`   | `entered`,`human_present`,`cleared`      | 1   |
| `fleet/{cmd}` / `robot/{id}/cmd`            | engine→robot actions                      | 1   |
| `fleet/{site}/ruleset/{ruleset_name}`       | ruleset publish/subscribe (added for #75) | 1   |

**`src/semantic.rs` / `src/engine.rs` / `src/transport.rs` changes:**

- `semantic.rs` `expand_when` emits the PRD §5 topics (e.g. `robot/{id}/local/zone`,
  `site/{site}/human_present`, `robot/{id}/local/proximity`) instead of the legacy
  `fleet/{site}/{robot_id}/state`.
- `engine.rs::collect_topics` / subscribe loop unchanged in shape — it already
  subscribes to one key per distinct `Trigger.topic`. The new topics flow through
  automatically. `EvalMode::Level` triggers re-evaluate every 50 ms tick
  (existing tick); `EvalMode::Edge` fires on payload transition only.
- `transport.rs` gains the new key-expression constants (incl.
  `fleet/{site}/ruleset/{name}`).

**Hybrid topology (deployment concern, noted not coded here):** robots run Zenoh
`peer` mode in a zero-hop local mesh AND connect to the cloud `flo-engine` router as
an alternate path. Default delivery is p2p; on partition the cloud-router bridges
the sample (Zenoh de-dupes by key+source). The router never sits in the hot p2p
path. This is configured via `auth::zenoh_config` + the #71 ACL; no new runtime
code beyond the server-mode flag (§H).

**Subscription scoping:** the engine collects every distinct `Trigger.topic` from the
loaded `Ruleset` and declares exactly one subscriber per key (wildcards collapse via
`.intersects()`). Combined with the §2 ACLs, a robot sees only its own
`/robot/<id>/**` plus the site/zone topics its rules reference.

---

## E. #75 — Registry + collision + audit/SHA (SQLite WORM store)

**New module `src/registry.rs`** backed by **SQLite** (new dependency — requires
admin approval per AGENTS.md before implementation).

**Ownership-exclusive registry:**

- `ruleset_name` globally unique, owned by exactly one authenticated `robot_id`.
- New name → insert. Same owner re-push → **UPDATE**. Different owner →
  **REJECT-WITH-CONFLICT**.
- Server is the single writer of `version`; bumped **only on a SHA change**
  (idempotent no-op pushes accepted, not recorded).
- Owner reassignment requires explicit `release(ruleset_name)` (or admin revoke) by
  the current owner — never inferred.
- Races resolved under a single registry lock / atomic CAS (first writer wins).
- Delete → **tombstone**, not erase.

**Audit copy + SHA:**

- **Per-ruleset SHA-256** over canonical serialized `Ruleset` (whole doc).
- **Per-rule SHA-256** over each `Rule` (canonical per-rule serialization) for
  *which-rule-changed* diagnosis and MOC evidence.
- Canonical/deterministic serialization (field order fixed, no PrettyPrint) so
  semantically-equal rulesets hash equal.
- **On mismatch** (robot's reported `ruleset_sha` ≠ server's for owner+name):
  **alert first, always** (log + fleet monitor/ops); never silently re-sync.
  Newer same-owner copy → managed update. Divergent/unowned/unverifiable →
  **quarantine** (keep last-good audit copy, flag robot out-of-policy, robot
  falls back to fail-safe).
- **Storage:** persisted **append-only / WORM** SQLite table (each accepted/rejected
  push = one row: name, owner, version, sha, timestamp, full ruleset blob, status)
  **+ hot in-memory index** `HashMap<name, (owner, version, sha)>` rebuilt from the
  log on startup. In-memory-only is explicitly rejected (PRD §6: defeats ISO
  3691-4 re-verification).

**`version` vs SHA:** `version` is human-facing monotonic (client bumps on
intentional change); SHA-256 is content identity (detects tampering/drift). Server
stores both; on push recomputes SHA and compares to the prior SHA for that
`version`.

**On invalid push:** alert + keep last-good + WORM audit row recording the rejection
(reason + timestamp + claiming `robot_id`); good ruleset's audit copy NOT
overwritten.

---

## F. #76 — Hot-reload + failure modes

**Flow:** owning client republishes the whole `Ruleset` to
`fleet/{site}/ruleset/{ruleset_name}`. Server:

1. Validates (TOML parse + primitive-only + name normalization + ownership check).
2. Computes SHA-256 (per-ruleset + per-rule).
3. On success: atomically swaps `RuleStore::current()` (engine re-ticks against the
   new set), bumps `version` only if SHA changed, writes WORM audit copy + SHA.
4. On failure: **reject, keep last-good, log on BOTH client and server** (client
   logs locally with reason; server writes to the audit trail). No partial apply, no
   swap-on-invalid.

Invalid push ⇒ reject + keep-last-good + log both sides (PRD §6). The client must
be able to receive the rejection reason over `fleet/{cmd}` / `robot/{id}/cmd` and
record it locally.

**Success criteria:** a valid republish swaps the live ruleset with zero dropped
evaluations beyond the atomic boundary; an invalid republish leaves the prior
ruleset active and produces audit rows on both sides.

---

## G. #77 — Rule-passing event semantics (edge vs level)

**Pinned default (PRD §1 fog resolved):**

- **Edge-triggered** for boolean / entry-exit events (`zone` entered, `human_present`
  toggled): the action fires once on the *transition* (payload changes from the
  non-matching to the matching state). Implemented by tracking the last-seen payload
  per `(trigger, topic)` and firing only on change.
- **Level-evaluated** for continuous numeric state (`proximity(r) < x`, `speed`):
  re-evaluated on every 50 ms engine tick against the latest held sample.

`Trigger.mode` carries the choice; default is selected by primitive type at compile
time (`SemanticWhen` → `Trigger.mode`), but is overridable in authoring. This is
recorded explicitly in the spec so the choice is auditable.

**Success criteria:** a `zone` entry rule fires exactly once on entry and once on
exit (edge); a `proximity < x` rule holds its action continuously while true
(level).

---

## H. Server process (#71 follow-up)

**Single `flo` binary, `--mode server` subcommand:**

1. Build the Zenoh **router** session from `auth::zenoh_config(&robot_id)`
   (mTLS + the #71 per-robot ACL). The router enforces the 1:1
   cert(SAN)=robot_id namespace scoping.
2. Start the rule-evaluation engine **in-process** (`engine::run_engine`) against a
   `RuleStore` backed by the #75 `registry` (SQLite WORM).
3. The router and eval are co-located in one process; the router is the Zenoh
   session config (not a separate binary).

Robot (client) mode is the existing peer/loopback path with `auth::zenoh_config`
wired in (#71). No second binary is introduced.

---

## I. Sequencing & dependencies

```
#72 envelope ─┐
#73 predicate  ├─▶ #74 topic contract ─▶ #75 registry/audit (SQLite)
#77 event sem  ┘            │                      │
                           │                      ▼
                           └──────────────▶ #76 hot-reload (consumes #74 + #75)
#71 (done) ───────────────▶ #H server mode (consumes #71 + #74 + #75)
```

Each ticket is independently implementable and testable; the order above is the
recommended implementation order.

## J. Open dependencies / gates

- **SQLite dependency (AGENTS.md):** #75 requires a new crate (e.g. `rusqlite` or
  `sqlx`). This needs admin approval before any #75 implementation work begins.
- **Cert SAN → robot_id extraction (#71 follow-up):** the server extracts the
  client cert SAN to derive `robot_id` for the ACL. This needs a minimal DER walk
  (no `x509-parser` dependency unless approved). Tracked as a pre-requisite for
  full #H enforcement; the ACL builder (#71) already accepts `robot_id`.
- **Hybrid topology** is a deployment configuration concern, not a code block in
  this design; it is satisfied by `auth::zenoh_config` + the #71 ACL.
