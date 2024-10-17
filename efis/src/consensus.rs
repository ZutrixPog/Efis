use core::time;
use rand::Rng;
use std::fmt::Debug;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{self, AtomicI32, AtomicUsize};
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, time::SystemTime};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Notify, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::client::Client;
use crate::errors::ConsensusError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Follower,
    Candidate,
    Leader,
    Dead,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    pub command: String,
    pub term: usize,
}

pub struct CommitEntry {
    pub command: String,
    pub term: usize,
    pub index: usize,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
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

#[derive(Debug)]
pub struct RequestVote {
    pub term: usize,
    pub candidate_id: usize,
    pub last_log_index: usize,
    pub last_long_term: usize,
}

pub struct RequestVoteReply {
    pub term: usize,
    pub voted: bool,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppendEntries {
    pub term: usize,
    pub leader: usize,

    pub prev_log_index: i32,
    pub prev_log_term: usize,
    pub entries: Vec<LogEntry>,
    pub leader_commit: i32,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct AppendEntriesReply {
    pub term: usize,
    pub success: bool,

    pub conflict_index: i32,
    pub conflict_term: i32,
}

pub struct Consensus {
    id: usize,
    peers: Vec<usize>,
    storage: Arc<dyn Storage + Send + Sync>,
    commit_chan: mpsc::Receiver<CommitEntry>,
    new_commit_ntfy: Notify,
    shutdown_ntfy: Notify,
    ae_trigger: Notify,

    current_term: AtomicUsize,
    voted_for: AtomicI32,
    logs: RwLock<Vec<LogEntry>>,

    commit_index: AtomicI32,
    last_applied: Option<usize>,
    state: RwLock<State>,
    election_reset_event: RwLock<Option<SystemTime>>,

    next_index: HashMap<i64, i64>,
    match_index: HashMap<i64, i64>,
}

impl Consensus {
    pub async fn new(
        id: usize,
        peers: Vec<usize>,
        storage: Arc<dyn Storage + Send + Sync>,
        ready: Notify,
        commit_chan: mpsc::Receiver<CommitEntry>,
    ) -> Arc<Self> {
        let mut consensus = Consensus {
            id,
            peers,
            storage,
            current_term: AtomicUsize::new(0),
            voted_for: AtomicI32::new(-1),
            logs: RwLock::new(Vec::new()),
            commit_index: AtomicI32::new(-1),
            last_applied: None,
            state: RwLock::new(State::Follower),
            election_reset_event: RwLock::new(None),
            next_index: HashMap::new(),
            match_index: HashMap::new(),

            commit_chan,
            new_commit_ntfy: Notify::new(),
            shutdown_ntfy: Notify::new(),
            ae_trigger: Notify::new(),
        };

        consensus.restore_state().await;
        let consensus = Arc::new(consensus);
        let consensus_clone = Arc::clone(&consensus);

        // TODO: clean up
        tokio::spawn(async move {
            ready.notified().await;
            *consensus_clone.election_reset_event.write().await = Some(SystemTime::now());
            consensus_clone.run_election_timer();
        });

        consensus.start_sending_commits();
        consensus
    }

    pub async fn report(&self) -> (usize, usize, State) {
        return (
            self.id,
            self.current_term.load(SeqCst),
            *self.state.read().await,
        );
    }

    pub async fn submit(&self, cmd: String) -> bool {
        let curr_state = *self.state.read().await;
        info!("submiting a new command by {:?}", curr_state);

        if curr_state == State::Leader {
            self.logs.write().await.push(LogEntry {
                command: cmd,
                term: self.current_term.load(SeqCst),
            });
            self.persist_state();
            info!("submited successfuly");

            return true;
        }

        false
    }

    pub async fn stop(&self) {
        *self.state.write().await = State::Dead;
        self.shutdown_ntfy.notify_waiters();
    }

    async fn restore_state(&mut self) {
        if let Ok(old_state) = self.storage.restore().await {
            self.current_term.store(old_state.current_term, SeqCst);
            self.logs = RwLock::new(old_state.logs);
            self.voted_for.store(old_state.voted_for, SeqCst);
        } else {
            error!("failed to restore state from storage");
        }
    }

    async fn persist_state(&self) {
        let res = self
            .storage
            .store(PersistentState {
                current_term: self.current_term.load(SeqCst),
                voted_for: self.voted_for.load(SeqCst),
                logs: self.logs.read().await.clone(),
            })
            .await;

        if let Err(err) = res {
            error!("failed to persist node state: {}", err);
        }
    }

    pub async fn request_vote(&self, req: RequestVote) -> Result<RequestVoteReply, ConsensusError> {
        if *self.state.read().await == State::Dead {
            return Err(ConsensusError::DeadNode);
        }

        let mut reply = RequestVoteReply {
            term: 0,
            voted: false,
        };
        let last_log = self.last_log().await;
        // TODO: DELOG
        let voted_for = self.voted_for.load(SeqCst);
        let current_term = self.current_term.load(SeqCst);

        if req.term > current_term {
            debug!("outdated term in RequestForVote");
            self.become_follower(req.term).await;
        }

        if current_term == req.term
            && (voted_for == -1 || voted_for == req.candidate_id as i32)
            && (req.last_long_term > last_log.term
                || (req.last_long_term == last_log.term && req.last_log_index > last_log.index))
        {
            reply.voted = true;
            self.voted_for.store(req.candidate_id as i32, SeqCst);
            *self.election_reset_event.write().await = Some(SystemTime::now());
        } else {
            reply.voted = false;
        }

        reply.term = current_term;
        self.persist_state();
        debug!("reply to RequestForVote: {}", reply.voted);

        Ok(reply)
    }

    pub async fn append_entries(
        &self,
        req: AppendEntries,
    ) -> Result<AppendEntriesReply, ConsensusError> {
        if *self.state.read().await == State::Dead {
            return Err(ConsensusError::DeadNode);
        }

        debug!("append_entries: {:?}", req);

        let current_term = self.current_term.load(SeqCst);
        let mut reply = AppendEntriesReply {
            success: false,
            ..Default::default()
        };

        if req.term > current_term {
            debug!("outdated term in RequestForVote");
            self.become_follower(req.term).await;
        }

        if req.term == current_term {
            if *self.state.read().await != State::Follower {
                self.become_follower(req.term);
            }

            *self.election_reset_event.write().await = Some(SystemTime::now());

            let curr_logs = self.logs.read().await;
            if req.prev_log_index == -1
                || (req.prev_log_index < curr_logs.len() as i32
                    && req.prev_log_term == curr_logs[req.prev_log_index as usize].term)
            {
                reply.success = true;

                let mut insert_index = req.prev_log_index as usize + 1;
                let mut new_index = 0;

                while (insert_index < curr_logs.len() && new_index <= req.entries.len())
                    && (curr_logs[insert_index].term == req.entries[new_index].term)
                {
                    insert_index += 1;
                    new_index += 1;
                }

                if new_index < req.entries.len() {
                    debug!("inserting new entries from index {}", insert_index);
                    self.logs
                        .write()
                        .await
                        .splice(insert_index.., req.entries[new_index..].iter().cloned());
                }

                let curr_commit_index = self.commit_index.load(SeqCst);
                if req.leader_commit > curr_commit_index {
                    self.commit_index.store(
                        curr_commit_index.min(self.logs.read().await.len() as i32 - 1),
                        SeqCst,
                    );
                }

                self.new_commit_ntfy.notify_waiters();
            }
        } else {
            let logs = self.logs.read().await;
            if req.prev_log_index > logs.len() as i32 {
                reply.conflict_index = logs.len() as i32;
                reply.conflict_term = -1;
            } else {
                reply.conflict_term = logs[req.prev_log_index as usize].term as i32;

                let mut index = req.prev_log_index;
                while index >= 0 && logs[index as usize].term as i32 != reply.conflict_term {
                    index -= 1;
                }

                reply.conflict_index = index + 1;
            }
        }

        reply.term = current_term;
        self.persist_state().await;
        Ok(reply)
    }

    fn generate_timout(&self) -> time::Duration {
        time::Duration::from_millis(rand::thread_rng().gen_range(150..=300))
    }

    async fn run_election_timer(&self) {
        let tm_duration = self.generate_timout();
        let starting_term = self.current_term.load(SeqCst);
        // TODO: log

        let mut timer = interval(tm_duration);
        loop {
            timer.tick().await;
            let curr_state = *self.state.read().await;

            if curr_state != State::Candidate && curr_state != State::Follower {
                return;
            }

            let current_term = self.current_term.load(SeqCst);
            if starting_term != current_term {
                return;
            }

            if let Some(election_event) = *self.election_reset_event.read().await {
                if SystemTime::now()
                    .duration_since(election_event)
                    .unwrap_or(Duration::from_secs(0))
                    >= tm_duration
                {
                    self.start_election().await;
                    return;
                }
            }
        }
    }

    async fn start_election(&self) {
        *self.state.write().await = State::Candidate;
        let current_term = self.current_term.fetch_add(1, SeqCst);
        *self.election_reset_event.write().await = Some(SystemTime::now());
        self.voted_for.store(self.id as i32, SeqCst);

        let mut votes = 1;

        // for peer in self.peers.clone().into_iter() {
        //     tokio::spawn(async move {
        //         let last_log = self.last_log().await;
        //
        //         let args = RequestVote{
        //             term: current_term,
        //             candidate_id: self.id,
        //             last_log_index: last_log.index,
        //             last_long_term: last_log.term,
        //         };
        //
        //         info!("sending RequestVote to {}: {:?}", peer, args);
        //
        //         //let mut reply = ;
        //     });
        // }
    }

    async fn become_follower(&self, term: usize) {}

    async fn last_log(&self) -> CommitEntry {
        CommitEntry {
            index: 0,
            term: 0,
            command: String::new(),
        }
    }

    // TODO: should spawn its own coroutine
    fn start_sending_commits(&self) {}
}
