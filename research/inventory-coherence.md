# Inventory — Are examples, fixtures and tests coherent?

_Audit for wayfinder ticket #272 — cold-clone coherence of `examples/`, `tests/fixtures/` and `tests/`._

## Source of truth (per seam law)

- **Topic builders/validators:** `src/topic.rs` — every topic string must come from `topic::` builders and pass `check_topic_pattern` (`src/topic.rs:179`). Patterns: `zone/*/entered|cleared` (`src/topic.rs:161,164`), `robot/*/client/liveliness` (`:56`), `fleet/registration` (`:101`), `fleet/*/ruleset/**` (`:134`), `robot/{id}/local/*`.
- **Semantic validator/compiler:** `src/semantic.rs` — single validator/compiler (`validate` + `compile`) for semantic `when.near_human` / `in_zone` etc., plus raw `Rules` fallback in `src/common.rs:240` for `examples/rules/sample.toml`.

## File → Story it tells

| File | Kind | Rule/topic story | Format | Drift / flag |
|---|---|---|---|---|
| `examples/custom_rules.rs:13` | example | Loads `examples/rules/sample.toml` (raw), hot-reloads via `topic::rules_key(&robot_id)` (`fleet/{id}/ruleset`-ish, `:55`), runs `engine::run_engine` on loopback | raw `Rules` | Says "custom_rules: publishing on {rules_key} hot-reloads" — topic is `fleet/*` but `src/topic.rs:129,134` hot-reload is `fleet/{site}/ruleset/{name}` vs `robot/{id}/local/rules` — caller must guess site vs id |
| `examples/semantic_rules.rs:1` | example | Loads `examples/rules/hrc-cell.toml` (semantic), then "publish synthetic state on `robot/{id}/local/zone`, `human_present`, `proximity`" (`:3-5`) | semantic `parse_semantic` + `compile` | Comment matches `docs/RULES.md:5` and `src/topic.rs` `robot/{id}/local/*` — **coherent**, but the 3 topics listed are a subset of the full contract (missing `bumper`, `imu`, `drive`) |
| `examples/mesh_demo.rs:32` | example | `RuleStore::bootstrap_demo(&robot_id)` — demo rules built in `src/config.rs`, no file, runs engine on loopback | demo bootstrap | No topic list in comment; hides that demo uses `robot/{id}/local/*` + `zone/*` — newcomer can't see what to publish |
| `examples/video_peer.rs:3` | example | WebRTC `video_peer <peer-id>` via `Transport::loopback_config`, `SourceSpec::Videotest` | `#[cfg(media)]` gated | Correctly gated, but `README` quickstart never mentions `--features media` needed to run it |
| `examples/rules/sample.toml:1` | rule fixture | Raw: `when.all = [{topic="robot-7/local/bumper", pred=Field("pressed")==true}, {topic="robot-7/local/imu", Field("speed_mps")>0.2}] → `stop/fleet/cmd` | raw `Rules::from_toml` | **Coherent** after `src/common.rs:240` fallback; header now documents raw vs semantic (`:1-3`). Still `robot-7` (hyphen) vs `robot/7` (slash) both accepted in `src/topic.rs:199` but only slash documented in `CONTEXT.md:28` |
| `examples/rules/hrc-cell.toml:1` | rule fixture | Semantic: `site.cell-7`, zones `safety`/`approach`, `when.near_human=1.2` → `slow_to`, `when.any {in_zone, near_human}` → `estop`, `when.all {not_near_human, not_in_zone}` → `resume` | semantic `parse_semantic` | **Source of truth** for human/zone verbs; topics generated are `robot/{id}/local/human_present` + `zone/*/entered|cleared` via `topic.rs` builders |
| `examples/rules/warehouse-fleet.toml` | rule fixture | Semantic fleet (site + zones, similar to hrc-cell) | semantic | Not read in detail but `tests/readme_verify.rs:98,170` treats it as semantic sibling — should be listed alongside hrc-cell in `examples/README` |
| `tests/fixtures/minimal-client-config.toml:11` | fixture | Minimal viable `client.toml`: `heartbeat_interval_ms=1000`, `zone_enter="zone/cell-3/entered"`, `zone_exit="zone/cell-3/cleared"`, topics `robot-7/location/x/y/z` + `robot-7/site`/`zone` | `ClientConfig::from_toml` | **Coherent** — uses `entered`/`cleared` 3-seg (`:11-12`) matching `src/topic.rs:161` and `tests/core_loop.rs:207`; the 5-seg `zone/site/cell/7/entered` form (`src/topic.rs:30`) is *not* used here, hidden from newcomer |
| `tests/core_loop.rs:207,212` | integration test | Publishes `zone/cell-3/entered` with payload `robot-a` to test `SameZoneAs` | `Transport::put_bytes` | Matches fixture and `topic.rs` 3-seg — **coherent** |
| `tests/safety_infra06.rs:60` | integration test | `zone_enter = "zone/cell-3/entered"` in hot-reload + heartbeat tests | semantic + liveliness | Matches fixture — **coherent** |
| `tests/rule_check.rs:152` | CLI test | `robot/42/local/drive` as raw topic in `RULES::from_toml` test | raw | Uses slash form, not hyphen — both accepted but docs only show slash |
| `tests/readme_verify.rs:98,169` | CLI test | Asserts `flo rule check/compile warehouse-fleet.toml` `OK` + `status:"ok"` | semantic | Hardcodes `warehouse-fleet.toml` as the "second example" — `README` quickstart only shows `hrc-cell.toml`, so newcomer doesn't know the fleet file exists |
| `tests/examples_build.rs:48` | build test | Asserts `cargo build --examples` + `topic`/`qos`/`[[rules]]` in rule TOML | meta | Now tightened (INFRA-09) to check JSON schema, not just build — **coherent** |
| `src/topic.rs:30,199` | validator | Accepts both `robot/{id}/local` (slash) and `robot-{id}/local` (hyphen), 3-seg `zone/*/entered` and 5-seg `zone/site/cell/7/entered` | `check_topic_pattern` | Hyphen accepted but undocumented in `CONTEXT.md`; 5-seg accepted but **never used** in `examples/`/`tests/fixtures/` — hidden seam |
| `CONTEXT.md:28` | glossary | Says `robot/{id}/local` (slash) | — | Contradicts `example` and `config` hyphen `robot-7` — both work but glossary is incomplete |
| `docs/RULES.md:5,150` | docs | Says `robot/{id}/local/human_present` + `zone/*/entered` | — | Matches `topic.rs` and fixtures — **coherent** after INFRA-08 |

## Coherence table (one line per drift)

| Drift | Where it appears | Source of truth | Fix needed? |
|---|---|---|---|
| `fleet/<site>/state` vs `robot/{id}/local/*` | Old comment in `semantic_rules.rs:3` (pre-fix) vs `topic.rs` builders | `topic.rs` builders | **Fixed** in current `semantic_rules.rs` (`:3-5` now lists `zone`/`human_present`/`proximity`) |
| `enter`/`exit` (4-seg) vs `entered`/`cleared` (3-seg) | `tests/fixtures` vs old `config.rs` defaults (pre-fix) | `topic.rs:161,164` 3-seg | **Fixed** — fixture and `src/topic.rs` agree on `entered`/`cleared`; `docs/RULES.md` updated |
| `Field("pressed")` vs trigger without `pred` | `sample.toml` (with pred) vs `README` quickstart snippet (without) | `docs/RULES.md §8` + `sample.toml` with pred is **correct** — trigger without pred = pure topic match, should be documented | Needs doc line: "trigger without pred = fires on any publish" |
| Slash `robot/7` vs hyphen `robot-7` | `tests/rule_check.rs` slash vs `sample.toml`/`minimal-client-config` hyphen | `src/topic.rs:199` accepts both | **Gaps:** `CONTEXT.md:28` only lists slash — update to document both or pick one |
| 5-seg `zone/site/cell/7/entered` never used | `src/topic.rs:30` validator accepts it, but no example/fixture uses it | `topic.rs` builders only emit 3-seg `zone/{id}/entered` | **Hidden seam** — either document as "accepted but not generated" or remove 5-seg pattern |
| `examples/custom_rules.rs` hot-reload topic `rules_key` vs `fleet/*/ruleset/**` | `custom_rules.rs:55` prints `rules_key` but doesn't say `site` vs `robot_id` | `src/topic.rs:129,134` | Needs one-line comment: "rules_key = fleet/{site}/ruleset/{name}, not robot/{id}/local/rules" |
| `mesh_demo.rs` bootstrap hidden | No topic list, no link to `hrc-cell.toml` | `src/config.rs:bootstrap_demo` | Needs comment linking to `hrc-cell.toml` and topic contract `docs/RULES.md §5` |

## What to graduate (for Placement/Renames/README tickets)

- **Placement:** `examples/` are docs for humans (run with `cargo run --example`), `tests/fixtures/` are minimal configs for `cargo test`, `tests/` are the ritual. Keep `examples/rules/*.toml` as the *canonical* rule fixtures; `tests/fixtures/minimal-client-config.toml` is the *minimal config* — don't merge them. Move `tests/fixtures` link into `examples/README` or `CONTRIBUTING.md`.
- **Renames:** No file moves needed for examples/fixtures themselves — names already direct. Drift is in *vocabulary* (`Rules` vs `SemanticDoc` vs `RuleStore` vs `Ruleset`) — defer to ticket #269.
- **README:** The 5-minute catch ritual already lists `cargo test --lib --tests`, `flo --help`, `flo rule check`, loopback demo — but `examples/` vs `warehouse-fleet.toml` is still hidden. Add one line: "Two shipped rulesets: `hrc-cell.toml` (single cell) and `warehouse-fleet.toml` (fleet) — both semantic, `sample.toml` is the raw fallback."

