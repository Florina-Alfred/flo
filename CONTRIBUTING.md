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

- `.github/workflows/ci.yml` — minimal gate (fmt, clippy, test matrix) on every
  push + PR. Required status-check gate.
- `.github/workflows/security.yml` — full security + release artifact, **only on
  `main`** push + `v*` tags.
- `.github/workflows/publish.yml` — publishes to crates.io on `v*` tags only, using
  the `CARGO_REGISTRY_TOKEN` encrypted secret.

All jobs use `ubuntu-latest` standard runners (free & unlimited on a public repo).
No larger/self-hosted runners. Third-party actions are pinned to verified commit
SHAs (see AGENTS.md).

## Local CI testing

Validate workflows before pushing with [nektos/act](https://github.com/nektos/act)
(`.actrc` is committed):

```bash
act pull_request -W .github/workflows/ci.yml --container-architecture linux/amd64
```

## Releasing to crates.io

1. Bump `version` in `Cargo.toml`.
2. Commit and merge to `main`.
3. Tag the release commit: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. The `publish.yml` workflow publishes using the `CARGO_REGISTRY_TOKEN` secret
   (set in repo Settings → Secrets and variables → Actions). The token is never
   committed and GitHub masks it in logs.

## Secrets handling

- **Never** commit tokens, keys, or `.env` files. Use GitHub encrypted secrets.
- The crates.io token lives only in `CARGO_REGISTRY_TOKEN` (repo secret) and in
  your local `~/.cargo/credentials` (from `cargo login`). Rotate if exposed.
