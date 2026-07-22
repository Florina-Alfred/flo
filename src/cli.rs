//! Command-line interface, defined with `clap` (derive).
//!
//! Replaces the previous hand-rolled argument scanner. `Args` is the top-level
//! parser; `VideoArgs` is flattened in so callers keep addressing `args.video.*`.
//! The `rule` subcommand (`flo rule check <path>`) is captured as
//! `Command::Rule { args }` and handed to the existing `run_rule_command`.

use crate::codec::Codec;
use clap::{Args as ClapArgs, Parser, Subcommand};

/// flo - robot orchestration client.
///
/// With no arguments, runs the local demo (simulated sensors + rule engine on a
/// loopback zenoh mesh). Provide `--robot-id` / `--config` for production mode.
#[derive(Parser, Debug)]
#[command(name = "flo", version, about, long_about = None)]
pub struct Args {
    /// Robot/node id (also via FLO_ROBOT_ID env).
    #[arg(long, value_name = "ID")]
    pub robot_id: Option<String>,

    /// Rules TOML (production mode); omit for the built-in demo rules.
    #[arg(long, value_name = "PATH")]
    pub config: Option<String>,

    /// Authentication mode: `mtls` (default), `ed25519`, or `none` (dev/air-gapped
    /// only; production blocks it unless --auth-allow-insecure is set).
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

    /// Publish synthetic sensor samples (demo input).
    #[arg(long)]
    pub simulate: bool,

    /// Sensor round interval in milliseconds (default 1000; demo fires 1/s).
    #[arg(long, value_name = "N", default_value_t = 1000)]
    pub simulate_period_ms: u64,

    /// Run mode: client (default) or server (co-located router + rule engine).
    #[arg(long, value_name = "MODE", default_value_t = Mode::Client)]
    pub mode: Mode,

    #[command(flatten)]
    pub video: VideoArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Server vs client mode.
#[derive(clap::ValueEnum, Clone, Debug, Default, PartialEq)]
pub enum Mode {
    #[default]
    Client,
    Server,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Client => write!(f, "client"),
            Mode::Server => write!(f, "server"),
        }
    }
}

/// Subcommands. Only `rule` exists today.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Validate / inspect a semantic ruleset (extended TOML) before deploy.
    Rule {
        /// `check <path>` — validate the ruleset at `path`.
        #[arg(trailing_var_arg = true, num_args = 1..)]
        args: Vec<String>,
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
