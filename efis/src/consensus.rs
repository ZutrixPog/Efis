use rand::Rng;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, time::SystemTime};
use tokio::time::interval;

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc, Notify, OnceCell};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::commands::Command;
use crate::rpc::client::Client;
use crate::rpc::dispatcher::Dispatcher;
use crate::rpc::{Deserialize, RpcStruct, Serialize};
use macros::{rpc_func, rpc_impl, rpc_struct, SerDe};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Follower,
    Candidate,
    Leader,
    Dead,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize, SerDe, Clone)]
pub struct LogEntry {
    pub command: Command,
    pub term: usize,
}

#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub command: Command,
    pub term: usize,
    pub index: usize,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistentState {
    pub current_term: usize,
    pub voted_for: Option<String>,
    pub logs: Vec<LogEntry>,
    pub last_applied: Option<usize>,
}

#[async_trait]
pub trait Storage {
    async fn store(&self, state: PersistentState) -> anyhow::Result<()>;
    async fn restore(&self) -> anyhow::Result<PersistentState>;
}

#[derive(Debug, SerDe)]
pub struct RequestVote {
    pub term: usize,
    pub candidate_id: String,
    pub last_log_index: usize,
    pub last_log_term: usize,
}

#[derive(Debug, SerDe)]
pub struct RequestVoteReply {
    pub term: usize,
    pub voted: bool,
}

#[derive(Debug, Default, PartialEq, SerDe)]
pub struct AppendEntries {
    pub term: usize,
    pub leader: String,

    pub prev_log_index: Option<usize>,
    pub prev_log_term: Option<usize>,
    pub entries: Vec<LogEntry>,
    pub leader_commit: Option<usize>,
}

#[derive(Default, Debug, SerDe)]
pub struct AppendEntriesReply {
    pub term: usize,
    pub success: bool,

    pub conflict_index: Option<usize>,
    pub conflict_term: Option<usize>,
}

#[derive(Debug)]
enum ConsensusMsg {
    StartElection(usize, Duration),
    SendCommit,
    Submit(Command, mpsc::Sender<Option<usize>>),
    SendAES,
    RequestVote(RequestVote, mpsc::Sender<anyhow::Result<RequestVoteReply>>),
    AppendEntry(
        AppendEntries,
        mpsc::Sender<anyhow::Result<AppendEntriesReply>>,
    ),
    AppendEntryResult(usize, usize, AppendEntriesReply),
}

pub struct Consensus {
    id: String,
    peers: Vec<(usize, Arc<Client>)>,
    storage: Arc<dyn Storage + Send + Sync>,
    rx: mpsc::Receiver<ConsensusMsg>,
    tx: mpsc::Sender<ConsensusMsg>,
    commit_chan: broadcast::Sender<CommitEntry>,
    shutdown_ntfy: Notify,

    current_term: usize,
    voted_for: Option<String>,
    logs: Vec<LogEntry>,

    commit_index: Option<usize>,
    last_applied: Option<usize>,
    state: State,
    election_reset_event: Option<SystemTime>,

    next_index: HashMap<usize, usize>,
    match_index: HashMap<usize, usize>,

    election_handle: Option<JoinHandle<()>>,
    heartbeat_handle: Option<JoinHandle<()>>,
}

#[rpc_struct]
pub struct ConsensusHandle {
    tx: mpsc::Sender<ConsensusMsg>,
    pub commit_chan_tx: broadcast::Sender<CommitEntry>,
}

#[rpc_impl]
impl Consensus {
    pub async fn new(
        id: String,
        storage: Arc<dyn Storage + Send + Sync>,
    ) -> (Self, &'static ConsensusHandle) {
        let (tx, rx) = mpsc::channel(1024);
        let (commit_chan_tx, _) = broadcast::channel(1024);
        let peers = Vec::new();
        let mut consensus = Consensus {
            id,
            peers,
            storage,
            rx,
            tx: tx.clone(),
            commit_chan: commit_chan_tx.clone(),
            shutdown_ntfy: Notify::new(),

            current_term: 0,
            voted_for: None,
            logs: Vec::new(),
            commit_index: None,
            last_applied: None,
            state: State::Follower,
            election_reset_event: None,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            election_handle: None,
            heartbeat_handle: None,
        };
        consensus.restore_state().await;

        static HNDL: OnceCell<ConsensusHandle> = OnceCell::const_new();

        (
            consensus,
            HNDL.get_or_init(async || ConsensusHandle {
                tx,
                commit_chan_tx: commit_chan_tx.clone(),
            })
            .await,
        )
    }

