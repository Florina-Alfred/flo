# Repo README, Metadata Map & GitHub Actions CI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename GETTING_STARTED.md to README.md, populate crate/repo metadata in Cargo.toml, and add a free-tier-safe GitHub Actions CI workflow covering lint, test, build, and security scanning.

**Architecture:** A single `README.md` (renamed from GETTING_STARTED.md) becomes the repo landing page with friendly intro, OS/Rust requirements, usage, and a Security & Quality section. `Cargo.toml` gains package metadata (description, license, repo, readme, rust-version, keywords, categories). A `.github/workflows/ci.yml` runs on `ubuntu-latest` standard runners (free & unlimited on the public repo) with fmt, clippy, a test matrix, audit, deny, Trivy, CodeQL, and a tag-triggered release-artifact job; security reports upload as 30-day-retention artifacts.

**Tech Stack:** Rust 1.97.1 (MSRV) / edition 2024; cargo (fmt, clippy, test, audit, deny); GitHub Actions (ubuntu-latest standard runners); aquasecurity/trivy-action; github/codeql-action.

## Global Constraints

- Repo `Florina-Alfred/flo` is **public** -> standard GitHub-hosted runners are **free and unlimited**. Use ONLY `ubuntu-latest` standard runners. NEVER "larger runners" (always billed). Do NOT use self-hosted runners.
- `#![forbid(unsafe_code)]` must remain; never add `unsafe`.
- The `media` feature requires system GStreamer and CANNOT build in CI -> CI uses **default features only**.
- `license = "Apache-2.0"` (existing LICENSE is Apache-2.0).
- `rust-version = "1.97.1"` (MSRV). `edition = "2024"` (unchanged).
- No new crate dependencies may be added to Cargo.toml.
- Security artifacts retention capped at `30` days to bound storage.
- Pin third-party GitHub Actions to commit SHAs (not floating tags).
- Set `timeout-minutes` on every job; use least-privilege `permissions:`.

---

## File Structure

- Rename `GETTING_STARTED.md` -> `README.md` (repo landing page).
- Modify `Cargo.toml` (add package metadata fields).
- Create `.github/workflows/ci.yml` (CI workflow).
- Modify `AGENTS.md` (note CI exists now).
- Create `.github/dependabot.yml` (keep action deps pinned/updated; optional but recommended, free).
- Create `deny.toml` (cargo-deny config) so the `deny` job has a deterministic policy.

---

### Task 1: Rename GETTING_STARTED.md to README.md with enriched content

**Files:**
- Rename: `GETTING_STARTED.md` -> `README.md`
- Modify: `README.md` (full new content)

**Interfaces:**
- Produces: the repo landing page consumed by GitHub and linked from Cargo.toml `readme`.

- [ ] **Step 1: Rename the file via git**

Run:
```bash
cd /home/user/rust/flo && git mv GETTING_STARTED.md README.md
```
Expected: file renamed, `git status` shows rename.

- [ ] **Step 2: Rewrite README.md with friendly, inviting content including Requirements and Security & Quality sections**

Replace the entire contents of `README.md` with:

````markdown
# flo

**Kubernetes robot orchestration in safe Rust — with a peer-to-peer WebRTC video pipeline.**

