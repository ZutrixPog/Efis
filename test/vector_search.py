import socket
import time
import random
import re
import numpy as np
import os

HOST = "127.0.0.1"
PORT = 3333

DIM = 128
NUM_VECTORS = 20_000
NUM_QUERIES = 100
KEY = "bench"
K = 100

NODE_BASE_PORT = 3333


def format_vector(vec):
    return "[" + ",".join(f"{x:.6f}" for x in vec) + "]"


def parse_redirect(response):
    if 'not_leader' not in response:
        return None
    match = re.search(r'leader=(\w+)', response)
    if match:
        return match.group(1)
    return None


def connect(host, port):
    sock = socket.create_connection((host, port))
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    return sock


def send_cmd(sock, cmd, host, port):
    sock.sendall((cmd + "\n").encode())
    response = sock.recv(4096).decode(errors="ignore")
    
    leader = parse_redirect(response)
    if leader:
        leader_num = int(leader[1:])
        new_port = NODE_BASE_PORT + leader_num - 1
        print(f"Redirected to leader {leader} at {host}:{new_port}")
        sock.close()
        new_sock = connect(host, new_port)
        sock = new_sock
        sock.sendall((cmd + "\n").encode())
        response = sock.recv(4096).decode(errors="ignore")
    
    return sock, response


def main():
    print("Connected")

    BATCH_SIZE = 1
    vectors = np.random.randn(NUM_VECTORS, DIM).astype(np.float32)
    start = time.time()
    batch_items = []
    sock = None

    try:
        sock = connect(HOST, PORT)
        
        for i, vec in enumerate(vectors):
            item = f"{{id={i} vec={format_vector(vec)}}}"
            batch_items.append(item)

            if len(batch_items) == BATCH_SIZE:
                cmd = f"vset key={KEY} vecs=[{','.join(batch_items)}]"
                sock, _ = send_cmd(sock, cmd, HOST, PORT)

                batch_items.clear()

                if (i + 1) % 1000 == 0:
                    print(f"Inserted {i+1}/{NUM_VECTORS}")

        if batch_items:
            cmd = f"vset key={KEY} vecs=[{','.join(batch_items)}]"
            sock, _ = send_cmd(sock, cmd, HOST, PORT)

        insert_time = time.time() - start

        print(f"\nInserted {NUM_VECTORS} vectors in {insert_time:.2f}s")
        print(f"Insert throughput: {NUM_VECTORS / insert_time:.2f} vec/s")

        sock.close()
        
        sock = connect(HOST, PORT)

        queries = np.random.randn(NUM_QUERIES, DIM).astype(np.float32)

        latencies = []
        for q in queries:
            cmd = f"vsearch key={KEY} query={format_vector(q)} k={K}"
            t0 = time.time()
            sock, _ = send_cmd(sock, cmd, HOST, PORT)
            latencies.append(time.time() - t0)

        latencies.sort()
        print("\nSearch latency:")
        print(f"  p50: {latencies[int(0.50 * len(latencies))] * 1000:.2f} ms")
        print(f"  p90: {latencies[int(0.90 * len(latencies))] * 1000:.2f} ms")
        print(f"  p99: {latencies[int(0.99 * len(latencies))] * 1000:.2f} ms")

    finally:
        if sock:
            sock.close()


if __name__ == "__main__":
    main()
