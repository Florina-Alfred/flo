# Ticket 04: Signaling module skeleton (zenoh key-expr wiring + message structs)

Label: `wayfinder:task`
Status: resolved
Blocked by: 01

## Question

Task (unblocks implementation): author a `signaling` module skeleton that wires the
locked zenoh signal key-exprs and defines the offer/answer/ICE message structs, per
ticket 01. No live media yet — just the signaling handshake over zenoh.

Resolve when: a `src/signaling.rs` (or equivalent) exists with —
- The key-expr builders `robot/<id>/signal/<peer>/{offer,answer,ice}` (reusing the
  `flo` zenoh session from the transport module).
- `serde` structs for Offer/Answer/IceCandidate (JSON).
- A handler that subscribes to inbound signal key-exprs, correlates by peer, and
  exposes callbacks for offer-received / answer-received / ice-received.
- Honors ferrous: `#![forbid(unsafe_code)]`, safe Rust only.
This is a skeleton for review; live WebRTC peer connection wiring is later work.
