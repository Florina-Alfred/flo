# AGENTS.md

Single-binary Rust crate (package `flo`, edition 2024). No dependencies, no tests, no CI yet.

## Commands
- Build: `cargo build`
- Run: `cargo run`
- Check (fast, no build artifacts): `cargo check`
- Test: `cargo test` (no tests exist yet)
- Lint: `cargo clippy` / Format: `cargo fmt` (not configured; standard Rust defaults apply)
- CI: GitHub Actions (`.github/workflows/ci.yml`) runs fmt, clippy, test matrix (stable/beta/1.97.1),
  cargo-audit, cargo-deny, Trivy, and CodeQL on every push/PR; a tag `v*` push builds a release
  artifact. All jobs use `ubuntu-latest` standard runners (free on the public repo). The `media`
  feature is excluded from CI (requires system GStreamer).

## Notes
- Entrypoint is `src/main.rs` (`fn main`).
- `/target` is gitignored; `Cargo.lock` is committed.
- Toolchain: cargo/rustc 1.97.1.

## Agent skills

### Issue tracker

Issues live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
