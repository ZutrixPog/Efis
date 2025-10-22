use serde::Deserialize;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{mpsc, Notify};
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
}

async fn run_rpc(backup_dur: Option<Duration>, persist_path: Option<String>) {
    let store = DatastoreGuard::new(backup_dur, persist_path).await;
    let pubsub = PubSubGuard::new();

    // Consensus
    let con_storage = ConFileStorage::new(PathBuf::from_str("/var/efis").unwrap());
    let ready_ntf = Notify::new();
    let (_, commit_chan_rx) = mpsc::channel(1024);
    let cons = Consensus::new(0, vec![], con_storage, ready_ntf, commit_chan_rx).await;

    let efis = Efis::singleton(store, pubsub, cons);

    let rpc_server = RpcServer::new();

    rpc_server.register_struct(efis).await;
    // rpc_server.register_struct(cons);

    _ = rpc_server.run("0.0.0.0:8080").await;
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let config = envy::from_env::<Config>().expect("please provide the required information!");

    let subscriber = FmtSubscriber::new();
    subscriber::set_global_default(subscriber)?;

    let mut backup_dur = None;
    if let Some(interval) = config.backup_interval {
        backup_dur = Some(Duration::from_secs(interval));
    }

    run_rpc(backup_dur, config.backup_path).await;

    Ok(())
}