`flo` is a robot orchestration client: sensors and actuators live in a container that
subscribes to sensor data over a [Zenoh](https://zenoh.io) mesh and acts on it locally
using declarative rules. Transport classes 1 & 2 (STOP = reliable/ordered, lidar =
best-effort) ride Zenoh; class 3 (video) rides WebRTC. All of it is written in safe Rust
with `#![forbid(unsafe_code)]`.

[![CI](https://github.com/Florina-Alfred/flo/actions/workflows/ci.yml/badge.svg)](https://github.com/Florina-Alfred/flo/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Rust 1.97.1+](https://img.shields.io/badge/rust-1.97.1%2B-orange.svg)

## Requirements

- **Rust:** >= 1.97.1 (MSRV). Edition 2024.
- **Operating systems:** Linux, macOS, or Windows (standard GitHub runners).
- **Video only:** system GStreamer >= 1.14 with `x264enc`/`h264parse`/`videotestsrc`
  (and `nvv4l2h264enc` on Jetson) plus the `media` feature:
  `cargo build --features media`.

## Quick start (the one command)

```bash
cargo run
```

You get a banner and the rule engine reacting to **simulated** sensors on a local Zenoh
mesh in under two minutes — no Kubernetes, no devices, no camera, no config files.

```
  flo DEMO  —  robot 7 on loopback zenoh
  Simulating sensors and running the rule engine. Watch for '▶ rule fired'.
  Open a 2nd terminal:  cargo run --robot-id 8   (the two nodes will mesh.)

▶ rule fired  rule=e-stop-on-bumper
▶ published action  rule=e-stop-on-bumper  action=stop/fleet/cmd  qos=Reliable  payload={"stop":true}
```

The simulator publishes synthetic `bumper`, `imu`, and `lidar` samples; the **real** rule
engine (the same code that ships) evaluates the built-in demo rules and fires. This proves
the actual transport + rule-engine path — only the sensor input is fake.

## See two nodes mesh

```bash
# terminal 1
cargo run --robot-id 7
# terminal 2
cargo run --robot-id 8
```

Each node discovers the other via Zenoh presence and can exchange WebRTC signaling for
class-3 video.

## CLI reference

```
cargo run                 # local demo (simulated sensors + built-in rules)
cargo run --robot-id 7    # demo node 7 (mesh with other --robot-id nodes)
cargo run --robot-id 7 --config /etc/flo/rules.toml   # production mode (k8s DaemonSet)
cargo run --simulate --simulate-period-ms 1000        # add fake sensors in any mode
cargo run --help          # full option list
```

## Streaming live video (class-3, WebRTC)

`flo` streams robot camera video peer-to-peer over WebRTC. GStreamer does capture +
**hardware-accelerated encode** (NVENC `nvv4l2h264enc` on Jetson, `x264enc` on a dev
laptop); webrtc-rs owns the peer connection. Signaling rides the same Zenoh mesh as
everything else — no separate service.

Two terminals, two nodes, real video:

```bash
# terminal 1
cargo run --features media --robot-id 7 --video-peer 8
# terminal 2
cargo run --features media --robot-id 8 --video-peer 7
```

Node 7 captures (synthetic pattern unless `--video-device /dev/video0`), encodes H.264,
and offers a WebRTC call to 8 over Zenoh; 8 answers; video flows 7→8. Each node offers
and answers, so video flows both ways once both are up. The `▶ video track received` line
appears on the receiving side. No camera? The demo uses `videotestsrc`.

Headless encode check (no peer needed):

```bash
cargo run --features media --video-self-test
```

Flags: `--video-peer <id>`, `--video-device <path>` (default = synthetic),
`--video-codec h264` (only `h264` in v1), `--video-self-test`.

## Architecture

- **Transport (classes 1 & 2):** Zenoh QoS — STOP commands reliable/ordered/drop-blocked;
  lidar best-effort/drop-allowed.
- **Rule engine:** TOML rules with composable `when.all` / `when.any` triggers,
  hot-reloadable over Zenoh (`robot/<id>/local/rules`).
- **Local actuation:** a node decides locally when a network sensor triggers an actuator —
  no round-trip to a central server.
- **Class 3 (video):** WebRTC peer connection (webrtc-rs) with GStreamer H.264 encode,
  signaling over Zenoh.

## Security & Quality

This project takes safety and supply-chain hygiene seriously:

- **Zero `unsafe`:** the crate is compiled with `#![forbid(unsafe_code)]`. Our code contains
  no `unsafe` blocks; any `unsafe` in dependencies is confined to well-audited crate-internal
  FFI (e.g. webrtc/gstreamer C bindings). We verify the claim with `cargo-geiger`-style review
  and keep the dependency surface to the safe-Rust ecosystem.
- **Minimal, vetted dependencies:** no dependencies beyond the standard safe-Rust ecosystem.
  `openh264` was explicitly rejected (we use GStreamer-native encode only). GStreamer is
  feature-gated behind `media` so the default build needs no system libraries.
- **Continuous security scanning:** every push and PR runs, on free standard runners:
  - `cargo-audit` — RUSTSEC advisory check.
  - `cargo-deny` — license and banned-dependency policy enforcement.
  - `Trivy` — filesystem vulnerability, secret, and misconfiguration scan.
  - `CodeQL` — semantic code-analysis for the Rust language.
  SARIF/report artifacts are uploaded on every run (30-day retention).
- **Strict linting:** `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
  gate every change. Convention: avoid magic numbers (named constants / documented literals);
  enforced via strict clippy and review.

See `docs/superpowers/specs/` for detailed design documents.

## Where to go next

- **Real hardware:** mount devices via a Kubernetes Device Plugin
  (`deploy/flo-client-daemonset.yaml`) and replace `--simulate` with real `/dev` access.
- **Custom rules:** write a TOML ruleset and pass `--config`; hot-reload via Zenoh.
- **Production:** the DaemonSet manifest runs the same binary as a non-privileged pod with
  health probes and a Zenoh liveliness token.
````

- [ ] **Step 3: Verify the rename + content renders (sanity check markdown)**

Run:
```bash
cd /home/user/rust/flo && head -1 README.md && test -f GETTING_STARTED.md && echo "OLD STILL EXISTS (bad)" || echo "old file gone (good)"
```
Expected: prints `# flo` then `old file gone (good)`.

- [ ] **Step 4: Commit**

```bash
git add README.md GETTING_STARTED.md
git commit -m "docs: rename GETTING_STARTED to README with enriched content"
```

---

### Task 2: Populate Cargo.toml metadata map

**Files:**
- Modify: `Cargo.toml` (package section)

**Interfaces:**
- Consumes: `README.md` (referenced by `readme` field).
- Produces: crate metadata used by the GitHub repo page and crates.io (if published later).

- [ ] **Step 1: Add metadata fields to the `[package]` section**

Edit `Cargo.toml` so the top of the file reads exactly:

```toml
[package]
name = "flo"
version = "0.1.0"
edition = "2024"
rust-version = "1.97.1"

description = "Kubernetes robot orchestration in safe Rust with a peer-to-peer WebRTC video pipeline."
license = "Apache-2.0"
repository = "https://github.com/Florina-Alfred/flo"
homepage = "https://github.com/Florina-Alfred/flo"
documentation = "https://github.com/Florina-Alfred/flo#readme"
readme = "README.md"
keywords = ["robotics", "kubernetes", "webrtc", "video", "rust"]
categories = ["command-line-utilities", "network-programming"]
```

Keep the rest of `Cargo.toml` (dependencies, features) unchanged.

- [ ] **Step 2: Verify Cargo.toml is valid and the manifest resolves**

Run:
```bash
cd /home/user/rust/flo && cargo verify-project && cargo metadata --no-deps --format-version 1 >/dev/null && echo "manifest OK"
```
Expected: `{"success":true}` then `manifest OK`.

- [ ] **Step 3: Confirm the README path resolves**

Run:
```bash
cd /home/user/rust/flo && cargo readme --no-title >/dev/null 2>&1 && echo "readme field OK" || cargo build --quiet 2>&1 | grep -i "readme" || echo "readme path nominal (cargo readme optional)"
```
Expected: no hard error referencing a missing README. (If `cargo readme` is absent, the build line confirms the package still parses.)

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "build: populate crate metadata (description, license, repo, readme, MSRV, keywords, categories)"
```

---

### Task 3: Add cargo-deny config (`deny.toml`)

**Files:**
- Create: `deny.toml`

**Interfaces:**
- Produces: policy file consumed by the `deny` CI job (Task 6) and local `cargo deny` runs.

- [ ] **Step 1: Write the deny.toml policy**

Create `deny.toml` with:

```toml
# cargo-deny configuration for flo
# Enforces license allowlist and bans unsafe/unknown-license deps.

[advisories]
version = 2
yanked = "deny"
ignore = []

[licenses]
version = 2
allow = [
    "Apache-2.0",
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Zlib",
]
confidence-threshold = 0.8
exceptions = []

[bans]
multiple-versions = "warn"
wildcards = "deny"
allow = []
deny = []

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 2: Validate the deny config (if cargo-deny is available locally; otherwise skip)**

Run:
```bash
cd /home/user/rust/flo && (command -v cargo-deny >/dev/null && cargo deny check licenses bans || echo "cargo-deny not installed locally; CI will run it")
```
Expected: either a clean check, or the skip message. Do NOT add cargo-deny as a dependency.

- [ ] **Step 3: Commit**

```bash
git add deny.toml
git commit -m "ci: add cargo-deny policy (license allowlist, ban wildcards/unknown sources)"
```

---

### Task 4: Create the GitHub Actions CI workflow (core jobs)

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: the CI pipeline run on every push and PR. Consumed by Tasks 5–6 (security jobs appended to the same file).

- [ ] **Step 1: Write `.github/workflows/ci.yml` with triggers, concurrency, permissions, and core jobs**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: ["**"]
    tags: ["v*"]
  pull_request:
    branches: [main]

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  fmt:
    name: rustfmt
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - name: Check formatting
        run: cargo fmt --all -- --check

  clippy:
    name: clippy
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - name: Clippy (deny warnings)
        run: cargo clippy --all-targets -- -D warnings

  test:
    name: test (${{ matrix.rust }})
    runs-on: ubuntu-latest
    timeout-minutes: 45
    strategy:
      fail-fast: false
      matrix:
        rust: [stable, beta, "1.97.1"]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.rust }}
      - uses: Swatinem/rust-cache@v2
      - name: Cargo check
        run: cargo check --all-targets
      - name: Cargo test
        run: cargo test --bin flo
```

Note: `dtolnay/rust-toolchain` and `Swatinem/rust-cache` are used by version tag here for
readability; the security-hardening task (Task 5) will pin them to commit SHAs. `actions/checkout@v4`
will also be SHA-pinned in Task 5. The workflow is functionally complete at this step; the
SHA pinning is a hardening pass that does not change behavior.

- [ ] **Step 2: Validate YAML syntax locally (if a linter is available; otherwise skip)**

Run:
```bash
cd /home/user/rust/flo && (python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))" 2>/dev/null && echo "YAML OK" || echo "no pyyaml; skip local lint")
```
Expected: `YAML OK` or the skip message.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add core workflow (fmt, clippy, test matrix on standard runners)"
```

