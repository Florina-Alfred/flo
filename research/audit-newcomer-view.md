# Audit — What does the repo look like to a newcomer today? (2026-08-30, `origin/main@2639a34`)

Cold-clone inventory against `src/lib.rs`, `Cargo.toml`, `src/bin/*`, `src/cli.rs --help`, `src/topic.rs`/`src/transport.rs`/`src/semantic.rs`/`src/rules.rs`/`src/runtime.rs`/`src/common.rs`/`src/engine.rs`, `README.md`, `docs/RULES.md`, `CONTEXT.md`, `examples/*`, `tests/fixtures/*`, `AGENTS.md`/`CONTRIBUTING.md`, `scripts/verify-readme-demo.sh`. Read-only; no decisions.

## 1. Naming & placement drifts

**Crate vs binary vs file stutter (seam violation).** `Cargo.toml:2` package is `flo-rs` (explained `CONTRIBUTING.md:91` “bare `flo` taken on crates.io”), bin `flo` lives at `src/bin/flo-client.rs:1` (`Cargo.toml:69-70`), bin `flo-server` at `src/bin/flo-server.rs:1` (`Cargo.toml:72-73`). `AGENTS.md:152` correctly notes “`src/bin/flo-server.rs` and `src/bin/flo-client.rs` (both `fn main`)”, but `README.md:3` says “`flo-server` and `flo-client`” while the user actually runs `cargo run --bin flo` / `flo` (not `flo-client`). `CONTRIBUTING.md:104` further says `cargo install flo-rs` then run `flo`. Newcomer grep for `flo-client` binary fails; `cargo run --bin flo-client` fails.

**Overloaded `rule` vocabulary (deep-module cut missed).** Five public names cover “rules”: `src/rules.rs:113` `Rules` (runtime `[[rules]]` wire), `src/rules.rs:132` `Ruleset` (ownership envelope `ruleset_name/version/robot_owner`), `src/semantic.rs:176` `SemanticDoc` (authoring `[site]/[zones]/[[rules]]` with `when.near_human`), `src/semantic.rs:576` `SemanticRuleset` (envelope+semantic), `src/config.rs:14` `RuleStore` (hot-swappable `Arc<RwLock<Arc<Rules>>>`). `docs/RULES.md:3-6` calls both “semantic files” and “raw engine files” “rules files” without a prefix. `src/runtime.rs:231` `compile_rules_or_default` silently tries semantic then raw, so a typo’d semantic can masquerade as raw. A newcomer can’t tell which type a `.toml` file is until `flo rule check`’s stdout says `is a valid semantic/raw ruleset` (`src/common.rs:198-225`).

**Topic key stutter.** Two hot-reload keys: per-robot `src/topic.rs:43` `rules_key` → `robot/{id}/local/rules` (used `src/config.rs:84`), and fleet-scoped `src/topic.rs:129` `ruleset_pub_key` → `fleet/{site}/ruleset/{name}` (`src/config.rs:119` `RULESET_PUB_PATTERN`). `docs/RULES.md` never mentions the `fleet/*/ruleset/**` path; `README.md` “Hot-reload” mentions only `--ruleset` file.

**Module tree flat but seams duplicated.** `src/lib.rs:8-25` exposes 18 modules flat (`auth`, `codec`, `common`, `config`, `device`, `engine`, `health`, `mutation`, `registration`, `registry`, `rules`, `runtime`, `semantic`, `server`, `signaling`, `topic`, `transport`). `src/runtime.rs:1` claims “single deep entry point… replaces three competing startup flows” and `src/common.rs:1` “keeping these here lets `runtime.rs` stay focused” — yet health/registration/engine still split across `src/health.rs`, `src/server.rs`, `src/common.rs`, `src/runtime.rs`, `src/engine.rs`. `Transport` is billed as “the single low-level seam” (`src/transport.rs:12-18`) but `zenoh::Config` leaks at construction (`Transport::open_with`, `loopback_config`, `connect_config`) by design and `AuthConfig::zenoh_config` in `src/auth.rs` also builds config.

