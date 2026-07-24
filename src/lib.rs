//! flo-rs — Kubernetes robot orchestration client library surface.
//! Exposes all modules so the `flo` and `flo-server` binaries can import them.

#![forbid(unsafe_code)]

pub mod auth;
pub mod cli;
pub mod codec;
pub mod common;
pub mod config;
pub mod demo;
pub mod device;
pub mod engine;
pub mod health;
pub mod production;
pub mod registration;
pub mod registry;
pub mod rules;
pub mod semantic;
pub mod server;
pub mod signaling;
pub mod transport;

#[cfg(feature = "media")]
pub mod media;
#[cfg(feature = "media")]
pub mod mesh;
#[cfg(feature = "media")]
pub mod video;
