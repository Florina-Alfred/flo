# Contributing to `flo`

## Workflow

1. **Never push directly to `main`.** Always work on a feature branch off `main`
   (e.g. `ci/branch-protection`, `feat/video-self-test`).
2. Open a **pull request** against `main`. Branch protection requires a PR and
   passing status checks (`fmt`, `clippy`, `test (stable/beta/1.97.1)`) before merge.
3. **Every PR must pass an independent code review before merge.**
   This repo uses the `code-review` agent skill: a *different* agent context than
   the implementer reviews the diff along two axes — **Standards** (does it follow
   this repo's documented conventions?) and **Spec** (does it implement the
   originating spec/issue correctly?). Merge only after that review reports no
   blocking findings.
4. Merge via squash. The full security pipeline (audit/deny/trivy/codeql) runs on
   `main` after merge.

## CI structure (free-tier safe — public repo)

- `.github/workflows/ci.yml` — minimal gate (fmt, clippy, test matrix, media, coverage, package) on every
  push + PR. `test` runs `cargo test --lib --tests` (not `--bin flo`, INFRA-01);
  `media` runs `cargo test --features media --lib --tests` plus
  `cargo test -- --ignored --list` to ensure the ignored suite compiles (INFRA-09);
  `coverage` runs `cargo llvm-cov --workspace --all-targets` (advisory 50% threshold,
  artifact retained, Codecov upload conditional on `CODECOV_TOKEN`); `package` runs
  `cargo package --list` + `cargo publish --dry-run` (CI-only contributor path, see Package verification below).
  Required status-check gate is `fmt` / `clippy` / `test`; `coverage` is advisory and not required yet (ratchet upward).
- `.github/workflows/security.yml` — full security + release artifact, **only on
  `main`** push + `v*` tags.
- `.github/workflows/publish.yml` — publishes to crates.io on `v*` tags only, using
  the `CARGO_REGISTRY_TOKEN` encrypted secret.

All jobs use **standard hosted runners** (free & unlimited on a public repo):
`ubuntu-latest` for x64 and the free `ubuntu-24.04-arm` for native arm64
container-image builds. No larger/self-hosted runners. Third-party actions are
pinned to verified commit SHAs (see AGENTS.md).

## Ritual — 4-step verification (locked per #271)

