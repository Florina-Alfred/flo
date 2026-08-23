//! Command-line interface, defined with `clap` (derive).
//!
//! Replaces the previous hand-rolled argument scanner. `Args` is the top-level
//! parser for the `flo` client; `ServerArgs` is the top-level parser for
//! `flo-server` and omits client-only flags (`--ruleset`, `--video-*`).
//! `VideoArgs` is flattened into `Args` so callers keep addressing
//! `args.video.*`. The `rule` subcommand is a proper clap enum so
//! `flo rule --help` and `flo rule check --help` work.

use crate::codec::Codec;
use clap::{Args as ClapArgs, Parser, Subcommand};

/// flo - robot orchestration client.
///
/// Connects to a flo-server over Zenoh, registers, and runs the rule engine.
/// Missing or invalid config starts flo in a fail-safe state (see README).
#[derive(Parser, Debug)]
#[command(name = "flo", version, about, long_about = None)]
pub struct Args {
    /// Robot/node id (also via FLO_ROBOT_ID env).
    #[arg(long, value_name = "ID")]
    pub robot_id: Option<String>,

    /// Optional. Missing/unreadable → fail-safe empty ruleset (no motion commands); valid config with no --ruleset → built-in demo rules
    #[arg(long, value_name = "PATH")]
    pub config: Option<String>,

    /// Ruleset TOML file path (optional; uses built-in demo rules otherwise).
    #[arg(long, value_name = "PATH")]
    pub ruleset: Option<String>,

    /// Authentication mode: `mtls` (default), `ed25519` (not yet implemented — fails closed), or `none` (dev/air-gapped only; production blocks it unless --auth-allow-insecure is set).
    #[arg(long, value_name = "MODE", default_value = "mtls")]
    pub auth_mode: String,

    /// Allow `auth: none` in production (dev/air-gapped only; disables
    /// impersonation protection). Off by default.
    #[arg(long)]
    pub auth_allow_insecure: bool,

    /// Path to this node's TLS certificate (PEM) for mTLS.
    #[arg(long, value_name = "PATH")]
    pub auth_cert: Option<String>,

    /// Path to this node's TLS private key (PEM) for mTLS.
    #[arg(long, value_name = "PATH")]
    pub auth_key: Option<String>,

    /// Path to the trust anchor: CA cert (mTLS) or authorized-key allowlist
    /// (ed25519).
    #[arg(long, value_name = "PATH")]
    pub auth_trust: Option<String>,

    /// Zenoh peer endpoints to connect to (e.g. "tcp/127.0.0.1:7600").
    /// Overrides multicast scouting. May be specified multiple times.
    #[arg(long, value_name = "ENDPOINT")]
    pub connect: Vec<String>,

    /// One-shot liveness probe for container HEALTHCHECKs. Connects to the
    /// address from `FLO_HEALTH_ADDR` (default `127.0.0.1:8080`), exits 0 on a
    /// 200 from `/healthz`, 1 otherwise.
    #[arg(long)]
    pub healthcheck: bool,

    #[command(flatten)]
    pub video: VideoArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// flo-server - fleet coordinator.
///
/// Opens a Zenoh router, handles registration and heartbeats.
#[derive(Parser, Debug)]
#[command(name = "flo-server", version, about, long_about = None)]
pub struct ServerArgs {
    /// Robot/node id (also via FLO_ROBOT_ID env).
    #[arg(long, value_name = "ID")]
    pub robot_id: Option<String>,

    /// Optional. Missing/unreadable → fail-safe empty ruleset (no motion commands); valid config with no --ruleset → built-in demo rules
    #[arg(long, value_name = "PATH")]
    pub config: Option<String>,

    /// Authentication mode: `mtls` (default), `ed25519` (not yet implemented — fails closed), or `none` (dev/air-gapped only; production blocks it unless --auth-allow-insecure is set).
    #[arg(long, value_name = "MODE", default_value = "mtls")]
    pub auth_mode: String,

    /// Allow `auth: none` in production (dev/air-gapped only; disables
    /// impersonation protection). Off by default.
    #[arg(long)]
    pub auth_allow_insecure: bool,

