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
    port: String,
    backup_interval: Option<u64>,
    backup_path: Option<String>,
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

    let listener = TcpListener::bind(&format!("0.0.0.0:{}", config.port)).await?;
    server::run(listener, signal::ctrl_c(), backup_dur, config.backup_path).await;

    Ok(())
}