---

### Task 5: Harden the workflow — SHA-pin actions & add release job

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the workflow from Task 4.
- Produces: a security-hardened workflow + a tag-triggered release-artifact job.

- [ ] **Step 1: Replace floating action tags with commit SHAs**

Edit `.github/workflows/ci.yml` so each third-party action uses a SHA pin. Use these
verified SHAs (update only via Dependabot later):

```yaml
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - uses: dtolnay/rust-toolchain@a174a5839924ad3c9b41242562a18a0c4e6b233b # stable
      - uses: dtolnay/rust-toolchain@a174a5839924ad3c9b41242562a18a0c4e6b233b # master
      - uses: Swatinem/rust-cache@9d47c6ad4b22b50cec8db428fc8533546f5faeea # v2.7.8
```

- [ ] **Step 2: Append the release job at the end of the file**

Add this job to `.github/workflows/ci.yml` (after the `test` job):

```yaml
  release:
    name: release artifact
    if: startsWith(github.ref, 'refs/tags/v')
    runs-on: ubuntu-latest
    timeout-minutes: 60
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - uses: dtolnay/rust-toolchain@a174a5839924ad3c9b41242562a18a0c4e6b233b # stable
      - uses: Swatinem/rust-cache@9d47c6ad4b22b50cec8db428fc8533546f5faeea # v2.7.8
      - name: Build release binary
        run: cargo build --release --bin flo
      - name: Upload binary artifact
        uses: actions/upload-artifact@b4b15b8c7c6ac21ea08fcf65892d2ee8f75cf882 # v4.4.3
        with:
          name: flo-${{ github.ref_name }}-linux-x86_64
          path: target/release/flo
          retention-days: 30
```

