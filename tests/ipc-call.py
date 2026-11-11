#!/usr/bin/env python3
"""Send a single IPC request to the SynthPanel core and print the response.

Usage:
    SYNTHPANEL_CORE_SOCKET=... SYNTHPANEL_IPC_TOKEN_FILE=... \
        tests/ipc-call.py sites.create '{"domain":"example.com"}'
"""
import json
import os
import socket
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: ipc-call.py <method> [json-params]", file=sys.stderr)
        return 2

    method = sys.argv[1]
    params = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}

    socket_path = os.environ.get("SYNTHPANEL_CORE_SOCKET", "/run/synthpanel/core.sock")
    token_path = os.environ.get("SYNTHPANEL_IPC_TOKEN_FILE", "/etc/synthpanel/ipc-token")
    with open(token_path, encoding="utf-8") as handle:
        token = handle.read().strip()

    request = {"v": 1, "id": 1, "method": method, "token": token, "params": params}
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.connect(socket_path)
        client.sendall((json.dumps(request) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = client.recv(65536)
            if not chunk:
                break
            data += chunk

    if not data:
        print("the core closed the connection", file=sys.stderr)
        return 1

    envelope = json.loads(data.decode())
    compact = {"separators": (",", ":")}
    if envelope.get("ok"):
        print(json.dumps(envelope.get("result"), **compact))
    else:
        print(json.dumps(envelope.get("error") or envelope, **compact))
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
