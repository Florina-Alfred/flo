# Ticket 01: Research zenoh loopback/peer config for zero-config cargo run

Label: `wayfinder:research`
Status: resolved
Blocked by:

## Question

Decide the zenoh configuration that lets `cargo run` (no args, possibly multiple
times on one host) work with zero config: peer mode over loopback, processes
auto-scouting each other, no router.

Resolve via a `/research` subagent. Investigate:
- zenoh `Config` for peer mode with loopback/local endpoints: what `connect`/
  `listen` endpoints (e.g. `tcp/127.0.0.1:...`, `udp/224.0.0.1` multicast scouting)
  enable same-host multi-process discovery with no router. Whether default scouting
  already covers loopback or needs an explicit endpoint.
- Whether a single `cargo run` (one process) still works standalone (simulate +
  engine in one process) — i.e. the demo doesn't REQUIRE two terminals.
- Env var vs programmatic `Config` builder: which is cleanest to apply from `main`
  when no `--robot-id`/`--config` is given. Confirm it stays ferrous-safe (config is
  data, not code).
- Flag if any option forces `unsafe` (it won't — config is safe Rust).

Capture findings on a throwaway `research/zenoh-loopback` branch and post a gist +
branch/commit reference as the resolution comment.

## Resolution summary

**Branch:** `research/zenoh-loopback` (created from `f1e3866`).
**Notes file:** `.scratch/research-zenoh-loopback.md` (full sources + snippet).

**Gist — minimal zero-config loopback peer mesh:**

`Config::default()` in zenoh 1.9.0 is *already* a peer (`mode:"peer"`) that listens on
a random TCP port and multicasts scouts on `224.0.0.224:7446` (TTL 1, link-local →
loopback). So two `cargo run` processes on one host auto-discover and mesh with **no
router and no config** — default multicast scouting already covers loopback. A single
`cargo run` works standalone too (scouts, finds nothing, proceeds).

For robustness (multicast may be dropped in containers/hardened hosts), pin a loopback
listener via `insert_json5` (no `connect` endpoints needed — discovery stays automatic):

```rust
use zenoh::Config;
fn cfg() -> Config {
    let mut c = Config::default();
    c.insert_json5("mode", "\"peer\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "true").unwrap();
    c.insert_json5("listen/endpoints/peer", "[\"tcp/127.0.0.1:0\"]").unwrap();
    c
}
let session = zenoh::open(cfg()).await?;
```

**Config mutations (exact):** `mode="peer"`, `scouting/multicast/enabled=true`,
`listen/endpoints/peer=["tcp/127.0.0.1:0"]`. No `connect` keys, no router.

**Programmatic vs env:** programmatic is cleanest for zero-config `cargo run` — env
(`ZENOH_CONFIG`) would need a shipped file. `Config` fields are private/unstable, so
the only supported mutation surface is `Config::insert_json5(key, json5)` /
`Config::from_json5(&str)` — both pure safe Rust.

**`unsafe`:** none. `Config::default()` / `from_json5` / `insert_json5` are plain
`#[derive(Default, Clone, Serialize, Deserialize)]` safe APIs; ferrous constraint holds.
