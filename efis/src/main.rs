use core::arch;
use serde::Deserialize;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{subscriber, Level};
use tracing_subscriber::FmtSubscriber;

use efis::consensus::Consensus;
use efis::efis::Efis;
use efis::pubsub::PubSubGuard;
use efis::rpc::server::RpcServer;
use efis::storage::consensus::ConFileStorage;
use efis::store::DatastoreGuard;

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct Config {
    port: String,
    backup_interval: Option<u64>,
    backup_path: Option<String>,
    peers: Option<Vec<String>>,
}

async fn run_rpc(cfg: Config) {
    let mut backup_dur = None;
    if let Some(interval) = cfg.backup_interval {
        backup_dur = Some(Duration::from_secs(interval));
    }

    let store = DatastoreGuard::new(backup_dur, cfg.backup_path.clone()).await;
    let pubsub = PubSubGuard::new();

    // Consensus
    let con_path = cfg.backup_path.unwrap_or("/tmp/efis".to_string());
    let con_storage = ConFileStorage::new(PathBuf::from_str(&con_path).unwrap());

    let rpc_server = RpcServer::new();

    let id = (cfg.port.chars().last().unwrap() as u8 - '0' as u8) as usize;
    let mut chandle = None;
    if let Some(peers) = cfg.peers {
        let (mut cons, crpc) = Consensus::new(id, con_storage).await;
        tokio::spawn(async move {
            cons.start(peers).await;
        });

        rpc_server.register_struct(crpc).await;
        chandle = Some(crpc);
    }

    let efis = Efis::singleton(store, pubsub, chandle);
    rpc_server.register_struct(efis).await;

    _ = rpc_server
        .run(format!("0.0.0.0:{}", cfg.port).as_str())
        .await;
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let config = envy::from_env::<Config>().expect("please provide the required information!");

    let subscriber = FmtSubscriber::builder()
        // .with_max_level(Level::DEBUG)
        .finish();
    subscriber::set_global_default(subscriber)?;

    run_rpc(config).await;

    Ok(())
}