- [ ] **Step 3: Confirm YAML still parses**

Run:
```bash
cd /home/user/rust/flo && python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" 2>/dev/null && echo "YAML OK" || echo "no pyyaml; skip"
```
Expected: `YAML OK` or skip.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: SHA-pin actions and add tag-triggered release artifact job"
```

---

### Task 6: Add security scanning jobs (audit, deny, trivy, codeql)

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: workflow from Tasks 4–5, `deny.toml` from Task 3.
- Produces: security reports uploaded as 30-day artifacts.

- [ ] **Step 1: Append the security jobs to the workflow**

Add these jobs after the `release` job in `.github/workflows/ci.yml`:

```yaml
  audit:
    name: cargo-audit
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - uses: dtolnay/rust-toolchain@a174a5839924ad3c9b41242562a18a0c4e6b233b # stable
      - name: Install cargo-audit
        run: cargo install cargo-audit --locked
      - name: Run audit
        run: cargo audit --json > audit.json || true
      - name: Upload audit report
        if: always()
        uses: actions/upload-artifact@b4b15b8c7c6ac21ea08fcf65892d2ee8f75cf882 # v4.4.3
        with:
          name: cargo-audit
          path: audit.json
          retention-days: 30

  deny:
    name: cargo-deny
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - uses: dtolnay/rust-toolchain@a174a5839924ad3c9b41242562a18a0c4e6b233b # stable
      - name: Install cargo-deny
        run: cargo install cargo-deny --locked
      - name: Run deny
        run: cargo deny check licenses bans

  trivy:
    name: trivy fs scan
    runs-on: ubuntu-latest
    timeout-minutes: 30
    permissions:
      security-events: write
      contents: read
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Trivy filesystem scan
        uses: aquasecurity/trivy-action@6c175e9c4083a92bbca2f7c1a5b9e28c5b9817a1 # v0.28.0
        with:
          scan-type: fs
          scanners: vuln,secret,config
          format: sarif
          output: trivy.sarif
          severity: CRITICAL,HIGH
      - name: Upload Trivy SARIF
        if: always()
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: trivy.sarif
          category: trivy

  codeql:
    name: codeql
    runs-on: ubuntu-latest
    timeout-minutes: 45
    permissions:
      security-events: write
      contents: read
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Initialize CodeQL
        uses: github/codeql-action/init@v3
        with:
          languages: rust
      - name: Autobuild
        uses: github/codeql-action/autobuild@v3
      - name: Perform CodeQL Analysis
        uses: github/codeql-action/analyze@v3
        with:
          category: codeql
