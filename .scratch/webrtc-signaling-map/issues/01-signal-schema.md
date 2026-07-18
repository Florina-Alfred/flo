# Ticket 01: Lock WebRTC signaling message schema + flow

Label: `wayfinder:grilling`
Status: resolved
Blocked by:

## Question

Lock the signaling protocol: the exact JSON envelope for offer/answer/ICE, how a
peer discovers *which* other peer to call, and the offer/answer/ICE-trickle flow —
all over the locked zenoh mesh via `robot/<id>/signal/<peer>/{offer,answer,ice}`.

Resolve via `/grilling` + `/domain-modeling` with the human. Decide:
- **Envelope**: the JSON shape of an offer/answer (the SDP string + metadata) and
  an ICE candidate message. Keep it minimal and zenoh-friendly (small JSON).
- **Peer discovery / call initiation**: how does the offerer know a peer's id to
  target? Options: a `robot/<id>/signal/announce` presence key-expr, a zenoh query,
  or out-of-band config. Lock one.
- **Flow**: offerer publishes offer on `robot/<offerer>/signal/<answerer>/offer`;
  answerer publishes answer on `robot/<answerer>/signal/<offerer>/answer`; ICE
  trickles on the respective `…/ice` key-exprs. Confirm this bidirectional shape.
- **Ferrous**: messages are plain `serde_json`; the signaling handler is safe Rust.

This is a HITL ticket — resolves only through live exchange. Outcomes feed ticket
04 (signaling module skeleton) and the eventual implementation.
