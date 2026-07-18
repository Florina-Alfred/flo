# Ticket 01 — zenoh loopback peer discovery (research findings)

**Goal:** `cargo run` with zero args, run multiple times on ONE host, must auto-discover
peers over loopback and form a peer mesh with NO zenoh router. A single `cargo run`
(one process) must work standalone for the demo.

**Crate:** `zenoh = "1.9.0"` with `features = ["unstable"]`. No `unsafe` in our code.

---

## 1. What the default config already does

Source: `DEFAULT_CONFIG.json5` shipped in `zenoh-1.9.0` (verified on disk under
`~/.cargo/registry/.../zenoh-1.9.0/DEFAULT_CONFIG.json5`):

- `mode: "peer"` — default is already **peer**, not router/client.
- `listen.endpoints`: `{ router: ["tcp/[::]:7447"], peer: ["tcp/[::]:0"] }`
  → a peer listens on a random free TCP port on all interfaces (including loopback).
- `scouting.multicast.enabled: true`, `address: "224.0.0.224:7446"`, `ttl: 1`,
  `autoconnect.peer: ["router","peer","client"]`, `listen.peer: true`
  → multicast UDP scout on `224.0.0.224:7446` (link-local multicast, TTL 1).

**Conclusion:** `Config::default()` already produces a peer that (a) listens on a
random TCP port, and (b) multicasts scout datagrams. Two `cargo run` processes on the
same host will discover each other via multicast and auto-connect (TCP). A single
process runs fine alone — it simply never receives a scout reply. **Zero config
already works on loopback for same-host multi-process**, because the default multicast
group `224.0.0.224` is within the link-local scope and reaches loopback/local
interfaces, and `ttl: 1` keeps it on-host.

## 2. Is default scouting enough, or add an explicit endpoint?

Default multicast scouting **already covers loopback discovery** for same-host peers.
You do NOT need an explicit `connect`/`listen` endpoint for discovery to work.

Caveats that justify a *pinned loopback listen* for robustness:
- Multicast depends on the OS delivering `224.0.0.224` to loopback. On some setups
  (containers, hardened networks) multicast may be dropped.
- A single explicit loopback listen (`tcp/127.0.0.1:0`) guarantees the process always
  has a local listener and advertises it via gossip, removing reliance on multicast.

**Recommended minimal config** (belt-and-suspenders, still zero-config):
keep defaults, ensure peer mode, and add a loopback TCP listener. No `connect`
endpoint needed — peers find each other via multicast + gossip.

## 3. Env var vs programmatic builder

- Env var: `ZENOH_CONFIG=/path/to/file` via `Config::from_env()` — requires shipping a
  file. Not ideal for zero-config `cargo run`.
- **Programmatic builder is cleanest.** Because `Config` fields are private (the zenoh
  config API is marked unstable; no public field access), the only supported mutation
  surface is `Config::insert_json5(key, json5_value)` (and `Config::from_json5(&str)`
  to build the whole tree from a JSON5 string). This is pure safe Rust, no `unsafe`.

`Config::insert_json5` signature (docs.rs, `zenoh::config::Config`, 1.9.0):
```rust
pub fn insert_json5(&mut self, key: &str, value: &str) -> ZResult<()>
```
Keys use dotted paths matching the config tree (e.g. `"mode"`, `"listen/endpoints/peer"`,
`"scouting/multicast/enabled"`). `from_json5` takes a whole JSON5 document string.

## 4. `unsafe` guarantee

`Config::default()`, `Config::from_json5`, and `Config::insert_json5` are all plain
safe-Rust APIs on a `#[derive(Default, Clone, Serialize, Deserialize)]` struct. No
`unsafe` is required or exposed. Our app stays `unsafe`-free per the ferrous constraint.

---

## Copy-pasteable `Config` snippet (compiles against zenoh 1.9.0)

```rust
use zenoh::Config;

fn zero_config_loopback_config() -> Config {
    let mut config = Config::default();

    // Peer mode (already the default, but be explicit for the demo contract).
    config
        .insert_json5("mode", "\"peer\"")
        .expect("valid mode");

    // Keep multicast scouting on so same-host processes auto-discover (default: true).
    config
        .insert_json5("scouting/multicast/enabled", "true")
        .expect("valid bool");

    // Pin a loopback listener so discovery never depends solely on multicast
    // reaching 224.0.0.224. ":0" = OS picks a free port; peers learn it via gossip.
    config
        .insert_json5("listen/endpoints/peer", "[\"tcp/127.0.0.1:0\"]")
        .expect("valid endpoint list");

    config
}
```

Then open the session:
```rust
let session = zenoh::open(zero_config_loopback_config()).await?;
```

**Notes:**
- No `connect` endpoints are set — discovery is fully automatic (multicast scout +
  gossip), satisfying "zero config" and "no router".
- `tcp/127.0.0.1:0` binds only loopback, so two processes never collide on a fixed
  port and still see each other through multicast/gossip.
- A single `cargo run` works standalone: it listens on loopback and scouts, finds
  nothing, and proceeds (no `exit_on_failure` for peers by default).
- If you later want a *fixed* demo port, replace `:0` with e.g. `:7447` on each
  process (still no router).

## Sources
- docs.rs `zenoh::config::Config` (1.9.0): struct is private-fields, mutation only via
  `from_file` / `from_json5` / `insert_json5` / `remove`.
- `zenoh-1.9.0/DEFAULT_CONFIG.json5` (cargo registry copy): `mode:"peer"`,
  `listen.endpoints.peer:["tcp/[::]:0"]`, `scouting.multicast.{enabled:true,
  address:"224.0.0.224:7446", ttl:1, autoconnect.peer:[...peers], listen.peer:true}`.
- `zenoh/src/api/config.rs` (1.9.0) `insert_json5` impl: pure safe Rust, no `unsafe`.