    /// Path to this node's TLS certificate (PEM) for mTLS.
    #[arg(long, value_name = "PATH")]
    pub auth_cert: Option<String>,

    /// Path to this node's TLS private key (PEM) for mTLS.
    #[arg(long, value_name = "PATH")]
    pub auth_key: Option<String>,

    /// Path to the trust anchor: CA cert (mTLS) or authorized-key allowlist
    /// (ed25519).
    #[arg(long, value_name = "PATH")]
    pub auth_trust: Option<String>,

    /// Zenoh peer endpoints to connect to (e.g. "tcp/127.0.0.1:7600").
    /// Overrides multicast scouting. May be specified multiple times.
    #[arg(long, value_name = "ENDPOINT")]
    pub connect: Vec<String>,

    /// One-shot liveness probe for container HEALTHCHECKs. Connects to the
    /// address from `FLO_HEALTH_ADDR` (default `127.0.0.1:8080`), exits 0 on a
    /// 200 from `/healthz`, 1 otherwise.
    #[arg(long)]
    pub healthcheck: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands. Only `rule` exists today.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Validate / inspect a semantic ruleset (extended TOML) before deploy.
    Rule {
        #[command(subcommand)]
        command: RuleSubcommand,
    },
}

/// Rule subcommands.
#[derive(Subcommand, Debug)]
pub enum RuleSubcommand {
    /// Validate the ruleset at PATH (TOML or JSON).
    Check {
        /// Path to the ruleset file (TOML or JSON).
        #[arg(value_name = "PATH")]
        path: String,
        /// Output result as JSON (machine-readable).
        #[arg(long)]
        json: bool,
    },
    /// Compile the ruleset at PATH to runtime JSON.
    Compile {
        /// Path to the ruleset file (TOML or JSON).
        #[arg(value_name = "PATH")]
        path: String,
        /// Robot id to scope topics (default: 7).
        #[arg(value_name = "ROBOT_ID")]
        robot_id: Option<String>,
        /// Output result as JSON (machine-readable).
        #[arg(long)]
        json: bool,
    },
}

/// Video / WebRTC options, flattened into [`Args`].
#[derive(ClapArgs, Debug, Default)]
pub struct VideoArgs {
    /// Peer robot id to stream WebRTC video to (needs --features media + GStreamer).
    #[arg(long = "video-peer", value_name = "ID")]
    pub peer: Option<String>,

    /// Video source device path (default: synthetic test pattern).
    #[arg(long = "video-device", value_name = "PATH")]
    pub device: Option<String>,

    /// Video codec (default h264).
    #[arg(long = "video-codec", value_name = "NAME", default_value = "h264")]
    pub codec: Codec,

    /// Encode-only self-test (no peer needed). Media feature only.
    #[arg(long = "video-self-test")]
    pub self_test: bool,
}

/// Parse the process arguments. Exits with a clap usage error (including `--help`)
/// on invalid input.
pub fn parse_args() -> Args {
    Args::parse()
}

/// Parse the server process arguments. Exits with a clap usage error (including `--help`)
/// on invalid input.
pub fn parse_server_args() -> ServerArgs {
    ServerArgs::parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_args() {
        let args = Args::parse_from([
            "flo",
            "--robot-id",
            "7",
            "--video-peer",
            "8",
            "--video-device",
            "/dev/video0",
            "--video-codec",
            "h264",
        ]);
        assert_eq!(args.robot_id.as_deref(), Some("7"));
        assert_eq!(args.video.peer.as_deref(), Some("8"));
        assert_eq!(args.video.device.as_deref(), Some("/dev/video0"));
        assert_eq!(args.video.codec, Codec::H264);
    }

    #[test]
    fn defaults_to_h264_and_no_peer() {
        let args = Args::parse_from(["flo"]);
        assert_eq!(args.video.codec, Codec::H264);
        assert!(args.video.peer.is_none());
        assert!(args.robot_id.is_none());
    }

    #[test]
    fn rejects_unknown_codec() {
        // clap exits the process on an unparseable --video-codec value.
        let status = std::process::Command::new(std::env::args().next().unwrap())
            .args(["flo", "--video-codec", "vp8"])
            .status()
            .unwrap();
        assert!(!status.success());
    }
}
