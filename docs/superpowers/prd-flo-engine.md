# PRD: `flo-engine` — Cloud Rule-Server & Authenticated Robot Clients

**Status:** Draft for implementation planning
**Source:** Wayfinder map #63 (decisions #64–#70) + research docs (`docs/superpowers/research/audit-sha.md` and the resolution comments on issues #64–#68)
**Scope:** A spec/contract for the `flo-engine` cloud rule-server and its authenticated robot clients. Reuses the existing `flo` crate transport (Zenoh) and `Engine` core — no separate transport. The cloud engine is a *server mode* (authenticated Zenoh router + rule-evaluation service); the robot is a *client mode*.

> **Safety posture (carried from `docs/RULES.md` and the industrial-rules plan):** `flo` is the *software pre-estop / coordination layer*. Hardware STO / certified Safety-PLC remains the primary stop authority. The audit/SHA machinery here is a *safety-management / assurance* artefact, not a safety function.

---

## 1. Destination & non-goals

**Destination.** A contract that lets an implementation agent build `flo-engine` ticket-by-ticket:

1. Client-authentication model (opt-out capable).
2. Ruleset/rule TOML schema with unique `ruleset_name` and per-rule `rule_name`, hot-reloadable, client-owned with a server audit copy + SHA.
3. The five rule primitives (site, zone, robot, proximity, human_presence) expressed by extending the existing `When`/`Trigger` model over **primitive-only** observed payloads.
4. The Zenoh key-expression topic/transport contract (p2p + cloud-router fail-safe).
5. The ruleset-name collision policy.

