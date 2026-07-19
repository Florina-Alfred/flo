# AGENTS.md

Single-binary Rust crate (package `flo`, edition 2024). No dependencies, no tests, no CI yet.

## Commands
- Build: `cargo build`
- Run: `cargo run`
- Check (fast, no build artifacts): `cargo check`
- Test: `cargo test`
- Lint: `cargo clippy` / Format: `cargo fmt`

## CI / GitHub Actions (free-tier safe)

Repo is **public** → standard `ubuntu-latest` runners are free & unlimited. All CI uses
only `ubuntu-latest`; no larger/self-hosted runners.

- `.github/workflows/ci.yml` — **minimal gate**, runs on every branch push and every PR.
  Jobs: `fmt`, `clippy` (`-D warnings`), `test` matrix (`stable`, `beta`, `1.97.1`).
  This is the required status-check gate for merging into `main`.
- `.github/workflows/security.yml` — **full security + release**, runs ONLY on `main`
  (push) and `v*` tags. Jobs: `cargo-audit` (hard gate — fails on any unlisted
  RUSTSEC advisory; reviewed exceptions in `audit.toml`), `cargo-deny`, `trivy` (SARIF,
  all severities), `codeql`
  (rust), and a tag-triggered `release` artifact build (30-day retention).
- The `media` feature is excluded from CI (needs system GStreamer); default features only.
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
```bash
# minimal pipeline on a PR
act pull_request -W .github/workflows/ci.yml --container-architecture linux/amd64
# full security pipeline on a main push (heavy: pulls Trivy/CodeQL images)
act push -W .github/workflows/security.yml --container-architecture linux/amd64
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
- Entrypoint is `src/main.rs` (`fn main`).
- `/target` is gitignored; `Cargo.lock` is committed.
- Toolchain: cargo/rustc 1.97.1 (MSRV).

## Agent skills

### Issue tracker

Issues live as GitHub Issues, organised on the `flo` Projects V2 board (see `docs/agents/issue-tracker.md`). The old local-markdown `.scratch/` tracker has been removed from the repo.

### Triage labels

Default five roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
