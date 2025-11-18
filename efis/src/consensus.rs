use rand::Rng;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, time::SystemTime};

use async_trait::async_trait;
use tokio::sync::{mpsc, Notify, OnceCell};
use tokio::task::JoinHandle;
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
    pub voted_for: Option<usize>,
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
    Submit(String, mpsc::Sender<bool>),
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
    commit_chan: mpsc::Sender<CommitEntry>,
    shutdown_ntfy: Notify,

    current_term: usize,
    voted_for: Option<usize>,
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
    pub commit_chan_rx: mpsc::Receiver<CommitEntry>,
}

#[rpc_impl]
impl Consensus {
    pub async fn new(
        id: usize,
        storage: Arc<dyn Storage + Send + Sync>,
    ) -> (Self, &'static ConsensusHandle) {
        let (tx, rx) = mpsc::channel(1024);
        let (commit_chan_tx, commit_chan_rx) = mpsc::channel(1024);
        let peers = Vec::new();
        let mut consensus = Consensus {
            id,
            peers,
            storage,
            rx,
            tx: tx.clone(),
            commit_chan: commit_chan_tx,
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
            HNDL.get_or_init(async || ConsensusHandle { tx, commit_chan_rx })
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
        // start a cancellable election timer and keep handle
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
                            info!("submiting a new command by {:?}", curr_state);

                            let mut res = false;
                            if curr_state == State::Leader {
                                self._submit(cmd).await;
                                info!("submited successfuly");
                                res = true;
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
                            // ignore send errors if receiver is gone
                            let _ = tx.send(res).await;
                        },
                        ConsensusMsg::AppendEntry(req, tx) => {
                            let res = self._append_entries(req).await;
                            let _ = tx.send(res).await;
                        }
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

    pub async fn report(&self) -> (usize, usize, State) {
        (self.id, self.current_term, self.state)
    }

    async fn _submit(&mut self, cmd: String) {
        self.logs.push(LogEntry {
            command: cmd,
            term: self.current_term,
        });
        self.persist_state().await;
        let _ = self.tx.send(ConsensusMsg::SendAES).await;
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

        let last_log = self.last_log().await;

        if req.term > self.current_term {
            debug!(
                "request_vote: incoming term {} > current {}",
                req.term, self.current_term
            );
            self.become_follower(req.term).await;
        }

        let mut reply = RequestVoteReply {
            term: self.current_term,
            voted: false,
        };

        let voted_for = self.voted_for;
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
                    // find first index of that term
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
        info!(
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
        self.voted_for = Some(self.id);
        self.persist_state().await;

        let mut votes = 1usize;

        let candidate_id = self.id;
        let peers = self.peers.clone();

        for (peer_id, client) in peers {
            let last_log = self.last_log().await;
            let req = RequestVote {
                term: self.current_term,
                candidate_id,
                last_log_index: last_log.index,
                last_log_term: last_log.term,
            };

            info!("sending RequestVote to {}: {:?}", peer_id, req);

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
                    debug!("term out of date in request vote reply");
                    self.become_follower(reply.term).await;
                    continue;
                } else if reply.term == self.current_term {
                    if reply.voted {
                        votes += 1;
                        let cluster_size = self.peers.len() + 1;
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

        for (peer_id, client) in self.peers.iter() {
            let ni = *self.next_index.get(peer_id).unwrap_or(&self.logs.len());
            let ni = ni.min(self.logs.len()); // clamp
            self.next_index.insert(*peer_id, ni);

            let prev_log_index = ni.checked_sub(1);
            let prev_log_term = prev_log_index.map(|i| self.logs[i].term);
            let entries = self.logs[ni..].to_vec();

            let req = AppendEntries {
                term: saved_curr_term,
                leader: self.id,
                prev_log_index,
                prev_log_term,
                entries: entries.clone(),
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
                            .insert(*peer_id, self.next_index[&peer_id].saturating_sub(1));

                        let saved_commit_index = self.commit_index.unwrap_or(0);
                        for i in saved_commit_index + 1..self.logs.len() {
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

                        if self.commit_index.is_some()
                            && self.commit_index.unwrap() != saved_commit_index
                        {
                            debug!("leader set commit_index = {}", self.commit_index.unwrap());
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
                                self.next_index.insert(*peer_id, lti + 1);
                            } else {
                                self.next_index
                                    .insert(*peer_id, reply.conflict_index.unwrap_or(0));
                            }
                        } else {
                            self.next_index
                                .insert(*peer_id, reply.conflict_index.unwrap_or(0));
                        }

                        debug!(
                            "append_entries reply from {} !success: nextIndex updated",
                            peer_id,
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
        let saved_term = self.current_term;

        // If commit_index <= last_applied or commit_index None => nothing to do
        if let Some(commit_index) = self.commit_index {
            let last_applied = self.last_applied.unwrap_or(usize::MAX);
            let start_index = if last_applied == usize::MAX {
                0
            } else {
                last_applied + 1
            };

            if commit_index >= start_index && start_index < self.logs.len() {
                let slice = &self.logs[start_index..=commit_index];
                // send each entry
                for (i, entry) in slice.iter().enumerate() {
                    let send_idx = start_index + i;
                    let _ = commit_chan_tx
                        .send(CommitEntry {
                            command: entry.command.clone(),
                            index: send_idx,
                            term: saved_term,
                        })
                        .await;
                }
                // update last_applied
                self.last_applied = Some(commit_index);
            }
        }
    }
}

#[rpc_impl]
impl ConsensusHandle {
    pub async fn submit(&self, cmd: String) -> bool {
        let (tx, mut rx) = mpsc::channel(1);
        let _ = self.tx.send(ConsensusMsg::Submit(cmd, tx)).await;

        match tokio::time::timeout(Duration::from_secs_f32(1.0), async { rx.recv().await }).await {
            Ok(Some(res)) => res,
            _ => false,
        }
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
    use super::*;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

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

    async fn make_consensus(id: usize) -> Consensus {
        let ps = PersistentState {
            current_term: 0,
            voted_for: None,
            logs: Vec::new(),
        };
        let storage = Arc::new(MockStorage::new_with(ps));
        let c = Consensus::new(id, storage).await;
        c.0
    }

    #[tokio::test]
    async fn test_restore_state() {
        // prepare persisted state
        let persisted = PersistentState {
            current_term: 42,
            voted_for: Some(5),
            logs: vec![
                LogEntry {
                    command: "a".into(),
                    term: 1,
                },
                LogEntry {
                    command: "b".into(),
                    term: 2,
                },
            ],
        };
        let storage = Arc::new(MockStorage::new_with(persisted.clone()));
        let (c, _) = Consensus::new(1, storage.clone()).await;

        // restore_state is called in new(); verify fields were copied
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
        let mut c = make_consensus(1).await;

        // set current term and logs so that last_log is term=1, index=0
        c.current_term = 1;
        c.logs.push(LogEntry {
            command: "x".into(),
            term: 1,
        });

        // request from candidate 2 with same term and at least as up-to-date log
        let req = RequestVote {
            term: 1,
            candidate_id: 2,
            last_log_index: 0,
            last_log_term: 1,
        };

        let res = c._request_vote(req).await.expect("rpc should not error");
        println!("{:?}", res);
        assert!(res.voted, "should vote for up-to-date candidate");
        assert_eq!(
            c.voted_for,
            Some(2),
            "voted_for should be set to candidate id"
        );
    }

    #[tokio::test]
    async fn test_append_entries_accepts_and_appends_entries_and_updates_commit() {
        let mut c = make_consensus(1).await;

        // follower initially empty
        assert!(c.logs.is_empty());

        // leader sends AppendEntries with one entry and leader_commit = 0
        let req = AppendEntries {
            term: 1,
            leader: 2,
            prev_log_index: None, // leader has no previous log
            prev_log_term: None,
            entries: vec![LogEntry {
                command: "cmd1".into(),
                term: 1,
            }],
            leader_commit: Some(0),
        };

        // Ensure follower's term is lower so the accept path is exercised.
        c.current_term = 1;

        let res = c
            ._append_entries(req)
            .await
            .expect("append_entries should not error");
        assert!(res.success, "append entries should succeed");
        assert_eq!(c.logs.len(), 1, "one entry should be appended");
        assert_eq!(c.logs[0].command, "cmd1");
        assert_eq!(
            c.commit_index,
            Some(0),
            "commit_index should be set to leader_commit (0)"
        );
    }

    #[tokio::test]
    async fn test_send_commits_sends_committed_entries() {
        let mut c = make_consensus(1).await;

        // create 3 entries and set commit_index to 2
        c.logs.push(LogEntry {
            command: "a".into(),
            term: 1,
        });
        c.logs.push(LogEntry {
            command: "b".into(),
            term: 1,
        });
        c.logs.push(LogEntry {
            command: "c".into(),
            term: 1,
        });

        c.last_applied = Some(0); // already applied index 0
        c.commit_index = Some(2); // leader advanced commit to index 2
        c.current_term = 1;

        let (tx, mut rx) = mpsc::channel(4);
        c.send_commits(tx.clone()).await;

        // we expect entries for indices 1 and 2 (since last_applied was 0)
        let e1 = rx.recv().await.expect("should receive first commit");
        let e2 = rx.recv().await.expect("should receive second commit");

        assert_eq!(e1.command, "b");
        assert_eq!(e1.index, 1);
        assert_eq!(e1.term, 1);

        assert_eq!(e2.command, "c");
        assert_eq!(e2.index, 2);
        assert_eq!(e2.term, 1);

        // last_applied should now be updated to commit_index
        assert_eq!(c.last_applied, Some(2));
    }

    #[tokio::test]
    async fn test_submit_appends_log_and_persists() {
        // Prepare storage that we can inspect after submit
        let initial = PersistentState {
            current_term: 7,
            voted_for: None,
            logs: Vec::new(),
        };
        let storage = Arc::new(MockStorage::new_with(initial.clone()));
        let (mut c, _) = Consensus::new(3, storage.clone()).await;

        // become leader in order to allow submit() to push
        c.state = State::Leader;
        c.current_term = 7;

        // let ok = c.submit("mycmd".into()).await;
        // assert!(ok, "submit should return true for leader");

        // submitted asynchronously via channel to main loop; but _submit modifies logs
        // In this implementation submit sends a message into consensus.tx channel; however tests
        // call submit on this object directly - _submit is processed by the main loop in start().
        // For deterministic unit test, call _submit directly to verify behavior:
        c._submit("mycmd2".into()).await;

        // _submit pushes to logs and persists; verify logs contains the last command
        assert_eq!(c.logs.last().unwrap().command, "mycmd2");

        // persisted state in storage should have latest logs
        let persisted = storage.restore().await.expect("restore should succeed");
        assert_eq!(persisted.current_term, c.current_term);
        assert_eq!(persisted.logs, c.logs);
    }

    // Additional tests you may add:
    // - election timeout behavior and start_election leading to leader transition (requires mocking Clients)
    // - leader AppendEntries behavior and next_index/match_index handling (requires fake Client)
    // - tests for conflict handling in AppendEntries replies (requires more complete implementation)
}
