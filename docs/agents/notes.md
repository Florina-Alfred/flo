# Agent notes (project memory)

Personal/project memory for the `flo` repo agent work. Not part of the shipped
crate — operational notes only.

## GitHub Project (`flo` board, #2)

- URL: https://github.com/users/Florina-Alfred/projects/2
- Node ID: `PVT_kwHOBn8aXc4Bd1-y` (recorded in `docs/agents/project-id.txt`)
- Columns include (at least): the default Kanban columns plus a custom column
  **"Needs admin attention"**.
  - **Standing directive:** whenever an issue/ticket needs the user's (admin/
    human) attention or help, move it to the **"Needs admin attention"** column
    (not `ready-for-agent`) and tell the user about it. The user will take care
    of it. Proactively apply this when a ticket is blocked on a human decision,
    credentials, or anything outside agent scope.

## Ticket migration

- The 22 former `.scratch/` tickets were migrated to GitHub Issues #19–#40 and
  added to the `flo` board. Bodies are stubs (originals were scrubbed from disk
  + git history); blocking edges not reconstructed.
- **2026-07-19 consolidation:** a code-coverage review found most of the 22
  stubs were already implemented (rule engine, observability probes, DaemonSet,
  demo, zenoh/webrtc signaling, industrial semantic layer). The 22 stubs were
  **closed as obsolete** and **removed from the board**; replaced by 5 area
  epics:
  - #43 client-container (Todo — open: device-access, Prometheus metrics)
  - #44 local-demo (Done)
  - #45 transport-protocol (Done)
  - #46 webrtc-signaling (Todo — open: two-way connectivity)
  - #47 industrial-robotics (Done)
  - #42 CI status-discrepancy (Needs admin attention)
  - Open child tickets created 2026-07-19 under the open epics:
    - #48 Prometheus /metrics endpoint (Todo, child of #43)
    - #49 Two-way WebRTC media connectivity (Todo, child of #46)
    - #50 Device discovery/access API (Todo, child of #43)
    - #51 Dependency-policy conflict: AGENTS.md "no deps" vs reality (Needs
      admin attention — decision required before implementing #48/#49/#50
      that may need new crates).
- New tickets from `/to-tickets` are published as GitHub Issues (one per ticket,
  blocker-first) tagged `ready-for-agent` + a feature-area label, and added to
  the board via `gh issue create --project "flo"`.
- Note: GitHub Projects V2 GraphQL `items` list can return truncated/eventually
  consistent results right after many rapid mutations; verify membership by
  node ID lookup, not by the count.
