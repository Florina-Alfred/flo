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
     - #48 Prometheus /metrics endpoint (**Done** — PR #52 merged 2026-07-19)
     - #49 Two-way WebRTC media connectivity (Todo, child of #46)
     - #50 Device discovery/access API (**Done** — PR #53 merged 2026-07-19;
       slice 1 = validation + descriptor; udev enumeration deferred)
    - #51 Dependency-policy conflict: AGENTS.md "no deps" vs reality (Needs
      admin attention — decision required before implementing #48/#49/#50
      that may need new crates).
- New tickets from `/to-tickets` are published as GitHub Issues (one per ticket,
  blocker-first) tagged `ready-for-agent` + a feature-area label, and added to
  the board via `gh issue create --project "flo"`.
- Note: GitHub Projects V2 GraphQL `items` list can return truncated/eventually
  consistent results right after many rapid mutations; verify membership by
  node ID lookup, not by the count.

## Board state (2026-07-19/20, after merges)

- #43 client-container: #48 (metrics) and #50 (device-access) closed+DONE.
- #46 webrtc-signaling: **#49 merged (#58)** — two-way *signaling/connectivity*
  delivered (MeshSignalHandler auto-answers inbound offers; latent H.264 codec
  registration bug fixed). #49 moved to DONE on the board.
- #59 created as the **media-feature** follow-up to #49 (answer-side media
  production, `on_track` render/forward, remove `signaling.rs` `#![allow(dead_code)]`).
  All three are GStreamer/`media`-feature blocked (not in default build, excluded
  from CI). #59 added to board, status Todo, child of #46.
- #52 (metrics) + #53 (device-access) + #58 (two-way WebRTC) merged to `main`.

## CI / branch-protection fix (2026-07-20)

- PRs were perpetually blocked by a *pending* `fmt` required status check. Root
  cause: branch protection required context `fmt`, but `ci.yml`'s job had
  `name: rustfmt` (only its `id` was `fmt`); GitHub keys required checks by job
  **name**, so `fmt` never reported. Fixed by setting the job `name: fmt`
  (commit `4aa7ffb` on main, cherry-picked to PR #58's branch). All 5 required
  checks now report and pass. This fix applies to all future PRs branching from
  main.

## Branch / PR hygiene

- PR #52, #53, #58 merged via squash; branches deleted by GitHub.
- CAUTION: `git checkout -b X` aborts silently if the working tree diverges from
  the assumed base — always verify the current branch before committing. A stray
  commit on the wrong branch was recovered by branching then `git reset --hard <prior-sha>`.