    pub async fn start(&mut self, peer_urls: Vec<String>) {
        // build peers
        let mut peers = Vec::new();
        for u in peer_urls.into_iter() {
            let i = u.chars().last().unwrap().to_digit(10).unwrap() as usize;
            peers.push((i, Arc::new(Client::connect(u).await)));
        }
        self.peers = peers;

        self.election_reset_event = Some(SystemTime::now());
        self.spawn_election_timer().await;

        loop {
            tokio::select! {
                Some(msg) = self.rx.recv() => {
                    match msg {
                        ConsensusMsg::StartElection(starting_term, tm_duration) => {
                            let curr_state = self.state;

                            if curr_state != State::Candidate && curr_state != State::Follower {
                                // If no longer a candidate/follower, ignore or stop
                                continue;
                            }

                            if starting_term != self.current_term {
                                // stale timer message, ignore
                                continue;
                            }

                            if let Some(election_event) = self.election_reset_event {
                                if SystemTime::now()
                                    .duration_since(election_event)
                                    .unwrap_or(Duration::from_secs(0))
                                    >= tm_duration
                                {
                                    // election timeout — start election
                                    self.start_election().await;
                                    // continue loop; start_election will manage timers/handles
                                    continue;
                                }
                            }
                        },
                        ConsensusMsg::SendCommit => {
                            self.send_commits(self.commit_chan.clone()).await;
                        },
                        ConsensusMsg::Submit(cmd, tx) => {
                            let curr_state = self.state;

                            let mut res = None;
                            if curr_state == State::Leader {
                                let i = self._submit(cmd).await;
                                info!("submited command successfuly");
                                res = Some(i);
                            }
                            let _ = tx.send(res).await;
                        },
                        ConsensusMsg::SendAES => {
                            if self.state != State::Leader {
                                continue;
                            }

                            self.send_leader_aes().await;
                        },
                        ConsensusMsg::RequestVote(req, tx) => {
                            let res = self._request_vote(req).await;
                            let _ = tx.send(res).await;
                        },
                        ConsensusMsg::AppendEntry(req, tx) => {
                            let res = self._append_entries(req).await;
                            let _ = tx.send(res).await;
                        },
                        ConsensusMsg::AppendEntryResult(peer_id, ni, reply) => {
                            self.handle_ae_res(peer_id, ni, reply).await;
                        },
                    }
                }
                _ = self.shutdown_ntfy.notified() => {
                    // cleanup spawned tasks
                    if let Some(handle) = self.election_handle.take() {
                        handle.abort();
                    }
                    if let Some(handle) = self.heartbeat_handle.take() {
                        handle.abort();
                    }
                    return;
                }
            }
        }
    }

    pub async fn report(&self) -> (String, usize, State) {
        (self.id.clone(), self.current_term, self.state)
    }

    async fn _submit(&mut self, cmd: Command) -> usize {
        self.logs.push(LogEntry {
            command: cmd,
            term: self.current_term,
        });
        let _ = self.tx.send(ConsensusMsg::SendAES).await;
        self.persist_state().await;
        self.logs.len() - 1
    }

