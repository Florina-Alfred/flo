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

- `.github/workflows/ci.yml` — minimal gate (fmt, clippy, test matrix, coverage) on every
  push + PR. `coverage` runs `cargo llvm-cov --workspace --all-targets` (advisory 50% threshold,
  artifact retained, Codecov upload conditional on `CODECOV_TOKEN`). Required status-check gate is
  `fmt` / `clippy` / `test`; `coverage` is advisory and not required yet (ratchet upward).
- `.github/workflows/security.yml` — full security + release artifact, **only on
  `main`** push + `v*` tags.
- `.github/workflows/publish.yml` — publishes to crates.io on `v*` tags only, using
  the `CARGO_REGISTRY_TOKEN` encrypted secret.

All jobs use **standard hosted runners** (free & unlimited on a public repo):
`ubuntu-latest` for x64 and the free `ubuntu-24.04-arm` for native arm64
container-image builds. No larger/self-hosted runners. Third-party actions are
pinned to verified commit SHAs (see AGENTS.md).

## Local CI testing

Validate workflows before pushing with [nektos/act](https://github.com/nektos/act)
(`.actrc` is committed):

```bash
act push -W .github/workflows/ci.yml --container-architecture linux/amd64 --defaultbranch main
```

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
