use tokio::sync::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::consensus::{AppendEntries, AppendEntriesReply, RequestVoteReply};

// TODO: add auth
pub struct Client {
    conn: Mutex<TcpStream>,
}

impl Client {
    async fn connect(addr: String) -> Self {
        let conn = TcpStream::connect(addr).await.unwrap();

        Client {
            conn: Mutex::new(conn),
        }
    }

    // async fn append_entries(&self, req: AppendEntries) -> anyhow::Result<AppendEntriesReply> {
    //     Ok()
    // }
    //
    // async fn request_vote() -> anyhow::Result<RequestVoteReply> {}
}
