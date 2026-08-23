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

- `.github/workflows/ci.yml` — minimal gate (fmt, clippy, test matrix, media) on every
  push + PR. `test` runs `cargo test --lib --tests` (not `--bin flo`, INFRA-01);
  `media` runs `cargo test --features media --lib --tests` plus
  `cargo test -- --ignored --list` to ensure the ignored suite compiles (INFRA-09).
  Required status-check gate.
- `.github/workflows/security.yml` — full security + release artifact, **only on
  `main`** push + `v*` tags.
- `.github/workflows/publish.yml` — publishes to crates.io on `v*` tags only, using
  the `CARGO_REGISTRY_TOKEN` encrypted secret.

All jobs use **standard hosted runners** (free & unlimited on a public repo):
`ubuntu-latest` for x64 and the free `ubuntu-24.04-arm` for native arm64
container-image builds. No larger/self-hosted runners. Third-party actions are
pinned to verified commit SHAs (see AGENTS.md).

### Testing

```bash
cargo test --lib --tests                 # full suite (fast, no GStreamer)
cargo test --features media --lib --tests # media tests (needs GStreamer)
cargo test -- --ignored --list            # ensure ignored suite compiles
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Flaky sleeps are hardened via ready-gate `oneshot` (like `engine::subscribed`)
and deadline-based retry (10s for `core_loop`, 2s for transport, 20s for media)
— see `tests/core_loop.rs` and `src/transport.rs` for the pattern.

## Local CI testing

Validate workflows before pushing with [nektos/act](https://github.com/nektos/act)
(`.actrc` is committed):

```bash
act push -W .github/workflows/ci.yml --container-architecture linux/amd64 --defaultbranch main
```

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
