use std::sync::Arc;

use flo_rs::config::{RuleStore, run_hot_reload};
use flo_rs::engine;
use flo_rs::transport::{RULES_KEY, Transport};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let robot_id = "7".to_string();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/rules/sample.toml".to_string());
    let toml = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
    let store = RuleStore::bootstrap(&toml)?;
    println!("custom_rules: loaded {path}");

    let mut transport = Transport::open_with(Transport::loopback_config())
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    transport
        .declare_liveliness(&robot_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let transport = Arc::new(transport);

    {
        let t = transport.clone();
        let s = store.clone();
        let r = robot_id.clone();
        tokio::spawn(async move {
            if let Err(e) = run_hot_reload(&t, &r, s).await {
                eprintln!("hot-reload exited: {e}");
            }
        });
    }
    {
        let t = transport.clone();
        let s = store.clone();
        tokio::spawn(async move {
            if let Err(e) = engine::run_engine(
                t,
                s,
                std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            )
            .await
            {
                eprintln!("engine exited: {e}");
            }
        });
    }

    println!("custom_rules: publishing on {RULES_KEY} hot-reloads the ruleset");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