**Zone/topic verb drift latent.** `src/topic.rs:32-34` and `CONTEXT.md:49` insist `entered/cleared` (reject `enter/exit`). `src/config.rs:200-202` `ZoneSubscriptions` fields are `site_id/zone_enter/zone_exit` (values like `zone/cell-3/entered` `zone/cell-3/cleared` in `tests/fixtures/minimal-client-config.toml:11-12`), but the struct field is `zone_exit` while the topic verb is `cleared` — a rename trap. `src/topic.rs:237` allows both 3-seg `zone/{id}/entered` and 5-seg `zone/{site}/{cell}/{id}/entered`, undocumented in `README.md:174` diagram which only shows `zone/*/entered, zone/*/cleared`.

## 2. README / `--help` / `docs/RULES.md` / `examples` disagreements

**Health port — intentional mismatch, still confusing.** Serve default is `0.0.0.0:0` (random) (`src/common.rs:52`, `src/server.rs:60`, `README.md:303`); probe default is `127.0.0.1:8080` (`src/bin/flo-client.rs:21`, `src/bin/flo-server.rs:20`, `README.md:339-352`). `README.md:338-353` does document the mismatch and the `FLO_HEALTH_ADDR=0.0.0.0:8080` fix, and `scripts/verify-readme-demo.sh:162-163` parses both ports, but `AGENTS.md`/`CONTRIBUTING.md` never mention health. First `curl localhost:8080/healthz` without env fails with `Connection refused` — the “5-minute fail” the ticket asked about. Container default `0.0.0.0:8080` (`README.md:306`, `Dockerfile`) fixes it only inside Docker.

**Rule format — two grammars, one validator message.** `README.md:91-113` shows raw `when.all = [{topic=...}]` (pure topic match, no `pred`) and typed `pred={Comparison...{Field...}}`; `docs/RULES.md:220-250` codifies that `Field`/`Prim`/`Bool` grammar and notes `Both robot/{id}/local/... and robot-{id}/local/... forms accepted`. `examples/rules/sample.toml:1` is raw with `pred`, `examples/rules/hrc-cell.toml:1` and `warehouse-fleet.toml:1` are semantic with `when.near_human`. `flo rule check` accepts both via fallback (`src/common.rs:193-279`), printing `valid semantic` vs `valid raw` — `README.md:246` documents the two example checks, but `README.md:91` “ trigger without `pred` is a pure topic match” is only in a parenthetical; a newcomer copying the semantic `when.near_human` into a raw file gets `E001` parse error with no hint about the other grammar.

**Zone grammar — consistent but scattered.** All of `src/topic.rs:235`, `CONTEXT.md:49`, `docs/RULES.md:149-151`, `README.md:175` agree `entered/cleared`; the README quickstart client config `README.md:79-80` `zone_enter = "zone/cell-3/entered"` matches `src/config.rs` validation. No current drift, but the old `enter/exit` is still in test names (`src/topic.rs:339-347` `rejects_non_canonical_zone_events`) — a newcomer grepping history will find the rejected spelling.

**CLI `--help` now aligned, one residual mention.** `cargo run --bin flo -- --help` lists `flo [OPTIONS] [COMMAND]` with `rule validate/inspect` (`src/cli.rs:126-127`); `flo rule --help` → `flo rule <COMMAND>` with `check/compile` (`src/cli.rs:126-157`); `flo-server --help` hides `--ruleset`/`--video-*` (`tests/help_text.rs:163-199` enforced). Remaining drift: `flo-server --help` still mentions `--ruleset` inside the `--config` help text (“Missing/unreadable → fail-safe… valid config with no --ruleset → built-in demo rules” `src/cli.rs:82-83`), so `grep --ruleset` on server help hits a false positive (test avoids it by checking `line.trim_start().starts_with("--ruleset ")`).

**`--config` / `--ruleset` semantics.** Help says “Optional. Missing/unreadable → fail-safe empty ruleset (no motion commands); valid config with no --ruleset → built-in demo rules” (`src/cli.rs:24-30`). `README.md:257` says “Every field is required (missing fields are a fatal validation error)” for the client config — true for structure (`src/config.rs:228-284` `validate` requires `[default_subscriptions]`/`[default_publishers]`), but the file itself is optional (runtime falls back `src/runtime.rs:189-193`). Newcomer reads “every field required” as “must pass --config”.

