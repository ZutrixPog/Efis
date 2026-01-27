import socket
import time
import random
import numpy as np

HOST = "127.0.0.1"
PORT = 3335

n = 300

def format_vector(vec):
    return "[" + ",".join(f"{x:.6f}" for x in vec) + "]"

def send_cmd(sock, cmd):
    sock.sendall((cmd + "\n").encode())
    return sock.recv(4096).decode(errors="ignore")

def main():
    sock = socket.create_connection((HOST, PORT))
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

    print("Connected")

    start = time.time()

    for i in range(n):
        cmd = f"set key=test value=sth{i}sth"
        send_cmd(sock, cmd)

        if i % 2 == 0:
            print(f"[{i}/{n}]")

    insert_time = time.time() - start

    print(f"\nInserted {n} vectors in {insert_time:.2f}s")
    print(f"Insert throughput: {n / insert_time:.2f} vec/s")

    sock.close()

if __name__ == "__main__":
    main()