This is the full home for the ritual. `README.md` shows only the 30-second catch on the first screen (Architecture first, then catch per #270); this file documents every step, script entry point, and cross-platform note.

| Ritual step | `README.md` (first screen) | `CONTRIBUTING.md` (full) | `AGENTS.md` (one-liner) | Script / CI |
|---|---|---|---|---|
| `cargo test --lib --tests` | 30-sec catch code block only | `cargo test --lib --tests` + `cargo test --features media --lib --tests` + `cargo test -- --ignored --list` + `CARGO_INCREMENTAL=0 -j2` | "Tests: `cargo test --lib --tests` (full), `media` + `ignored` — see CONTRIBUTING" | — |
| `flo --help` / `flo-server --help` | "Each binary only shows its own flags" + `grep` example | `help_text` tightened (no `--video-*` on server) | — | — |
| `flo rule check` | `sample.toml` → `OK: valid raw ruleset` vs `hrc-cell.toml` → `OK: valid semantic` | `rule_check` + `examples_build` JSON schema | — | — |
| Loopback demo | Two-terminal `flo-server` + `flo --connect tcp/…` + `FLO_HEALTH_ADDR` note + link to `scripts/verify-readme-demo.sh` | `verify-readme-demo.sh` usage + `ss`/`lsof`/`pkill`/`pgrep` + Zenoh vs health port | `act` for local CI | `scripts/verify-readme-demo.sh` — `ss`→`lsof` fallback, `/tmp`/`xargs -r`/`bash` only |

`cargo package --list` + `cargo publish --dry-run` stay **CI-only + contributor path** (`CONTRIBUTING.md` + `ci.yml` `package` job) — not in `README.md` first screen.

### Testing

```bash
cargo test --lib --tests                 # full suite (fast, no GStreamer)
cargo test --features media --lib --tests # media tests (needs GStreamer)
cargo test -- --ignored --list            # ensure ignored suite compiles (loopback demo)
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Flaky sleeps are hardened via ready-gate `oneshot` (like `engine::subscribed`)
and deadline-based retry (10s for `core_loop`, 2s for transport, 20s for media)
— see `tests/core_loop.rs` and `src/transport.rs` for the pattern.

The ignored suite contains `tests/readme_verify.rs: readme_demo_server_starts_multicast` (`#[ignore]`). Verify it compiles and is listed:

```bash
cargo test -- --ignored --list   # should list readme_demo_server_starts_multicast
```

### Low-disk

A full `media` build + test can exhaust a small volume (a `target/` dir can reach ~60 GB).

- Build with `CARGO_INCREMENTAL=0` and `-j 2` to cut disk pressure:
  ```bash
  CARGO_INCREMENTAL=0 cargo test --lib --tests -j2
  CARGO_INCREMENTAL=0 cargo test --features media --lib --tests -j2
  CARGO_INCREMENTAL=0 cargo build -j2
  ```
- To reclaim space, `cargo clean` (full `target/` wipe) frees the bulk of it — `cargo clean -p flo-rs` only removes the crate's own artifacts, leaving the heavy dependency tree (webrtc/gstreamer/zenoh).
- `cargo llvm-cov` reuses the `target/` build cache; use `CARGO_INCREMENTAL=0` and `cargo llvm-cov clean` if disk pressure appears during coverage.

### Linting (clippy + fmt + actionlint)

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Workflows are linted with `actionlint` (and `yamllint` on touch):

```bash
# install
go install github.com/rhysd/actionlint/cmd/actionlint@latest
# run (from repo root)
actionlint .github/workflows/*.yml
```

CI runs `actionlint` on every workflow change; fix findings before requesting review.

## Local CI testing

Validate workflows before pushing with [nektos/act](https://github.com/nektos/act)
(`.actrc` is committed):

```bash
act push -W .github/workflows/ci.yml --container-architecture linux/amd64 --defaultbranch main
```

`act pull_request` for `ci.yml` fails locally because there is no remote `github.base_ref`. Use `act push` (which bypasses the git-diff scope check and always sets `rust=true`) to test the full Rust toolchain pipeline. For local CI matrix coverage, `AGENTS.md` points here for `act` detail.

## Coverage (llvm-cov + Codecov)

CI collects line coverage via `cargo-llvm-cov` (v0.6.18, pinned) and uploads
`lcov.info` to Codecov. The artifact is retained for 30 days (`lcov-coverage`);
Codecov posts a delta comment on PRs when `CODECOV_TOKEN` is configured. The
upload step is conditional on `secrets.CODECOV_TOKEN` so forks without the
secret do not fail.

**Threshold:** 50% line coverage, advisory / non-blocking (ratchet upward over
time). CI runs `cargo llvm-cov report --fail-under-lines 50` with
`continue-on-error: true` — it annotates but does not gate the merge. The job
is not a required branch-protection check yet (see `AGENTS.md`).

**Run locally (requires `llvm-tools-preview`):**

```bash
# one-time setup
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.6.18 --locked

# collect + view
cargo llvm-cov --workspace --all-targets --lcov --output-path lcov.info
cargo llvm-cov report --html   # open target/llvm-cov/html/index.html
cargo llvm-cov report --fail-under-lines 50  # local gate
```

Low-disk note: `cargo llvm-cov` reuses the `target/` build cache; use
`CARGO_INCREMENTAL=0` and `cargo llvm-cov clean` if disk pressure appears.

## Loopback demo — `scripts/verify-readme-demo.sh`

The README Quickstart is Architecture first, then the 4-step ritual. Step 4 (loopback demo) is scripted cross-platform:

```bash
chmod +x scripts/verify-readme-demo.sh
./scripts/verify-readme-demo.sh
# or run steps manually per README Quickstart
```

What the script does:

- Builds `flo` + `flo-server`, creates `/tmp/flo-*.toml` configs and `/tmp/flo-*.log` logs (uses `/tmp`, not repo root).
- Verifies `flo rule check examples/rules/sample.toml` + `hrc-cell.toml` + `warehouse-fleet.toml`.
- Starts `flo-server` in background on `tcp/127.0.0.1:0` → random Zenoh port (`src/auth.rs:152`, `src/transport.rs:86`) and captures `health server listening` (`FLO_HEALTH_ADDR`, default `0.0.0.0:0` → random on host, `0.0.0.0:8080` in containers) separately.
- Discovers the Zenoh port via `ss -tlnp` (Linux, `iproute2`, shows `flo-server` owner) with fallback to `lsof -i -P -n | grep LISTEN` (macOS, `brew install lsof`). Correctly parses Zenoh vs health: `grep -vx $HEALTH_PORT` so `--connect` never uses the health port.
- Cleans up previous runs via `pkill -f` with fallback to `pgrep | xargs -r kill` (Linux `xargs -r` / macOS fallback documented). Script is `bash` only (`#!/usr/bin/env bash`, `set -euo pipefail`).
- Starts `flo --robot-id robot-7` and `robot-8` with explicit `--connect tcp/127.0.0.1:<zenoh-port>` (works with or without multicast; `224.0.0.224:7446` often filtered on Docker/WSL/CI/VPN).

Cross-platform notes: Linux uses `ss` (preferred, shows process); macOS uses `lsof -i -P -n`. `xargs -r` is GNU (Linux) — macOS `xargs` lacks `-r` and is handled via fallback. All temp files are under `/tmp`; no repo pollution.

## Package verification (CI-only + contributor path)

`cargo package --list` + `cargo publish --dry-run` are **not** on the README first screen. They run in `ci.yml` `package` job and are documented here for contributors:

```bash
cargo package --list            # verify crate contents (no .agents/, docs/superpowers/ leak — see Cargo.toml `include`)
cargo publish --dry-run         # verify publish without pushing
```

CI fails if `cargo package --list` leaks internal files (`.agents/`, `manual_test.log`, `skills-lock.json`, `.actrc`, `docs/superpowers`, `deploy/`). See `.github/workflows/ci.yml:package`.

## Releasing to crates.io

The crate publishes as **`flo-rs`** (the bare name `flo` is taken on crates.io by an
unrelated 2018 crate). The CLI binary users run stays `flo`.

1. Bump `version` in `Cargo.toml`.
2. Refresh the lockfile: run `cargo generate-lockfile` on the MSRV toolchain
   (1.97.1); if `Cargo.lock` changed, commit it (conventional
   `release: refresh Cargo.lock ...`). The publish workflow rejects a stale
   lockfile, so this must land before the tag.
3. Commit and merge to `main`.
4. Tag the release commit: `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. The `publish.yml` workflow publishes using the `CARGO_REGISTRY_TOKEN` secret
   (set in repo Settings → Secrets and variables → Actions). The token is never
   committed and GitHub masks it in logs.
6. After publish, users install with `cargo install flo-rs` and run the `flo` binary.

## Secrets handling

- **Never** commit tokens, keys, or `.env` files. Use GitHub encrypted secrets.
- The crates.io token lives only in `CARGO_REGISTRY_TOKEN` (repo secret) and in
  your local `~/.cargo/credentials` (from `cargo login`). Rotate if exposed.
