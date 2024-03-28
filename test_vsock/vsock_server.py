#!/usr/bin/env python3

import socket

CID = socket.VMADDR_CID_HOST
PORT = 1234

s = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
s.bind((CID, PORT))
s.listen()
(conn, (remote_cid, remote_port)) = s.accept()

print(f"Connection opened by cid={remote_cid} port={remote_port}")

while True:
    buf = conn.recv(64)
    if not buf:
        break

    print(f"Received bytes: {buf}")

    conn.send(b'Hello from host')