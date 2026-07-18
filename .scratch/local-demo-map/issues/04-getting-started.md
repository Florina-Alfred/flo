# Ticket 04: GETTING_STARTED.md + --help polish

Label: `wayfinder:task`
Status: resolved
Blocked by: 01, 02

## Question

Task (onboarding): a foolproof entrypoint doc + polished CLI help so a new user
can't get stuck.

Resolve when:
- `GETTING_STARTED.md` at repo root: prerequisites (just `cargo`), the one command
  (`cargo run`), what they'll see (the rule firing), the two-terminal recipe to see
  two nodes mesh over loopback, troubleshooting (zenoh scouting/loopback, port
  conflicts), and "where to go next" (real hardware, k8s DaemonSet, webrtc video).
- `--help` / arg parsing prints the same happy path concisely (demo by default;
  flags for production). No external tooling assumed.
- Keep it short and concrete — a new user should reach the "aha" in under 2 minutes.
  This is docs only; no architecture change.
