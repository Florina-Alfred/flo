# AGENTS.md

Single-binary Rust crate (package `flo`, edition 2024). No dependencies, no tests, no CI yet.

## Commands
- Build: `cargo build`
- Run: `cargo run`
- Check (fast, no build artifacts): `cargo check`
- Test: `cargo test` (no tests exist yet)
- Lint: `cargo clippy` / Format: `cargo fmt` (not configured; standard Rust defaults apply)

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
