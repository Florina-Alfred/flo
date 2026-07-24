# Agent notes (project memory)

Personal/project memory for the `flo` repo agent work. Not part of the shipped
crate — operational notes only.

## GitHub Project (`flo` board, #2)

- URL: https://github.com/users/Florina-Alfred/projects/2
- Node ID: `PVT_kwHOBn8aXc4Bd1-y` (recorded in `docs/agents/project-id.txt`)
- Columns include a custom column **"Needs admin attention"**.
  - **Standing directive:** whenever an issue/ticket needs the user's (admin/
    human) attention or help, move it to the **"Needs admin attention"** column
    (not `ready-for-agent`) and tell the user about it.
- Note: GitHub Projects V2 GraphQL `items` list can return truncated/eventually
  consistent results right after many rapid mutations; verify membership by
  node ID lookup, not by the count.

## Historical (resolved — kept for context)

- The old local `.scratch/` tracker was removed from the repo; all issues live
  as GitHub Issues on the `flo` board.
- The `main.rs` entrypoint was split into `src/bin/flo-client.rs` and
  `src/bin/flo-server.rs` (see `docs/agents/issue-tracker.md`).
