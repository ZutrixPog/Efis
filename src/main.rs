use Efis::server;

use std::time::Duration;
use tracing::subscriber;
use tracing_subscriber::FmtSubscriber;
use tokio::signal;
use tokio::net::TcpListener;

const PORT: &str = "8080";

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let backup_dur = std::env::var("REDIS_BACKUP").map(|val| Duration::from_secs(val.parse::<u64>().unwrap())).ok();
    let subscriber = FmtSubscriber::new();
    subscriber::set_global_default(subscriber)?;

    let listener = TcpListener::bind(&format!("127.0.0.1:{}", PORT)).await?;

    server::run(listener, signal::ctrl_c(), backup_dur).await;

    Ok(())
}