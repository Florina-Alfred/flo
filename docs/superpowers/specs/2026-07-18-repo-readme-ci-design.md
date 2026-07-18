# Repo README, Metadata Map & GitHub Actions CI — Design

Date: 2026-07-18
Status: Approved (brainstorming)
Scope: Rename GETTING_STARTED.md -> README.md, populate crate/repo metadata, and add a
GitHub Actions CI workflow focused on lint, test, build, and security — all within the
GitHub Free tier.

## Hard constraint: GitHub Free tier (verified 2026-07-18)

- The repo `Florina-Alfred/flo` is **public** -> standard GitHub-hosted runners are
  **free and unlimited** for CI.
- Free-tier guardrails that this design obeys:
  - Use only **standard runners** (`ubuntu-latest`). NEVER "larger runners" (always billed).
  - Do NOT use self-hosted runners (post-2026-03-01 platform charge of $0.002/min).
  - Upload security artifacts but cap retention (default 90d -> set 30d) to bound storage.
  - Public repo has no minute cap, but keep jobs reasonably scoped (matrix kept small).
- Any feature considered for CI/otherwise MUST be confirmed free-tier-safe before use.
  This design uses only: ubuntu-latest standard runners, free open-source scanners
  (cargo-audit, cargo-deny, Trivy, CodeQL), and GitHub artifact storage.

## 1. File rename

- Rename `GETTING_STARTED.md` -> `README.md`.
- GitHub renders `README.md` on the repo landing page; this populates the remote view.

## 2. README content (friendly + inviting, with requirements + security sections)

Structure:
- Title + one-line pitch: Kubernetes robot orchestration in safe Rust, with a WebRTC
  class-3 video pipeline (GStreamer encode + webrtc-rs, zenoh signaling).
- CI / license / Rust MSRV badges (shields.io or GitHub-built-in).
- **Requirements**: minimum OS = Linux / macOS / Windows (standard runners);
  **Rust >= 1.97.1 (MSRV)**.
- Quick start: `cargo run` (foolproof local demo), then `cargo run -- --help`.
- WebRTC video 2-terminal recipe (from prior work):
  - terminal 1: `cargo run --features media -- --robot-id 7 --video-peer 8`
  - terminal 2: `cargo run --features media -- --robot-id 8 --video-peer 7`
  - self-test: `cargo run --features media -- --robot-id 7 --video-self-test`
- Features / architecture summary (short).
- **Security & Quality** section (informative, repo-facing):
  - `#![forbid(unsafe_code)]` — zero unsafe in our code.
  - No dependencies beyond the std ecosystem; `openh264` rejected; GStreamer gated
    behind the `media` feature so default build needs no system libs.
  - CI runs `cargo-audit`, `cargo-deny`, `Trivy` (fs vuln/secret/config), and `CodeQL`;
    SARIF/report artifacts are uploaded per run.
  - Documented convention: no magic numbers (enforced via strict clippy + review;
    native rustfmt cannot forbid them, so it is a lint/review rule, not a CI hard gate
    beyond `-D warnings`).
- Links to `docs/superpowers/specs/` for detailed designs.

## 3. Metadata map (Cargo.toml)

Add package metadata so the crate page and repo are well-populated:
- `description = "Kubernetes robot orchestration in safe Rust with a WebRTC video pipeline."`
- `license = "Apache-2.0"` (existing LICENSE is Apache-2.0).
- `repository = "https://github.com/Florina-Alfred/flo"`
- `homepage = "https://github.com/Florina-Alfred/flo"`
- `documentation = "https://github.com/Florina-Alfred/flo#readme"`
- `readme = "README.md"`
- `rust-version = "1.97.1"` (MSRV).
- `keywords = ["robotics", "kubernetes", "webrtc", "video", "rust"]`
- `categories = ["command-line-utilities", "network-programming"]`

Note: keep `edition = "2024"`. Do not add dependencies.

## 4. GitHub Actions CI (design)

File: `.github/workflows/ci.yml`.

Triggers:
- `push` (all branches)
- `pull_request` (to `main`)

Runner: `ubuntu-latest` (standard, free on public).

Concurrency:
- `group: ci-${{ github.ref }}`, `cancel-in-progress: true`.

Permissions (least privilege per job; at workflow level set `contents: read`).

Jobs:
1. **fmt** — `cargo fmt --all -- --check`. Fail on diff (strict formatting).
2. **clippy** — `cargo clippy --all-targets -- -D warnings`. Strict. Documented
   "no magic numbers" convention enforced by review + clippy; no extra hard gate.
3. **test** (matrix: `rust: [stable, beta, "1.97.1"]`) — `cargo check` and
   `cargo test --bin flo` as **parallel steps** within the job. **Default features
   only** (the `media` feature requires system GStreamer, unavailable in CI).
4. **audit** — `cargo install cargo-audit` (or use a pinned action) -> `cargo audit`.
5. **deny** — `cargo install cargo-deny` -> `cargo deny check licenses bans`.
   Ensures no copyleft/unsafe deps are introduced.
6. **trivy** — `aquasecurity/trivy-action` (pinned to SHA) fs scan:
   `scanners: vuln,secret,config`. Upload SARIF/report artifact.
7. **codeql** — `github/codeql-action` (init/analyze) with language `rust`. Upload
   SARIF artifact.
8. **release** — trigger `on: push: tags: ["v*"]` (separate or same workflow with
   `if: startsWith(github.ref, 'refs/tags/v')`): `cargo build --release`, upload the
   binary as a workflow artifact (retention 30 days). No crates.io publish (would need
   a secret token the user must set; out of scope).

Artifacts:
- Trivy report, CodeQL SARIF, audit/deny logs uploaded with `retention-days: 30`.

Safety practices:
- Pin third-party actions to commit SHAs (not floating tags).
- Set `timeout-minutes` on every job (e.g. 30).
- `permissions:` minimal at job level.

### Free-tier confirmation (per-job)
| Job | Runner | Billed? |
|-----|--------|---------|
| fmt, clippy, test, audit, deny, trivy, codeql, release | ubuntu-latest (standard) | No (public repo) |
All within free tier. No larger runners, no self-hosted, no paid services.

## 5. Out of scope
- crates.io publishing (requires secret token setup).
- Self-hosted runners.
- Coverage gating / codecov (optional, can be added later; keep minimal now).
- The `media` feature build in CI (blocked by missing system GStreamer).

## 6. Acceptance criteria
- `README.md` exists and renders on GitHub; references MSRV 1.97.1 and OS requirements.
- `Cargo.toml` has description/license/repository/readme/rust-version/keywords/categories.
- `.github/workflows/ci.yml` runs on push + PR, passes on `main`, covers fmt/clippy/
  test(matrix)/audit/deny/trivy/codeql, and a tag-triggered release artifact job.
- All jobs use only standard runners; no free-tier violations.