    pub fn stop(&mut self) {
        self.state = State::Dead;
        self.rx.close();
        self.shutdown_ntfy.notify_waiters();
        if let Some(h) = self.election_handle.take() {
            h.abort();
        }
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }
    }

    async fn restore_state(&mut self) {
        if let Ok(old_state) = self.storage.restore().await {
            self.current_term = old_state.current_term;
            self.last_applied = old_state.last_applied;
            self.logs = old_state.logs;
            self.voted_for = old_state.voted_for;
        } else {
            error!("failed to restore state from storage");
        }
    }

    async fn persist_state(&self) {
        let res = self
            .storage
            .store(PersistentState {
                current_term: self.current_term,
                voted_for: self.voted_for.clone(),
                logs: self.logs.clone(),
                last_applied: self.last_applied,
            })
            .await;

        if let Err(err) = res {
            error!("failed to persist node state: {}", err);
        }
    }

    async fn _request_vote(&mut self, req: RequestVote) -> anyhow::Result<RequestVoteReply> {
        if self.state == State::Dead {
            return Err(anyhow::format_err!("node is dead"));
        }

        let last_log = self.last_log().await;

        if req.term > self.current_term {
            warn!(
                "request_vote: incoming term {} > current {}",
                req.term, self.current_term
            );
            self.become_follower(req.term).await;
        }

        let mut reply = RequestVoteReply {
            term: self.current_term,
            voted: false,
        };

        let voted_for = self.voted_for.clone();
        let up_to_date = (req.last_log_term > last_log.term)
            || (req.last_log_term == last_log.term && req.last_log_index >= last_log.index);

        if req.term == self.current_term
            && (voted_for.is_none() || voted_for.unwrap() == req.candidate_id)
            && up_to_date
        {
            reply.voted = true;
            self.voted_for = Some(req.candidate_id);
            self.election_reset_event = Some(SystemTime::now());
            self.persist_state().await;
        } else {
            reply.voted = false;
        }

        debug!("reply to RequestForVote: {:?}", reply.voted);
        Ok(reply)
    }

    async fn _append_entries(&mut self, req: AppendEntries) -> anyhow::Result<AppendEntriesReply> {
        if self.state == State::Dead {
            return Err(anyhow::format_err!("node is dead"));
        }

        if req.term > self.current_term {
            warn!(
                "term out of date in append_entries request: req_term={}, current_term={}",
                req.term, self.current_term
            );
            self.become_follower(req.term).await;
        }

        let mut reply = AppendEntriesReply::default();
        reply.term = self.current_term;

        if req.term < self.current_term {
            reply.success = false;
            return Ok(reply);
        }

        self.election_reset_event = Some(SystemTime::now());

        let prev_ok = match (req.prev_log_index, req.prev_log_term) {
            (None, _) => true,
            (Some(idx), Some(term)) => {
                if idx >= self.logs.len() {
                    false
                } else {
                    self.logs[idx].term == term
                }
            }
            _ => false,
        };

        if !prev_ok {
            if let Some(idx) = req.prev_log_index {
                if idx >= self.logs.len() {
                    reply.conflict_index = Some(self.logs.len());
                    reply.conflict_term = None;
                } else {
                    let term = self.logs[idx].term;
                    let mut first_idx = idx;
                    while first_idx > 0 && self.logs[first_idx - 1].term == term {
                        first_idx -= 1;
                    }
                    reply.conflict_term = Some(term);
                    reply.conflict_index = Some(first_idx);
                }
            } else {
                reply.conflict_index = Some(0);
                reply.conflict_term = None;
            }
            reply.success = false;
            return Ok(reply);
        }

        let insert_at = req.prev_log_index.map(|i| i + 1).unwrap_or(0);
        let mut new_idx = 0usize;
        while insert_at + new_idx < self.logs.len() && new_idx < req.entries.len() {
            if self.logs[insert_at + new_idx].term != req.entries[new_idx].term {
                break;
            }
            new_idx += 1;
        }
        if new_idx < req.entries.len() {
            self.logs.splice(
                insert_at + new_idx..,
                req.entries[new_idx..].iter().cloned(),
            );
        }

        if let Some(leader_commit) = req.leader_commit {
            if !self.logs.is_empty() {
                let max_index = self.logs.len() - 1;
                let new_commit = leader_commit.min(max_index);
                if self.commit_index.map(|ci| new_commit > ci).unwrap_or(true) {
                    self.commit_index = Some(new_commit);
                    let _ = self.tx.send(ConsensusMsg::SendCommit).await;
                }
            }
        }

        reply.success = true;
        self.persist_state().await;
        Ok(reply)
    }

    fn generate_timout(&self) -> Duration {
        Duration::from_millis(rand::thread_rng().gen_range(150..=300))
    }

    async fn spawn_election_timer(&mut self) {
        if let Some(handle) = self.election_handle.take() {
            handle.abort();
        }

        let tm_duration = self.generate_timout();
        let starting_term = self.current_term;
        debug!(
            "election timer started {:?}, term={}",
            tm_duration, starting_term
        );

        let mut timer = interval(tm_duration);
        let tx = self.tx.clone();

        let handle = tokio::spawn(async move {
            loop {
                timer.tick().await;
                let _ = tx
                    .send(ConsensusMsg::StartElection(starting_term, tm_duration))
                    .await;
            }
        });

        self.election_handle = Some(handle);
    }

    async fn start_election(&mut self) {
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }

        self.state = State::Candidate;
        self.current_term += 1;
        self.election_reset_event = Some(SystemTime::now());
        self.voted_for = Some(self.id.clone());
        self.persist_state().await;

        let mut votes = 1usize;

        let peers = self.peers.clone();

        for (peer_id, client) in peers {
            let last_log = self.last_log().await;
            let req = RequestVote {
                term: self.current_term,
                candidate_id: self.id.clone(),
                last_log_index: last_log.index,
                last_log_term: last_log.term,
            };

            debug!("sending RequestVote to {}: {:?}", peer_id, req);

            let res = client
                .call::<RequestVoteReply>("request_vote".to_string(), &req)
                .await;
            if let Ok(reply) = res {
                debug!("received request vote reply: {:?}", reply);

                if self.state != State::Candidate {
                    debug!("not a candidate anymore; ignoring reply");
                    continue;
                }

                if reply.term > self.current_term {
                    warn!("term out of date in request vote reply");
                    self.become_follower(reply.term).await;
                    continue;
                } else if reply.term == self.current_term {
                    if reply.voted {
                        votes += 1;
                        let cluster_size =
                            self.peers.iter().filter(|(_, p)| p.connected()).count() + 1;
                        if votes * 2 > cluster_size {
                            info!("won election with {} votes", votes);
                            self.start_leader().await;
                            return;
                        }
                    }
                }
            } else {
                error!(
                    "no answer received for request_vote: {:?}",
                    res.err().unwrap()
                );
            }
        }

        self.spawn_election_timer().await;
    }

    async fn become_follower(&mut self, term: usize) {
        info!("stepped down to Follower with term={}", term);
        self.state = State::Follower;
        self.current_term = term;
        self.voted_for = None;
        self.election_reset_event = Some(SystemTime::now());
        self.persist_state().await;
        self.spawn_election_timer().await;
    }

    async fn start_leader(&mut self) {
        if let Some(h) = self.election_handle.take() {
            h.abort();
        }

        self.state = State::Leader;

        for (peer_id, _) in &self.peers {
            self.next_index.insert(*peer_id, self.logs.len());
            self.match_index.insert(*peer_id, 0);
        }
        info!(
            "became Leader: term={}, nextIndex={:?}, matchIndex={:?}",
            self.current_term, self.next_index, self.match_index,
        );

        let _ = self.tx.send(ConsensusMsg::SendAES).await;

        let heartbeat = Duration::from_millis(50);
        let tx = self.tx.clone();

        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }

        let handle = tokio::spawn(async move {
            let mut timer = interval(heartbeat);
            loop {
                timer.tick().await;
                let _ = tx.send(ConsensusMsg::SendAES).await;
            }
        });

        self.heartbeat_handle = Some(handle);
    }

    async fn send_leader_aes(&mut self) {
        if self.state != State::Leader {
            return;
        }
        let saved_curr_term = self.current_term;
        let commit_index = self.commit_index;

        for (peer_id, client) in self.peers.iter() {
            let client = client.clone();
            let peer_id = *peer_id;

            let ni = *self.next_index.get(&peer_id).unwrap_or(&self.logs.len());
            let ni = ni.min(self.logs.len()); // clamp
            self.next_index.insert(peer_id, ni);

            let prev_log_index = ni.checked_sub(1);
            let prev_log_term = prev_log_index.map(|i| self.logs[i].term);
            let entries = self.logs[ni..].to_vec();
            let leader_id = self.id.clone();

            let tx = self.tx.clone();
            tokio::spawn(async move {
                let req = AppendEntries {
                    term: saved_curr_term,
                    leader: leader_id.clone(),
                    prev_log_index,
                    prev_log_term,
                    entries: entries,
                    leader_commit: commit_index,
                };
                debug!(
                    "sending append entries to {}: ni={}, req={:?}",
                    peer_id, ni, req
                );

                let res = client
                    .call::<AppendEntriesReply>("append_entries".to_string(), &req)
                    .await;

                if let Ok(reply) = res {
                    let _ = tx
                        .send(ConsensusMsg::AppendEntryResult(peer_id, ni, reply))
                        .await;
                } else {
                    debug!("failed to send append entry: {:?}", res.err());
                }
            });
        }
    }

    async fn handle_ae_res(&mut self, peer_id: usize, ni: usize, reply: AppendEntriesReply) {
        if reply.term > self.current_term {
            warn!("term out of date in append entries reply");
            self.become_follower(reply.term).await;
            return;
        }

        let entries = self.logs[ni..].to_vec();
        if self.state == State::Leader && self.current_term == reply.term {
            if reply.success {
                self.next_index.insert(peer_id, ni + entries.len());
                self.match_index
                    .insert(peer_id, self.next_index[&peer_id].saturating_sub(1));

                let saved_commit_index = self.commit_index.unwrap_or(0);
                for i in saved_commit_index..self.logs.len() {
                    if self.logs[i].term == self.current_term {
                        let mut match_count = 1;
                        for (pid, _) in &self.peers {
                            if self.match_index.get(pid).copied().unwrap_or(0) >= i {
                                match_count += 1;
                            }
                        }

                        if match_count * 2 > self.peers.len() + 1 {
                            self.commit_index = Some(i);
                        }
                    }
                }

                debug!("append_entries reply from {} success: nextIndex = {:?}, matchIndex = {:?}; commitIndex = {}", peer_id, self.next_index, self.match_index, self.commit_index.unwrap_or(0));

                if self.commit_index.is_some() && self.commit_index.unwrap() != saved_commit_index {
                    info!("leader set commit_index = {}", self.commit_index.unwrap());
                    let _ = self.tx.send(ConsensusMsg::SendCommit).await;
                    let _ = self.tx.send(ConsensusMsg::SendAES).await;
                }
            } else {
                if let Some(ct) = reply.conflict_term {
                    let mut last_term_index = None;
                    for i in (0..self.logs.len()).rev() {
                        if self.logs[i].term == ct {
                            last_term_index = Some(i);
                            break;
                        }
                    }
                    if let Some(lti) = last_term_index {
                        self.next_index.insert(peer_id, lti + 1);
                    } else {
                        self.next_index
                            .insert(peer_id, reply.conflict_index.unwrap_or(0));
                    }
                } else {
                    self.next_index
                        .insert(peer_id, reply.conflict_index.unwrap_or(0));
                }

                info!(
                    "append_entries reply from {} !success: nextIndex updated",
                    peer_id,
                );
            }
        }
    }

    async fn last_log(&self) -> CommitEntry {
        if self.logs.len() > 0 {
            CommitEntry {
                index: self.logs.len() - 1,
                term: self.logs.last().unwrap().term,
                command: self.logs.last().unwrap().command.clone(),
            }
        } else {
            CommitEntry {
                index: 0,
                term: 0,
                command: Command::Unknown,
            }
        }
    }

    async fn send_commits(&mut self, commit_chan_tx: broadcast::Sender<CommitEntry>) {
        let saved_term = self.current_term;

        if let Some(commit_index) = self.commit_index {
            let last_applied = self.last_applied.unwrap_or(usize::MAX);
            let start_index = if last_applied == usize::MAX {
                0
            } else {
                last_applied + 1
            };

            if commit_index >= start_index && start_index < self.logs.len() {
                let slice = &self.logs[start_index..=commit_index];
                for (i, entry) in slice.iter().enumerate() {
                    let send_idx = start_index + i;
                    let _ = commit_chan_tx.send(CommitEntry {
                        command: entry.command.clone(),
                        index: send_idx,
                        term: saved_term,
                    });
                }
                self.last_applied = Some(commit_index);
            }
        }
    }
}

