# Efis – Distributed Key-Value Store
Efis is a lightweight, educational distributed key-value store written in Rust. It now includes a full **Raft-based consensus engine**, **replicated storage**, **durable pub/sub**, and a **simple human-readable RPC protocol** designed to be easy to understand and implement.

Efis is intentionally minimal: it is **not intended for production use**, but rather as a platform to study consensus algorithms, distributed storage, serialization, and server design. Read the code, hack on it, break it, fix it — that’s the point.

## Features

### **Distributed Consensus (Raft)**

Efis now runs as a **cluster** using the Raft consensus algorithm:

* Strong consistency across nodes
* Automatic leader election
* Log replication
* Durable state machine
* Commit and apply pipeline
* Cluster can continue functioning after node failures

All writes go through the Raft log and are replicated to the cluster before becoming visible.

### **Replicated Key-Value Store**

Efis supports string, list, set, and sorted-set data types which are:

* replicated
* durable
* applied through a deterministic state machine

This means every node agrees on the exact same state.

### **Durable Pub/Sub Through Raft**

Pub/Sub is backed by the Raft log:

* Messages are committed through consensus
* Subscribers on any node see the same sequence
* Delivery is consistent cluster-wide
* Subscriptions are kept open over the same TCP connection

### **Custom Human-Readable RPC Protocol**

Efis implements a simple TCP-based RPC protocol:

* Line-based, human-readable
* Easy to test manually with `nc` or `telnet`
* Each Raft message and client command is encoded as a structured text packet
* Very easy to port to other languages (great learning exercise)

### **Automatic Persistent Backups**

Efis periodically snapshots its in-memory database to disk:

* Configurable backup interval
* Restores state on startup
* Provides crash recovery even for single-node runs

## Commands

Efis speaks a simple RPC command set over TCP:

### **Key-Value Operations**

* `set key=<key> value=<value> ttl=<ttl>`
* `get key=<key>`
* `del key=<key>`
* `increment key=<key>`
* `decrement key=<key>`
* `expire key=<key> ttl=<ttl>`
* `ttl key=<key>`

### **List Operations**

* `lpush key=<key> value=<value>`
* `rpush key=<key> value=<value>`
* `lpop key=<key>`
* `rpop key=<key>`

### **Set Operations**

* `sadd key=<key> value=<value>`
* `smembers key=<key>`

### **Sorted Set Operations**

* `zadd key=<key> score=<score> value=<value>`
* `zrange key=<key> start=<start> end=<end>`

### **Pub/Sub**

* `publish chan=<channel> value=<message>`
* `subscribe chan=<channel>`
  Subscription messages are streamed back on the same TCP connection.

### **Consensus / Cluster**

Cluster management is handled internally by Raft through Efis’ custom RPC protocol.

## How to Build

Requires Rust:

```
git clone https://github.com/zutrixpog/efis.git
cd efis
```

Using `just`:

```
just build
```

Or with Cargo:

```
cargo build --release
```

## How to Run

Start the server:

```
just run
```

Configuration is stored in the `justfile`:

* `port` — TCP listening port
* `id` — Node ID (required for cluster mode)
* `persist_path` — directory for persistent backups (required for cluster mode)
* `persist_interval` — snapshot interval in seconds (required for cluster mode)

You can connect with:

```
nc localhost 4000
```

and start issuing commands.

To run a Raft cluster, simply start multiple Efis instances with different ports and cluster configurations.

## Docker Usage

### Build the image:

```
just buildi
```

### Run the container:

```
docker run \
    --env ID=YOUR_ID \
    --env PORT=YOUR_PORT \
    --env BACKUP_PATH=/data \
    --env BACKUP_INTERVAL=60 \
    -p YOUR_PORT:YOUR_PORT \
    erfansafari/efis
```

## Contributions

Efis is an educational project — contributions are always welcome!

Ideas to explore:

* Better client libraries
* Performance optimizations
* More commands
* Advanced Raft features (snapshots, log compaction)
* Multi-partition or sharded storage
* Metrics & observability
* A vector search engine

---
