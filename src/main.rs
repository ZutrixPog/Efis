use Efis::server;

use tokio::signal;
use tokio::net::TcpListener;

const PORT: &str = "8080";

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind(&format!("127.0.0.1:{}", PORT)).await?;

    println!("server started!");
    server::run(listener, signal::ctrl_c()).await;

    Ok(())
}