**Non-goals (out of scope, defers to a future effort):** client+server *mutual* rule-passing confirmation (independent dual evaluation reconciled by SHA); complex payload types in observed topics (publishing client's concern); iroh-ed25519 auth upgrade if mTLS is the default; media/WebRTC (already built, #59).

**Open fog (spec-authoring detail, not a blocking decision):** rule-passing *event semantics* — edge-vs-level trigger timing ("entered zone" fires once on edge or continuously while true?). The implementation agent should pin this with a default (recommended: edge-triggered for entry/exit events, level-evaluated for continuous state like `proximity < x`) and record the choice.

---

## 2. Authentication model

**Decision (#64, #69).** Default **mTLS** on the axis `auth: none | mtls | ed25519`. mTLS is Zenoh-native (config-only `enable_mtls`), adds **no new dependency** (`flo` already pulls `rustls` transitively via `zenoh 1.9`), encrypts by default, auto-expires links on cert expiry (Zenoh ≥1.0.3), and drives least-privilege topic ACLs via `cert_common_names` subjects — mirroring the ROS 2 DDS-Security / SROS2 X.509+PKI norm for functional-safety field robotics.

- **Why not ed25519 by default:** iroh-style ed25519 requires a custom nonce-signing handshake over Zenoh (new attack surface) or minting ed25519 into self-signed X.509 to reuse mTLS; adds a crypto crate needing admin approval. It remains a first-class *opt-in* for key-based fleets.
- **Opt-out (`auth: none`):** allowed only for loopback/air-gapped dev (same trust as today's `Transport::loopback_config`). Production must hard-block `none` unless an explicit `allow_insecure` + local-bind override is set.

**Identity binding (#69).** 1:1, server-enforced:
- `robot_id` = the client certificate **SAN** (e.g. `robot_id=robot_7` or SAN DNS `robot-7.flo.local`). Robot container provisioned at launch with a mounted client cert+key (secret storage in mounted volumes, **never baked into images**).
- Server trust = CA that issued robot certs (or an allowlist of authorized cert fingerprints). On TLS handshake the server extracts the SAN → canonical `robot_id` and applies a **Zenoh ACL permitting that client to publish/subscribe only under `/robot/<robot_id>/**`** (plus the site/zone topics its rulesets reference). A robot cannot impersonate another's namespace.
- `auth: none`: `robot_id` is taken from the ruleset's `robot_owner` field (or a `--robot-id` launch flag); **no topic-enforcement ACL** is applied. Explicitly dev/trusted-network only — the spec MUST warn `auth: none` gives no impersonation protection and must never be used on untrusted networks. Downstream identity logic is identical; only the *source* of `robot_id` and the *enforcement* differ.

**Spec must mandate:** per-robot identity at provisioning; a fleet CA / server allowlist; mounted secret storage; Zenoh ACL least-privilege mapping; rotation/revocation (cert expiry for mTLS; allowlist redeploy for ed25519).

---

## 3. Ruleset / rule TOML schema

**Decision (#65).** A single named, **client-owned** `ruleset` wraps many `rule`s, each with its own `rule_name`. Extends the existing `When`/`Trigger` model via `SemanticWhen` (`src/semantic.rs`) compiling down to `Trigger{topic,pred}` — **no engine change needed**; only an outer `Ruleset` envelope is added around `Rule` (rename `name` → `rule_name`).

```toml
ruleset_name = "acme-site-a-fleet"   # unique on server; collision => reject (#68)
version = 3                           # monotonic, bump on every client edit
robot_owner = "robot/7"               # sole writer / re-publisher

[[rule]]
rule_name = "slow_near_human"
when.in_zone = "zone_1"              # primitive: string
when.near_human = 1.2                 # primitive: float
when.human_presence = true           # primitive: bool
[[rule.actions]]
topic = "robot/7/local/drive"
qos = "reliable"
payload = { speed_mps = 0.3 }        # PRIMITIVE payload only
```

- **Primitive-only payloads** (bool/int/float/string) are native to `engine.rs::eval_predicate`. Validation rejects non-primitives at author time; complex payloads stay the publisher's concern.
- **Hot-reload (#65, #70):** owning client re-publishes the whole ruleset to `fleet/{site}/ruleset/{ruleset_name}`; server validates, atomically swaps `RuleStore::current()` (re-tick), keeps last-good on failure, stores an audit copy + SHA (#68). **Invalid push → reject, keep last-good, log on BOTH client and server** (client logs locally with reason; server writes to audit trail). No partial-apply, no swap-on-invalid.
- **Precedent:** ROS 2 param/launch (named wrapper + schema validation + runtime reconfigure), Behavior Trees (composable re-ticked guards), semantic safety filters, ISO 3691-4 / ANSI B56.5 (auditable behavior; zone/proximity/human-presence as first-class primitives).

---

## 4. Rule primitives & predicate grammar

**Decision (#66).** Rules keep the existing `When { all, any }` AND/OR shape. Each `Trigger` gains a **typed primitive reference** (`on`) plus an optional parsed `Predicate`, replacing today's free-text `pred: Option<String>` with a statically-auditable tree.

**Grammar (non-Turing-complete, deterministic, O(1), statically auditable — PL d / SIL 2 fit):**
```
Predicate      := Comparison (and | or | not)* Comparison
Comparison     := Operand OP Operand
Operand        := bool | float | int | string | primitive-ref
OP             := == | != | < | > | <= | >= | same_zone_as
primitive-ref  := site | zone | robot | proximity(robot_id) | human_presence
```
No arithmetic, loops, functions, or time-windows. Floats use epsilon equality.

**The five primitives:**
- `site` / `zone` — match published **edge events**: `/site_a/entered {robot}`, `/zone_1/human_present {human}`.
- `robot` — client **auto-subscribes** to `/robot/<id>/**` for robots named in its rules; predicates read that stream.
- `proximity(robot_id)` — reads the safety-rated separation float from that stream.
- `human_presence` — bool/event derived from zone/site topics.
- Observed values stay primitive (bool/float/int/string).

**Safety fit:** ISO 3691-4:2023 (operating zones, PDS, 0.3 m/s docking limit), ISO 10218-1 §5.10.4 / ISO/TS 15066 §5.5.4 SSM (`proximity(r) < const` = protective separation distance), ISO 13855, ISO 13849/IEC 61508, ANSI/RIA R15.08, ROS 2 nav2 costmap zone idiom. Zenoh key-expressions (`*`, `**`, canonical unicity) give an exact, auditable subscription set per client.

---

## 5. Topic / key-expression contract & topology

**Decision (#67).** Pinned Zenoh key-expression namespace (QoS per `flo`'s locked decision: class 1 Reliable / class 2 BestEffort):

- `robot/{robot_id}/local/{signal}` — robot state: `proximity` (float m), `zone` (string), `pose` (float×3), `bumper` (bool), `human_present` (bool), `battery`, `velocity`.
- `site/{id}/{event}` and `zone/{id}/{event}` — `entered`, `human_present`, `cleared` (string/bool, class 1).
- `fleet/{cmd}`, `robot/{id}/cmd` — engine→robot actions (class 1).
- Existing WebRTC signaling + liveliness + rules keys stay locked.

**Hybrid topology (fail-safe requirement):** robots run Zenoh `peer` mode in a **zero-hop local mesh** (multicast/gossip scouting) for lowest latency and to offload cloud. Each robot **also connects to the cloud `flo-engine` Zenoh router** as an alternate path. Default delivery is p2p; if a robot is partitioned from the peer mesh, the cloud-router subscription still bridges the sample through, and Zenoh de-dupes by key+source so rules fire once. **The router never sits in the hot p2p path.**

**Subscription scoping by ruleset:** the engine collects every distinct `Trigger.topic` from the loaded `Rules` doc and declares **exactly one subscriber per key** (wildcards collapse to one route entry via `.intersects()`); predicates filter locally in the callback. Hot-reload diffs the set. Result: least-privilege — a robot only pulls keys its active ruleset references. Combined with §2 ACLs, this enforces that a robot sees only its own `/robot/<id>/**` plus the site/zone topics its rules reference.

---

## 6. Ruleset-name collision policy & server audit/SHA

**Decision (#68, #70).** Ownership-exclusive namespace: `ruleset_name` is globally unique in the fleet registry, owned by exactly one authenticated `robot_id`.

**Collision / registry:**
- New name → insert. Same owner re-push → **UPDATE**. Different owner → **REJECT-WITH-CONFLICT**.
- Server is the **single writer of `version`** (client never supplies it), bumped **only on a SHA change** (idempotent no-op pushes accepted, not recorded).
- `ruleset_name` syntax normalized up front (e.g. `[a-z0-9-]{1,64}`, lowercased); invalid names → `BadRequest`, not `Conflict`.
- Owner reassignment requires explicit `release(ruleset_name)` (or admin revoke) by current owner — never inferred. Robot re-imaged with a new `robot_id` but same name = *different* owner → conflict (fleet identity must be stable across reboots/re-images).
- Races resolved under a single registry lock / atomic CAS (first writer wins). Delete `(ruleset_name)` only by owner/admin → **tombstone**, not erase. `push` before auth ⇒ `Unauthorized`.

**Audit copy + SHA:**
- **Per-ruleset SHA-256** over canonical serialized `Rules` (whole doc) — cheap always-checked guard.
- **Per-rule SHA-256** over each `Rule` (canonical per-rule serialization) — enables *which rule changed* diagnosis and MOC evidence.
- Canonical/deterministic serialization required so semantically-equal rulesets hash equal.
- **On mismatch** (robot's reported `ruleset_sha` ≠ server's for that owner+name): **Alert first, always** (log + fleet monitor/ops). **Never silently re-sync.** If robot's copy is newer and same-owner → managed update (ingest, bump version, recompute, record). If divergent/unowned/unverifiable → **quarantine** (keep last-good audit copy, flag robot out-of-policy; robot falls back to fail-safe). Keep-last-good for the audit record.
- **Storage:** **persisted append-only / WORM** (each accepted push writes a new versioned record: name, owner, version, sha, timestamp, full ruleset blob; deletes = tombstones) **+ hot in-memory index** (name → latest version+sha) as a reconstructable cache. In-memory-only loses the audit trail on restart, defeating ISO 3691-4:2023 re-verification/records and IEC 61508 Management of Change.

**What the audit is for (standards basis):** change traceability & MOC (IEC 61508 / IEC 61511); re-verification after any change (ISO 3691-4:2023); documented records for auditors/incident investigators (ISO 3691-4); forensic replay/reconstruction; tamper/unauthorized-modification detection (server copy = trusted reference). ANSI/RIA R15.08-2 (multi-robot coordination) is squarely in scope.

**Versioning (#70):** explicit `version` field (human-facing monotonic, client bumps on intentional change) is **complementary** to the SHA-256 fingerprint (content identity, independent of claimed version; detects tampering/drift). Server stores both; on push recomputes SHA and compares to the prior SHA recorded for that `version`.

**Server behavior on invalid push (#70):** alert + keep last-good + **WORM audit record of the rejection** (reason + timestamp + claiming `robot_id`); good ruleset's audit copy NOT overwritten. Quarantine-style: bad push isolated in audit, live fleet stays safe.

---

## 7. Implementation sequencing (suggested tickets)

1. **Auth/server mode skeleton** — `flo-engine` server mode: Zenoh router + mTLS link config + CA trust store; `auth: none` opt-out with warning. ACL scoping from cert SAN → `robot_id` (#64, #69).
2. **Ruleset envelope + TOML schema** — `Ruleset{ruleset_name,version,robot_owner}` + `SemanticWhen` → `Trigger` compile; primitive-only validation (#65).
3. **Predicate grammar** — typed `on` + parsed `Predicate` tree replacing free-text `pred`; epsilon float eq (#66).
4. **Topic contract + hybrid topology** — pinned key-expressions; peer-mesh + cloud-router fail-safe; per-ruleset subscriber scoping (#67).
5. **Registry + collision + audit/SHA** — ownership-exclusive registry, reject-with-conflict, version monotonic, per-ruleset/per-rule SHA, WORM store + in-memory index, mismatch→alert/quarantine (#68).
6. **Hot-reload + failure modes** — validate-before-swap, dual-side reject logging, `version`+SHA model, alert+WORM audit record of rejection (#70).
7. **(Authoring detail)** Pin rule-passing event semantics (edge vs level) — resolve during implementation.

---

## 8. Open items handed to implementation

- Rule-passing event semantics (edge vs level) — §1 fog.
- The four detailed research findings docs (`auth-mechanism.md`, `ruleset-schema.md`, `rule-primitives.md`, `topic-contract.md`) were reported by the research subagents but are **not present on their branches**; this PRD was synthesized from the recorded issue-resolution comments + `audit-sha.md`. Recommend regenerating those four docs or treating this PRD as the canonical source.