**Version/tag drift note.** `Cargo.toml:3` `version = "0.1.5"`; publish/container workflows validate `Cargo.toml version == ${GITHUB_REF_NAME#v}` (`AGENTS.md:132-140`). Not a doc drift, but the version in `README.md` installation snippet is absent, so tag vs crate version confusion is unguarded in docs.

## 3. 4-step ritual — does it work on this machine?

Run on `origin/main@2639a34`, Linux `cargo 1.97.1`-compatible, no GStreamer/media:

| Step | Command | Result |
|------|---------|--------|
| `cargo test --lib --tests -- --list` | `cargo test --lib --tests -- --list 2>&1 \| grep -c ': test'` | ✅ exits 0, lists 200+ tests (lib 60+ incl. `cli::tests`, `engine::tests`; integration `core_loop`, `examples_build`, `help_text`, `rule_check`, `semantic_compile`, etc.; 3 `ignored` not listed unless `--ignored`) |
| `cargo run --bin flo -- --help` | as above | ✅ `Usage: flo [OPTIONS] [COMMAND]` with `Commands: rule` and all flags incl. `--video-*` |
| `cargo run --bin flo-server -- --help` | as above | ✅ `Usage: flo-server` hides `--ruleset`/`--video-*` as flags, shows shared `--auth-*`/`--connect`/`--healthcheck` |
| `cargo run --bin flo -- rule check examples/rules/sample.toml` | `cargo run --bin flo -- rule check examples/rules/sample.toml` | ✅ `OK: examples/rules/sample.toml is a valid raw ruleset` (also `hrc-cell.toml` → `valid semantic`, `warehouse-fleet.toml` → `valid semantic`) |
| `scripts/verify-readme-demo.sh` | `bash scripts/verify-readme-demo.sh` | ✅ on Linux with `ss`/`curl` — full mesh demo passes (server + 2 clients register, `/healthz`/`/readyz`/`/metrics` 200). Captured run produced `✓ Server started`, `Zenoh port: 36351`, `✓ robot-7/8 registered`, `✓ /healthz` |

**Cross-platform gaps in `scripts/verify-readme-demo.sh:1-304`:**

- Shebang `#!/usr/bin/env bash` + `set -euo pipefail`, `ss -tlnp` Linux-first, `lsof -i -P -n` macOS fallback (`:162-184`); no BusyBox `ss` (Alpine without `iproute2`), no Windows/PowerShell (`ss`/`lsof`/`pkill`/`pgrep` absent) — script `exit 1` with “Neither ss nor lsof found” (`:180-183`). CI `ubuntu-latest` is fine; host-novelty fails.
- `pkill -f "target/debug/flo-server"` (`:28-30`) not portable to Windows, needs `pkill`/`pgrep`; bare `pgrep | xargs -r kill` assumes GNU `xargs -r`.
- Port discovery `ss -tlnp | grep "flo-server"` relies on proc name in `ss` output; containerized or `cargo run` under `rust-analyzer` may not show `flo-server`. Fallback `ss -tln | grep 127.0.0.1:` can pick wrong `127.0.0.1` listener if another service binds.
- Requires `curl` for health probe; Windows or minimal images without `curl` skip silently? Actually fails `curl -sf` → `FAIL` but script continues to `PASS=false`.
- Uses hard `/tmp/flo-*.toml` and `/tmp/flo-*.log` — on Windows `%TEMP%` not `/tmp`; concurrent runs collide.
- `cargo build --bin flo-server --bin flo` before run assumes toolchain already installed; docs `scripts/setup-dev.sh` for media deps not invoked.

Loopback demo without explicit `--connect` still works on this host (multicast not filtered); on Docker/WSL/VPN it hangs at `registering with server...` until `--connect tcp/127.0.0.1:<zenoh-port>` per `README.md:39-62` — the script already does `--connect` so it survives filtered multicast.

## 4. `examples/` vs `tests/` vs `tests/fixtures/` — different stories

**`examples/` tells “pick your API level”:**

