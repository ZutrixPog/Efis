import socket
import time
import random
import numpy as np
import os

HOST = "127.0.0.1"
PORT = 8080

DIM = 128
NUM_VECTORS = 10_000
NUM_QUERIES = 100
KEY = "bench"
K = 100

def format_vector(vec):
    return "[" + ",".join(f"{x:.6f}" for x in vec) + "]"

def send_cmd(sock, cmd):
    sock.sendall((cmd + "\n").encode())
    return sock.recv(4096).decode(errors="ignore")

def main():
    sock = socket.create_connection((HOST, PORT))
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    print("Connected")

    BATCH_SIZE = 1
    vectors = np.random.randn(NUM_VECTORS, DIM).astype(np.float32)
    start = time.time()
    batch_items = []

    for i, vec in enumerate(vectors):
        item = f"{{id={i} vec={format_vector(vec)}}}"
        batch_items.append(item)

        if len(batch_items) == BATCH_SIZE:
            cmd = f"vset key={KEY} vecs=[{','.join(batch_items)}]"
            send_cmd(sock, cmd)

            batch_items.clear()

            if (i + 1) % 1000 == 0:
                print(f"Inserted {i+1}/{NUM_VECTORS}")

    if batch_items:
        cmd = f"vset key={KEY} vecs=[{','.join(batch_items)}]"
        send_cmd(sock, cmd)

    insert_time = time.time() - start

    print(f"\nInserted {NUM_VECTORS} vectors in {insert_time:.2f}s")
    print(f"Insert throughput: {NUM_VECTORS / insert_time:.2f} vec/s")

    sock = socket.create_connection((HOST, PORT))
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    queries = np.random.randn(NUM_QUERIES, DIM).astype(np.float32)

    latencies = []
    for q in queries:
        cmd = f"vsearch key={KEY} query={format_vector(q)} k={K}"
        t0 = time.time()
        send_cmd(sock, cmd)
        latencies.append(time.time() - t0)

    latencies.sort()
    print("\nSearch latency:")
    print(f"  p50: {latencies[int(0.50 * len(latencies))] * 1000:.2f} ms")
    print(f"  p90: {latencies[int(0.90 * len(latencies))] * 1000:.2f} ms")
    print(f"  p99: {latencies[int(0.99 * len(latencies))] * 1000:.2f} ms")

    sock.close()

if __name__ == "__main__":
    main()

# vsearch key=bench query=[1.372723,-0.411662,-0.259478,-1.312458,0.686234,0.547786,0.336126,-2.547809,0.871068,0.212028,-0.383464,1.187449,0.178806,0.602068,-0.827266,0.633416,-0.216027,1.216404,-1.833634,-1.550864,1.958271,0.944253,0.979665,0.983205,-0.172080,-0.697683,-0.374501,-0.302798,-0.201654,-1.089395,-0.564444,0.986035,-0.184805,-0.412074,0.311826,-1.408170,0.794716,1.275839,-0.767838,2.225008,-0.666753,-0.875156,0.540436,-1.022683,0.121672,0.632513,1.137608,0.290293,-1.335612,-0.805320,1.956039,0.026505,-1.324549,0.543810,-1.456440,-1.063397,0.286818,-1.555779,0.167049,1.326148,-1.000258,0.802713,0.340890,1.658057,-0.954674,-0.746046,0.568789,0.859764,-0.900261,-1.288648,-0.511993,-1.907745,-0.373599,-1.118733,0.911070,0.918956,0.575171,1.076434,-0.408238,-0.624719,-1.592605,3.464602,0.086831,-1.938990,-0.497328,-1.005936,0.355446,0.376128,-2.298544,0.280743,0.054170,-0.023232,0.236613,2.631679,-0.805512,0.654526,0.356837,1.736890,0.630279,-0.077302,1.083304,0.851043,1.115256,1.523171,-0.233828,1.085880,1.527579,-0.690120,0.908892,0.222218,0.328416,2.284618,-0.201753,0.092093,1.685577,1.126876,-1.477836,0.299711,0.427412,1.502171,-0.501212,1.219779,0.371209,-0.662026,2.069393,-0.353533,-0.244751,-1.552696] k=100
