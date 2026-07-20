# Research: Ruleset-name collision policy & server audit/SHA model

**Ticket:** #68 — "Ruleset-name collision policy & server audit/SHA model"
**Branch:** `research/audit-sha` (off `origin/main`)
**Scope:** (a) collision policy implementation + edge cases; (b) server audit-copy + SHA model, grounded in functional-safety standards.

---

## 0. Context from the codebase

`flo` compiles a semantic ruleset (`src/semantic.rs`) down to `rules::Rules`
(`src/rules.rs`), a serializable TOML document of `Rule`s. A **client** (one robot,
authenticated per ticket #64) **owns and drives** its ruleset; the **server** keeps a
copy used for audit. The server therefore needs:

1. A **registry** keyed by `ruleset_name` (the collision authority).
2. An **audit copy + integrity hash** per ruleset/rule (the tamper/change detector).

The `RobotId`/`robot_id` is already the fleet identity primitive (`src/main.rs`,
`src/mesh.rs`, `src/signaling.rs`) and is the natural owner key from the #64 auth
ticket.

---

## (a) Ruleset-name collision policy — implementation

### Decision (confirmed by user)

> The server **REJECTS WITH CONFLICT** if a *second, different* robot pushes a
> `ruleset_name` already registered. The **same** robot re-pushing the same
> `ruleset_name` is treated as an **UPDATE** (allowed).

This is an **ownership-exclusive namespace**: `ruleset_name` is globally unique in the
fleet registry, owned by exactly one authenticated robot identity.

### Data model

```
ServerRegistry {
    by_name: HashMap<RulesetName, Entry>
}
Entry {
    owner:     RobotId          // from #64 auth ticket
    ruleset:   Rules            // audit copy
    ruleset_sha:   Sha256       // whole-ruleset hash
    rules_sha:     Vec<Sha256>  // per-rule hash (optionally)
    version:   u64              // monotonic, bumped on update
    updated_at: Timestamp
}
```

The server is the **single writer of `version`** (client never supplies it). This
prevents lost-update races and gives a clean audit sequence.

### Push protocol (pseudo)

```
fn push(ruleset_name, candidate, auth: RobotId) -> Result<()> {
    let lock = registry.write();
    match lock.by_name.get(&ruleset_name) {
        None => {                                   // new name
            lock.insert(ruleset_name, Entry{owner: auth, ..});
            Ok(())
        }
        Some(e) if e.owner == auth => {             // same owner => UPDATE
            e.ruleset = candidate;
            e.version += 1;
            e.recompute_sha();
            Ok(())
        }
        Some(_) => Err(Conflict(ruleset_name)),     // different owner => REJECT
    }
}
```

### Edge cases to handle explicitly

1. **Same-owner re-push of identical content.** Should still succeed (idempotent
   update) but need not bump `version` if the SHA is unchanged. Decision: **only bump
   `version` on a SHA change** — keeps the audit log free of no-op noise while still
   accepting the request.
2. **Owner reassignment / handover.** A robot should never silently steal a name. If a
   different owner must take over a name (decommission, re-image), require an explicit
   **`release(ruleset_name)`** (or admin revoke) by the current owner, *then* the new
   owner may `push`. Never infer reassignment from a stale lease.
3. **Owner identity churn.** Robot re-imaged with a new `RobotId` but same `ruleset_name`
   must be treated as a *different* owner → conflict, not silent takeover. Fleet
   identity (#64) must be stable across reboots/re-images (persistent credential, not
   ephemeral UUID).
4. **Race: two robots push the same new name near-simultaneously.** Resolve under a
   single registry lock / atomic CAS — first writer wins, second gets `Conflict`. No
   split-brain.
5. **Name normalization.** Define `ruleset_name` syntax up front (e.g. `[a-z0-9-]{1,64}`,
   lowercased) and reject invalid names with `BadRequest` rather than `Conflict`, so
   namespace pollution is impossible.
6. **Delete / deregister.** `delete(ruleset_name)` only by owner (or admin). Server keeps
   the last audit copy + SHA in a tombstone record (WORM — see §b) so forensic history is
   not lost on delete.
7. **Auth missing / unverifiable (#64).** No auth ticket ⇒ reject `push` with
   `Unauthorized` before any registry lookup. Collision policy only applies to
   *authenticated* owners.

---

## (b) Server audit copy + SHA model

### What is the audit copy FOR? (standards research)

`flo` is explicitly the **software pre-estop / coordination layer** (see `RULES.md` and
the industrial-rules plan); hardware STO / certified Safety-PLC remains the primary stop
authority. The audit copy is therefore not a safety *function* but a **safety
*management / assurance* artefact**. Its purposes:

1. **Change traceability & Management of Change (MOC).** IEC 61508 (the parent
   functional-safety standard) treats the whole safety lifecycle as requiring controlled
   modification: any change to safety-related logic must be managed, reviewed, and
   recorded. See IEC 61508 Part 1 / IEC 61511 §"Management of Change" — modifications
   affecting safety functions pass through a formal MOC process
   (https://www.iec.ch/functional-safety, https://industrialautomationauthority.com/functional-safety-iec-61508-61511).
2. **Re-verification after any change.** ISO 3691-4:2023 (the AMR/AGV standard; current
   2023 edition, supersedes 2020 — https://www.iso.org/standard/80653.html) requires each
   safety function to be verified by test/measurement/inspection, and **"re-verification
   after any change: new routes, field sets, software updates, or load profiles"**. The
   server's recorded copy is the baseline an auditor compares a live robot against.
3. **Documented records for auditors and incident investigators.** ISO 3691-4 explicitly
   calls for **"documented records of every check, failure, and repair, because auditors
   and incident investigators will ask for them"** (summary: https://www.fabrico.io/blog/iso-3691-4-driverless-industrial-trucks/). The audit copy is exactly that record for
   *rule configuration*.
4. **Forensic incident reconstruction / replay.** Given a timestamped, versioned ruleset
   copy, an investigator can reconstruct *which rules were in force* at the moment of an
   incident — essential for root-cause analysis and liability.
5. **Tamper / unauthorized-modification detection.** The client owns the ruleset; the
   server's copy is the *trusted reference*. A SHA mismatch between what a robot is
   running and what the server holds means "something changed outside the managed
   channel" — a safety-relevant integrity event.

ANSI/RIA R15.08 (https://www.ansi.org, / R15.08-1 base robot, -2 integration, -3
application) and ISO 10218-1/2 align with and defer to ISO 3691-4 for mobility behaviors;
R15.08-2 specifically covers multi-robot coordination and facility safety-system
coordination, where a fleet-wide ruleset registry is squarely in scope.

### Recommendation 1 — SHA granularity: **per-rule AND per-ruleset**

Store **both**:

- `ruleset_sha` = SHA-256 over the canonical serialized `Rules` (whole document).
- `rules_sha[i]` = SHA-256 over each individual `Rule` (canonical per-rule serialization).

Rationale:
- **Per-ruleset SHA** is the cheap, always-checked guard: a single compare tells the
  server "did anything change at all?" It is what the live robot sends on each heartbeat
  / sync so the server can flag *any* drift instantly.
- **Per-rule SHA** enables *which rule changed* diagnosis and surgical diffing in the
  audit log ("rule `hrc-protective-stop-on-breach` v3→v4 changed; others unchanged"). For
  functional-safety traceability and MOC evidence, per-rule provenance is far more useful
  than a single opaque blob. The marginal cost (one hash per rule) is trivial.
- Use **SHA-256** (not MD5/SHA-1) — collision-resistant, standard, and what auditors
  expect. Canonical serialization (stable field order / deterministic TOML or a
  normalized JSON) is required so semantically-equal rulesets hash equal.

### Recommendation 2 — server behavior on mismatch

On a **live mismatch** (robot's reported `ruleset_sha` ≠ server's stored `ruleset_sha`
for that owner+name):

1. **ALERT first, always.** Raise a safety-relevant integrity alert (log + fleet
   monitor / ops channel). Mismatch is not "noisy" — it is exactly the unauthorized-change
   / unmanaged-change signal the standards care about.
2. **Do NOT auto re-sync silently.** The whole point of an owner-exclusive namespace is
   that the server copy is the *managed* truth. Auto-pushing the server copy back to the
   robot could mask a legitimate local safety override or a compromised client. Instead:
   - If the robot's copy is **newer and owned by the same robot**: treat as a managed
     update — server ingests it, bumps `version`, recomputes SHA, records the change
     (MOC trail).
   - If the robot's copy is **divergent / unowned / unverifiable**: server **keeps its
     last-good audit copy** (WORM) and flags the robot as *out-of-policy*. The robot side
     should, per its own safe-state posture, fall back to fail-safe (no unrestricted
     motion) until reconciled — consistent with `flo`'s existing safe-state design.
3. **Keep-last-good for the audit record.** The server never overwrites its stored audit
   copy with an unverified one; it appends a new version on a *verified* update and
   retains history. This gives replay/forensics a complete, tamper-evident timeline.

Net: **Alert → reconcile via managed update (same owner) or quarantine (divergent) →
keep last-good audit history.** Never silently overwrite.

### Recommendation 3 — storage: **persisted, append-only / WORM — not in-memory only**

- In-memory alone loses the audit trail on restart — defeating the entire compliance and
  forensic purpose. The server must **persist** the registry.
- Use an **append-only / WORM** model: each accepted `push` writes a new versioned record
  (name, owner, version, sha, timestamp, full ruleset blob). Deletes become *tombstones*,
  not erasures. This matches ISO 3691-4's "documented records … auditors and incident
  investigators will ask for" and IEC 61508 MOC traceability.
- Concrete options (no new heavy deps; `flo` already allows small crates):
  - Minimal: an append-only file / SQLite (`rusqlite`, if approved per AGENTS.md
    dependency policy) keyed by `(ruleset_name, version)`.
  - Cloud fleet: object store with immutable versioning; out of scope for v1.
- Keep a **hot in-memory index** (name → latest version + sha) for fast live mismatch
  checks, backed by the persisted log. The index is reconstructable from the log, so it
  is a cache, not the source of truth.

### Why not in-memory-only (summary)

The audit copy exists *because* of the standards' record-keeping and MOC requirements.
If it vanishes on restart or live update, there is no evidence, no replay, no
traceability — the feature would satisfy neither ISO 3691-4:2023 re-verification/records
nor IEC 61508 Management of Change.

---

## References

- ISO 3691-4:2023, *Industrial trucks — Safety requirements and verification — Part 4:
  Driverless industrial trucks and their systems* — https://www.iso.org/standard/80653.html
  (re-verification after change; documented records for auditors/incident investigators).
- IEC 61508 (functional safety of E/E/PE systems) — lifecycle + Management of Change:
  https://www.iec.ch/functional-safety ; overview https://www.tuvsud.com/en-us/services/functional-safety/iec-61508
- IEC 61511 / ANSI-ISA-61511 — Management of Change for safety-instrumented systems:
  https://industrialautomationauthority.com/functional-safety-iec-61508-61511
- ANSI/RIA R15.08 (industrial mobile robots; R15.08-2 multi-robot coordination):
  https://www.ansi.org ; https://www.fabrico.io/blog/iso-3691-4-driverless-industrial-trucks/
- ISO 10218-1/2 (industrial robots) defers mobility to ISO 3691-4.
- `flo` safety posture: `docs/RULES.md`, `docs/superpowers/plans/2026-07-19-industrial-rules.md`.

---

## Decisions at a glance

| Topic | Recommendation |
| --- | --- |
| Collision policy | Owner-exclusive namespace; reject-with-conflict for a *different* owner; same owner re-push = managed update |
| Owner key | Stable `RobotId` from #64 auth ticket (not ephemeral) |
| Version | Server-monotonic, bumped only on SHA change (idempotent no-op pushes accepted, not recorded) |
| Handover | Explicit `release`/admin revoke required; never inferred |
| SHA granularity | **Both** per-ruleset (SHA-256 over canonical doc) and per-rule (SHA-256 per rule) |
| Hash algo | SHA-256 over canonical/deterministic serialization |
| On mismatch | Alert → reconcile as managed update (same owner) or quarantine (divergent) → keep last-good |
| Storage | Persisted append-only / WORM + hot in-memory index (cache, not source of truth) |
| Deletes | Tombstone, not erase — audit history preserved |
