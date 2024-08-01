use std::{collections::HashMap, hash::Hash, time::SystemTime};
use tokio::sync::{mpsc, Notify};

enum State {
    Follower,
    Candidate,
    Leader,
    Dead,
}

pub struct LogEntry {
    pub command: String,
    pub term: i64,
}

pub struct CommitEntry {
    pub command: String,
    pub term: i64,
    pub index: i64,
}

struct Storage {}

struct Consensus {
    id: usize,
    peers: Vec<usize>,
    storage: Storage,
    commit_chan: mpsc::Receiver<CommitEntry>,
    new_commit_ntfy: Notify, 
    ae_trigger: Notify,

    current_term: usize,
    voted_for: Option<usize>,
    logs: Vec<LogEntry>,

    commit_index: Option<usize>,
    last_applied: Option<usize>,
    state: State,
    election_reset_event: Option<SystemTime>,

    next_index: HashMap<i64, i64>,
    match_index: HashMap<i64, i64>,
}

impl Consensus {
    pub fn new(id: usize, peers: Vec<usize>, storage: Storage) -> Self {
        let mut consensus = Consensus{
            id: id,
            peers: peers,
            storage: storage,
            current_term: 0,
            voted_for: None,
            logs: Vec::new(),
            commit_index: None,
            last_applied: None,
            state: State::Follower,
            election_reset_event: None,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
        };

        // TODD: load state from persistent storage

        consensus
    }
}

