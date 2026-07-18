# Ticket 04: Provision ferrous build spike for zenoh + webrtc

Label: `wayfinder:task`
Status: resolved
Blocked by: 01, 02

## Question

Task (unblocks later design): stand up a minimal build spike proving the chosen
zenoh + webrtc crates compile under ferrous with zero `unsafe` in our code.
Nothing to decide here — the decision is the crates from 01/02; this just proves
the hard constraint holds before architecture work begins.

Resolve when: a throwaway crate builds under ferrous, `cargo build` is clean, and
a grep for `unsafe` in `src/` of our code returns nothing (crate-internal unsafe
in dependencies is acceptable but must be documented). Record the build invocation
and any unsafe-in-dependency notes as the resolution.