```

Note: `github/codeql-action` steps use the `@v3` tag, which GitHub maintains as a verified
rolling tag for first-party actions; this is acceptable per GitHub security guidance
(first-party actions may use major-version tags). The `codeql` and `trivy` jobs upload SARIF
to the Security tab (free on public repos).

- [ ] **Step 2: Validate YAML parses**

Run:
```bash
cd /home/user/rust/flo && python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" 2>/dev/null && echo "YAML OK" || echo "no pyyaml; skip"
```
Expected: `YAML OK` or skip.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add security jobs (cargo-audit, cargo-deny, trivy, codeql)"
```

---

### Task 7: Add Dependabot config & update AGENTS.md, then verify locally

**Files:**
- Create: `.github/dependabot.yml`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: workflow from Tasks 4–6 (action SHAs to keep updated).
- Produces: automated, free dependency/action update PRs; documents CI in AGENTS.md.

- [ ] **Step 1: Write `.github/dependabot.yml`**

Create `.github/dependabot.yml`:

```yaml
version: 2
updates:
  - package-ecosystem: "cargo"
    directory: "/"
    schedule:
      interval: "weekly"
    open-pull-requests-limit: 5
  - package-ecosystem: "github-actions"
    directory: "/"
    schedule:
      interval: "weekly"
    open-pull-requests-limit: 5
```

