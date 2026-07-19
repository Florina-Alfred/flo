# Issue tracker: GitHub Issues + `flo` Project board

Issues and specs (you may know a spec as a PRD) for this repo live as
**GitHub Issues**, organised on the **`flo` Project board** (a Projects V2
board of type *Board*). This replaces the old local-markdown `.scratch/`
tracker, which has been removed from the repo.

## Conventions

- One **feature / effort** per GitHub Issue thread, or a small set of linked
  issues. A spec lives as the body of a parent issue (or a `docs/` file linked
  from it).
- Implementation work is broken into **one issue per ticket** (tracer-bullet
  vertical slices), published in dependency order (blockers first) so blocking
  edges can reference real issue numbers.
- Triage state is recorded via the **Project board column** + the five triage
  **labels** (see `triage-labels.md`). Do not store status in issue bodies.
- Blocking edges are recorded natively: an issue's **"Blocked by"** section
  lists the issue numbers that gate it. GitHub's issue dependency / sub-issue
  relationship is used where available; otherwise plain issue references.

## Repo labels (must exist)

The five canonical triage roles are GitHub labels: `needs-triage`,
`needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. Create them with
`gh label create` if missing.

## When a skill says "publish to the issue tracker"

Use `gh` to create a GitHub Issue (and add it to the `flo` project):

```bash
# Create an issue for a ticket
gh issue create \
  --title "<NN> — <Ticket title>" \
  --body "$(cat <<'EOF'
## What to build

<end-to-end behaviour this ticket makes work>

## Acceptance criteria

- [ ] Criterion 1
- [ ] Criterion 2

## Blocked by

- #<blocking-issue-number>, or "None — can start immediately".
EOF
)" \
  --label "ready-for-agent" \
  --project "flo"

# NOTE: `gh issue create --project "flo"` already adds the new issue to the
# `flo` board (by project title), so a separate `gh project item-add` step is
# normally unnecessary. If you must add an existing issue, use:
#   gh project item-add 2 --owner Florina-Alfred --url "<issue-url>"
# (project number 2 = the flo board; node ID in docs/agents/project-id.txt).
```

Apply the `ready-for-agent` label unless instructed otherwise — tickets are
agent-grabbable by construction. The `flo` project's number (2) and node ID
are recorded in `docs/agents/project-id.txt`.

## When a skill says "fetch the relevant ticket"

Fetch the issue via `gh`:

```bash
gh issue view <NUMBER> --json title,body,labels,comments
```

The user will normally pass the issue number or URL directly.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a GitHub **parent issue** (or a `docs/`
file); **child tickets** are linked GitHub issues.

- **Map**: a parent issue whose body holds Notes / Decisions-so-far / Fog. Or a
  `docs/superpowers/` file referenced from it.
- **Child ticket**: a GitHub issue, numbered from `01` in dependency order,
  with the question in the body. A `Type:` line records the ticket type
  (`research`/`prototype`/`grilling`/`task`); resolution is recorded by closing
  or by a `Status: resolved` comment.
- **Blocking**: a "Blocked by" section listing issue numbers. A ticket is
  unblocked when every listed issue is closed/resolved.
- **Frontier**: scan open, unblocked, unclaimed issues (by number) — first
  wins.
- **Claim**: assign the issue to yourself (or comment "claiming") before any
  work.
- **Resolve**: record the answer in a comment, then append a context pointer
  (gist + link) to the map's Decisions-so-far.
