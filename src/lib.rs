//! flo-rs — Kubernetes robot orchestration client library surface.
//! The `flo` binary is the primary entrypoint; this crate also exposes its
//! transport, rule engine, and simulation modules for examples and embedding.

#![forbid(unsafe_code)]

pub mod auth;
pub mod codec;
pub mod config;
pub mod engine;
pub mod rules;
pub mod semantic;
pub mod signaling;
pub mod simulate;
pub mod transport;

#[cfg(feature = "media")]
pub mod media;
#[cfg(feature = "media")]
pub mod video;
