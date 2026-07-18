# Ticket 03: Lock Zenoh topology (hybrid vs P2P vs router backbone)

Label: `wayfinder:grilling`
Status: resolved
Blocked by: 01

## Question

Lock the Zenoh deployment topology for class 1 & 2 traffic. User's stated leaning
is hybrid (local P2P mesh on the node; routers only for cross-cluster/edge), but
it is explicitly open.

Resolve via `/grilling` + `/domain-modeling` with the human. Use ticket 01's
research findings (zenoh topology trade-offs, session-multiplexing) as the
concrete artifact to react to. Outcome: a single locked topology decision and,
if needed, the session-multiplexing answer (single session vs one per QoS class).

This is a HITL ticket — it only resolves through live exchange with the human;
the agent must not answer its own questions.