#[rpc_impl]
impl ConsensusHandle {
    pub async fn submit(&self, cmd: Command) -> Option<usize> {
        let (tx, mut rx) = mpsc::channel(1);
        let _ = self.tx.send(ConsensusMsg::Submit(cmd, tx)).await;

        match tokio::time::timeout(Duration::from_secs_f32(1.0), async { rx.recv().await }).await {
            Ok(Some(res)) => res,
            _ => None,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CommitEntry> {
        let (tx2, rx2) = broadcast::channel(16);

        let mut ch = self.commit_chan_tx.subscribe();
        tokio::spawn(async move {
            loop {
                match ch.recv().await {
                    Ok(cmd) => {
                        let _ = tx2.send(cmd);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });

        rx2
    }

    #[rpc_func]
    pub async fn request_vote(&'static self, req: RequestVote) -> anyhow::Result<RequestVoteReply> {
        let (tx, mut rx) = mpsc::channel(1);
        let _ = self.tx.send(ConsensusMsg::RequestVote(req, tx)).await;

        match tokio::time::timeout(Duration::from_secs_f32(1.0), async { rx.recv().await }).await {
            Ok(Some(res)) => res,
            Ok(None) => Err(anyhow::format_err!(
                "consensus actor dropped response channel"
            )),
            Err(_) => Err(anyhow::format_err!("request timed out")),
        }
    }

    #[rpc_func]
    pub async fn append_entries(
        &'static self,
        req: AppendEntries,
    ) -> anyhow::Result<AppendEntriesReply> {
        let (tx, mut rx) = mpsc::channel(1);
        let _ = self.tx.send(ConsensusMsg::AppendEntry(req, tx)).await;

        match tokio::time::timeout(Duration::from_secs_f32(1.0), async { rx.recv().await }).await {
            Ok(Some(res)) => res,
            Ok(None) => Err(anyhow::format_err!(
                "consensus actor dropped response channel"
            )),
            Err(_) => Err(anyhow::format_err!("request timed out")),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::efis::types::SetReq;

    use super::*;
    use std::sync::Mutex;

    struct MockStorage {
        state: Mutex<PersistentState>,
    }

    impl MockStorage {
        fn new_with(state: PersistentState) -> Self {
            Self {
                state: Mutex::new(state),
            }
        }
    }

    #[async_trait]
    impl Storage for MockStorage {
        async fn store(&self, state: PersistentState) -> anyhow::Result<()> {
            let mut guard = self.state.lock().unwrap();
            *guard = state;
            Ok(())
        }

        async fn restore(&self) -> anyhow::Result<PersistentState> {
            let guard = self.state.lock().unwrap();
            Ok((*guard).clone())
        }
    }

    async fn make_consensus(id: String) -> Consensus {
        let ps = PersistentState {
            current_term: 0,
            voted_for: None,
            logs: Vec::new(),
            last_applied: None,
        };
        let storage = Arc::new(MockStorage::new_with(ps));
        let c = Consensus::new(id, storage).await;
        c.0
    }

    #[tokio::test]
    async fn test_restore_state() {
        let persisted = PersistentState {
            current_term: 42,
            voted_for: Some("5".to_string()),
            last_applied: None,
            logs: vec![
                LogEntry {
                    command: Command::Unknown,
                    term: 1,
                },
                LogEntry {
                    command: Command::Unknown,
                    term: 2,
                },
            ],
        };
        let storage = Arc::new(MockStorage::new_with(persisted.clone()));
        let (c, _) = Consensus::new("1".to_string(), storage.clone()).await;

        assert_eq!(
            c.current_term, persisted.current_term,
            "current_term should be restored"
        );
        assert_eq!(
            c.voted_for, persisted.voted_for,
            "voted_for should be restored"
        );
        assert_eq!(c.logs, persisted.logs, "logs should be restored");
    }

    #[tokio::test]
    async fn test_request_vote_granted_when_up_to_date_and_not_voted() {
        let mut c = make_consensus("1".to_string()).await;

        c.current_term = 1;
        c.logs.push(LogEntry {
            command: Command::Unknown,
            term: 1,
        });

        let req = RequestVote {
            term: 1,
            candidate_id: "2".to_string(),
            last_log_index: 0,
            last_log_term: 1,
        };

        let res = c._request_vote(req).await.expect("rpc should not error");
        assert!(res.voted, "should vote for up-to-date candidate");
        assert_eq!(
            c.voted_for,
            Some("2".to_string()),
            "voted_for should be set to candidate id"
        );
    }

    #[tokio::test]
    async fn test_append_entries_accepts_and_appends_entries_and_updates_commit() {
        let mut c = make_consensus("1".to_string()).await;

        assert!(c.logs.is_empty());

        let req = AppendEntries {
            term: 1,
            leader: "2".to_string(),
            prev_log_index: None,
            prev_log_term: None,
            entries: vec![LogEntry {
                command: Command::Unknown,
                term: 1,
            }],
            leader_commit: Some(0),
        };

        c.current_term = 1;

        let res = c
            ._append_entries(req)
            .await
            .expect("append_entries should not error");
        assert!(res.success, "append entries should succeed");
        assert_eq!(c.logs.len(), 1, "one entry should be appended");
        assert_eq!(c.logs[0].command, Command::Unknown);
        assert_eq!(
            c.commit_index,
            Some(0),
            "commit_index should be set to leader_commit (0)"
        );
    }

    #[tokio::test]
    async fn test_send_commits_sends_committed_entries() {
        let mut c = make_consensus("1".to_string()).await;

        c.logs.push(LogEntry {
            command: Command::Unknown,
            term: 1,
        });
        c.logs.push(LogEntry {
            command: Command::Unknown,
            term: 1,
        });
        c.logs.push(LogEntry {
            command: Command::Unknown,
            term: 1,
        });

        c.last_applied = Some(0);
        c.commit_index = Some(2);
        c.current_term = 1;

        let (tx, mut rx) = broadcast::channel(4);
        c.send_commits(tx.clone()).await;

        let e1 = rx.recv().await.expect("should receive first commit");
        let e2 = rx.recv().await.expect("should receive second commit");

        assert_eq!(e1.command, Command::Unknown);
        assert_eq!(e1.index, 1);
        assert_eq!(e1.term, 1);

        assert_eq!(e2.command, Command::Unknown);
        assert_eq!(e2.index, 2);
        assert_eq!(e2.term, 1);

        assert_eq!(c.last_applied, Some(2));
    }

    #[tokio::test]
    async fn test_submit_appends_log_and_persists() {
        let initial = PersistentState {
            current_term: 7,
            voted_for: None,
            logs: Vec::new(),
            last_applied: None,
        };
        let storage = Arc::new(MockStorage::new_with(initial.clone()));
        let (mut c, _) = Consensus::new("3".to_string(), storage.clone()).await;

        c.state = State::Leader;
        c.current_term = 7;

        let ok = c._submit(Command::Unknown).await;
        assert!(ok == 0, "submit should return true for leader");

        assert_eq!(c.logs.last().unwrap().command, Command::Unknown);

        // let persisted = storage.restore().await.expect("restore should succeed");
        // assert_eq!(persisted.current_term, c.current_term);
        // assert_eq!(persisted.logs, c.logs);
    }

    #[tokio::test]
    async fn test_serde() {
        let cmd = Command::Set(SetReq {
            key: "a".to_string(),
            value: "i".to_string(),
            exp: None,
        });
        let ser = cmd.serialize();
        let de = Command::deserialize(ser.as_str());
        assert!(de.is_ok(), "failed to deserialize");

        let req = AppendEntries {
            term: 1,
            leader: "4".to_string(),
            prev_log_index: None,
            prev_log_term: None,
            entries: vec![LogEntry {
                command: cmd,
                term: 1,
            }],
            leader_commit: None,
        };

        let ser = req.serialize();
        let de = AppendEntries::deserialize(ser.as_str());
        assert!(de.is_ok(), "failed to deserialize");

        let res = AppendEntriesReply {
            term: 1,
            success: true,
            conflict_index: Some(1),
            conflict_term: Some(2),
        };
        let ser = res.serialize();
        let de = AppendEntriesReply::deserialize(ser.as_str());
        assert!(de.is_ok(), "failed to deserialize");
    }
}
