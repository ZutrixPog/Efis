use efis::rpc::client::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::subscriber;
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
    peers: Vec<String>,
}

async fn run_rpc(cfg: Config) {
    let mut backup_dur = None;
    if let Some(interval) = cfg.backup_interval {
        backup_dur = Some(Duration::from_secs(interval));
    }

    let store = DatastoreGuard::new(backup_dur, cfg.backup_path).await;
    let pubsub = PubSubGuard::new();

    // Consensus
    let con_storage = ConFileStorage::new(PathBuf::from_str("/var/efis").unwrap());
    let (commit_chan_tx, commit_chan_rx) = mpsc::channel(1024);

    let (cons, crpc) = Consensus::singleton(0, con_storage).await;
    tokio::spawn(async {
        cons.start(cfg.peers, commit_chan_tx).await;
    });

    let efis = Efis::singleton(store, pubsub);

    let rpc_server = RpcServer::new();

    rpc_server.register_struct(efis).await;
    rpc_server.register_struct(crpc).await;

    _ = rpc_server
        .run(format!("0.0.0.0:{}", cfg.port).as_str())
        .await;
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let config = envy::from_env::<Config>().expect("please provide the required information!");

    let subscriber = FmtSubscriber::new();
    subscriber::set_global_default(subscriber)?;

    run_rpc(config).await;

    Ok(())
}
