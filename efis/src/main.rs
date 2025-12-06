use serde::Deserialize;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tracing::{error, subscriber};
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
    id: Option<String>,
    port: String,
    persist_interval: Option<u64>,
    persist_path: Option<String>,
    peers: Option<Vec<String>>,
}

async fn run_rpc(cfg: Config) {
    let persist_interval = if let Some(interval) = cfg.persist_interval {
        Some(Duration::from_secs(interval))
    } else {
        None
    };

    let store = DatastoreGuard::new(persist_interval, cfg.persist_path.clone()).await;
    let pubsub = PubSubGuard::new();

    let rpc_server = RpcServer::new();

    let chandle = if let Some(peers) = cfg.peers {
        match (
            cfg.id.is_some(),
            cfg.persist_interval.is_some(),
            cfg.persist_path.is_some(),
        ) {
            (true, true, true) => {}
            _ => {
                error!("A unique ID, persistance interval and persistance path is required in consensus mode");
                return;
            }
        }
        let con_path = cfg.persist_path.unwrap();
        let con_storage = ConFileStorage::new(PathBuf::from_str(&con_path).unwrap());

        let (mut cons, crpc) = Consensus::new(cfg.id.unwrap(), con_storage).await;
        tokio::spawn(async move {
            cons.start(peers).await;
        });

        rpc_server.register_struct(crpc).await;
        Some(crpc)
    } else {
        None
    };

    let efis = Efis::singleton(store, pubsub, chandle);
    rpc_server.register_struct(efis).await;

    _ = rpc_server
        .run(format!("0.0.0.0:{}", cfg.port).as_str())
        .await;
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let config = envy::from_env::<Config>().expect("please provide the required information!");

    let subscriber = FmtSubscriber::builder().finish();
    subscriber::set_global_default(subscriber)?;

    run_rpc(config).await;

    Ok(())
}
