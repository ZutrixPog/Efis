use efis::server;

use serde::Deserialize;
use std::time::Duration;
use tracing::subscriber;
use tracing_subscriber::FmtSubscriber;
use tokio::signal;
use tokio::net::TcpListener;

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct Config{
    PORT: String,
    BACKUP_INTERVAL: Option<u64>,
    BACKUP_PATH: Option<String>,
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let config = envy::from_env::<Config>().expect("please provide the required information!");

    let subscriber = FmtSubscriber::new();
    subscriber::set_global_default(subscriber)?;

    let mut backup_dur = None;
    if let Some(interval) = config.BACKUP_INTERVAL {
        backup_dur = Some(Duration::from_secs(interval));
    }

    let listener = TcpListener::bind(&format!("127.0.0.1:{}", config.PORT)).await?;
    server::run(listener, signal::ctrl_c(), backup_dur, config.BACKUP_PATH).await;

    Ok(())
}