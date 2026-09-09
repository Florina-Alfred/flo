//! Registration — split into two deep modules per ARCH-01.
//!
//! * `lease` — owns `HashMap<ClientState>` + poison semantics, testable
//!   without Zenoh (the `if state==Registered { poison }` check is atomic
//!   under one `write()` lock).
//! * `transport` — owns typed envelopes (`RegistrationRequest`/`RegistrationResponse`)
//!   and the `fleet/registration` + `fleet/registration/response/{robot_id}`
//!   topics, with blocking `register(robot_id, config) -> Result<...>` (no
//!   retry loop). Retry/backoff is caller policy.
//!
//! This file is the seam: it re-exports the public surface so existing
//! `use crate::registration::{RegistrationServer, RegistrationError, ...}`
//! imports continue to work.

pub mod lease;
pub mod transport;

// Lease exports — the state authority.
pub use lease::{
    ClientEntry, ClientState, RegistrationError, RegistrationLease, RegistrationServer,
};

// Transport exports — the wire authority. `SampleKind::Put/Delete` and raw
// `put_bytes` stay behind this seam; callers never see `zenoh::sample::Sample`.
pub use transport::{
    RegistrationRequest, RegistrationResponse, RegistrationStatus, deregister_with_server,
    register_with_client, run_heartbeat_monitor, run_registration_handler,
};
