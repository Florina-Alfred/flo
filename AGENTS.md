# AGENTS.md

Two-binary Rust crate (`flo-server` + `flo`) in package `flo-rs`, edition 2024.

## Dependencies

The crate uses third-party Rust dependencies (e.g. `zenoh`, `webrtc`, `gstreamer`
behind the `media` feature, `axum`, `tokio`). The original "no dependencies"
stance has been relaxed to reflect reality:

- **Keep dependencies minimal** — add a crate only when it earns its place; prefer
  small, well-maintained crates over large frameworks.
- **Every new dependency requires admin approval** before it is added to
  `Cargo.toml`. Do not introduce dependencies unilaterally.
- **No `unsafe`**: the crate remains `#![forbid(unsafe_code)]`. Any dependency
  that requires `unsafe` in our code, or pulls in an unvetted `unsafe` transitively,
  needs explicit admin sign-off.
- **Supply chain**: third-party GitHub Actions remain pinned to full commit SHAs
  (see CI section); Rust crates are resolved via `Cargo.lock` (committed).

## Tests & CI

- Tests: `cargo test` (the crate now has a test setup — run it in CI).
- CI: GitHub Actions workflows in `.github/workflows/` (minimal gate `ci.yml` on
  every branch/PR; full security + release pipeline `security.yml` on `main`/tags).


## Commands
- Build: `cargo build`
- Run: `cargo run`
- Check (fast, no build artifacts): `cargo check`
- Test: `cargo test`
- Lint: `cargo clippy` / Format: `cargo fmt`

## CI / GitHub Actions (free-tier safe)

Repo is **public** → standard `ubuntu-latest` runners are free & unlimited. All CI uses
only standard hosted runners (`ubuntu-latest` for x64, plus the free `ubuntu-24.04-arm`
for the native arm64 image build in `container.yml`); no larger/self-hosted runners.

- `.github/workflows/ci.yml` — **minimal gate**, runs on every branch push and every PR.
  Jobs: `changes`, `fmt`, `clippy` (`-D warnings`), `test` matrix (`stable`, `beta`, `1.97.1`),
  `media`. Required status checks: `fmt`, `clippy`, `test (stable)`, `test (beta)`, `test (1.97.1)`.
  The `changes` job detects scope — docs-only PRs skip the Rust toolchain steps
  (jobs always run and succeed, only the expensive steps are conditional).
- `.github/workflows/security.yml` — **full security + release**, runs ONLY on `main`
  (push) and `v*` tags. Jobs: `cargo-audit` (hard gate — fails on any unlisted
  RUSTSEC advisory; reviewed exceptions in `audit.toml`), `cargo-deny`, `trivy` (SARIF,
  all severities), `codeql`
  (rust), and a tag-triggered `release` artifact build (30-day retention).
- `.github/workflows/container.yml` — **container images**, runs on every branch push and PR
  (PRs: build only, no push). Matrix of 4 images (`server`, `client`, `server-media`,
  `client-media`) × 2 platforms. Each platform is built **natively** — `linux/amd64` on
  `ubuntu-latest`, `linux/arm64` on the free `ubuntu-24.04-arm` hosted runner — so no QEMU
  emulation (which previously blew the 60-min timeout on cold cache; see #152). A `merge`
  job (main/tags only) assembles the multi-arch manifest index for the final tags: `latest`
  + `sha-*` on `main` push, semver tags on `v*` tag. **Signing:** keyless Cosign via Sigstore
  (GitHub OIDC → Fulcio + Rekor). **SBOM:** Syft SPDX attested with Cosign.
  **Provenance:** SLSA via
  `actions/attest-build-provenance`. Cosign signs by digest, never by tag.
- `.github/workflows/publish.yml` — publishes to **crates.io** on `v*` tags only, using
  the `CARGO_REGISTRY_TOKEN` encrypted repo secret (Settings → Secrets and variables →
  Actions). The token is never committed; GitHub masks it in logs. `.env` files are NOT
  used for secrets.

### Review before merge
Every PR into `main` must pass an **independent code review by a different agent** before
merge — use the `code-review` skill (two axes: Standards + Spec). See `CONTRIBUTING.md`.

### Supply-chain hardening
- Every third-party action is pinned to a **full commit SHA** (verified via
  `git ls-remote`), not a mutable tag. First-party `github/codeql-action` uses `@v3`
  (GitHub-maintained rolling tag, acceptable). Dependabot keeps SHAs fresh.
- After the 2026-03 `aquasecurity/trivy-action` compromise, the Trivy pin is
  `57a97c7e7821a5776cebc9bb87c984fa69cba8f1` (v0.35.0, the known-good signed release).

### Local CI testing with `act`
Before pushing, validate workflows locally with [nektos/act](https://github.com/nektos/act)
(Docker required). `.actrc` maps `ubuntu-latest` to the act image.

**Important:** `act pull_request` for `ci.yml` fails locally because there is no remote
`github.base_ref`. Use `act push` (which bypasses the git-diff scope check and always
sets `rust=true`) to test the full Rust toolchain pipeline.
```bash
# Rust toolchain pipeline (use push event — pull_request requires remote base ref)
act push -W .github/workflows/ci.yml --container-architecture linux/amd64 --defaultbranch main
# full security pipeline on a main push (heavy: pulls Trivy/CodeQL images)
act push -W .github/workflows/security.yml --container-architecture linux/amd64
# Container build (tests Dockerfile planner stage + cross-build; uses Docker-in-Docker)
act push -W .github/workflows/container.yml --container-architecture linux/amd64
```

## Branch protection (configure in repo Settings → Branches)
`main` is protected:
- **No force-pushes, no deletion.**
- **Require a pull request** before merging (no direct pushes).
- **Require status checks to pass** before merge: `fmt`, `clippy`, `test (stable)`,
  `test (beta)`, `test (1.97.1)` (all from `ci.yml`).
- **Require branches to be up to date** before merging.
- Dismiss stale approvals; restrict who can push to `main`.

## Workflow best practices observed
- Least-privilege `permissions:` per job (`contents: read`; only Trivy/CodeQL get
  `security-events: write`). Workflow-level default is `contents: read`.
- `concurrency:` with `cancel-in-progress: true` to kill superseded runs (saves minutes).
- `timeout-minutes` on every job (no 6-hour default hang).
- Artifact retention capped at `30` days.
- `cargo cache` via `Swatinem/rust-cache` to cut build minutes.
- Dependabot (`cargo` + `github-actions`, weekly) keeps deps and action SHAs current.

## Notes
- Entrypoints are `src/bin/flo-server.rs` and `src/bin/flo-client.rs` (both `fn main`).
- `/target` is gitignored; `Cargo.lock` is committed.
- Toolchain: cargo/rustc 1.97.1 (MSRV).
- Container images: `ghcr.io/<owner>/flo-server`, `flo-client`, `flo-server-media`, `flo-client-media`.
  Built by `container.yml` workflow with Cosign signing, SPDX SBOM, and SLSA provenance.

## Agent skills

### Issue tracker

Issues live as GitHub Issues, organised on the `flo` Projects V2 board (see `docs/agents/issue-tracker.md`). The old local-markdown `.scratch/` tracker has been removed from the repo.

### Triage labels

Default five roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