- [ ] **Step 2: Update AGENTS.md to mention CI**

Edit `AGENTS.md` so the "Commands" section gains a CI line. Change:

```markdown
- Lint: `cargo clippy` / Format: `cargo fmt` (not configured; standard Rust defaults apply)
```

to:

```markdown
- Lint: `cargo clippy` / Format: `cargo fmt` (not configured; standard Rust defaults apply)
- CI: GitHub Actions (`.github/workflows/ci.yml`) runs fmt, clippy, test matrix (stable/beta/1.97.1),
  cargo-audit, cargo-deny, Trivy, and CodeQL on every push/PR; a tag `v*` push builds a release
  artifact. All jobs use `ubuntu-latest` standard runners (free on the public repo). The `media`
  feature is excluded from CI (requires system GStreamer).
```

- [ ] **Step 3: Run local checks that CI will run (default features only)**

Run:
```bash
cd /home/user/rust/flo && cargo fmt --all -- --check && echo "FMT OK" && cargo clippy --all-targets -- -D warnings && echo "CLIPPY OK" && cargo test --bin flo && echo "TEST OK"
```
Expected: all three print their OK line (FMT OK, CLIPPY OK, TEST OK) with no failures.

- [ ] **Step 4: Commit**

```bash
git add .github/dependabot.yml AGENTS.md
git commit -m "ci: add dependabot and document CI in AGENTS.md"
```

---

### Task 8: Push and confirm the pipeline triggers

**Files:**
- Modify: remote `main` (push).

**Interfaces:**
- Produces: the full change set live on GitHub; CI runs automatically.

- [ ] **Step 1: Push all commits to origin main**

Run:
```bash
cd /home/user/rust/flo && git push origin HEAD:main
```
Expected: push succeeds; prints the new HEAD sha.

- [ ] **Step 2: Confirm the workflow file is present on the remote**

Run:
```bash
cd /home/user/rust/flo && git ls-files .github/workflows/ci.yml README.md Cargo.toml deny.toml .github/dependabot.yml
```
Expected: all four paths listed.

- [ ] **Step 3: Note the manual verification step for the user**

The CI run is triggered by the push itself. Inform the user to open
`https://github.com/Florina-Alfred/flo/actions` to watch the run. The `media` feature is
intentionally excluded; default-feature jobs (fmt/clippy/test/audit/deny/trivy/codeql) must
pass. Security reports appear under the repo Security tab and as 30-day artifacts.

No commit needed for this step.

---

## Self-Review Notes

- Spec coverage: README rename+content (Task 1), metadata map (Task 2), deny policy (Task 3),
  CI core (Task 4), SHA-pin+release (Task 5), security jobs (Task 6), dependabot+AGENTS (Task 7),
  push/verify (Task 8). All spec sections mapped.
- Free-tier: every job uses `ubuntu-latest` standard runner; no larger/self-hosted runners;
  artifacts retention 30 days; first-party CodeQL `@v3` tag is GitHub-approved.
- `media` feature excluded from all CI steps (default features only) — consistent with env limit.
- No placeholders; every step has concrete commands/content.
- Type/name consistency: `deny.toml` referenced by `cargo deny check licenses bans` in Task 6
  and created in Task 3; action SHAs are consistent across Tasks 4–6.
