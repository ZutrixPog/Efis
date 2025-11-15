use core::time;
use rand::Rng;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, time::SystemTime};

use async_trait::async_trait;
use tokio::sync::{mpsc, Notify, OnceCell};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

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
    pub command: String,
    pub term: usize,
}

#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub command: String,
    pub term: usize,
    pub index: usize,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistentState {
    pub current_term: usize,
    pub voted_for: i32,
    pub logs: Vec<LogEntry>,
}

#[async_trait]
pub trait Storage {
    async fn store(&self, state: PersistentState) -> anyhow::Result<()>;
    async fn restore(&self) -> anyhow::Result<PersistentState>;
}

#[derive(Debug, SerDe)]
pub struct RequestVote {
    pub term: usize,
    pub candidate_id: usize,
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
    pub leader: usize,

    pub prev_log_index: usize,
    pub prev_log_term: usize,
    pub entries: Vec<LogEntry>,
    pub leader_commit: i32,
}

#[derive(Default, Debug, SerDe)]
pub struct AppendEntriesReply {
    pub term: usize,
    pub success: bool,

    pub conflict_index: i32,
    pub conflict_term: i32,
}

#[derive(Debug)]
enum ConsensusMsg {
    StartElection(usize, Duration),
    SendCommit,
    Submit(String),
    SendAES,
    RequestVote(RequestVote, mpsc::Sender<anyhow::Result<RequestVoteReply>>),
    AppendEntry(
        AppendEntries,
        mpsc::Sender<anyhow::Result<AppendEntriesReply>>,
    ),
}

pub struct Consensus {
    id: usize,
    peers: Vec<(usize, Arc<Client>)>,
    storage: Arc<dyn Storage + Send + Sync>,
    rx: mpsc::Receiver<ConsensusMsg>,
    tx: mpsc::Sender<ConsensusMsg>,
    shutdown_ntfy: Notify,

    current_term: usize,
    voted_for: i32,
    logs: Vec<LogEntry>,

    commit_index: i32,
    last_applied: Option<usize>,
    state: State,
    election_reset_event: Option<SystemTime>,

    next_index: HashMap<usize, usize>,
    match_index: HashMap<usize, usize>,
}

#[rpc_struct]
pub struct ConsensusRPC {
    tx: mpsc::Sender<ConsensusMsg>,
}

#[rpc_impl]
impl Consensus {
    pub async fn new(id: usize, storage: Arc<dyn Storage + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        let peers = Vec::new();
        let mut consensus = Consensus {
            id,
            peers,
            storage,
            rx,
            tx: tx.clone(),
            shutdown_ntfy: Notify::new(),

            current_term: 0,
            voted_for: -1,
            logs: Vec::new(),
            commit_index: -1,
            last_applied: None,
            state: State::Follower,
            election_reset_event: None,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
        };
        consensus.restore_state().await;
        consensus
    }

    pub async fn singleton(
        id: usize,
        storage: Arc<dyn Storage + Send + Sync>,
    ) -> (&'static mut Self, &'static ConsensusRPC) {
        static mut con: OnceCell<Consensus> = OnceCell::const_new();
        static mut rpc: OnceCell<ConsensusRPC> = OnceCell::const_new();

        unsafe {
            let c = if let Some(c) = con.get_mut() {
                c
            } else {
                let c = Self::new(id, storage).await;
                con.set(c);
                con.get_mut().unwrap()
            };
            let r = rpc
                .get_or_init(async || ConsensusRPC { tx: c.tx.clone() })
                .await;

            (c, r)
        }
    }

    pub async fn start(&mut self, peer_urls: Vec<String>, commit_chan: mpsc::Sender<CommitEntry>) {
        let mut peers = Vec::new();
        for (i, u) in peer_urls.into_iter().enumerate() {
            peers.push((i, Arc::new(Client::connect(u).await)));
        }
        self.peers = peers;

        self.election_reset_event = Some(SystemTime::now());
        self.run_election_timer().await;

        loop {
            tokio::select! {
                Some(msg) = self.rx.recv() => {
                    match msg {
                        ConsensusMsg::StartElection(starting_term, tm_duration) => {
                            let curr_state = self.state;

                            if curr_state != State::Candidate && curr_state != State::Follower {
                                return;
                            }

                            if starting_term != self.current_term {
                                return;
                            }

                            if let Some(election_event) = self.election_reset_event {
                                if SystemTime::now()
                                    .duration_since(election_event)
                                    .unwrap_or(Duration::from_secs(0))
                                    >= tm_duration
                                {
                                    self.start_election().await;
                                    return;
                                }
                            }
                        },
                        ConsensusMsg::SendCommit => {
                            self.send_commits(commit_chan.clone()).await;
                        },
                        ConsensusMsg::Submit(cmd) => {
                            self._submit(cmd).await;
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
                        }
                        _ => {
                            warn!("failed to process internal event of type: {:?}", msg);
                        }
                    }
                }
                _ = self.shutdown_ntfy.notified() => {
                    return;
                }
            }
        }
    }

