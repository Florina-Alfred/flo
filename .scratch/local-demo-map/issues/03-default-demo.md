# Ticket 03: Default-demo wiring + visible verdict

Label: `wayfinder:task`
Status: open
Blocked by: 01, 02

## Question

Task (the front door): make `cargo run` with no args launch the demo, and make rule
fires visibly obvious.

Resolve when:
- `main.rs` parses args: no `--robot-id`/`--config` => demo mode. Demo mode applies
  the loopback zenoh config (ticket 01), uses the **embedded map-02 rules** as the
  bootstrap `RuleStore` (no file read), starts `--simulate` (ticket 02), and starts
  the rule engine. Explicit flags => production mode (existing behavior).
- The rule engine prints a **loud verdict** when a rule fires, e.g.:
  `▶ rule "e-stop-on-bumper" fired → published stop/fleet/cmd {"stop":true}`.
  (Widen the existing `info!` in `engine.rs` to a clear, human-readable line; keep
  structured fields too.)
- A short startup banner explains what's running and what to watch for, e.g.:
  "flo demo: simulating sensors on loopback zenoh. Open a 2nd terminal:
   cargo run --robot-id 8  (they'll see each other)."
- Ferrous: `#![forbid(unsafe_code)]` preserved; safe Rust only.
