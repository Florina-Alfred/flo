# PROTOTYPE — README first 30 seconds (throwaway, branch prototype/readme-outline)

**Question for #270:** How should the README read in the first 30 seconds so a newcomer trusts the catch?

This is a *rough outline stub* to react to — not the final README. Read it as "does this order and first code block make you want to clone?"

---

## Proposed section order (top → bottom, first screen → deeper)

1. **Title + one-line why** — "flo — Zenoh-mesh robot fleet in safe Rust — declarative rules, hot-reload, pub/sub + liveliness"
2. **30-second catch (first code block, copy-paste)** — the *only* thing a newcomer should need to trust it:
   ```bash
   git clone https://github.com/Florina-Alfred/flo && cd flo
   cargo test --lib --tests          # 170+ tests, 0 ignored
   cargo run --bin flo -- --help     # client flags (no --video-* leak)
   cargo run --bin flo -- rule check examples/rules/sample.toml  # OK: valid raw ruleset
   # loopback demo without multicast/Docker — two terminals:
   #   cargo run --bin flo-server -- --auth-mode none --auth-allow-insecure
   #   cargo run --bin flo -- --config tests/fixtures/minimal-client-config.toml --connect tcp/127.0.0.1:<zenoh-port>
   ```
   *Note under block:* "Multicast blocked on Docker/WSL? Add `--connect` — find the Zenoh port in the server log `health server listening` is *not* the Zenoh port, see Quickstart."

3. **Architecture in one diagram + one paragraph** — Zenoh mesh diagram (pub/sub + liveliness, not Queryable), `flo` = client runtime (`topic.rs` builders, `transport.rs` adapter, `semantic.rs` validator), `flo-server` = `try_join!` supervision

4. **Quickstart (the 4-step ritual expanded, 5 minutes)** — each ritual step → doc location → script entry point → macOS/Linux note (`scripts/verify-readme-demo.sh` with `lsof` fallback)

5. **Rule authoring (one example, `hrc-cell.toml` semantic + `sample.toml` raw fallback)** — `docs/RULES.md` is the deep dive; README only shows the two shipped rulesets (`hrc-cell` + `warehouse-fleet`) and the raw `Field("pressed")` vs pure topic-match note

6. **Configuration** — `tests/fixtures/minimal-client-config.toml` is the minimal `client.toml` (copy it), `zone/cell-3/entered` 3-seg, `robot-7` vs `robot/7` both accepted but slash is canonical per `CONTEXT.md`

7. **Contributing / Low-disk / Coverage** — *not* on first screen: link to `CONTRIBUTING.md` for `CARGO_INCREMENTAL=0 -j2`, `cargo llvm-cov`, `actionlint`, `cargo test --features media`, `cargo test -- --ignored --list`

8. **Install** — `cargo install flo-rs` (both bins), `ghcr.io` images, `cargo package` — after the catch, not before

---

## First code block alternatives to react to

**A (above):** 4 commands, no server running — pure local verification first, then two-terminal demo.

**B (demo-first):** Start with `cargo run --bin flo-server` in one terminal, then `flo` client — but that requires two terminals in the first 30s, higher friction.

**C (single-terminal loopback):** One `cargo run --example mesh_demo` that runs both — but hides the `flo`/`flo-server` split that the architecture diagram tries to teach.

**My recommendation:** **A** — local checks first, then demo. The "why Zenoh-mesh, not K8s" story lands in §3, not §1 — the catch is "it works", the why is "why it works this way."

---

## Open for grilling

- Does Quickstart come before Architecture, or vice versa?
- How much `AGENTS.md` low-disk/act-matrix detail leaks into README vs stays in `CONTRIBUTING.md`?
- Where does the "why Zenoh-mesh, not K8s" story land — §3 or a separate `docs/adr` link?
- Do we list both `hrc-cell.toml` and `warehouse-fleet.toml` in the first 30s, or just `hrc-cell`?