    pub async fn report(&self) -> (usize, usize, State) {
        return (self.id, self.current_term, self.state);
    }

    pub async fn submit(&mut self, cmd: String) -> bool {
        let curr_state = self.state;
        info!("submiting a new command by {:?}", curr_state);

        if curr_state == State::Leader {
            let _ = self.tx.send(ConsensusMsg::Submit(cmd)).await;
            info!("submited successfuly");
            return true;
        }

        false
    }

    async fn _submit(&mut self, cmd: String) {
        self.logs.push(LogEntry {
            command: cmd,
            term: self.current_term,
        });
        self.persist_state().await;
    }

    pub fn stop(&mut self) {
        self.state = State::Dead;
        self.rx.close();
    }

    async fn restore_state(&mut self) {
        if let Ok(old_state) = self.storage.restore().await {
            self.current_term = old_state.current_term;
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
                voted_for: self.voted_for,
                logs: self.logs.clone(),
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

        let mut reply = RequestVoteReply {
            term: 0,
            voted: false,
        };
        let last_log = self.last_log().await;
        let voted_for = self.voted_for;
        let current_term = self.current_term;

        if req.term > current_term {
            debug!("outdated term in RequestForVote");
            self.become_follower(req.term).await;
        }

        if current_term == req.term
            && (voted_for == -1 || voted_for == req.candidate_id as i32)
            && (req.last_log_term > last_log.term
                || (req.last_log_term == last_log.term && req.last_log_index > last_log.index))
        {
            reply.voted = true;
            self.voted_for = req.candidate_id as i32;
            self.election_reset_event = Some(SystemTime::now());
        } else {
            reply.voted = false;
        }

        reply.term = current_term;
        self.persist_state().await;
        debug!("reply to RequestForVote: {}", reply.voted);

        Ok(reply)
    }

    async fn _append_entries(&mut self, req: AppendEntries) -> anyhow::Result<AppendEntriesReply> {
        if self.state == State::Dead {
            return Err(anyhow::format_err!("node is dead"));
        }

        debug!("append_entries: {:?}", req);

        let current_term = self.current_term;
        let mut reply = AppendEntriesReply {
            success: false,
            ..Default::default()
        };

        if req.term > current_term {
            debug!("outdated term in append_entries");
            self.become_follower(req.term).await;
        }

        if req.term == current_term {
            if self.state != State::Follower {
                self.become_follower(req.term).await;
            }

            self.election_reset_event = Some(SystemTime::now());

            if req.prev_log_index == 0
                || (req.prev_log_index < self.logs.len()
                    && req.prev_log_term == self.logs[req.prev_log_index as usize].term)
            {
                reply.success = true;

                let mut insert_index = req.prev_log_index as usize + 1;
                let mut new_index = 0;

                while (insert_index < self.logs.len() && new_index <= req.entries.len())
                    && (self.logs[insert_index].term == req.entries[new_index].term)
                {
                    insert_index += 1;
                    new_index += 1;
                }

                if new_index < req.entries.len() {
                    debug!("inserting new entries from index {}", insert_index);
                    self.logs
                        .splice(insert_index.., req.entries[new_index..].iter().cloned());
                }

                if req.leader_commit > self.commit_index {
                    self.commit_index = self.commit_index.min(self.logs.len() as i32 - 1);
                    // self.new_commit_ntfy.notify_waiters(); // NOTE
                    let _ = self.tx.send(ConsensusMsg::SendCommit).await;
                }
            }
        } else {
            if req.prev_log_index > self.logs.len() {
                reply.conflict_index = self.logs.len() as i32;
                reply.conflict_term = -1;
            } else {
                reply.conflict_term = self.logs[req.prev_log_index as usize].term as i32;

                let mut index = req.prev_log_index;
                while index >= 0 && self.logs[index as usize].term as i32 != reply.conflict_term {
                    index -= 1;
                }

                reply.conflict_index = index as i32 + 1;
            }
        }

        reply.term = current_term;
        self.persist_state().await;
        Ok(reply)
    }

    fn generate_timout(&self) -> time::Duration {
        time::Duration::from_millis(rand::thread_rng().gen_range(150..=300))
    }

    async fn run_election_timer(&mut self) {
        let tm_duration = self.generate_timout();
        let starting_term = self.current_term;
        info!(
            "election timer started {:?}, term={}",
            tm_duration, starting_term
        );

        let mut timer = interval(tm_duration);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                timer.tick().await;
                let _ = tx
                    .send(ConsensusMsg::StartElection(starting_term, tm_duration))
                    .await;
                // let curr_state = self.state;
                //
                // if curr_state != State::Candidate && curr_state != State::Follower {
                //     return;
                // }
                //
                // if starting_term != self.current_term {
                //     return;
                // }
                //
                // if let Some(election_event) = self.election_reset_event {
                //     if SystemTime::now()
                //         .duration_since(election_event)
                //         .unwrap_or(Duration::from_secs(0))
                //         >= tm_duration
                //     {
                //         // self.start_election().await;
                //         return;
                //     }
                // }
            }
        });
    }

    async fn start_election(&mut self) {
        self.state = State::Candidate;
        self.current_term += 1;
        self.election_reset_event = Some(SystemTime::now());
        self.voted_for = self.id as i32;

        let mut votes = 1;

        let candidate_id = self.id;
        let peers = self.peers.clone();
        for (peer_id, client) in peers {
            let last_log = self.last_log().await;
            let req = RequestVote {
                term: self.current_term,
                candidate_id: candidate_id,
                last_log_index: last_log.index,
                last_log_term: last_log.term,
            };

            info!("sending RequestVote to {}: {:?}", peer_id, req);

            if let Ok(reply) = client
                .call::<RequestVoteReply>("request_vote".to_string(), &req)
                .await
            {
                debug!("received request vote reply: {:?}", reply);

                if self.state == State::Candidate {
                    debug!("while waiting for reply, state = {:?}", self.state);
                    continue;
                }

                if reply.term > self.current_term {
                    debug!("term out of date in request vote reply");
                    self.become_follower(reply.term).await;
                    continue;
                } else if reply.term == self.current_term {
                    if reply.voted {
                        votes += 1;
                        if votes * 2 > self.peers.len() + 1 {
                            debug!("won election with {} votes", votes);
                            self.start_leader().await;
                        }
                    }
                }
            }
        }

        // TODO
        // tokio::task::spawn_local(async move {
        // self.run_election_timer().await;
        // });
    }

    async fn become_follower(&mut self, term: usize) {
        debug!("stepped down as Follower with term={}", term);
        self.state = State::Follower;
        self.current_term = term;
        self.voted_for = -1;
        self.election_reset_event = Some(SystemTime::now());

        // TODO
        // tokio::task::spawn_local(async move {
        // self.run_election_timer().await;
        // });
    }

    async fn start_leader(&mut self) {
        self.state = State::Leader;

        for (peer_id, _) in &self.peers {
            self.next_index.insert(*peer_id, self.logs.len());
            self.match_index.insert(*peer_id, 0);
        }
        debug!(
            "became Leader: term={}, nextIndex={:?}, matchIndex={:?}",
            self.current_term, self.next_index, self.match_index,
        );

        let heartbeat = Duration::from_millis(50);
        let _ = self.tx.send(ConsensusMsg::SendAES).await;
        let mut timer = interval(heartbeat);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                timer.tick().await;
                let _ = tx.send(ConsensusMsg::SendAES).await;
                // let mut do_send = false;
                // tokio::select! {
                //     _ = timer.tick() => {
                //         // do_send = true;
                //         timer.reset();
                //     }
                //     // NOTE
                //     _ = self.ae_trigger.notified() => {
                //         do_send = true;
                //     }
                // }
                //
                // if do_send {
                //     if self.state != State::Leader {
                //         return;
                //     }
                //
                //     self.send_leader_aes().await;
                // }
            }
        });
    }

    async fn send_leader_aes(&mut self) {
        if self.state != State::Leader {
            return;
        }
        let saved_curr_term = self.current_term;

        for (peer_id, client) in self.peers.iter() {
            let ni = *self.next_index.get(&peer_id).unwrap_or(&0);
            let prev_log_index = ni - 1;
            let mut prev_log_term = 0;
            if prev_log_index >= 0 {
                prev_log_term = self.logs[prev_log_index].term;
            }
            let entries = &self.logs[ni..];

            let req = AppendEntries {
                term: saved_curr_term,
                leader: self.id,
                prev_log_index: prev_log_index,
                prev_log_term: prev_log_term,
                entries: entries.to_vec(),
                leader_commit: self.commit_index,
            };
            debug!(
                "sending append entries to {}: ni={}, req={:?}",
                peer_id, ni, req
            );

            if let Ok(reply) = client
                .call::<AppendEntriesReply>("append_entries".to_string(), &req)
                .await
            {
                if reply.term > self.current_term {
                    debug!("term out of date in append entries reply");
                    self.become_follower(reply.term).await;
                    return;
                }

                if self.state == State::Leader && saved_curr_term == reply.term {
                    if reply.success {
                        self.next_index.insert(*peer_id, ni + entries.len());
                        self.match_index
                            .insert(*peer_id, self.next_index[&peer_id] - 1);

                        let saved_commit_index = self.commit_index as usize;
                        for i in saved_commit_index + 1..self.logs.len() {
                            if self.logs[i].term == self.commit_index as usize {
                                let mut match_count = 1;
                                for (pid, _) in &self.peers {
                                    if self.match_index[pid] >= i {
                                        match_count += 1;
                                    }
                                }

                                if match_count * 2 > self.peers.len() + 1 {
                                    self.commit_index = i as i32;
                                }
                            }
                        }

                        debug!("append_entries reply from {} success: nextIndex = {:?}, matchIndex = {:?}; commitIndex = {}", peer_id, self.next_index, self.match_index, self.commit_index);

                        let curr_commit_index = self.commit_index;
                        if curr_commit_index != saved_commit_index as i32 {
                            debug!("leader set commit_index = {}", curr_commit_index);
                            // NOTE
                            // self.new_commit_ntfy.notify_waiters();
                            // self.ae_trigger.notify_waiters();
                            let _ = self.tx.send(ConsensusMsg::SendCommit).await;
                            let _ = self.tx.send(ConsensusMsg::SendAES).await;
                        }
                    } else {
                        if reply.conflict_term >= 0 {
                            let mut last_term_index = -1;
                            for i in (0..self.logs.len()).rev() {
                                if self.logs[i].term == reply.conflict_term as usize {
                                    last_term_index = i as i32;
                                    break;
                                }
                            }
                            if last_term_index >= 0 {
                                self.next_index
                                    .insert(*peer_id, last_term_index as usize + 1);
                            } else {
                                self.next_index
                                    .insert(*peer_id, reply.conflict_index as usize);
                            }
                        } else {
                            self.next_index
                                .insert(*peer_id, reply.conflict_index as usize);
                        }

                        debug!(
                            "append_entries reply from {} !success: nextIndex := {}",
                            peer_id,
                            ni - 1
                        );
                    }
                }
            }
        }
    }

    async fn last_log(&self) -> CommitEntry {
        if self.logs.len() > 0 {
            CommitEntry {
                index: self.logs.len() - 1,
                term: self.logs.last().unwrap().term,
                command: String::new(),
            }
        } else {
            CommitEntry {
                index: 0,
                term: 0,
                command: String::new(),
            }
        }
    }

    async fn send_commits(&mut self, commit_chan_tx: mpsc::Sender<CommitEntry>) {
        // tokio::spawn(async move {
        //     loop {
        // self.new_commit_ntfy.notified().await;
        let saved_term = self.current_term;
        let saved_last_applied = self.last_applied.unwrap_or(0);

        let mut entries = Vec::new();
        if self.commit_index as usize > self.last_applied.unwrap_or(0) {
            entries.append(
                self.logs[self.last_applied.unwrap_or(0) + 1..self.commit_index as usize + 1]
                    .to_vec()
                    .as_mut(),
            );
            self.last_applied = Some(self.commit_index as usize);
        }
        debug!(
            "commit_chan_sender entries={:?}, saved_last_applied={}",
            entries, saved_last_applied
        );

        for (i, entry) in entries.into_iter().enumerate() {
            debug!("sending on commit_chan i={}, entry={:?}", i, entry);
            let _ = commit_chan_tx
                .send(CommitEntry {
                    command: entry.command,
                    index: saved_last_applied + i + 1,
                    term: saved_term,
                })
                .await;
        }
        // }
        // });
        // debug!("commit_chan_sender done")
    }
}

#[rpc_impl]
impl ConsensusRPC {
    #[rpc_func]
    pub async fn request_vote(&'static self, req: RequestVote) -> anyhow::Result<RequestVoteReply> {
        let (tx, mut rx) = mpsc::channel(32);
        let _ = self.tx.send(ConsensusMsg::RequestVote(req, tx)).await;

        if let Ok(res) =
            tokio::time::timeout(Duration::from_secs_f32(1.0), async { rx.recv().await }).await
        {
            res.unwrap()
        } else {
            Err(anyhow::format_err!("request timed out"))
        }
    }

    #[rpc_func]
    pub async fn append_entries(
        &'static self,
        req: AppendEntries,
    ) -> anyhow::Result<AppendEntriesReply> {
        let (tx, mut rx) = mpsc::channel(32);
        let _ = self.tx.send(ConsensusMsg::AppendEntry(req, tx)).await;

        if let Ok(res) =
            tokio::time::timeout(Duration::from_secs_f32(1.0), async { rx.recv().await }).await
        {
            res.unwrap()
        } else {
            Err(anyhow::format_err!("request timed out"))
        }
    }
}