- `examples/custom_rules.rs:1` — raw `RuleStore::bootstrap(&toml)` on `examples/rules/sample.toml` (raw topic+pred), plus `run_hot_reload` on `robot/{id}/local/rules` (`:55`). No semantic.
- `examples/semantic_rules.rs:1` — `semantic::parse_semantic` + `compile` on `examples/rules/hrc-cell.toml`, then `engine::run_engine` on `loopback_config`. No `RuleStore` hot-reload.
- `examples/mesh_demo.rs:1` — `bootstrap_demo` (`src/config.rs:38` 2-rule demo) on `loopback_config`, ignoring files entirely — the “zero-config cargo run” story.
- `examples/video_peer.rs:1` — `media`-gated, `SourceSpec::Videotest`, peer `video_peer <peer-id>` — requires `--features media`.

`docs/RULES.md:10` says “examples live in `examples/rules/` and pass `flo rule check`” — true (`sample.toml` raw, `hrc-cell`/`warehouse-fleet` semantic), but no index maps which `examples/*.rs` goes with which `examples/rules/*.toml`; a newcomer may run `semantic_rules.rs` on `sample.toml` and get `E009 When is empty`-style failure.

**`tests/fixtures/` tells “minimal valid config”:**

- `tests/fixtures/minimal-client-config.toml` = `README.md:68-88` client config verbatim (minus comments) — heartbeat `1000`, `default_subscriptions.location x/y/z`, `zone.site_id/zone_enter/zone_exit`, `default_publishers.location/zone`.
- `tests/fixtures/minimal-server-config.toml` = 1 `expected_client robot-7` — matches `README.md:18-19` single-entry form.

But fixtures are only used by `tests/safe_state.rs`/`registration` helpers, never referenced in `README.md` or `--help`; a newcomer looking for “copy-paste minimal config” finds 3 places: `README.md` inline, `tests/fixtures/*`, and `scripts/verify-readme-demo.sh:46-122` generated `/tmp/flo-*-config.toml` — the latter two diverge: script robot configs use hyphenated topics `robot-7/location/x` while `src/topic.rs:199` `is_robot_ns` accepts both `robot/7/...` and `robot-7/...`, and fixtures/README use `robot-7` hyphen form consistently, but `src/topic.rs` docs table shows `robot/{id}/local/...` slash form (`CONTEXT.md:28-30` slash vs `README.md:72-82` hyphen).

**`tests/` tells “edge cases exist”:**

- `tests/rule_check.rs:192` `rule_check_accepts_empty_file` — empty file is valid (0 rules) — contradicts `docs/RULES.md:28` “[site].id is required for compilation” unless you mean semantic compile path; `src/common.rs:198` enforces non-empty `when` for raw fallback but `parse_semantic_auto("")` on empty yields `site.id=""` valid parse then `validate` passes (no zones/rules) — empty is “valid” only via fallback.
- `tests/rule_check.rs:225` `rule_check_rejects_ruleset_envelope` — writes `ruleset_name/version/robot_owner` alone and asserts it’s “valid (empty ruleset)”, documenting that envelope without `[[rule]]` is not rejected — `docs/RULES.md` never describes envelope authoring.
- `tests/help_text.rs:3` pins exact help strings (`Usage: flo [OPTIONS] [COMMAND]`, `fails closed`, `GStreamer`) — a contract no example mentions.

**Net:** `examples/` teaches 3 orthogonal entry points with no cross-link; `tests/fixtures` are the true minimal configs but are hidden in `tests/`; `tests` encode invariants (empty=valid, envelope without rules=valid, hyphen=slash) that docs don’t surface.

## 5. Contributor surface (AGENTS/CONTRIBUTING/CONTEXT) — quick notes

- `AGENTS.md:22-44` commands: `cargo test --lib --tests` (not `--bin flo` INFRA-01), `cargo test --features media --lib --tests`, `cargo test -- --ignored --list` must compile (`:25`, `:172`). All verified above.
- `CONTEXT.md:12-47` topic table lists `robot/{id}/local/{resource}` slash form but `README.md` and fixtures use `robot-7/...` hyphen form — `src/topic.rs:199-224` reconciles by accepting both via `is_robot_ns`.
- No drift in `CONTRIBUTING.md:91-104` install story; `docs/RULES.md:13` prerequisite “`cargo install flo-rs`” matches.

---
*Inventory only. Placement/rename/README/ritual decisions await #266 follow-ups.*
