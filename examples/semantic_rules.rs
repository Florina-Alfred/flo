//! Load an extended-TOML semantic ruleset, compile it, and run the rule engine.
//! Run:  cargo run --example semantic_rules -- examples/rules/hrc-cell.toml
//! Then publish synthetic state on `fleet/<site>/<id>/state` and
//! `fleet/<site>/proximity/<id>/human` to watch rules fire.

use std::sync::Arc;

use flo_rs::config::RuleStore;
use flo_rs::engine;
use flo_rs::semantic::{compile, parse_semantic};
use flo_rs::transport::Transport;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let robot_id = "7".to_string();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/rules/hrc-cell.toml".to_string());
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
    let doc = parse_semantic(&text).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    let rules = compile(&doc, &robot_id).map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    println!("semantic_rules: compiled {} rule(s) from {path}", rules.rules.len());

    let mut transport = Transport::open_with(Transport::loopback_config())
        .await
        .map_err(|e| anyhow::anyhow!("open transport: {e}"))?;
    transport
        .declare_liveliness(&robot_id)
        .await
        .map_err(|e| anyhow::anyhow!("declare liveliness: {e}"))?;
    let transport = Arc::new(transport);

    let store = RuleStore::bootstrap(&rules.to_toml())
        .map_err(|e| anyhow::anyhow!("bootstrap: {e}"))?;

    engine::run_engine(transport, store)
        .await
        .map_err(|e| anyhow::anyhow!("run_engine: {e}"))?;
    Ok(())
}
